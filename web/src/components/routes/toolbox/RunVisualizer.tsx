import { curveMonotoneX } from "@visx/curve";
import { CircleAlert } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Area } from "@/components/charts/area";
import { AreaChart } from "@/components/charts/area-chart";
import { Grid } from "@/components/charts/grid";
import {
	type KilnMarker,
	KilnMarkers,
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
	detectRunType,
	type LogDataPoint,
	lttbDownsample,
	parseLogCSV,
} from "@/lib/csv-parser";
import { FileSourceSelector } from "./FileSourceSelector";

interface RunChartPoint {
	t: Date;
	temp: number;
	target: number;
	ssr: number;
	rate: number;
	[key: string]: unknown;
}

/** Target points fed to the chart; long logs are decimated to keep phones responsive. */
const MAX_CHART_POINTS = 800;

const CHART_MARGIN = { top: 18, right: 16, bottom: 28, left: 48 };

export function RunVisualizer() {
	const [logData, setLogData] = useState<LogDataPoint[] | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [runType, setRunType] = useState<"TUNING" | "FIRING">("FIRING");

	const handleFileSelected = useCallback((content: string) => {
		try {
			const { data } = parseLogCSV(content);

			if (data.length === 0) {
				throw new Error("No valid data points in log file");
			}

			setLogData(data);
			setRunType(detectRunType(data));
			setError(null);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to parse log file");
			setLogData(null);
		}
	}, []);

	const chartData = useMemo<RunChartPoint[]>(() => {
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
			target: point.target_temp_c,
			ssr: point.ssr_output_percent,
			rate: point.measured_rate_c_per_hour ?? 0,
		}));
	}, [logData]);

	const stats = useMemo(() => {
		if (!logData || logData.length === 0) return null;

		const temps = logData.map((p) => p.current_temp_c);
		const maxTemp = Math.max(...temps);
		const minTemp = Math.min(...temps);
		const duration = logData[logData.length - 1].elapsed_seconds / 3600;
		const startTime = logData[0]?.timestamp || "";

		return { maxTemp, minTemp, duration, startTime };
	}, [logData]);

	const hasRateData = useMemo(
		() =>
			logData?.some((p) => p.measured_rate_c_per_hour !== undefined) ?? false,
		[logData],
	);

	// Step-change boundaries become vertical markers shared across all charts.
	const markers = useMemo<KilnMarker[]>(() => {
		if (!logData) return [];
		const result: KilnMarker[] = [];
		let prevStep = -1;
		for (let i = 0; i < logData.length; i++) {
			const stepIdx = logData[i].step_index;
			if (
				stepIdx !== undefined &&
				stepIdx !== prevStep &&
				stepIdx >= 0 &&
				i > 0
			) {
				result.push({
					atSeconds: logData[i].elapsed_seconds,
					label: String(stepIdx + 1),
				});
				prevStep = stepIdx;
			}
		}
		return result;
	}, [logData]);

	return (
		<Card>
			<CardHeader>
				<CardTitle>Run Visualizer</CardTitle>
				<CardDescription>
					Visualize kiln firing or tuning runs - see temperature, SSR output,
					and rate data
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					directory="logs"
					accept=".csv"
					onFileSelected={handleFileSelected}
					label="Select Log File"
					description="Choose a log file to visualize"
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
								Kiln {runType} - Temperature Profile
							</h3>
							{stats && (
								<p className="text-sm text-muted-foreground mt-1">
									Started: {stats.startTime} | Duration:{" "}
									{stats.duration.toFixed(2)}h | Temp Range:{" "}
									{stats.minTemp.toFixed(1)}°C - {stats.maxTemp.toFixed(1)}°C
								</p>
							)}
						</div>

						<div className="space-y-8">
							<div>
								<div className="flex items-center justify-between mb-2">
									<h4 className="text-base font-semibold">Temperature (°C)</h4>
									<div className="flex gap-3 text-xs">
										<span className="flex items-center gap-1.5">
											<span className="h-0.5 w-3 rounded bg-chart-heating" />
											Current
										</span>
										<span className="flex items-center gap-1.5">
											<span className="h-0.5 w-3 rounded bg-muted-foreground" />
											Target
										</span>
									</div>
								</div>
								<LineChart
									data={chartData}
									xDataKey="t"
									aspectRatio="auto"
									className="h-72"
									margin={CHART_MARGIN}
									animationDuration={0}
								>
									<Grid horizontal />
									<YAxis formatValue={(v) => `${Math.round(v)}°`} />
									<Line
										dataKey="target"
										stroke="var(--muted-foreground)"
										strokeWidth={1.5}
										curve={curveMonotoneX}
										fadeEdges={false}
										showHighlight={false}
									/>
									<Line
										dataKey="temp"
										stroke="var(--chart-heating)"
										strokeWidth={2.5}
										curve={curveMonotoneX}
										fadeEdges={false}
										showHighlight={false}
									/>
									<KilnMarkers items={markers} />
									<KilnTimeAxis />
									<ChartTooltip
										showDatePill={false}
										content={({ point }) => (
											<KilnTooltipContent
												point={point}
												rows={[
													{
														color: "var(--chart-heating)",
														label: "Current",
														value: `${(point.temp as number).toFixed(1)}°C`,
													},
													{
														color: "var(--muted-foreground)",
														label: "Target",
														value: `${(point.target as number).toFixed(1)}°C`,
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
									Solid State Relay duty cycle over time
								</p>
								<AreaChart
									data={chartData}
									xDataKey="t"
									aspectRatio="auto"
									className="h-44"
									margin={CHART_MARGIN}
									animationDuration={0}
								>
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
									<KilnMarkers items={markers} />
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

							{hasRateData && (
								<div>
									<h4 className="text-base font-semibold mb-1">
										Heating Rate (°C/h)
									</h4>
									<p className="text-sm text-muted-foreground mb-2">
										Measured temperature change rate
									</p>
									<LineChart
										data={chartData}
										xDataKey="t"
										aspectRatio="auto"
										className="h-44"
										margin={CHART_MARGIN}
										animationDuration={0}
									>
										<Grid
											horizontal
											highlightRowValues={[0]}
											highlightRowStroke="var(--muted-foreground)"
										/>
										<YAxis formatValue={(v) => `${Math.round(v)}`} />
										<Line
											dataKey="rate"
											stroke="var(--chart-rate)"
											strokeWidth={2}
											curve={curveMonotoneX}
											fadeEdges={false}
											showHighlight={false}
										/>
										<KilnMarkers items={markers} />
										<KilnTimeAxis />
										<ChartTooltip
											showDatePill={false}
											content={({ point }) => (
												<KilnTooltipContent
													point={point}
													rows={[
														{
															color: "var(--chart-rate)",
															label: "Rate",
															value: `${(point.rate as number).toFixed(1)}°C/h`,
														},
													]}
												/>
											)}
										/>
									</LineChart>
								</div>
							)}

							<div className="pt-2 border-t">
								<h4 className="text-base font-semibold mb-4">Run Statistics</h4>
								<div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
									<div>
										<div className="text-muted-foreground">Run Type</div>
										<div className="font-semibold text-lg">{runType}</div>
									</div>
									<div>
										<div className="text-muted-foreground">Duration</div>
										<div className="font-semibold text-lg">
											{stats?.duration.toFixed(2)}h
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">Max Temperature</div>
										<div className="font-semibold text-lg">
											{stats?.maxTemp.toFixed(1)}°C
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">Data Points</div>
										<div className="font-semibold text-lg">
											{logData.length.toLocaleString()}
										</div>
									</div>
								</div>
							</div>
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
