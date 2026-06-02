"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { elapsedSecondsToDate, formatElapsed } from "@/lib/chart-time";
import { useChartHover, useChartStable } from "../chart-context";
import { TooltipContent, type TooltipRow } from "../tooltip/tooltip-content";

/**
 * Elapsed-time X axis. bklit's own `<XAxis>` formats ticks as calendar dates;
 * kiln series are elapsed durations, so this renders evenly-spaced ticks
 * labelled with `formatElapsed` (e.g. "1.5h", "12m") into the chart container.
 */
export function KilnTimeAxis({ numTicks = 5 }: { numTicks?: number }) {
	const { containerRef } = useChartStable();
	const [mounted, setMounted] = useState(false);

	useEffect(() => {
		setMounted(true);
	}, []);

	const container = containerRef.current;
	if (!(mounted && container)) {
		return null;
	}

	return <KilnTimeAxisInner container={container} numTicks={numTicks} />;
}

function KilnTimeAxisInner({
	numTicks,
	container,
}: {
	numTicks: number;
	container: HTMLDivElement;
}) {
	const { xScale, margin } = useChartStable();

	const labels = useMemo(() => {
		const [start, end] = xScale.domain();
		if (!(start && end)) {
			return [];
		}
		const startT = start.getTime();
		const endT = end.getTime();
		const ticks = Math.max(2, numTicks);
		return Array.from({ length: ticks }, (_, i) => {
			const t = startT + (i / (ticks - 1)) * (endT - startT);
			return {
				key: i,
				x: (xScale(new Date(t)) ?? 0) + margin.left,
				label: formatElapsed(t / 1000),
			};
		});
	}, [xScale, margin.left, numTicks]);

	return createPortal(
		<div className="pointer-events-none absolute inset-0">
			{labels.map((l) => (
				<div
					className="absolute flex justify-center"
					key={l.key}
					style={{ left: l.x, bottom: 4, width: 0 }}
				>
					<span className="whitespace-nowrap text-chart-label text-xs">
						{l.label}
					</span>
				</div>
			))}
		</div>,
		container,
	);
}

export interface KilnMarker {
	atSeconds: number;
	label?: string;
	color?: string;
}

/**
 * Vertical event markers (step changes, phase boundaries) drawn in chart space.
 * Render as a chart child so it inherits the plot transform and scales.
 */
export function KilnMarkers({ items }: { items: KilnMarker[] }) {
	const { xScale, innerHeight } = useChartStable();

	return (
		<g className="kiln-markers">
			{items.map((m, i) => {
				const x = xScale(elapsedSecondsToDate(m.atSeconds)) ?? 0;
				const color = m.color ?? "var(--chart-grid)";
				return (
					<g key={`${m.atSeconds}-${i}`}>
						<line
							stroke={color}
							strokeDasharray="3,3"
							strokeOpacity={0.6}
							x1={x}
							x2={x}
							y1={0}
							y2={innerHeight}
						/>
						{m.label ? (
							<text
								fill="var(--chart-label)"
								fontSize={10}
								textAnchor="middle"
								x={x}
								y={-3}
							>
								{m.label}
							</text>
						) : null}
					</g>
				);
			})}
		</g>
	);
}

export interface KilnRegion {
	startSeconds: number;
	endSeconds: number;
	color: string;
	opacity?: number;
}

/**
 * Shaded background bands (e.g. detected tuning phases). Render this *before*
 * the series children so the bands sit behind the lines.
 */
export function KilnRegions({ items }: { items: KilnRegion[] }) {
	const { xScale, innerHeight } = useChartStable();

	return (
		<g className="kiln-regions">
			{items.map((r, i) => {
				const x1 = xScale(elapsedSecondsToDate(r.startSeconds)) ?? 0;
				const x2 = xScale(elapsedSecondsToDate(r.endSeconds)) ?? 0;
				return (
					<rect
						fill={r.color}
						height={innerHeight}
						key={`${r.startSeconds}-${i}`}
						opacity={r.opacity ?? 0.15}
						width={Math.max(0, x2 - x1)}
						x={x1}
						y={0}
					/>
				);
			})}
		</g>
	);
}

/** Tooltip body with an elapsed-time title (bklit's title formatter is date-only). */
export function KilnTooltipContent({
	point,
	rows,
}: {
	point: Record<string, unknown>;
	rows: TooltipRow[];
}) {
	const t = point.t;
	const title =
		t instanceof Date ? formatElapsed(t.getTime() / 1000) : undefined;
	return <TooltipContent rows={rows} title={title} />;
}

