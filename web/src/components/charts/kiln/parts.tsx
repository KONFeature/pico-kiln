"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { elapsedSecondsToDate, formatElapsed } from "@/lib/chart-time";
import { useChartStable } from "../chart-context";
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
