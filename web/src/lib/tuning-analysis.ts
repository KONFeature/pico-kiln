// PID tuning analysis — a faithful TypeScript port of scripts/analyzer/
// (data.py, thermal.py, pid.py, reporting.py) + analyze_tuning.py's
// select_recommended_method. Turns a tuning-run CSV into a fitted thermal model
// and suggested PID parameters (Ziegler-Nichols / Cohen-Coon / AMIGO).
//
// Verified against the reference Python's own output in tuning-analysis.test.ts.
// Keep this in sync with the Python if the reference algorithm changes.

import type { KilnConfig } from "./pico/types";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type PhaseType = "heating" | "cooling" | "plateau";
export type GainMethod = "plateau" | "heating" | "fallback";
export type HeatLossMethod = "plateau" | "cooling" | "fallback";
export type GainConfidence = "HIGH" | "MEDIUM" | "LOW";
export type TestQuality = "EXCELLENT" | "GOOD" | "POOR";
export type PidMethodKey = "zieglerNichols" | "cohenCoon" | "amigo";

export interface TuningData {
	time: number[]; // seconds since start, from the timestamp column
	temp: number[]; // °C
	ssr: number[]; // % output
	hasStepData: boolean;
	stepNames?: string[];
	stepIndices?: number[];
}

export interface Phase {
	startIdx: number;
	endIdx: number;
	phaseType: PhaseType;
	avgSsr: number;
	tempStart: number;
	tempEnd: number;
	stepName?: string;
}

export interface GainPoint {
	temp: number;
	gain: number;
	ssr: number;
}

export interface ThermalModel {
	deadTimeS: number;
	timeConstantS: number;
	steadyStateGain: number; // base gain K, °C per % SSR
	heatLossCoeff: number; // h in gain_scale(T) = 1 + h*(T - T_ambient)
	ambientTemp: number;
	gainVsTemp: GainPoint[];
	gainMethod: GainMethod;
	gainConfidence: GainConfidence;
	heatLossMethod: HeatLossMethod;
}

export interface PidMethod {
	kp: number;
	ki: number;
	kd: number;
	method: string;
	characteristics: string;
}

export interface TuningAnalysis {
	testInfo: {
		durationS: number;
		dataPoints: number;
		tempMin: number;
		tempMax: number;
		phasesDetected: number;
	};
	phases: Phase[];
	thermalModel: ThermalModel;
	pidMethods: Record<PidMethodKey, PidMethod>;
	recommended: PidMethodKey;
	testQuality: TestQuality;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Round like Python's round(): operate on the true double (no lossy `*10^n`
 * pre-scale) so e.g. round(32.45, 1) → 32.5, matching the reference output.
 * toFixed rounds half-up on the true value; Python rounds half-to-even, but
 * exact .5 ties never arise from these computed floats — so they agree here.
 * ponytail: half-up vs half-to-even differs only on exact ties; revisit if a
 * golden value ever lands on one.
 */
export function pyRound(x: number, ndigits = 0): number {
	if (!Number.isFinite(x)) return x;
	return Number(x.toFixed(ndigits));
}

function mean(xs: number[]): number {
	return xs.reduce((a, b) => a + b, 0) / xs.length;
}

function median(xs: number[]): number {
	const s = [...xs].sort((a, b) => a - b);
	return s[Math.floor(s.length / 2)]; // matches Python's len//2 index
}

/** Parse "YYYY-MM-DD HH:MM:SS" to epoch seconds as UTC (DST-immune, matching
 *  Python's naive datetime subtraction). */
function parseTimestamp(ts: string): number {
	const [date, clock] = ts.trim().split(" ");
	const [y, mo, d] = date.split("-").map(Number);
	const [h, mi, s] = (clock ?? "0:0:0").split(":").map(Number);
	return Date.UTC(y, mo - 1, d, h, mi, s) / 1000;
}

// ---------------------------------------------------------------------------
// Data loading (port of analyzer/data.py load_tuning_data)
// ---------------------------------------------------------------------------