export interface KilnProjectedAxisProps {
	domain: [number, number];
	/**
	 * Maps a real-unit axis value (e.g. an SSR percentage or a °C/h rate) into
	 * the chart's single plotted y-value space, so the shared `yScale` positions
	 * the tick. Must be the inverse of how the series was projected onto the
	 * temperature axis.
	 */
	mapToPlot: (value: number) => number;
	format: (value: number) => string;
	numTicks?: number;
	color?: string;
	side?: "left" | "right";
	/** Offset in px from `side`'s edge so two axes can stack in one gutter. */
	inset?: number;
	width?: number;
}

export function KilnProjectedAxis(props: KilnProjectedAxisProps) {
	const { containerRef } = useChartStable();
	const [mounted, setMounted] = useState(false);

	useEffect(() => {
		setMounted(true);
	}, []);

	const container = containerRef.current;
	if (!(mounted && container)) {
		return null;
	}

	return <KilnProjectedAxisInner {...props} container={container} />;
}

function KilnProjectedAxisInner({
	domain,
	mapToPlot,
	format,
	numTicks = 5,
	color,
	side = "right",
	inset = 0,
	width,
	container,
}: KilnProjectedAxisProps & { container: HTMLDivElement }) {
	const { yScale, margin } = useChartStable();
	const [lo, hi] = domain;
	const isRight = side === "right";
	const boxWidth = width ?? (isRight ? margin.right : margin.left);

	const ticks = useMemo(() => {
		const n = Math.max(2, numTicks);
		return Array.from({ length: n }, (_, i) => {
			const value = lo + (i / (n - 1)) * (hi - lo);
			return {
				key: i,
				y: (yScale(mapToPlot(value)) ?? 0) + margin.top,
				label: format(value),
			};
		});
	}, [lo, hi, numTicks, yScale, mapToPlot, format, margin.top]);

	return createPortal(
		<div
			className="pointer-events-none absolute top-0 bottom-0"
			style={{ [isRight ? "right" : "left"]: inset, width: boxWidth }}
		>
			{ticks.map((tick) => (
				<div
					className={
						isRight
							? "absolute left-0 flex items-center justify-start pl-2"
							: "absolute right-0 flex items-center justify-end pr-2"
					}
					key={tick.key}
					style={{ top: tick.y, transform: "translateY(-50%)" }}
				>
					<span
						className="text-xs"
						style={{ color: color ?? "var(--chart-label)" }}
					>
						{tick.label}
					</span>
				</div>
			))}
		</div>,
		container,
	);
}

export function KilnSelectionOverlay() {
	const { selection } = useChartHover();
	const { innerHeight } = useChartStable();

	if (!selection?.active) {
		return null;
	}

	const x = Math.min(selection.startX, selection.endX);
	const width = Math.abs(selection.endX - selection.startX);

	return (
		<g className="kiln-selection" pointerEvents="none">
			<rect
				fill="var(--chart-crosshair)"
				fillOpacity={0.1}
				height={innerHeight}
				width={width}
				x={x}
				y={0}
			/>
			<line
				stroke="var(--chart-crosshair)"
				strokeOpacity={0.5}
				x1={x}
				x2={x}
				y1={0}
				y2={innerHeight}
			/>
			<line
				stroke="var(--chart-crosshair)"
				strokeOpacity={0.5}
				x1={x + width}
				x2={x + width}
				y1={0}
				y2={innerHeight}
			/>
		</g>
	);
}

export interface KilnTrackingBandProps {
	tempKey: string;
	targetKey: string;
	fill?: string;
	opacity?: number;
	/** Targets at or below this collapse the band, so inactive stretches stay flat. */
	targetEps?: number;
}

export function KilnTrackingBand({
	tempKey,
	targetKey,
	fill = "var(--chart-rate)",
	opacity = 0.14,
	targetEps = 0,
}: KilnTrackingBandProps) {
	const { renderData, xScale, yScale, xAccessor } = useChartStable();

	const d = useMemo(() => {
		const top: string[] = [];
		const bottom: string[] = [];
		for (const p of renderData) {
			const temp = p[tempKey];
			const target = p[targetKey];
			if (typeof temp !== "number" || typeof target !== "number") {
				continue;
			}
			const x = xScale(xAccessor(p)) ?? 0;
			const tempY = yScale(temp) ?? 0;
			const targetY = target > targetEps ? (yScale(target) ?? 0) : tempY;
			top.push(`${x},${tempY}`);
			bottom.push(`${x},${targetY}`);
		}
		if (top.length < 2) {
			return "";
		}
		return `M${top.join("L")}L${bottom.reverse().join("L")}Z`;
	}, [renderData, xScale, yScale, xAccessor, tempKey, targetKey, targetEps]);

	if (!d) {
		return null;
	}

	return (
		<path className="kiln-tracking-band" d={d} fill={fill} opacity={opacity} />
	);
}
