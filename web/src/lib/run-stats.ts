import type { LogDataPoint } from "./csv-parser";

export interface HoldSegment {
	startIdx: number;
	endIdx: number;
	targetC: number;
}

export interface RunStats {
	durationS: number;
	peakTempC: number;
	avgSsrPercent: number;
	/** Time the relay was pinned at full output — the kiln "can't keep up" signal. */
	ssrSaturatedHighS: number;
	ssrSaturatedLowS: number;
	ssrSaturatedHighPct: number;
	/** Whether a meaningful setpoint exists (false for tuning runs that hold no target). */
	hasTracking: boolean;
	maxOvershootC: number | null;
	maxOvershootAtS: number | null;
	maxLagC: number | null;
	maxLagAtS: number | null;
	rmsErrorC: number | null;
	steadyStateErrorC: number | null;
	settlingTimeS: number | null;
	holdCount: number;
}

const SSR_HIGH_PERCENT = 99;
const SSR_LOW_PERCENT = 1;
const SETTLE_BAND_C = 5;
export const TARGET_EPS_C = 0.5;
const MIN_HOLD_S = 60;

/**
 * Holds are contiguous runs of (near-)constant positive target temperature.
 * Ramps change target continuously so they never coalesce into a hold, which
 * lets settling time and steady-state error be measured without step metadata.
 */
export function findHoldSegments(data: LogDataPoint[]): HoldSegment[] {
	const segments: HoldSegment[] = [];
	let start = -1;
	let segTarget = 0;

	const flush = (endIdx: number) => {
		if (start < 0) return;
		const durationS =
			data[endIdx].elapsed_seconds - data[start].elapsed_seconds;
		if (
			endIdx - start >= 2 &&
			durationS >= MIN_HOLD_S &&
			segTarget > TARGET_EPS_C
		) {
			segments.push({ startIdx: start, endIdx, targetC: segTarget });
		}
		start = -1;
	};

	for (let i = 0; i < data.length; i++) {
		const target = data[i].target_temp_c;
		const matches =
			start >= 0 &&
			target > TARGET_EPS_C &&
			Math.abs(target - segTarget) <= TARGET_EPS_C;
		if (matches) continue;
		flush(i - 1);
		if (target > TARGET_EPS_C) {
			start = i;
			segTarget = target;
		}
	}
	flush(data.length - 1);
	return segments;
}

function settlingTime(data: LogDataPoint[], hold: HoldSegment): number | null {
	const startS = data[hold.startIdx].elapsed_seconds;
	for (let i = hold.startIdx; i <= hold.endIdx; i++) {
		if (Math.abs(data[i].current_temp_c - hold.targetC) <= SETTLE_BAND_C) {
			return data[i].elapsed_seconds - startS;
		}
	}
	return null;
}

function steadyStateError(
	data: LogDataPoint[],
	hold: HoldSegment,
): number | null {
	const startS = data[hold.startIdx].elapsed_seconds;
	const endS = data[hold.endIdx].elapsed_seconds;
	const duration = endS - startS;
	if (duration <= 0) return null;
	const cutoff = endS - duration * 0.25;
	let sum = 0;
	let count = 0;
	for (let i = hold.startIdx; i <= hold.endIdx; i++) {
		if (data[i].elapsed_seconds >= cutoff) {
			sum += Math.abs(data[i].current_temp_c - hold.targetC);
			count++;
		}
	}
	return count > 0 ? sum / count : null;
}

function emptyStats(): RunStats {
	return {
		durationS: 0,
		peakTempC: 0,
		avgSsrPercent: 0,
		ssrSaturatedHighS: 0,
		ssrSaturatedLowS: 0,
		ssrSaturatedHighPct: 0,
		hasTracking: false,
		maxOvershootC: null,
		maxOvershootAtS: null,
		maxLagC: null,
		maxLagAtS: null,
		rmsErrorC: null,
		steadyStateErrorC: null,
		settlingTimeS: null,
		holdCount: 0,
	};
}

export function computeRunStats(data: LogDataPoint[]): RunStats {
	const n = data.length;
	if (n === 0) return emptyStats();

	let peakTempC = Number.NEGATIVE_INFINITY;
	let ssrSum = 0;
	let ssrSaturatedHighS = 0;
	let ssrSaturatedLowS = 0;
	let maxOvershootC = Number.NEGATIVE_INFINITY;
	let maxOvershootAtS = 0;
	let maxLagC = Number.NEGATIVE_INFINITY;
	let maxLagAtS = 0;
	let sqErrSum = 0;
	let targetSamples = 0;
	let maxTarget = 0;

	for (let i = 0; i < n; i++) {
		const p = data[i];
		if (p.current_temp_c > peakTempC) peakTempC = p.current_temp_c;
		if (p.target_temp_c > maxTarget) maxTarget = p.target_temp_c;
		ssrSum += p.ssr_output_percent;

		if (i > 0) {
			const dt = p.elapsed_seconds - data[i - 1].elapsed_seconds;
			if (dt > 0) {
				const prevSsr = data[i - 1].ssr_output_percent;
				if (prevSsr >= SSR_HIGH_PERCENT) ssrSaturatedHighS += dt;
				else if (prevSsr <= SSR_LOW_PERCENT) ssrSaturatedLowS += dt;
			}
		}

		if (p.target_temp_c > TARGET_EPS_C) {
			targetSamples++;
			const err = p.current_temp_c - p.target_temp_c;
			sqErrSum += err * err;
			if (err > maxOvershootC) {
				maxOvershootC = err;
				maxOvershootAtS = p.elapsed_seconds;
			}
			if (-err > maxLagC) {
				maxLagC = -err;
				maxLagAtS = p.elapsed_seconds;
			}
		}
	}

	const durationS = data[n - 1].elapsed_seconds - data[0].elapsed_seconds;
	const hasTracking = targetSamples >= 3 && maxTarget > TARGET_EPS_C;
	const holds = findHoldSegments(data);

	let principal: HoldSegment | null = null;
	let bestDuration = -1;
	for (const h of holds) {
		const d = data[h.endIdx].elapsed_seconds - data[h.startIdx].elapsed_seconds;
		if (d > bestDuration) {
			bestDuration = d;
			principal = h;
		}
	}

	return {
		durationS,
		peakTempC,
		avgSsrPercent: ssrSum / n,
		ssrSaturatedHighS,
		ssrSaturatedLowS,
		ssrSaturatedHighPct: durationS > 0 ? ssrSaturatedHighS / durationS : 0,
		hasTracking,
		maxOvershootC: hasTracking ? Math.max(0, maxOvershootC) : null,
		maxOvershootAtS: hasTracking ? maxOvershootAtS : null,
		maxLagC: hasTracking ? Math.max(0, maxLagC) : null,
		maxLagAtS: hasTracking ? maxLagAtS : null,
		rmsErrorC: hasTracking ? Math.sqrt(sqErrSum / targetSamples) : null,
		steadyStateErrorC:
			hasTracking && principal ? steadyStateError(data, principal) : null,
		settlingTimeS:
			hasTracking && principal ? settlingTime(data, principal) : null,
		holdCount: holds.length,
	};
}
