import { useEffect, useState } from "react";
import {
	Line,
	LineChart,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
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

export function LiveTempChart({ data, unit = "°C" }: LiveTempChartProps) {
	if (data.length < 2) {
		return (
			<div className="flex h-28 items-center justify-center rounded-md bg-muted/30 text-xs text-muted-foreground">
				Collecting live data…
			</div>
		);
	}

	const hasTarget = data.some((d) => d.target !== null);

	return (
		<ResponsiveContainer width="100%" height={112}>
			<LineChart data={data} margin={{ top: 6, right: 6, left: 6, bottom: 0 }}>
				<XAxis dataKey="t" type="number" domain={["dataMin", "dataMax"]} hide />
				<YAxis
					domain={["dataMin - 10", "dataMax + 10"]}
					hide
					allowDecimals={false}
				/>
				<Tooltip
					contentStyle={{
						background: "var(--popover)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-md)",
						fontSize: "0.75rem",
						color: "var(--popover-foreground)",
					}}
					labelFormatter={(label) =>
						new Date(Number(label)).toLocaleTimeString()
					}
					formatter={(value, name) => [
						`${Number(value).toFixed(1)}${unit}`,
						name === "temp" ? "Current" : "Target",
					]}
				/>
				{hasTarget && (
					<Line
						type="monotone"
						dataKey="target"
						stroke="var(--muted-foreground)"
						strokeWidth={1.5}
						strokeDasharray="4 4"
						dot={false}
						isAnimationActive={false}
						connectNulls
					/>
				)}
				<Line
					type="monotone"
					dataKey="temp"
					stroke="var(--chart-heating)"
					strokeWidth={2}
					dot={false}
					isAnimationActive={false}
				/>
			</LineChart>
		</ResponsiveContainer>
	);
}
