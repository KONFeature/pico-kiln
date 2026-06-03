import { Maximize, Minimize, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { SERIES_META, type SeriesKey } from "./types";
import type { FullscreenLandscape } from "./use-fullscreen-landscape";

interface RunChartToolbarProps {
	available: SeriesKey[];
	active: Set<SeriesKey>;
	onToggle: (key: SeriesKey) => void;
	isZoomed: boolean;
	onZoomIn: () => void;
	onZoomOut: () => void;
	onReset: () => void;
	fullscreen: FullscreenLandscape;
}

export function RunChartToolbar({
	available,
	active,
	onToggle,
	isZoomed,
	onZoomIn,
	onZoomOut,
	onReset,
	fullscreen,
}: RunChartToolbarProps) {
	return (
		<div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
			<div className="flex flex-wrap items-center gap-1.5">
				{available.map((key) => {
					const meta = SERIES_META[key];
					const isActive = active.has(key);
					return (
						<button
							aria-pressed={isActive}
							className={cn(
								"inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 font-medium text-xs transition-colors",
								isActive
									? "border-transparent bg-secondary text-secondary-foreground"
									: "border-border text-muted-foreground hover:bg-accent",
							)}
							key={key}
							onClick={() => onToggle(key)}
							type="button"
						>
							<span
								className="size-2 rounded-full"
								style={{
									backgroundColor: meta.color,
									opacity: isActive ? 1 : 0.4,
								}}
							/>
							{meta.label}
						</button>
					);
				})}
			</div>

			<div className="flex items-center gap-1">
				<Button
					aria-label="Zoom out"
					disabled={!isZoomed}
					onClick={onZoomOut}
					size="icon-sm"
					variant="ghost"
				>
					<ZoomOut />
				</Button>
				<Button
					aria-label="Zoom in"
					onClick={onZoomIn}
					size="icon-sm"
					variant="ghost"
				>
					<ZoomIn />
				</Button>
				<Button
					aria-label="Reset zoom"
					disabled={!isZoomed}
					onClick={onReset}
					size="icon-sm"
					variant="ghost"
				>
					<RotateCcw />
				</Button>
				{fullscreen.supported ? (
					<Button
						aria-label={
							fullscreen.isFullscreen ? "Exit fullscreen" : "Fullscreen"
						}
						onClick={fullscreen.toggle}
						size="icon-sm"
						variant="ghost"
					>
						{fullscreen.isFullscreen ? <Minimize /> : <Maximize />}
					</Button>
				) : null}
			</div>
		</div>
	);
}
