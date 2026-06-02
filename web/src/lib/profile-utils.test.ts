import { describe, expect, it } from "vitest";
import type { Profile } from "./pico/types";
import { buildProfileChart, calculateTrajectory } from "./profile-utils";

const profile: Profile = {
	name: "Test",
	temp_units: "c",
	description: "",
	steps: [
		{ type: "ramp", target_temp: 600, desired_rate: 100 },
		{ type: "hold", target_temp: 600, duration: 600 },
		{ type: "cooling" },
	],
};

describe("buildProfileChart", () => {
	const segments = calculateTrajectory(profile);
	const chart = buildProfileChart(segments);

	it("produces one colored series per segment", () => {
		expect(chart.series).toHaveLength(segments.length);
		for (const s of chart.series) {
			expect(s.color).toMatch(/var\(--chart-/);
			expect(s.data[0].t).toBeInstanceOf(Date);
		}
	});

	it("starts the flattened trajectory at elapsed zero", () => {
		expect(chart.chartData[0].t.getTime()).toBe(0);
		expect(chart.chartData[0].temp).toBe(20);
	});

	it("de-duplicates shared segment boundary points by time", () => {
		// 3 segments × 2 points − 2 shared boundaries = 4 unique points.
		expect(chart.chartData).toHaveLength(4);
		for (let i = 1; i < chart.chartData.length; i++) {
			expect(chart.chartData[i].t.getTime()).toBeGreaterThan(
				chart.chartData[i - 1].t.getTime(),
			);
		}
	});

	it("emits a numbered marker at each step boundary after the first", () => {
		expect(chart.markers).toHaveLength(segments.length - 1);
		expect(chart.markers[0].label).toBe("2");
		expect(chart.markers.at(-1)?.label).toBe(String(segments.length));
		expect(chart.markers[0].atSeconds).toBeGreaterThan(0);
	});

	it("returns empty output for an empty segment list", () => {
		const empty = buildProfileChart([]);
		expect(empty.chartData).toEqual([]);
		expect(empty.series).toEqual([]);
		expect(empty.markers).toEqual([]);
	});
});