export function loadTuningData(content: string): TuningData {
	const lines = content.trim().split("\n");
	if (lines.length < 2) throw new Error("Empty or headerless CSV");

	const headers = lines[0].split(",").map((h) => h.trim());
	const col = (name: string) => headers.indexOf(name);
	const iTemp = col("current_temp_c");
	const iSsr = col("ssr_output_percent");
	const iTs = col("timestamp");
	const iState = col("state");
	const iStepName = col("step_name");
	const iStepIdx = col("step_index");
	const iTotal = col("total_steps");
	if (iTemp < 0 || iSsr < 0 || iTs < 0) {
		throw new Error(
			"CSV missing required columns (timestamp, current_temp_c, ssr_output_percent)",
		);
	}
	const hasStepData = iStepName >= 0 && iStepIdx >= 0 && iTotal >= 0;

	const temp: number[] = [];
	const ssr: number[] = [];
	const timestamps: string[] = [];
	const stepNames: string[] = [];
	const stepIndices: number[] = [];

	for (let i = 1; i < lines.length; i++) {
		const line = lines[i].trim();
		if (!line) continue;
		const v = line.split(",");
		if (iState >= 0 && v[iState]?.trim() === "RECOVERY") continue; // skip recovery

		temp.push(parseFloat(v[iTemp]));
		ssr.push(parseFloat(v[iSsr]));
		timestamps.push(v[iTs].trim());
		if (hasStepData) {
			stepNames.push(v[iStepName]?.trim() ?? "");
			stepIndices.push(parseInt(v[iStepIdx], 10));
		}
	}

	if (timestamps.length === 0) throw new Error("No data rows in CSV");

	// Overall elapsed time from timestamps (per-step elapsed_seconds is unreliable)
	const start = parseTimestamp(timestamps[0]);
	const time = timestamps.map((ts) => parseTimestamp(ts) - start);

	const data: TuningData = { time, temp, ssr, hasStepData };
	if (hasStepData) {
		data.stepNames = stepNames;
		data.stepIndices = stepIndices;
	}
	return data;
}

// ---------------------------------------------------------------------------
// Phase detection (port of analyzer/data.py detect_phases)
// ---------------------------------------------------------------------------

export function detectPhases(
	data: TuningData,
	plateauThreshold = 0.5,
	ssrChangeThreshold = 10.0,
): Phase[] {
	const phases: Phase[] = [];
	const { time, temp, ssr } = data;
	if (time.length < 10) return phases;

	let currentSsr = ssr[0];
	let phaseStart = 0;

	for (let i = 1; i <= ssr.length; i++) {
		const isSsrChange =
			i < ssr.length && Math.abs(ssr[i] - currentSsr) > ssrChangeThreshold;
		const isEnd = i === ssr.length;
		if (!isSsrChange && !isEnd) continue;

		const phaseEnd = i - 1;
		const phaseDuration = time[phaseEnd] - time[phaseStart];

		// Skip very short phases (< 1 second)
		if (phaseDuration < 1) {
			if (!isEnd) {
				phaseStart = i;
				currentSsr = ssr[i];
			}
			continue;
		}

		const avgSsr = mean(ssr.slice(phaseStart, phaseEnd + 1));
		const tempStart = temp[phaseStart];
		const tempEnd = temp[phaseEnd];
		const tempChange = tempEnd - tempStart;
		const ratePerMin =
			phaseDuration > 0 ? (tempChange / phaseDuration) * 60 : 0;

		let phaseType: PhaseType;
		if (avgSsr < 5.0) {
			phaseType = "cooling";
		} else if (ratePerMin > plateauThreshold) {
			phaseType = "heating";
		} else if (Math.abs(ratePerMin) <= plateauThreshold) {
			phaseType = "plateau";
		} else {
			phaseType = "cooling";
		}

		const phase: Phase = {
			startIdx: phaseStart,
			endIdx: phaseEnd,
			phaseType,
			avgSsr,
			tempStart,
			tempEnd,
		};
		if (
			data.hasStepData &&
			data.stepNames &&
			phaseStart < data.stepNames.length
		) {
			phase.stepName = data.stepNames[phaseStart];
		}
		phases.push(phase);

		if (!isEnd) {
			phaseStart = i;
			currentSsr = ssr[i];
		}
	}

	return phases;
}

