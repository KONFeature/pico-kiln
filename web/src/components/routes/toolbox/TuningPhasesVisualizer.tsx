import { curveMonotoneX } from "@visx/curve";
import { CircleAlert } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Area } from "@/components/charts/area";
import { AreaChart } from "@/components/charts/area-chart";
import { Grid } from "@/components/charts/grid";
import {
	type KilnRegion,
	KilnRegions,
	KilnTimeAxis,
	KilnTooltipContent,
} from "@/components/charts/kiln/parts";
import { Line } from "@/components/charts/line";
import { LineChart } from "@/components/charts/line-chart";
import { ChartTooltip } from "@/components/charts/tooltip";
import { YAxis } from "@/components/charts/y-axis";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { elapsedSecondsToDate } from "@/lib/chart-time";
import {
	type LogDataPoint,
	lttbDownsample,
	parseLogCSV,
	secondsToMinutes,
} from "@/lib/csv-parser";
import { FileSourceSelector } from "./FileSourceSelector";

type PhaseType = "heating" | "cooling" | "plateau";

interface Phase {
	startIdx: number;
	endIdx: number;
	type: PhaseType;
	avgSsr: number;
	tempStart: number;
	tempEnd: number;
	stepName?: string;
}

interface TuningChartPoint {
	t: Date;
	temp: number;
	ssr: number;
	[key: string]: unknown;
}

const MAX_CHART_POINTS = 800;
const CHART_MARGIN = { top: 14, right: 16, bottom: 28, left: 44 };

/**
 * Detect tuning phases using physics-based detection:
 * - COOLING: SSR < 5% (natural cooling)
 * - HEATING: SSR ≥ 5% AND temperature rising (>0.5°C/min)
 * - PLATEAU: SSR ≥ 5% AND temperature stable (±0.5°C/min)
 */
function detectPhases(data: LogDataPoint[]): Phase[] {
	const phases: Phase[] = [];
	const SSR_THRESHOLD = 5;
	const RATE_THRESHOLD = 0.5;
	const WINDOW_SIZE = 10;

	let currentPhase: Phase | null = null;

	for (let i = 0; i < data.length; i++) {
		const point = data[i];
		const ssr = point.ssr_output_percent;

		let rate = 0;
		if (i >= WINDOW_SIZE) {
			const timeDiff =
				(data[i].elapsed_seconds - data[i - WINDOW_SIZE].elapsed_seconds) / 60;
			const tempDiff =
				data[i].current_temp_c - data[i - WINDOW_SIZE].current_temp_c;
			rate = timeDiff > 0 ? tempDiff / timeDiff : 0;
		}

		let phaseType: PhaseType;
		if (ssr < SSR_THRESHOLD) {
			phaseType = "cooling";
		} else if (rate > RATE_THRESHOLD) {
			phaseType = "heating";
		} else {
			phaseType = "plateau";
		}

		if (!currentPhase || currentPhase.type !== phaseType) {
			if (currentPhase) {
				currentPhase.endIdx = i - 1;
				currentPhase.tempEnd = data[i - 1].current_temp_c;
				phases.push(currentPhase);
			}

			currentPhase = {
				startIdx: i,
				endIdx: i,
				type: phaseType,
				avgSsr: ssr,
				tempStart: point.current_temp_c,
				tempEnd: point.current_temp_c,
				stepName: point.step_name,
			};
		} else {
			const count = i - currentPhase.startIdx + 1;
			currentPhase.avgSsr = (currentPhase.avgSsr * (count - 1) + ssr) / count;
		}
	}

	if (currentPhase) {
		currentPhase.endIdx = data.length - 1;
		currentPhase.tempEnd = data[data.length - 1].current_temp_c;
		phases.push(currentPhase);
	}

	return phases;
}

// Natural cooling is cyan everywhere (matches the profile editor / visualizer).
const PHASE_COLORS: Record<PhaseType, string> = {
	heating: "var(--chart-heating)",
	cooling: "var(--chart-natural-cooling)",
	plateau: "var(--chart-hold)",
};

const PHASE_TINT: Record<PhaseType, string> = {
	heating: "bg-chart-heating/15 border-chart-heating/40",
	cooling: "bg-chart-natural-cooling/15 border-chart-natural-cooling/40",
	plateau: "bg-chart-hold/15 border-chart-hold/40",
};

