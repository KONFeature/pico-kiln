import { describe, expect, it } from "vitest";
import { lttbDownsample } from "./csv-parser";

interface Pt {
	x: number;
	y: number;
}

const x = (p: Pt) => p.x;
const y = (p: Pt) => p.y;

function ramp(n: number): Pt[] {
	return Array.from({ length: n }, (_, i) => ({ x: i, y: i }));
}

describe("lttbDownsample", () => {
	it("returns the input unchanged when threshold >= length", () => {
		const data = ramp(10);
		expect(lttbDownsample(data, 10, x, y)).toEqual(data);
		expect(lttbDownsample(data, 20, x, y)).toEqual(data);
	});

	it("returns the input unchanged for degenerate thresholds (< 3)", () => {
		const data = ramp(10);
		expect(lttbDownsample(data, 2, x, y)).toEqual(data);
		expect(lttbDownsample(data, 0, x, y)).toEqual(data);
	});

	it("downsamples to exactly `threshold` points", () => {
		const data = ramp(1000);
		const out = lttbDownsample(data, 100, x, y);
		expect(out).toHaveLength(100);
	});

	it("always preserves the first and last points", () => {
		const data = ramp(8640);
		const out = lttbDownsample(data, 500, x, y);
		expect(out[0]).toEqual(data[0]);
		expect(out.at(-1)).toEqual(data.at(-1));
	});

	it("preserves a sharp spike that naive every-Nth sampling would drop", () => {
		const data = ramp(1000);
		// Inject a single tall spike between two regular sample strides.
		const spikeIndex = 503;
		data[spikeIndex] = { x: spikeIndex, y: 100000 };

		const out = lttbDownsample(data, 50, x, y);
		const keptSpike = out.some((p) => p.y === 100000);
		expect(keptSpike).toBe(true);
	});

	it("keeps points in ascending x order", () => {
		const out = lttbDownsample(ramp(2000), 200, x, y);
		for (let i = 1; i < out.length; i++) {
			expect(out[i].x).toBeGreaterThan(out[i - 1].x);
		}
	});
});
