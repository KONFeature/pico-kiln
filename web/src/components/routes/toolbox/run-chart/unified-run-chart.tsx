import { curveMonotoneX } from "@visx/curve";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Area } from "@/components/charts/area";
import { Grid } from "@/components/charts/grid";
import {
	type KilnMarker,
	KilnMarkers,
	KilnProjectedAxis,
	KilnSelectionOverlay,
	KilnTimeAxis,
	KilnTooltipContent,
	KilnTrackingBand,
} from "@/components/charts/kiln/parts";
import { Line } from "@/components/charts/line";
import { LineChart } from "@/components/charts/line-chart";
import { ChartTooltip } from "@/components/charts/tooltip";
import type { TooltipRow } from "@/components/charts/tooltip/tooltip-content";
import type { ChartSelection } from "@/components/charts/use-chart-interaction";
import { YAxis } from "@/components/charts/y-axis";
import type { LogDataPoint } from "@/lib/csv-parser";
import {
	buildRunSeriesModel,
	elapsedExtent,
	projectRunPoints,
	type RunChartPoint,
	sampleRunWindow,
} from "@/lib/run-series";
import { TARGET_EPS_C } from "@/lib/run-stats";
import { cn } from "@/lib/utils";
import { RunChartMinimap } from "./run-chart-minimap";
import { RunChartToolbar } from "./run-chart-toolbar";
import { MIN_SPAN_S, SERIES_META, type SeriesKey } from "./types";
import { useFullscreenLandscape } from "./use-fullscreen-landscape";
import { useThrottledValue } from "./use-throttled-value";

const MAX_POINTS = 800;
const MINIMAP_POINTS = 240;
const CHART_MARGIN_LEFT = 50;
const RIGHT_AXIS_WIDTH = 46;
const SSR_BAND_FRACTION = 1 / 3;
const VIEW_THROTTLE_MS = 90;

function buildStepMarkers(data: LogDataPoint[]): KilnMarker[] {
	const markers: KilnMarker[] = [];
	let prevStep = -1;
	for (let i = 0; i < data.length; i++) {
		const stepIdx = data[i].step_index;
		if (
			stepIdx !== undefined &&
			stepIdx !== prevStep &&
			stepIdx >= 0 &&
			i > 0
		) {
			markers.push({
				atSeconds: data[i].elapsed_seconds,
				label: String(stepIdx + 1),
			});
			prevStep = stepIdx;
		}
	}
	return markers;
}

function defaultActive(hasTarget: boolean): Set<SeriesKey> {
	return new Set<SeriesKey>(
		hasTarget ? ["temp", "target", "ssr"] : ["temp", "ssr"],
	);
}

interface ProjectedAxisSpec {
	key: SeriesKey;
	label: string;
	domain: [number, number];
	mapToPlot: (value: number) => number;
	format: (value: number) => string;
	color: string;
}

