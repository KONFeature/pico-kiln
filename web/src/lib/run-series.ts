import { elapsedSecondsToDate } from "./chart-time";
import { type LogDataPoint, lttbDownsample } from "./csv-parser";

export interface RunChartPoint {
	t: Date;
	elapsed: number;
	temp: number;
	target: number;
	ssr: number;
	rate: number;
	ssrPlot: number;
	ratePlot: number;
	[key: string]: unknown;
}

export interface RunSeriesModel {
	plotMax: number;
	rateDomain: [number, number];
	hasRate: boolean;
	ssrToPlot: (percent: number) => number;
	rateToPlot: (rate: number) => number;
}

/**
 * Projects every series onto a single temperature-valued plot axis so the
 * bklit chart can overlay them on one shared `yScale`. SSR (0–100 %) and the
 * heating rate (°C/h) are linearly mapped into `[0, plotMax]`; the matching
 * `KilnRightAxis` inverts these maps to label the right gutter in real units.
 * Domains are computed from the whole run so the axes stay fixed while zooming.
 */
export function buildRunSeriesModel(data: LogDataPoint[]): RunSeriesModel {
	let tempMax = 0;
	let rateMin = Number.POSITIVE_INFINITY;
	let rateMax = Number.NEGATIVE_INFINITY;
	let hasRate = false;

	for (const p of data) {
		if (p.current_temp_c > tempMax) tempMax = p.current_temp_c;
		if (p.target_temp_c > tempMax) tempMax = p.target_temp_c;
		const r = p.measured_rate_c_per_hour;
		if (typeof r === "number") {
			hasRate = true;
			if (r < rateMin) rateMin = r;
			if (r > rateMax) rateMax = r;
		}
	}

	const plotMax = tempMax > 0 ? tempMax : 100;

	if (!hasRate || rateMin === Number.POSITIVE_INFINITY) {
		rateMin = 0;
		rateMax = 1;
	}
	// Force the rate domain to straddle zero so the zero-rate baseline (the
	// boundary between heating and cooling) maps to a stable on-chart position.
	rateMin = Math.min(rateMin, 0);
	rateMax = Math.max(rateMax, 0);
	if (rateMin === rateMax) rateMax = rateMin + 1;
	const rateSpan = rateMax - rateMin;

	return {
		plotMax,
		rateDomain: [rateMin, rateMax],
		hasRate,
		ssrToPlot: (percent) => (percent / 100) * plotMax,
		rateToPlot: (rate) => ((rate - rateMin) / rateSpan) * plotMax,
	};
}

export function toRunChartPoint(
	p: LogDataPoint,
	model: RunSeriesModel,
): RunChartPoint {
	const ssr = p.ssr_output_percent;
	const rate = p.measured_rate_c_per_hour ?? 0;
	return {
		t: elapsedSecondsToDate(p.elapsed_seconds),
		elapsed: p.elapsed_seconds,
		temp: p.current_temp_c,
		target: p.target_temp_c,
		ssr,
		rate,
		ssrPlot: model.ssrToPlot(ssr),
		ratePlot: model.rateToPlot(rate),
	};
}

/**
 * Windows the raw log to `[start, end]` elapsed seconds (or the whole run when
 * omitted), then LTTB-downsamples the slice. Downsampling the window instead of
 * the full run means zooming in reveals progressively finer detail.
 */
export function buildRunChartData(
	data: LogDataPoint[],
	model: RunSeriesModel,
	maxPoints: number,
	window?: readonly [number, number] | null,
): RunChartPoint[] {
	let slice = data;
	if (window) {
		const [s0, s1] = window;
		const filtered = data.filter(
			(p) => p.elapsed_seconds >= s0 && p.elapsed_seconds <= s1,
		);
		if (filtered.length >= 2) slice = filtered;
	}
	const sampled = lttbDownsample(
		slice,
		maxPoints,
		(p) => p.elapsed_seconds,
		(p) => p.current_temp_c,
	);
	return sampled.map((p) => toRunChartPoint(p, model));
}

export function elapsedExtent(data: LogDataPoint[]): [number, number] {
	if (data.length === 0) return [0, 1];
	return [data[0].elapsed_seconds, data[data.length - 1].elapsed_seconds];
}
