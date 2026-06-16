import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { analyzeTuning, configPatchForMethod } from "./tuning-analysis";

// Golden values below were produced by running the reference Python analyzer
// (scripts/analyze_tuning.py) on these exact synthetic fixtures. The TS port
// must reproduce them. Fixtures are deterministic; regenerate with
// /tmp/gen_fixtures.py if the reference algorithm ever changes.
function fixture(name: string): string {
	return readFileSync(
		fileURLToPath(new URL(`./__fixtures__/${name}`, import.meta.url)),
		"utf-8",
	);
}

describe("analyzeTuning — heating+cooling fixture (heating gain, cooling heat-loss)", () => {
	const r = analyzeTuning(fixture("tuning_heatcool.csv"));

	it("reports test info", () => {
		expect(r.testInfo.dataPoints).toBe(1500);
		expect(r.testInfo.durationS).toBeCloseTo(2998.0, 1);
		expect(r.testInfo.tempMin).toBeCloseTo(25.0, 1);
		expect(r.testInfo.tempMax).toBeCloseTo(314.6, 1);
		expect(r.testInfo.phasesDetected).toBe(2);
	});

	it("fits the thermal model via heating gain + cooling heat-loss", () => {
		const m = r.thermalModel;
		expect(m.deadTimeS).toBeCloseTo(2.0, 1);
		expect(m.timeConstantS).toBeCloseTo(338.0, 1);
		expect(m.steadyStateGain).toBeCloseTo(3.3567, 3);
		expect(m.heatLossCoeff).toBeCloseTo(0.001486, 5);
		expect(m.ambientTemp).toBeCloseTo(32.5, 1);
		expect(m.gainVsTemp).toHaveLength(0);
		expect(m.gainMethod).toBe("heating");
		expect(m.gainConfidence).toBe("MEDIUM");
		expect(m.heatLossMethod).toBe("cooling");
	});

	it("computes all three PID methods", () => {
		expect(r.pidMethods.zieglerNichols.kp).toBeCloseTo(60.417, 2);
		expect(r.pidMethods.zieglerNichols.ki).toBeCloseTo(15.1042, 3);
		expect(r.pidMethods.zieglerNichols.kd).toBeCloseTo(60.417, 2);
		expect(r.pidMethods.cohenCoon.kp).toBeCloseTo(50.372, 2);
		expect(r.pidMethods.cohenCoon.ki).toBeCloseTo(7.6506, 3);
		expect(r.pidMethods.cohenCoon.kd).toBeCloseTo(36.595, 2);
		expect(r.pidMethods.amigo.kp).toBeCloseTo(22.716, 2);
		expect(r.pidMethods.amigo.ki).toBeCloseTo(0.029, 3);
		expect(r.pidMethods.amigo.kd).toBeCloseTo(22.676, 2);
	});

	it("recommends amigo and grades the test GOOD", () => {
		expect(r.recommended).toBe("amigo");
		expect(r.testQuality).toBe("GOOD");
	});

	it("builds a 5-field config patch from the chosen method", () => {
		const patch = configPatchForMethod(r, "amigo");
		expect(patch.PID_KP_BASE).toBeCloseTo(22.716, 2);
		expect(patch.PID_KI_BASE).toBeCloseTo(0.029, 3);
		expect(patch.PID_KD_BASE).toBeCloseTo(22.676, 2);
		expect(patch.THERMAL_H).toBeCloseTo(0.001486, 5);
		expect(patch.THERMAL_T_AMBIENT).toBeCloseTo(32.5, 1);
		expect(Object.keys(patch).sort()).toEqual([
			"PID_KD_BASE",
			"PID_KI_BASE",
			"PID_KP_BASE",
			"THERMAL_H",
			"THERMAL_T_AMBIENT",
		]);
	});
});

describe("analyzeTuning — heating+plateaus+cooling fixture (plateau gain)", () => {
	const r = analyzeTuning(fixture("tuning_full.csv"));

	it("detects 5 phases", () => {
		expect(r.testInfo.phasesDetected).toBe(5);
		expect(r.testInfo.dataPoints).toBe(1440);
	});

	it("fits gain from plateaus with a gain-vs-temp schedule", () => {
		const m = r.thermalModel;
		expect(m.deadTimeS).toBeCloseTo(2.0, 1);
		expect(m.timeConstantS).toBeCloseTo(232.0, 1);
		expect(m.steadyStateGain).toBeCloseTo(2.9755, 3);
		expect(m.heatLossCoeff).toBeCloseTo(0.0001, 6);
		expect(m.ambientTemp).toBeCloseTo(30.1, 1);
		expect(m.gainMethod).toBe("plateau");
		expect(m.gainConfidence).toBe("HIGH");
		expect(m.heatLossMethod).toBe("plateau");
		expect(m.gainVsTemp).toHaveLength(2);
		expect(m.gainVsTemp[0].temp).toBeCloseTo(164.0, 1);
		expect(m.gainVsTemp[0].gain).toBeCloseTo(2.9761, 3);
		expect(m.gainVsTemp[0].ssr).toBeCloseTo(45.0, 1);
		expect(m.gainVsTemp[1].temp).toBeCloseTo(313.6, 1);
		expect(m.gainVsTemp[1].gain).toBeCloseTo(4.0496, 3);
	});

	it("computes PID methods and grades EXCELLENT", () => {
		expect(r.pidMethods.amigo.kp).toBeCloseTo(17.61, 2);
		expect(r.pidMethods.amigo.ki).toBeCloseTo(0.0333, 3);
		expect(r.pidMethods.amigo.kd).toBeCloseTo(17.565, 2);
		expect(r.pidMethods.zieglerNichols.kp).toBeCloseTo(46.782, 2);
		expect(r.pidMethods.cohenCoon.kp).toBeCloseTo(39.013, 2);
		expect(r.recommended).toBe("amigo");
		expect(r.testQuality).toBe("EXCELLENT");
	});
});
