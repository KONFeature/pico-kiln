import { curveLinear } from "@visx/curve";
import { CircleAlert } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Grid } from "@/components/charts/grid";
import {
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
import type { Profile } from "@/lib/pico/types";
import { parseProfileText } from "@/lib/profile-schema";
import { buildProfileChart, calculateTrajectory } from "@/lib/profile-utils";
import { FileSourceSelector } from "./FileSourceSelector";

export function ProfileVisualizer() {
	const [profile, setProfile] = useState<Profile | null>(null);
	const [error, setError] = useState<string | null>(null);

	const handleFileSelected = useCallback((content: string) => {
		const result = parseProfileText(content);
		if (result.ok) {
			setProfile(result.profile);
			setError(null);
		} else {
			setError(result.error);
			setProfile(null);
		}
	}, []);

	const segments = useMemo(() => {
		if (!profile) return [];
		return calculateTrajectory(profile);
	}, [profile]);

	const chart = useMemo(() => buildProfileChart(segments), [segments]);

	const stats = useMemo(() => {
		if (segments.length === 0) return null;

		const allTemps = segments.flatMap((s) => s.data.map((p) => p.temp));
		const maxTemp = Math.max(...allTemps);
		const minTemp = Math.min(...allTemps);
		const lastSegment = segments[segments.length - 1];
		const duration = lastSegment.data[lastSegment.data.length - 1].time_hours;

		return { maxTemp, minTemp, duration };
	}, [segments]);

	const unit = profile?.temp_units.toUpperCase() ?? "C";

	return (
		<Card>
			<CardHeader>
				<CardTitle>Profile Visualizer</CardTitle>
				<CardDescription>
					Visualize kiln firing profiles - see temperature trajectory over time
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					directory="profiles"
					accept=".json"
					onFileSelected={handleFileSelected}
					label="Select Profile"
					description="Choose a profile file to visualize"
				/>

				{error && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{profile && chart.chartData.length > 0 && (
					<div className="space-y-6 pt-6 border-t">
						<div>
							<h3 className="text-lg font-semibold">{profile.name}</h3>
							{profile.description && (
								<p className="text-sm text-muted-foreground mt-1">
									{profile.description}
								</p>
							)}
						</div>

						<div className="space-y-4">
							{stats && (
								<div className="flex gap-6 text-sm">
									<div>
										<span className="text-muted-foreground">Duration:</span>{" "}
										<span className="font-semibold">
											{stats.duration.toFixed(2)}h
										</span>
									</div>
									<div>
										<span className="text-muted-foreground">Max Temp:</span>{" "}
										<span className="font-semibold">
											{stats.maxTemp.toFixed(0)}°{unit}
										</span>
									</div>
									<div>
										<span className="text-muted-foreground">Min Temp:</span>{" "}
										<span className="font-semibold">
											{stats.minTemp.toFixed(0)}°{unit}
										</span>
									</div>
								</div>
							)}

							<div className="flex gap-4 text-sm flex-wrap">
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-heating" />
									<span>Ramp (heating)</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-hold" />
									<span>Hold</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-cooling" />
									<span>Controlled Cooling</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-chart-natural-cooling" />
									<span>Natural Cooling</span>
								</div>
							</div>

							<p className="text-xs text-muted-foreground">
								Temperature (°{unit}) vs time — drag across the chart to
								inspect.
							</p>

							<LineChart
								data={chart.chartData}
								xDataKey="t"
								aspectRatio="auto"
								className="h-72"
								margin={{ top: 18, right: 16, bottom: 28, left: 44 }}
								animationDuration={0}
							>
								<Grid horizontal />
								<YAxis formatValue={(v) => `${Math.round(v)}°`} />
								{chart.series.map((s, i) => (
									<Line
										key={`${s.type}-${i}`}
										data={s.data}
										dataKey="temp"
										stroke={s.color}
										strokeWidth={2.5}
										curve={curveLinear}
										fadeEdges={false}
										showHighlight={false}
										animate={false}
									/>
								))}
								<KilnMarkers items={chart.markers} />
								<KilnTimeAxis />
								<ChartTooltip
									showDatePill={false}
									showDots={false}
									content={({ point }) => (
										<KilnTooltipContent
											point={point}
											rows={[
												{
													color: "var(--chart-line-primary)",
													label: "Temp",
													value: `${(point.temp as number).toFixed(0)}°${unit}`,
												},
											]}
										/>
									)}
								/>
							</LineChart>

							<div className="space-y-2 pt-4 border-t">
								<h4 className="font-semibold text-sm">Profile Steps:</h4>
								<div className="grid gap-2">
									{profile.steps.map((step, idx) => (
										<div
											key={idx}
											className="text-sm p-2 rounded bg-muted/50 flex items-center justify-between"
										>
											<span className="font-medium">Step {idx + 1}:</span>
											{step.type === "ramp" && (
												<span>
													Ramp to {step.target_temp}°{unit} at{" "}
													{step.desired_rate ?? 100}°{unit}/h
													{step.min_rate &&
														` (min: ${step.min_rate}°${unit}/h)`}
												</span>
											)}
											{step.type === "hold" && (
												<span>
													Hold at {step.target_temp}°{unit} for{" "}
													{((step.duration ?? 0) / 60).toFixed(0)} minutes
												</span>
											)}
											{step.type === "cooling" && (
												<span>
													Natural cooling
													{step.target_temp !== undefined
														? ` to ${step.target_temp}°${unit}`
														: " (no target)"}
												</span>
											)}
										</div>
									))}
								</div>
							</div>
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