// ---------------------------------------------------------------------------
// Thermal model fitting (port of analyzer/thermal.py)
// ---------------------------------------------------------------------------

function effectiveGainsByTemperature(
	data: TuningData,
	phases: Phase[],
	ambientTemp: number,
): GainPoint[] {
	const { time } = data;
	const points: GainPoint[] = [];
	const plateaus = phases.filter(
		(p) =>
			p.phaseType === "plateau" &&
			p.avgSsr > 20 &&
			time[p.endIdx] - time[p.startIdx] > 60,
	);
	for (const p of plateaus) {
		const plateauTemp = (p.tempStart + p.tempEnd) / 2;
		const aboveAmbient = plateauTemp - ambientTemp;
		if (p.avgSsr > 0 && aboveAmbient > 0) {
			const eff = aboveAmbient / p.avgSsr;
			if (eff >= 0.01 && eff <= 10.0) {
				points.push({
					temp: pyRound(plateauTemp, 1),
					gain: pyRound(eff, 4),
					ssr: pyRound(p.avgSsr, 1),
				});
			}
		}
	}
	points.sort((a, b) => a.temp - b.temp);
	return points;
}

function fitHeatLossFromGain(
	gainPoints: GainPoint[],
	baseGain: number,
	ambientTemp: number,
): number {
	if (gainPoints.length < 2) return 0.0001;
	const hs: number[] = [];
	for (const p of gainPoints) {
		const dT = p.temp - ambientTemp;
		if (dT > 10) {
			const kEff = p.gain;
			if (kEff > 0 && baseGain > 0) {
				const h = (baseGain / kEff - 1.0) / dT;
				if (h > 0 && h < 0.1) hs.push(h);
			}
		}
	}
	if (hs.length === 0) return 0.0001;
	return pyRound(median(hs), 6);
}

function fitHeatLossFromCooling(
	data: TuningData,
	phases: Phase[],
	ambientTemp: number,
): number {
	const { time, temp } = data;
	const coolingPhases = phases.filter(
		(p) =>
			p.phaseType === "cooling" &&
			p.tempStart - p.tempEnd > 5 &&
			p.tempStart > ambientTemp + 20,
	);
	if (coolingPhases.length === 0) return 0.0;

	const hs: number[] = [];
	for (const p of coolingPhases) {
		const phaseTime = time.slice(p.startIdx, p.endIdx + 1);
		const phaseTemp = temp.slice(p.startIdx, p.endIdx + 1);
		if (phaseTime.length < 10) continue;

		const t0 = phaseTime[0];
		const x: number[] = [];
		const y: number[] = [];
		for (let i = 0; i < phaseTemp.length; i++) {
			const delta = phaseTemp[i] - ambientTemp;
			if (delta > 5.0) {
				x.push(phaseTime[i] - t0);
				y.push(Math.log(delta));
			}
		}
		if (x.length < 10) continue;

		const n = x.length;
		const mx = mean(x);
		const my = mean(y);
		let cov = 0;
		let varx = 0;
		for (let i = 0; i < n; i++) {
			cov += (x[i] - mx) * (y[i] - my);
			varx += (x[i] - mx) ** 2;
		}
		cov /= n;
		varx /= n;
		if (varx < 1e-10) continue;

		const k = -cov / varx;
		if (k <= 0) continue;

		const avgTemp = mean(phaseTemp);
		const avgDelta = avgTemp - ambientTemp;
		if (avgDelta > 10) {
			const thermalTimeConstant = 100.0;
			const h = (k * thermalTimeConstant) / avgDelta;
			if (h >= 0.0001 && h <= 0.1) hs.push(h);
		}
	}
	if (hs.length === 0) return 0.0;
	return pyRound(median(hs), 6);
}