export function UnifiedRunChart({ logData }: { logData: LogDataPoint[] }) {
	const containerRef = useRef<HTMLDivElement>(null);
	const chartWrapRef = useRef<HTMLDivElement>(null);
	const fullscreen = useFullscreenLandscape(containerRef);

	const fullExtent = useMemo(() => elapsedExtent(logData), [logData]);
	const hasTarget = useMemo(
		() => logData.some((p) => p.target_temp_c > TARGET_EPS_C),
		[logData],
	);

	const [active, setActive] = useState<Set<SeriesKey>>(() =>
		defaultActive(hasTarget),
	);
	const [zoomWindow, setZoomWindow] = useState<[number, number] | null>(null);

	const zoomWindowRef = useRef(zoomWindow);
	zoomWindowRef.current = zoomWindow;

	// The minimap handle tracks `zoomWindow` immediately; the chart view follows
	// a throttled copy so dragging doesn't re-run filter + LTTB every frame.
	const viewWindow = useThrottledValue(zoomWindow, VIEW_THROTTLE_MS);

	const showsTemperature = active.has("temp") || active.has("target");
	// SSR rides the shared temperature axis; always clamp it to the lower third
	// so the relay trace can never swamp the temperature curve.
	const model = useMemo(
		() => buildRunSeriesModel(logData, SSR_BAND_FRACTION),
		[logData],
	);

	const sampled = useMemo(
		() => sampleRunWindow(logData, MAX_POINTS, viewWindow),
		[logData, viewWindow],
	);
	const chartData = useMemo(
		() => projectRunPoints(sampled, model),
		[sampled, model],
	);
	const chartDataRef = useRef<RunChartPoint[]>(chartData);
	chartDataRef.current = chartData;

	const minimapSampled = useMemo(
		() => sampleRunWindow(logData, MINIMAP_POINTS, null),
		[logData],
	);
	const minimapData = useMemo(
		() => projectRunPoints(minimapSampled, model),
		[minimapSampled, model],
	);

	const allMarkers = useMemo(() => buildStepMarkers(logData), [logData]);
	const visibleMarkers = useMemo(() => {
		const [w0, w1] = viewWindow ?? fullExtent;
		return allMarkers.filter((m) => m.atSeconds >= w0 && m.atSeconds <= w1);
	}, [allMarkers, viewWindow, fullExtent]);

	const available = useMemo<SeriesKey[]>(() => {
		const keys: SeriesKey[] = ["temp"];
		if (hasTarget) keys.push("target");
		keys.push("ssr");
		if (model.hasRate) keys.push("rate");
		if (hasTarget) keys.push("error");
		return keys;
	}, [hasTarget, model.hasRate]);

	// Each projected (non-temperature) series needs its own real-unit axis. A
	// temperature trace claims the left axis, pushing projected axes to the
	// right (stacked when both show); otherwise the first takes the free left.
	const { leftAxis, rightAxes } = useMemo(() => {
		const specs: ProjectedAxisSpec[] = [];
		if (active.has("ssr")) {
			specs.push({
				key: "ssr",
				label: "SSR %",
				domain: [0, 100],
				mapToPlot: model.ssrToPlot,
				format: (v) => `${Math.round(v)}%`,
				color: SERIES_META.ssr.color,
			});
		}
		if (active.has("rate")) {
			specs.push({
				key: "rate",
				label: "°C/h",
				domain: model.rateDomain,
				mapToPlot: model.rateToPlot,
				format: (v) => `${Math.round(v)}`,
				color: SERIES_META.rate.color,
			});
		}
		return {
			leftAxis: showsTemperature ? null : (specs[0] ?? null),
			rightAxes: showsTemperature ? specs : specs.slice(1),
		};
	}, [active, model, showsTemperature]);

	const zoomToRange = useCallback(
		(s0: number, s1: number) => {
			const [f0, f1] = fullExtent;
			const fullSpan = Math.max(1, f1 - f0);
			const minSpan = Math.max(MIN_SPAN_S, fullSpan * 0.002);
			let lo = Math.max(f0, Math.min(s0, s1));
			let hi = Math.min(f1, Math.max(s0, s1));
			if (hi - lo < minSpan) {
				const center = (lo + hi) / 2;
				lo = Math.max(f0, center - minSpan / 2);
				hi = Math.min(f1, lo + minSpan);
			}
			if (lo <= f0 + fullSpan * 0.001 && hi >= f1 - fullSpan * 0.001) {
				setZoomWindow(null);
			} else {
				setZoomWindow([lo, hi]);
			}
		},
		[fullExtent],
	);

	const zoomAtFrac = useCallback(
		(frac: number, factor: number) => {
			const [f0, f1] = fullExtent;
			const fullSpan = Math.max(1, f1 - f0);
			const [w0, w1] = zoomWindowRef.current ?? fullExtent;
			const span = w1 - w0;
			const cursorTime = w0 + frac * span;
			const minSpan = Math.max(MIN_SPAN_S, fullSpan * 0.002);
			const newSpan = Math.min(Math.max(span * factor, minSpan), fullSpan);
			let n0 = cursorTime - frac * newSpan;
			let n1 = n0 + newSpan;
			if (n0 < f0) {
				n0 = f0;
				n1 = f0 + newSpan;
			}
			if (n1 > f1) {
				n1 = f1;
				n0 = f1 - newSpan;
			}
			zoomToRange(n0, n1);
		},
		[fullExtent, zoomToRange],
	);

	const handleSelectionCommit = useCallback(
		(sel: ChartSelection) => {
			const data = chartDataRef.current;
			const a = data[sel.startIndex]?.elapsed;
			const b = data[sel.endIndex]?.elapsed;
			if (a == null || b == null) return;
			zoomToRange(a, b);
		},
		[zoomToRange],
	);

	const isFullscreen = fullscreen.isFullscreen;
	useEffect(() => {
		const el = chartWrapRef.current;
		if (!el) return;
		const onWheel = (e: WheelEvent) => {
			// Only hijack the wheel for zoom on an explicit gesture (ctrl/⌘, which
			// also covers trackpad pinch) or in fullscreen where there's no page
			// to scroll — otherwise let the wheel scroll the page past the chart.
			if (!(e.ctrlKey || e.metaKey || isFullscreen)) return;
			e.preventDefault();
			const rect = el.getBoundingClientRect();
			const frac = Math.min(
				1,
				Math.max(0, (e.clientX - rect.left) / Math.max(1, rect.width)),
			);
			zoomAtFrac(frac, e.deltaY > 0 ? 1.2 : 1 / 1.2);
		};
		el.addEventListener("wheel", onWheel, { passive: false });
		return () => el.removeEventListener("wheel", onWheel);
	}, [zoomAtFrac, isFullscreen]);

	const toggle = useCallback((key: SeriesKey) => {
		setActive((prev) => {
			const next = new Set(prev);
			if (next.has(key)) next.delete(key);
			else next.add(key);
			return next;
		});
	}, []);

	const tooltipRows = useCallback(
		(point: Record<string, unknown>): TooltipRow[] => {
			const rows: TooltipRow[] = [];
			if (active.has("temp")) {
				rows.push({
					color: SERIES_META.temp.color,
					label: "Current",
					value: `${(point.temp as number).toFixed(1)}°C`,
				});
			}
			if (active.has("target") && (point.target as number) > 0) {
				rows.push({
					color: SERIES_META.target.color,
					label: "Target",
					value: `${(point.target as number).toFixed(1)}°C`,
				});
			}
			if (active.has("ssr")) {
				rows.push({
					color: SERIES_META.ssr.color,
					label: "Heat",
					value: `${(point.ssr as number).toFixed(0)}%`,
				});
			}
			if (active.has("rate")) {
				rows.push({
					color: SERIES_META.rate.color,
					label: "Rate",
					value: `${(point.rate as number).toFixed(1)}°C/h`,
				});
			}
			return rows;
		},
		[active],
	);

	const margin = useMemo(
		() => ({
			top: 18,
			right: rightAxes.length > 0 ? rightAxes.length * RIGHT_AXIS_WIDTH : 16,
			bottom: 28,
			left: CHART_MARGIN_LEFT,
		}),
		[rightAxes.length],
	);

	const isZoomed = zoomWindow != null;

	return (
		<div
			className={cn(
				"flex flex-col gap-3",
				fullscreen.isFullscreen && "h-screen bg-background p-4",
			)}
			ref={containerRef}
		>
			<RunChartToolbar
				active={active}
				available={available}
				fullscreen={fullscreen}
				isZoomed={isZoomed}
				onReset={() => setZoomWindow(null)}
				onToggle={toggle}
				onZoomIn={() => zoomAtFrac(0.5, 1 / 1.6)}
				onZoomOut={() => zoomAtFrac(0.5, 1.6)}
			/>

			<div
				className={cn(
					"relative w-full",
					fullscreen.isFullscreen && "min-h-0 flex-1",
				)}
				ref={chartWrapRef}
			>
				<LineChart
					animationDuration={0}
					aspectRatio="auto"
					className={fullscreen.isFullscreen ? "h-full" : "h-80"}
					data={chartData}
					margin={margin}
					onSelectionCommit={handleSelectionCommit}
					xDataKey="t"
					yScaleDomainMax={model.plotMax}
				>
					<Grid
						highlightRowStroke={
							active.has("rate") ? "var(--chart-rate)" : undefined
						}
						highlightRowValues={
							active.has("rate") ? [model.rateToPlot(0)] : undefined
						}
						horizontal
					/>
					{active.has("error") ? (
						<KilnTrackingBand
							fill="var(--destructive)"
							opacity={0.12}
							targetEps={TARGET_EPS_C}
							targetKey="target"
							tempKey="temp"
						/>
					) : null}
					{active.has("ssr") ? (
						<Area
							curve={curveMonotoneX}
							dataKey="ssrPlot"
							fill="var(--chart-ssr)"
							fillOpacity={0.16}
							showHighlight={false}
							stroke="var(--chart-ssr)"
							strokeWidth={1.5}
						/>
					) : null}
					{active.has("rate") ? (
						<Line
							curve={curveMonotoneX}
							dataKey="ratePlot"
							fadeEdges={false}
							showHighlight={false}
							stroke="var(--chart-rate)"
							strokeWidth={1.5}
						/>
					) : null}
					{active.has("target") ? (
						<Line
							curve={curveMonotoneX}
							dataKey="target"
							fadeEdges={false}
							showHighlight={false}
							stroke="var(--muted-foreground)"
							strokeWidth={1.5}
						/>
					) : null}
					{active.has("temp") ? (
						<Line
							curve={curveMonotoneX}
							dataKey="temp"
							fadeEdges={false}
							showHighlight={false}
							stroke="var(--chart-heating)"
							strokeWidth={2.5}
						/>
					) : null}
					<KilnMarkers items={visibleMarkers} />
					<KilnSelectionOverlay />
					{showsTemperature ? (
						<YAxis formatValue={(v) => `${Math.round(v)}°`} label="°C" />
					) : null}
					{leftAxis ? (
						<KilnProjectedAxis
							color={leftAxis.color}
							domain={leftAxis.domain}
							format={leftAxis.format}
							label={leftAxis.label}
							mapToPlot={leftAxis.mapToPlot}
							side="left"
						/>
					) : null}
					{rightAxes.map((axis, i) => (
						<KilnProjectedAxis
							color={axis.color}
							domain={axis.domain}
							format={axis.format}
							inset={(rightAxes.length - 1 - i) * RIGHT_AXIS_WIDTH}
							key={axis.key}
							label={axis.label}
							mapToPlot={axis.mapToPlot}
							side="right"
							width={RIGHT_AXIS_WIDTH}
						/>
					))}
					<KilnTimeAxis />
					<ChartTooltip
						content={({ point }) => (
							<KilnTooltipContent point={point} rows={tooltipRows(point)} />
						)}
						showDatePill={false}
					/>
				</LineChart>
			</div>

			<RunChartMinimap
				data={minimapData}
				fullExtent={fullExtent}
				onWindowChange={setZoomWindow}
				plotMax={model.plotMax}
				window={zoomWindow}
			/>
		</div>
	);
}
