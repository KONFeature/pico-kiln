import { cn } from "@/lib/utils";

interface TemperatureGaugeProps {
	current: number;
	target?: number;
	unit?: string;
	/** Tailwind text-color class driving the arc + value color (currentColor). */
	accentClassName?: string;
	/** Short status word shown under the value, e.g. "Heating". */
	caption?: string;
}

const CENTER = 60;
const RADIUS = 50;
const START_ANGLE = 225;
const SWEEP = 270;

function polarToCartesian(angleDeg: number): [number, number] {
	const rad = ((angleDeg - 90) * Math.PI) / 180;
	return [CENTER + RADIUS * Math.cos(rad), CENTER + RADIUS * Math.sin(rad)];
}

// Drawn in the increasing-angle direction (sweep = 1) so the progress dash
// reveals from the start of the arc (bottom-left) as it fills.
function arcPath(fromAngle: number, toAngle: number): string {
	const [sx, sy] = polarToCartesian(fromAngle);
	const [ex, ey] = polarToCartesian(toAngle);
	const largeArc = toAngle - fromAngle <= 180 ? 0 : 1;
	return `M ${sx} ${sy} A ${RADIUS} ${RADIUS} 0 ${largeArc} 1 ${ex} ${ey}`;
}

const ARC = arcPath(START_ANGLE, START_ANGLE + SWEEP);

/**
 * Hero temperature readout: a 270° progress-to-target ring with the current
 * temperature reading at its center. The arc fills toward `target`; with no
 * target (idle) only the value is shown.
 */
export function TemperatureGauge({
	current,
	target,
	unit = "°C",
	accentClassName = "text-primary",
	caption,
}: TemperatureGaugeProps) {
	const rawProgress =
		target && target > 0 ? Math.min(Math.max(current / target, 0), 1) : 0;
	const progress = Number.isFinite(rawProgress) ? rawProgress : 0;

	return (
		<div className="relative mx-auto aspect-square w-44 sm:w-48">
			<svg
				viewBox="0 0 120 120"
				className={cn("h-full w-full", accentClassName)}
				role="img"
				aria-label={`Current temperature ${current.toFixed(0)}${unit}${
					target !== undefined ? ` of ${target.toFixed(0)}${unit}` : ""
				}`}
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
				<div
					className={cn("font-bold leading-none tabular-nums", accentClassName)}
				>
					<span className="text-5xl">{current.toFixed(0)}</span>
					<span className="align-top text-xl">{unit}</span>
				</div>
				{target !== undefined && (
					<div className="mt-1 text-xs text-muted-foreground tabular-nums">
						of {target.toFixed(0)}
						{unit}
					</div>
				)}
				{caption && (
					<div className="mt-0.5 text-xs font-medium text-muted-foreground">
						{caption}
					</div>
				)}
			</div>
		</div>
	);
}