export function fitThermalModel(
	data: TuningData,
	phases: Phase[],
): ThermalModel {
	const { time, temp } = data;
	const ambientTemp = mean(temp.slice(0, Math.min(10, temp.length)));

	const heatingPhases = phases.filter(
		(p) => p.phaseType === "heating" && p.avgSsr > 20,
	);

	let deadTimeS: number;
	let timeConstantS: number;
	if (heatingPhases.length > 0) {
		const p = heatingPhases[0];
		const pt = time.slice(p.startIdx, p.endIdx + 1);
		const pTemp = temp.slice(p.startIdx, p.endIdx + 1);

		const initialTemp = pTemp[0];
		const threshold = initialTemp + 0.5;
		let deadIdx = 0;
		for (let i = 0; i < pTemp.length; i++) {
			if (pTemp[i] >= threshold) {
				deadIdx = i;
				break;
			}
		}
		deadTimeS = deadIdx > 0 ? pt[deadIdx] - pt[0] : 5.0;

		const tempStart = deadIdx < pTemp.length ? pTemp[deadIdx] : pTemp[0];
		const tempFinal = pTemp[pTemp.length - 1];
		const temp63 = tempStart + 0.63 * (tempFinal - tempStart);
		let tauIdx = deadIdx;
		for (let i = deadIdx; i < pTemp.length; i++) {
			if (pTemp[i] >= temp63) {
				tauIdx = i;
				break;
			}
		}
		timeConstantS = tauIdx > deadIdx ? pt[tauIdx] - pt[deadIdx] : 60.0;
	} else {
		deadTimeS = 10.0;
		timeConstantS = 120.0;
	}

	// Steady-state gain — prefer plateau equilibrium
	let steadyStateGain: number;
	let gainMethod: GainMethod;
	let gainConfidence: GainConfidence;
	const plateaus = phases.filter(
		(p) =>
			p.phaseType === "plateau" &&
			p.avgSsr > 20 &&
			time[p.endIdx] - time[p.startIdx] > 60,
	);
	if (plateaus.length > 0) {
		const best = plateaus.reduce((a, b) => (b.tempEnd < a.tempEnd ? b : a));
		steadyStateGain = (best.tempEnd - ambientTemp) / best.avgSsr;
		gainMethod = "plateau";
		gainConfidence =
			steadyStateGain >= 0.01 && steadyStateGain <= 10.0 ? "HIGH" : "MEDIUM";
	} else if (heatingPhases.length > 0) {
		const p = heatingPhases[0];
		const pTemp = temp.slice(p.startIdx, p.endIdx + 1);
		const avgTemp = (pTemp[0] + pTemp[pTemp.length - 1]) / 2;
		const aboveAmbient = avgTemp - ambientTemp;
		const lossFraction = aboveAmbient > 0 ? 0.1 * (aboveAmbient / 100) : 0.1;
		const tempChange = pTemp[pTemp.length - 1] - pTemp[0];
		const corrected =
			lossFraction < 0.9 ? tempChange / (1 - lossFraction) : tempChange;
		steadyStateGain = p.avgSsr > 0 ? corrected / p.avgSsr : 0.5;
		gainMethod = "heating";
		gainConfidence =
			steadyStateGain >= 0.01 && steadyStateGain <= 10.0 ? "MEDIUM" : "LOW";
	} else {
		steadyStateGain = 0.5;
		gainMethod = "fallback";
		gainConfidence = "LOW";
	}

	const gainVsTemp = effectiveGainsByTemperature(data, phases, ambientTemp);

	let heatLossCoeff: number;
	let heatLossMethod: HeatLossMethod;
	if (gainVsTemp.length > 0) {
		heatLossCoeff = fitHeatLossFromGain(
			gainVsTemp,
			steadyStateGain,
			ambientTemp,
		);
		heatLossMethod = "plateau";
	} else {
		const hCooling = fitHeatLossFromCooling(data, phases, ambientTemp);
		if (hCooling > 0) {
			heatLossCoeff = hCooling;
			heatLossMethod = "cooling";
		} else {
			heatLossCoeff = 0.0001;
			heatLossMethod = "fallback";
		}
	}

	return {
		deadTimeS,
		timeConstantS,
		steadyStateGain,
		heatLossCoeff,
		ambientTemp,
		gainVsTemp,
		gainMethod,
		gainConfidence,
		heatLossMethod,
	};
}

