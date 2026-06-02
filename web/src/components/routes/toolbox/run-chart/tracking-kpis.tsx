import { formatElapsed } from "@/lib/chart-time";
import type { RunStats } from "@/lib/run-stats";
import { cn } from "@/lib/utils";

type Accent = "default" | "good" | "warn" | "bad";

interface Kpi {
	label: string;
	value: string;
	sub?: string;
	accent?: Accent;
}

const ACCENT_CLASS: Record<Accent, string> = {
	default: "text-foreground",
	good: "text-emerald-500",
	warn: "text-amber-500",
	bad: "text-destructive",
};

function temp(value: number): string {
	return `${value.toFixed(1)}°C`;
}

function band(value: number, warn: number, bad: number): Accent {
	if (value >= bad) return "bad";
	if (value >= warn) return "warn";
	return "good";
}

function buildKpis(stats: RunStats): Kpi[] {
	const kpis: Kpi[] = [
		{ label: "Peak temp", value: temp(stats.peakTempC) },
		{ label: "Duration", value: formatElapsed(stats.durationS) },
	];

	if (stats.hasTracking) {
		if (stats.maxOvershootC != null) {
			kpis.push({
				label: "Max overshoot",
				value: `+${stats.maxOvershootC.toFixed(1)}°C`,
				sub:
					stats.maxOvershootAtS != null
						? `at ${formatElapsed(stats.maxOvershootAtS)}`
						: undefined,
				accent: band(stats.maxOvershootC, 8, 20),
			});
		}
		if (stats.maxLagC != null) {
			kpis.push({
				label: "Max lag",
				value: `-${stats.maxLagC.toFixed(1)}°C`,
				sub:
					stats.maxLagAtS != null
						? `at ${formatElapsed(stats.maxLagAtS)}`
						: undefined,
				accent: band(stats.maxLagC, 15, 40),
			});
		}
		if (stats.rmsErrorC != null) {
			kpis.push({
				label: "RMS error",
				value: temp(stats.rmsErrorC),
				accent: band(stats.rmsErrorC, 8, 20),
			});
		}
		if (stats.steadyStateErrorC != null) {
			kpis.push({
				label: "Steady-state error",
				value: temp(stats.steadyStateErrorC),
				sub: "last 25% of hold",
				accent: band(stats.steadyStateErrorC, 3, 8),
			});
		}
		if (stats.settlingTimeS != null) {
			kpis.push({
				label: "Settling time",
				value: formatElapsed(stats.settlingTimeS),
				sub: "to ±5°C",
			});
		}
	}

	kpis.push({
		label: "Relay saturated",
		value: formatElapsed(stats.ssrSaturatedHighS),
		sub: `${(stats.ssrSaturatedHighPct * 100).toFixed(0)}% at full power`,
		accent: band(stats.ssrSaturatedHighPct, 0.25, 0.5),
	});

	kpis.push({ label: "Avg heat", value: `${stats.avgSsrPercent.toFixed(0)}%` });

	return kpis;
}

export function TrackingKpis({ stats }: { stats: RunStats }) {
	const kpis = buildKpis(stats);

	return (
		<div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
			{kpis.map((kpi) => (
				<div
					className="rounded-lg border bg-card/50 px-3 py-2.5"
					key={kpi.label}
				>
					<div className="text-muted-foreground text-xs">{kpi.label}</div>
					<div
						className={cn(
							"font-semibold text-lg tabular-nums leading-tight",
							ACCENT_CLASS[kpi.accent ?? "default"],
						)}
					>
						{kpi.value}
					</div>
					{kpi.sub ? (
						<div className="text-[11px] text-muted-foreground">{kpi.sub}</div>
					) : null}
				</div>
			))}
		</div>
	);
}
