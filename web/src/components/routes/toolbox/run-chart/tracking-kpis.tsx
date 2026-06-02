import { formatElapsed } from "@/lib/chart-time";
import type { RunStats } from "@/lib/run-stats";
import { cn } from "@/lib/utils";

type Accent = "default" | "good" | "warn" | "bad";

interface Kpi {
	label: string;
	value: string;
	sub?: string;
	accent?: Accent;
	description?: string;
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
		{
			label: "Peak temp",
			value: temp(stats.peakTempC),
			description: "Highest temperature the kiln reached during the run.",
		},
		{
			label: "Duration",
			value: formatElapsed(stats.durationS),
			description: "Total elapsed time from the first to the last log sample.",
		},
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
				description:
					"Largest amount the measured temperature rose above the target.",
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
				description:
					"Largest amount the measured temperature fell below the target.",
			});
		}
		if (stats.rmsErrorC != null) {
			kpis.push({
				label: "RMS error",
				value: temp(stats.rmsErrorC),
				accent: band(stats.rmsErrorC, 8, 20),
				description:
					"Root-mean-square deviation between measured and target temperature over the whole run — overall tracking tightness.",
			});
		}
		if (stats.steadyStateErrorC != null) {
			kpis.push({
				label: "Steady-state error",
				value: temp(stats.steadyStateErrorC),
				sub: "last 25% of hold",
				accent: band(stats.steadyStateErrorC, 3, 8),
				description:
					"Average distance from target over the final 25% of the longest hold, once the kiln has settled.",
			});
		}
		if (stats.settlingTimeS != null) {
			kpis.push({
				label: "Settling time",
				value: formatElapsed(stats.settlingTimeS),
				sub: "to ±5°C",
				description:
					"Time from the start of the longest hold until the temperature first reached within ±5°C of target.",
			});
		}
	}

	kpis.push({
		label: "Relay saturated",
		value: formatElapsed(stats.ssrSaturatedHighS),
		sub: `${(stats.ssrSaturatedHighPct * 100).toFixed(0)}% at full power`,
		accent: band(stats.ssrSaturatedHighPct, 0.25, 0.5),
		description:
			"Time the SSR was pinned at full power — the kiln could not heat any faster.",
	});

	kpis.push({
		label: "Avg heat",
		value: `${stats.avgSsrPercent.toFixed(0)}%`,
		description: "Average SSR duty cycle across the whole run.",
	});

	return kpis;
}

export function TrackingKpis({ stats }: { stats: RunStats }) {
	const kpis = buildKpis(stats);

	return (
		<div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
			{kpis.map((kpi) => (
				<div className="group relative" key={kpi.label}>
					<button
						aria-label={
							kpi.description
								? `${kpi.label}: ${kpi.value}. ${kpi.description}`
								: undefined
						}
						className={cn(
							"block h-full w-full rounded-lg border bg-card/50 px-3 py-2.5 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
							kpi.description && "cursor-help",
						)}
						type="button"
					>
						<span className="block text-muted-foreground text-xs">
							{kpi.label}
						</span>
						<span
							className={cn(
								"block font-semibold text-lg tabular-nums leading-tight",
								ACCENT_CLASS[kpi.accent ?? "default"],
							)}
						>
							{kpi.value}
						</span>
						{kpi.sub ? (
							<span className="block text-[11px] text-muted-foreground">
								{kpi.sub}
							</span>
						) : null}
					</button>
					{kpi.description ? (
						<div
							aria-hidden="true"
							className="pointer-events-none absolute bottom-full left-1/2 z-20 mb-1.5 w-max max-w-[14rem] -translate-x-1/2 rounded-md border bg-popover px-2.5 py-1.5 text-popover-foreground text-xs leading-snug opacity-0 shadow-md transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100"
						>
							{kpi.description}
						</div>
					) : null}
				</div>
			))}
		</div>
	);
}