// ---------------------------------------------------------------------------
// PID calculation methods (port of analyzer/pid.py)
// ---------------------------------------------------------------------------

function guards(m: ThermalModel): { L: number; T: number; K: number } {
	return {
		L: m.deadTimeS < 1 ? 1 : m.deadTimeS,
		T: m.timeConstantS < 1 ? 1 : m.timeConstantS,
		K: m.steadyStateGain > 0 ? m.steadyStateGain : 1.0,
	};
}

const ZN_CHARS =
	"Fast response with moderate overshoot (~25%). Good general-purpose tuning. May oscillate if system is noisy.";
const CC_CHARS =
	"Optimized for systems with significant dead time (L/T > 0.3). Faster response than Z-N with similar overshoot. Better disturbance rejection.";
const AMIGO_CHARS =
	"Very conservative tuning with minimal overshoot (<5%). Smooth, stable response. Excellent for preventing temperature overshoot in kilns.";

export function calculatePidMethods(
	m: ThermalModel,
): Record<PidMethodKey, PidMethod> {
	const { L, T, K } = guards(m);

	// Ziegler-Nichols
	const znKp = (1.2 * T) / (K * L);
	const znTi = 2.0 * L;
	const znTd = 0.5 * L;
	const zieglerNichols: PidMethod = {
		kp: znKp,
		ki: znTi > 0 ? znKp / znTi : 0,
		kd: znKp * znTd,
		method: "Ziegler-Nichols",
		characteristics: ZN_CHARS,
	};

	// Cohen-Coon
	const ratio = L / T;
	const ccKp = (1 / K) * (T / L) * (1.0 + L / (12 * T));
	const ccTi = (L * (30 + 3 * ratio)) / (9 + 20 * ratio);
	const ccTd = (L * 4) / (11 + 2 * ratio);
	const cohenCoon: PidMethod = {
		kp: ccKp,
		ki: ccTi > 0 ? ccKp / ccTi : 0,
		kd: ccKp * ccTd,
		method: "Cohen-Coon",
		characteristics: CC_CHARS,
	};

	// AMIGO
	const amKp = K > 0 ? (0.2 + 0.45 * (T / L)) / K : 0.45 * (T / L);
	const amTi =
		L + 0.1 * T > 0 ? ((0.4 * L + 0.8 * T) * (L + 0.3 * T)) / (L + 0.1 * T) : L;
	const amTd = 0.3 * L + T > 0 ? (0.5 * L * T) / (0.3 * L + T) : 0.5 * L;
	const amigo: PidMethod = {
		kp: amKp,
		ki: amTi > 0 ? amKp / amTi : 0,
		kd: amKp * amTd,
		method: "AMIGO",
		characteristics: AMIGO_CHARS,
	};

	return { zieglerNichols, cohenCoon, amigo };
}

// ---------------------------------------------------------------------------
// Quality assessment + recommendation (reporting.py + analyze_tuning.py)
// ---------------------------------------------------------------------------

