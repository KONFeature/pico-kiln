import { cn } from "@/lib/utils";

interface HeatOutputGaugeProps {
	value?: number;
}

const CENTER = 60;
const RADIUS = 50;
const START_ANGLE = 225;
const SWEEP = 270;

function polarToCartesian(angleDeg: number): [number, number] {
	const rad = ((angleDeg - 90) * Math.PI) / 180;
	return [CENTER + RADIUS * Math.cos(rad), CENTER + RADIUS * Math.sin(rad)];
}

function arcPath(fromAngle: number, toAngle: number): string {
	const [sx, sy] = polarToCartesian(fromAngle);
	const [ex, ey] = polarToCartesian(toAngle);
	const largeArc = toAngle - fromAngle <= 180 ? 0 : 1;
	return `M ${sx} ${sy} A ${RADIUS} ${RADIUS} 0 ${largeArc} 1 ${ex} ${ey}`;
}

const ARC = arcPath(START_ANGLE, START_ANGLE + SWEEP);

/**
 * Radial gauge for SSR duty cycle. 0-100% is the gauge's natural domain, so a
 * high-temp lag that pegs the element near 100% reads at a glance.
 */
export function HeatOutputGauge({ value }: HeatOutputGaugeProps) {
	const pct = typeof value === "number" ? Math.min(Math.max(value, 0), 100) : 0;
	const progress = pct / 100;
	const accent = pct > 0 ? "text-chart-ssr" : "text-muted-foreground";

	return (
		<div className="relative aspect-square w-28 shrink-0 sm:w-32">
			<svg
				viewBox="0 0 120 120"
				className={cn("h-full w-full", accent)}
				role="img"
				aria-label={`Heat output ${pct.toFixed(0)} percent`}
			>
				<path
					d={ARC}
					fill="none"
					stroke="var(--muted)"
					strokeWidth={10}
					strokeLinecap="round"
				/>
				<path
					d={ARC}
					fill="none"
					stroke="currentColor"
					strokeWidth={10}
					strokeLinecap="round"
					pathLength={1}
					strokeDasharray={1}
					strokeDashoffset={1 - progress}
					className="transition-[stroke-dashoffset] duration-700 ease-out"
				/>
			</svg>
			<div className="absolute inset-0 flex flex-col items-center justify-center text-center">
				<div className={cn("font-bold leading-none tabular-nums", accent)}>
					<span className="text-3xl">{pct.toFixed(0)}</span>
					<span className="align-top text-base">%</span>
				</div>
				<div className="mt-0.5 text-[0.7rem] font-medium text-muted-foreground">
					Heat output
				</div>
			</div>
		</div>
	);
}
