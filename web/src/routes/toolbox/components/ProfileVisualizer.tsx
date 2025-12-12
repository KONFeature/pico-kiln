import { AlertCircle } from "lucide-react";
import { useMemo, useState } from "react";
import {
	CartesianGrid,
	Legend,
	Line,
	LineChart,
	ReferenceArea,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import type { Profile } from "@/lib/pico/types";
import {
	calculateTrajectory,
	type Segment,
	type TrajectoryPoint,
} from "@/lib/profile-utils";
import { FileSourceSelector } from "./FileSourceSelector";

export function ProfileVisualizer() {
	const [profile, setProfile] = useState<Profile | null>(null);
	const [error, setError] = useState<string | null>(null);

	const handleFileSelected = (content: string, filename: string) => {
		try {
			const parsed = JSON.parse(content) as Profile;

			// Validate profile structure
			if (!parsed.name || !parsed.steps || !Array.isArray(parsed.steps)) {
				throw new Error("Invalid profile format: missing required fields");
			}

			setProfile(parsed);
			setError(null);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to parse profile");
			setProfile(null);
		}
	};

	const segments = useMemo(() => {
		if (!profile) return [];
		return calculateTrajectory(profile);
	}, [profile]);

	const stats = useMemo(() => {
		if (segments.length === 0) return null;

		const allTemps = segments.flatMap((s) => s.data.map((p) => p.temp));
		const maxTemp = Math.max(...allTemps);
		const minTemp = Math.min(...allTemps);
		const lastSegment = segments[segments.length - 1];
		const duration = lastSegment.data[lastSegment.data.length - 1].time_hours;

		return { maxTemp, minTemp, duration };
	}, [segments]);

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
						<AlertCircle className="h-4 w-4" />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{profile && segments.length > 0 && (
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
											{stats.maxTemp.toFixed(0)}°C
										</span>
									</div>
									<div>
										<span className="text-muted-foreground">Min Temp:</span>{" "}
										<span className="font-semibold">
											{stats.minTemp.toFixed(0)}°C
										</span>
									</div>
								</div>
							)}

							<div className="flex gap-4 text-sm flex-wrap">
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-red-500" />
									<span>Ramp (heating)</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-yellow-500" />
									<span>Hold</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-blue-500" />
									<span>Controlled Cooling</span>
								</div>
								<div className="flex items-center gap-2">
									<div className="w-4 h-4 rounded bg-cyan-500" />
									<span>Natural Cooling</span>
								</div>
							</div>

							<ResponsiveContainer width="100%" height={400}>
								<LineChart margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
									<CartesianGrid strokeDasharray="3 3" opacity={0.3} />

									<XAxis
										dataKey="time_hours"
										type="number"
										domain={[0, "dataMax"]}
										label={{
											value: "Time (hours)",
											position: "insideBottom",
											offset: -5,
										}}
									/>
									<YAxis
										label={{
											value: `Temperature (°${profile.temp_units.toUpperCase()})`,
											angle: -90,
											position: "insideLeft",
										}}
									/>
									<Tooltip
										content={({ active, payload }) => {
											if (!active || !payload || payload.length === 0)
												return null;

											const point = payload[0].payload as TrajectoryPoint & {
												segmentInfo?: Segment;
											};
											const segment = segments.find((s) =>
												s.data.some(
													(d) =>
														d.time_hours === point.time_hours &&
														d.temp === point.temp,
												),
											);

											return (
												<div className="bg-background border rounded-lg p-3 shadow-lg">
													<p className="font-semibold">
														Time: {point.time_hours.toFixed(2)}h
													</p>
													<p>Temperature: {point.temp.toFixed(1)}°C</p>
													{segment && (
														<>
															<p className="mt-2 font-semibold capitalize">
																{segment.type}
															</p>
															{segment.type === "hold" && (
																<p className="text-sm">
																	Duration: {segment.duration?.toFixed(0)} min
																</p>
															)}
															{(segment.type === "ramp" ||
																segment.type === "cooling") && (
																<>
																	{segment.desiredRate && (
																		<p className="text-sm">
																			Desired rate: {segment.desiredRate}°C/h
																		</p>
																	)}
																	{segment.minRate && (
																		<p className="text-sm">
																			Min rate: {segment.minRate}°C/h
																		</p>
																	)}
																</>
															)}
														</>
													)}
												</div>
											);
										}}
									/>
									<Legend />

									{/* Draw a separate line for each segment with its own color */}
									{segments.map((segment, idx) => (
										<Line
											key={idx}
											data={segment.data}
											type="linear"
											dataKey="temp"
											stroke={segment.color}
											strokeWidth={3}
											dot={{ r: 5, fill: segment.color }}
											name={idx === 0 ? "Temperature" : undefined}
											legendType={idx === 0 ? "line" : "none"}
											isAnimationActive={false}
										/>
									))}
								</LineChart>
							</ResponsiveContainer>

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
													Ramp to {step.target_temp}°C at{" "}
													{step.desired_rate ?? 100}°C/h
													{step.min_rate && ` (min: ${step.min_rate}°C/h)`}
												</span>
											)}
											{step.type === "hold" && (
												<span>
													Hold at {step.target_temp}°C for{" "}
													{((step.duration ?? 0) / 60).toFixed(0)} minutes
												</span>
											)}
											{step.type === "cooling" && (
												<span>
													Natural cooling
													{step.target_temp !== undefined
														? ` to ${step.target_temp}°C`
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
