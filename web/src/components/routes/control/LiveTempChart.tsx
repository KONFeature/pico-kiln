import { curveMonotoneX } from "@visx/curve";
import { useEffect, useState } from "react";
import { Line } from "@/components/charts/line";
import { LineChart } from "@/components/charts/line-chart";
import { ChartTooltip } from "@/components/charts/tooltip";
import { TooltipContent } from "@/components/charts/tooltip/tooltip-content";
import type { KilnStatus } from "@/lib/pico/types";

export interface TempSample {
	t: number;
	temp: number;
	target: number | null;
}

/**
 * Accumulates a rolling, in-memory window of temperature readings keyed by the
 * query's `updatedAt` so each successful poll appends at most one sample. Lives
 * only while mounted (no persistence) — it's a live glance, not a log.
 */
export function useTemperatureHistory(
	status: KilnStatus | undefined,
	updatedAt: number | undefined,
	max = 120,
): TempSample[] {
	const [history, setHistory] = useState<TempSample[]>([]);

	useEffect(() => {
		if (!status || updatedAt === undefined) return;
		setHistory((prev) => {
			const last = prev[prev.length - 1];
			if (last && last.t === updatedAt) return prev;
			const next = [
				...prev,
				{
					t: updatedAt,
					temp: status.current_temp,
					// 0 = idle / natural cooling (SSR off), not a real setpoint to plot.
					target:
						status.target_temp && status.target_temp > 0
							? status.target_temp
							: null,
				},
			];
			return next.length > max ? next.slice(next.length - max) : next;
		});
	}, [status, updatedAt, max]);

	return history;
}

interface LiveTempChartProps {
	data: TempSample[];
	unit?: string;
}

interface LiveChartRow {
	t: Date;
	temp: number;
	target?: number;
	[key: string]: unknown;
}

export function LiveTempChart({ data, unit = "°C" }: LiveTempChartProps) {
	if (data.length < 2) {
		return (
			<div className="flex h-28 items-center justify-center rounded-md bg-muted/30 text-xs text-muted-foreground">
				Collecting live data…
			</div>
		);
	}

	const rows: LiveChartRow[] = data.map((d) => ({
		t: new Date(d.t),
		temp: d.temp,
		...(d.target !== null ? { target: d.target } : {}),
	}));
	const targetRows = rows.filter((r) => typeof r.target === "number");
	const hasTarget = targetRows.length > 1;

	return (
		<LineChart
			data={rows}
			xDataKey="t"
			aspectRatio="auto"
			className="h-28"
			style={{ touchAction: "pan-y" }}
			margin={{ top: 8, right: 8, bottom: 8, left: 8 }}
			animationDuration={0}
		>
			{hasTarget && (
				<Line
					data={targetRows}
					dataKey="target"
					stroke="var(--muted-foreground)"
					strokeWidth={1.5}
					curve={curveMonotoneX}
					fadeEdges={false}
					showHighlight={false}
				/>
			)}
			<Line
				dataKey="temp"
				stroke="var(--chart-heating)"
				strokeWidth={2}
				curve={curveMonotoneX}
				fadeEdges={false}
				showHighlight={false}
			/>
			<ChartTooltip
				showDatePill={false}
				content={({ point }) => {
					const t = point.t;
					const title = t instanceof Date ? t.toLocaleTimeString() : undefined;
					const rowsOut = [
						{
							color: "var(--chart-heating)",
							label: "Current",
							value: `${(point.temp as number).toFixed(1)}${unit}`,
						},
					];
					if (typeof point.target === "number") {
						rowsOut.push({
							color: "var(--muted-foreground)",
							label: "Target",
							value: `${point.target.toFixed(1)}${unit}`,
						});
					}
					return <TooltipContent rows={rowsOut} title={title} />;
				}}
			/>
		</LineChart>
	);
}
