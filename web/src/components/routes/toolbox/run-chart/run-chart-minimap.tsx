import {
	type PointerEvent as ReactPointerEvent,
	useCallback,
	useMemo,
	useRef,
} from "react";
import type { RunChartPoint } from "@/lib/run-series";

const VB_W = 1000;
const VB_H = 100;
const MIN_SPAN_S = 30;

type DragMode = "create" | "move" | "resize-left" | "resize-right";

interface DragState {
	mode: DragMode;
	pointerId: number;
	grabFrac: number;
	left: number;
	right: number;
}

interface RunChartMinimapProps {
	data: RunChartPoint[];
	plotMax: number;
	fullExtent: readonly [number, number];
	window: readonly [number, number] | null;
	onWindowChange: (window: [number, number] | null) => void;
}

export function RunChartMinimap({
	data,
	plotMax,
	fullExtent,
	window: win,
	onWindowChange,
}: RunChartMinimapProps) {
	const trackRef = useRef<HTMLDivElement>(null);
	const dragRef = useRef<DragState | null>(null);
	const [full0, full1] = fullExtent;
	const fullSpan = Math.max(1, full1 - full0);

	const paths = useMemo(() => {
		if (data.length < 2) return { line: "", area: "" };
		const pts = data.map((p) => {
			const x = ((p.elapsed - full0) / fullSpan) * VB_W;
			const y = VB_H - Math.min(1, Math.max(0, p.temp / plotMax)) * VB_H;
			return `${x.toFixed(1)},${y.toFixed(1)}`;
		});
		const line = `M${pts.join("L")}`;
		return { line, area: `${line}L${VB_W},${VB_H}L0,${VB_H}Z` };
	}, [data, full0, fullSpan, plotMax]);

	const leftFrac = win ? (win[0] - full0) / fullSpan : 0;
	const rightFrac = win ? (win[1] - full0) / fullSpan : 1;
	const minGap = Math.max(0.004, MIN_SPAN_S / fullSpan);

	const clientXToFrac = useCallback((clientX: number) => {
		const rect = trackRef.current?.getBoundingClientRect();
		if (!rect || rect.width === 0) return 0;
		return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
	}, []);

	const apply = useCallback(
		(l: number, r: number) => {
			const left = Math.min(l, r);
			const right = Math.max(l, r);
			if (left <= 0.002 && right >= 0.998) {
				onWindowChange(null);
				return;
			}
			onWindowChange([full0 + left * fullSpan, full0 + right * fullSpan]);
		},
		[full0, fullSpan, onWindowChange],
	);

	const onPointerDown = useCallback(
		(e: ReactPointerEvent<HTMLDivElement>) => {
			const role = (e.target as HTMLElement).dataset.role as
				| DragMode
				| undefined;
			const frac = clientXToFrac(e.clientX);
			dragRef.current = {
				mode: role ?? "create",
				pointerId: e.pointerId,
				grabFrac: frac,
				left: leftFrac,
				right: rightFrac,
			};
			trackRef.current?.setPointerCapture(e.pointerId);
		},
		[clientXToFrac, leftFrac, rightFrac],
	);

	const onPointerMove = useCallback(
		(e: ReactPointerEvent<HTMLDivElement>) => {
			const drag = dragRef.current;
			if (!drag || drag.pointerId !== e.pointerId) return;
			const frac = clientXToFrac(e.clientX);
			if (drag.mode === "create") {
				apply(drag.grabFrac, frac);
			} else if (drag.mode === "move") {
				const width = drag.right - drag.left;
				const l = Math.min(
					Math.max(0, drag.left + (frac - drag.grabFrac)),
					1 - width,
				);
				apply(l, l + width);
			} else if (drag.mode === "resize-left") {
				apply(Math.min(frac, drag.right - minGap), drag.right);
			} else {
				apply(drag.left, Math.max(frac, drag.left + minGap));
			}
		},
		[apply, clientXToFrac, minGap],
	);

	const onPointerUp = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
		if (dragRef.current?.pointerId === e.pointerId) {
			dragRef.current = null;
			trackRef.current?.releasePointerCapture(e.pointerId);
		}
	}, []);

	return (
		<div
			className="relative h-14 w-full cursor-crosshair select-none overflow-hidden rounded-md border bg-card/40"
			onPointerDown={onPointerDown}
			onPointerMove={onPointerMove}
			onPointerUp={onPointerUp}
			ref={trackRef}
			style={{ touchAction: "none" }}
		>
			<svg
				aria-hidden="true"
				className="absolute inset-0 h-full w-full"
				preserveAspectRatio="none"
				viewBox={`0 0 ${VB_W} ${VB_H}`}
			>
				<path d={paths.area} fill="var(--chart-heating)" opacity={0.12} />
				<path
					d={paths.line}
					fill="none"
					stroke="var(--chart-heating)"
					strokeWidth={1.5}
					vectorEffect="non-scaling-stroke"
				/>
			</svg>

			<div
				className="pointer-events-none absolute inset-y-0 left-0 bg-background/60"
				style={{ width: `${leftFrac * 100}%` }}
			/>
			<div
				className="pointer-events-none absolute inset-y-0 right-0 bg-background/60"
				style={{ width: `${(1 - rightFrac) * 100}%` }}
			/>

			<div
				className="absolute inset-y-0 cursor-grab border-primary/70 border-x-2 bg-primary/5"
				data-role="move"
				style={{
					left: `${leftFrac * 100}%`,
					width: `${(rightFrac - leftFrac) * 100}%`,
				}}
			>
				<span
					className="-translate-x-1/2 absolute inset-y-0 left-0 w-3 cursor-ew-resize"
					data-role="resize-left"
				/>
				<span
					className="absolute inset-y-0 right-0 w-3 translate-x-1/2 cursor-ew-resize"
					data-role="resize-right"
				/>
			</div>
		</div>
	);
}