const PHASE_LABELS: Record<PhaseType, string> = {
	heating: "Heating",
	cooling: "Cooling",
	plateau: "Plateau",
};

export function TuningPhasesVisualizer() {
	const [logData, setLogData] = useState<LogDataPoint[] | null>(null);
	const [error, setError] = useState<string | null>(null);

	const handleFileSelected = useCallback((content: string) => {
		try {
			const { data } = parseLogCSV(content);

			if (data.length === 0) {
				throw new Error("No valid data points in log file");
			}

			const isTuning = data.some((d) => d.state === "TUNING");
			if (!isTuning) {
				console.warn("This log file may not be from a tuning run");
			}

			setLogData(data);
			setError(null);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to parse log file");
			setLogData(null);
		}
	}, []);

	const phases = useMemo<Phase[]>(() => {
		if (!logData) return [];
		return detectPhases(logData);
	}, [logData]);

	const chartData = useMemo<TuningChartPoint[]>(() => {
		if (!logData) return [];
		const sampled = lttbDownsample(
			logData,
			MAX_CHART_POINTS,
			(p) => p.elapsed_seconds,
			(p) => p.current_temp_c,
		);
		return sampled.map((point) => ({
			t: elapsedSecondsToDate(point.elapsed_seconds),
			temp: point.current_temp_c,
			ssr: point.ssr_output_percent,
		}));
	}, [logData]);

	const phaseRegions = useMemo<KilnRegion[]>(() => {
		if (!logData) return [];
		return phases.map((phase) => ({
			startSeconds: logData[phase.startIdx].elapsed_seconds,
			endSeconds: logData[phase.endIdx].elapsed_seconds,
			color: PHASE_COLORS[phase.type],
			opacity: 0.18,
		}));
	}, [logData, phases]);

	const stats = useMemo(() => {
		if (!logData || logData.length === 0) return null;
		const temps = logData.map((p) => p.current_temp_c);
		const maxTemp = Math.max(...temps);
		const minTemp = Math.min(...temps);
		const duration = secondsToMinutes(
			logData[logData.length - 1].elapsed_seconds,
		);
		const startTime = logData[0]?.timestamp || "";
		return { maxTemp, minTemp, duration, startTime };
	}, [logData]);

	return (
		<Card>
			<CardHeader>
				<CardTitle>Tuning Phases Visualizer</CardTitle>
				<CardDescription>
					Visualize PID tuning runs with physics-based phase detection
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					directory="logs"
					accept=".csv"
					onFileSelected={handleFileSelected}
					label="Select Tuning Log File"
					description="Choose a tuning log file to analyze"
				/>

				{error && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{logData && chartData.length > 0 && (
					<div className="space-y-6 pt-6 border-t">
						<div>
							<h3 className="text-lg font-semibold">
								Tuning Phases - Physics-Based Detection
							</h3>
							{stats && (
								<p className="text-sm text-muted-foreground mt-1">
									Started: {stats.startTime} | Duration:{" "}
									{stats.duration.toFixed(1)}min | Temp Range:{" "}
									{stats.minTemp.toFixed(1)}°C - {stats.maxTemp.toFixed(1)}°C
								</p>
							)}
						</div>

						<div className="space-y-8">
							<div className="flex gap-4 text-sm flex-wrap">
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-heating/40" />
									<span>Heating (SSR on, temp rising)</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-natural-cooling/40" />
									<span>Cooling (SSR off)</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-hold/40" />
									<span>Plateau (SSR on, temp stable)</span>
								</div>
							</div>

							<div>
								<h4 className="text-base font-semibold mb-2">
									Temperature (°C)
								</h4>
								<LineChart
									data={chartData}
									xDataKey="t"
									aspectRatio="auto"
									className="h-72"
									margin={CHART_MARGIN}
									animationDuration={0}
								>
									<KilnRegions items={phaseRegions} />
									<Grid horizontal />
									<YAxis formatValue={(v) => `${Math.round(v)}°`} />
									<Line
										dataKey="temp"
										stroke="var(--foreground)"
										strokeWidth={2}
										curve={curveMonotoneX}
										fadeEdges={false}
										showHighlight={false}
									/>
									<KilnTimeAxis />
									<ChartTooltip
										showDatePill={false}
										content={({ point }) => (
											<KilnTooltipContent
												point={point}
												rows={[
													{
														color: "var(--foreground)",
														label: "Temp",
														value: `${(point.temp as number).toFixed(1)}°C`,
													},
												]}
											/>
										)}
									/>
								</LineChart>
							</div>

							<div>
								<h4 className="text-base font-semibold mb-1">
									Heat Output (%)
								</h4>
								<p className="text-sm text-muted-foreground mb-2">
									Duty cycle, shaded by detected phase
								</p>
								<AreaChart
									data={chartData}
									xDataKey="t"
									aspectRatio="auto"
									className="h-44"
									margin={CHART_MARGIN}
									animationDuration={0}
								>
									<KilnRegions items={phaseRegions} />
									<Grid horizontal />
									<YAxis formatValue={(v) => `${Math.round(v)}%`} />
									<Area
										dataKey="ssr"
										fill="var(--chart-ssr)"
										fillOpacity={0.3}
										stroke="var(--chart-ssr)"
										strokeWidth={2}
										curve={curveMonotoneX}
										showHighlight={false}
									/>
									<KilnTimeAxis />
									<ChartTooltip
										showDatePill={false}
										content={({ point }) => (
											<KilnTooltipContent
												point={point}
												rows={[
													{
														color: "var(--chart-ssr)",
														label: "Heat",
														value: `${(point.ssr as number).toFixed(0)}%`,
													},
												]}
											/>
										)}
									/>
								</AreaChart>
							</div>

							<div className="pt-2 border-t">
								<h4 className="text-base font-semibold mb-1">
									Detected Phases Summary
								</h4>
								<p className="text-sm text-muted-foreground mb-4">
									{phases.length} phases detected using physics-based algorithm
								</p>
								<div className="space-y-2">
									{phases.map((phase, idx) => {
										const startTime = secondsToMinutes(
											logData[phase.startIdx].elapsed_seconds,
										);
										const endTime = secondsToMinutes(
											logData[phase.endIdx].elapsed_seconds,
										);
										const duration = endTime - startTime;
										const tempChange = phase.tempEnd - phase.tempStart;
										const rate =
											duration > 0 ? (tempChange / duration) * 60 : 0;

										return (
											<div
												key={idx}
												className={`p-3 rounded border ${PHASE_TINT[phase.type]}`}
											>
												<div className="flex items-center justify-between">
													<div className="font-semibold">
														Phase {idx + 1}:{" "}
														{PHASE_LABELS[phase.type].toUpperCase()}
														{phase.stepName && (
															<span className="ml-2 text-sm font-normal">
																({phase.stepName})
															</span>
														)}
													</div>
													<div className="text-sm">
														{startTime.toFixed(1)} - {endTime.toFixed(1)} min (
														{duration.toFixed(1)} min)
													</div>
												</div>
												<div className="mt-1 flex gap-4 text-sm">
													<span>SSR: {phase.avgSsr.toFixed(1)}%</span>
													<span>
														Temp: {phase.tempStart.toFixed(1)}°C →{" "}
														{phase.tempEnd.toFixed(1)}°C (
														{tempChange > 0 ? "+" : ""}
														{tempChange.toFixed(1)}°C)
													</span>
													<span>
														Rate: {rate > 0 ? "+" : ""}
														{rate.toFixed(1)}°C/h
													</span>
												</div>
											</div>
										);
									})}
								</div>

								<div className="mt-4 p-3 bg-muted/50 rounded text-sm">
									<div className="font-semibold mb-1">
										Phase Classification Logic:
									</div>
									<ul className="space-y-1 text-muted-foreground">
										<li>
											• <strong>COOLING:</strong> SSR &lt; 5% (natural cooling,
											no heat input)
										</li>
										<li>
											• <strong>HEATING:</strong> SSR ≥ 5% AND temp rising &gt;
											0.5°C/min
										</li>
										<li>
											• <strong>PLATEAU:</strong> SSR ≥ 5% AND temp stable
											±0.5°C/min
										</li>
									</ul>
								</div>
							</div>
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
