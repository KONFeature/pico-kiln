import { describe, expect, it } from "vitest";
import type { LogDataPoint } from "./csv-parser";
import { computeRunStats, findHoldSegments } from "./run-stats";

function pt(
	elapsed: number,
	current: number,
	target: number,
	ssr: number,
): LogDataPoint {
	return {
		elapsed_seconds: elapsed,
		current_temp_c: current,
		target_temp_c: target,
		ssr_output_percent: ssr,
		state: "RUNNING",
		timestamp: "",
	};
}

describe("computeRunStats — SSR saturation duration", () => {
	it("sums time the relay was pinned high vs low using interval start state", () => {
		const data = [
			pt(0, 0, 0, 100),
			pt(10, 0, 0, 100),
			pt(20, 0, 0, 0),
			pt(30, 0, 0, 0),
		];
		const stats = computeRunStats(data);
		expect(stats.ssrSaturatedHighS).toBe(20);
		expect(stats.ssrSaturatedLowS).toBe(10);
		expect(stats.durationS).toBe(30);
		expect(stats.ssrSaturatedHighPct).toBeCloseTo(20 / 30, 5);
	});
});

describe("computeRunStats — tracking error", () => {
	it("reports max overshoot, max lag and RMS error over targeted samples", () => {
		const data = [
			pt(0, 90, 100, 50),
			pt(10, 110, 100, 50),
			pt(20, 100, 100, 50),
		];
		const stats = computeRunStats(data);
		expect(stats.hasTracking).toBe(true);
		expect(stats.maxOvershootC).toBe(10);
		expect(stats.maxLagC).toBe(10);
		expect(stats.rmsErrorC).toBeCloseTo(Math.sqrt(200 / 3), 5);
	});

	it("marks tracking unavailable when no target is ever set (tuning run)", () => {
		const data = [pt(0, 20, 0, 25), pt(10, 60, 0, 25), pt(20, 90, 0, 25)];
		const stats = computeRunStats(data);
		expect(stats.hasTracking).toBe(false);
		expect(stats.maxOvershootC).toBeNull();
		expect(stats.rmsErrorC).toBeNull();
	});
});

describe("findHoldSegments", () => {
	it("detects a constant-target hold but not a rising ramp", () => {
		const ramp = [
			pt(0, 20, 100, 100),
			pt(20, 60, 200, 100),
			pt(40, 100, 300, 100),
		];
		expect(findHoldSegments(ramp)).toHaveLength(0);

		const hold = Array.from({ length: 13 }, (_, i) => pt(i * 10, 195, 200, 30));
		const segs = findHoldSegments(hold);
		expect(segs).toHaveLength(1);
		expect(segs[0].targetC).toBe(200);
	});
});

describe("computeRunStats — settling + steady-state on a hold", () => {
	it("measures time to enter the ±5°C band and the residual error", () => {
		const ramp = [180, 185, 190, 195, 200];
		const data = ramp
			.map((c, i) => pt(i * 10, c, 200, 80))
			.concat(
				Array.from({ length: 8 }, (_, i) => pt(50 + i * 10, 200, 200, 40)),
			);
		const stats = computeRunStats(data);
		expect(stats.holdCount).toBe(1);
		expect(stats.settlingTimeS).toBe(30);
		expect(stats.steadyStateErrorC).toBeCloseTo(0, 5);
	});
});