export function assessTestQuality(
	data: TuningData,
	phases: Phase[],
	model: ThermalModel,
): TestQuality {
	let score = 0;
	const maxScore = 6;

	if (data.time.length > 500) score += 1;
	else if (data.time.length > 200) score += 0.5;

	const tempSpan = Math.max(...data.temp) - Math.min(...data.temp);
	if (tempSpan > 100) score += 1;
	else if (tempSpan > 50) score += 0.5;

	if (phases.length >= 3) score += 1;
	else if (phases.length >= 2) score += 0.5;

	const heating = phases.filter((p) => p.phaseType === "heating");
	if (heating.length >= 2) score += 1;
	else if (heating.length >= 1) score += 0.5;

	if (
		model.deadTimeS >= 5 &&
		model.deadTimeS <= 60 &&
		model.timeConstantS >= 30 &&
		model.timeConstantS <= 600
	)
		score += 1;
	else if (
		model.deadTimeS >= 1 &&
		model.deadTimeS <= 120 &&
		model.timeConstantS >= 10 &&
		model.timeConstantS <= 1200
	)
		score += 0.5;

	const duration = data.time[data.time.length - 1] - data.time[0];
	if (duration > 1800) score += 1;
	else if (duration > 900) score += 0.5;

	const percentage = (score / maxScore) * 100;
	if (percentage >= 80) return "EXCELLENT";
	if (percentage >= 50) return "GOOD";
	return "POOR";
}

export function selectRecommended(
	model: ThermalModel,
	quality: TestQuality,
): PidMethodKey {
	const ratio =
		model.timeConstantS > 0 ? model.deadTimeS / model.timeConstantS : 0;
	if (quality === "POOR") return "amigo";
	if (ratio > 0.3) return "cohenCoon";
	return "amigo";
}

// ---------------------------------------------------------------------------
// Top-level pipeline
// ---------------------------------------------------------------------------

function roundPid(p: PidMethod): PidMethod {
	return {
		...p,
		kp: pyRound(p.kp, 3),
		ki: pyRound(p.ki, 4),
		kd: pyRound(p.kd, 3),
	};
}

/** Full analysis pipeline. Input is the raw tuning CSV text. */
export function analyzeTuning(content: string): TuningAnalysis {
	const data = loadTuningData(content);
	const phases = detectPhases(data);
	const rawModel = fitThermalModel(data, phases);
	const pidRaw = calculatePidMethods(rawModel);
	const testQuality = assessTestQuality(data, phases, rawModel);
	const recommended = selectRecommended(rawModel, testQuality);

	return {
		testInfo: {
			durationS: pyRound(data.time[data.time.length - 1] - data.time[0], 1),
			dataPoints: data.time.length,
			tempMin: pyRound(Math.min(...data.temp), 1),
			tempMax: pyRound(Math.max(...data.temp), 1),
			phasesDetected: phases.length,
		},
		phases,
		thermalModel: {
			...rawModel,
			deadTimeS: pyRound(rawModel.deadTimeS, 2),
			timeConstantS: pyRound(rawModel.timeConstantS, 1),
			steadyStateGain: pyRound(rawModel.steadyStateGain, 4),
			heatLossCoeff: pyRound(rawModel.heatLossCoeff, 6),
			ambientTemp: pyRound(rawModel.ambientTemp, 1),
		},
		pidMethods: {
			zieglerNichols: roundPid(pidRaw.zieglerNichols),
			cohenCoon: roundPid(pidRaw.cohenCoon),
			amigo: roundPid(pidRaw.amigo),
		},
		recommended,
		testQuality,
	};
}

/**
 * Build the sparse config PATCH that applies a chosen PID method to the kiln.
 * Mirrors generate_config_snippet(): base PID gains + heat-loss gain scheduling.
 */
export function configPatchForMethod(
	analysis: TuningAnalysis,
	methodKey: PidMethodKey,
): Pick<
	KilnConfig,
	| "PID_KP_BASE"
	| "PID_KI_BASE"
	| "PID_KD_BASE"
	| "THERMAL_H"
	| "THERMAL_T_AMBIENT"
> {
	const pid = analysis.pidMethods[methodKey];
	return {
		PID_KP_BASE: pid.kp,
		PID_KI_BASE: pid.ki,
		PID_KD_BASE: pid.kd,
		THERMAL_H: analysis.thermalModel.heatLossCoeff,
		THERMAL_T_AMBIENT: analysis.thermalModel.ambientTemp,
	};
}
