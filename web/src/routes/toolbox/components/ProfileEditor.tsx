import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	AlertCircle,
	CheckCircle,
	Download,
	FileUp,
	HardDrive,
	Plus,
	Trash2,
	Upload,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
	CartesianGrid,
	Legend,
	Line,
	LineChart,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { usePico } from "@/lib/pico/context";
import type {
	Profile,
	ProfileStep,
	ProfileStepType,
	TempUnits,
} from "@/lib/pico/types";
import { readFileAsText } from "@/lib/utils";

interface TrajectoryPoint {
	time_hours: number;
	temp: number;
}

interface Segment {
	data: TrajectoryPoint[];
	type: "ramp" | "hold" | "cooling";
	color: string;
	step: ProfileStep;
	desiredRate?: number;
	minRate?: number;
	duration?: number;
}

function calculateTrajectory(profile: Profile): Segment[] {
	const segments: Segment[] = [];
	let currentTime = 0;
	let currentTemp = 20;

	for (const step of profile.steps) {
		const targetTemp = step.target_temp ?? currentTemp;

		if (step.type === "hold") {
			const duration = step.duration ?? 0;
			const data: TrajectoryPoint[] = [
				{ time_hours: currentTime / 3600, temp: currentTemp },
				{ time_hours: (currentTime + duration) / 3600, temp: currentTemp },
			];

			segments.push({
				data,
				type: "hold",
				color: "#eab308",
				step,
				duration: duration / 60,
			});

			currentTime += duration;
		} else if (step.type === "ramp") {
			const desiredRate = step.desired_rate ?? 100;
			const tempChange = Math.abs(targetTemp - currentTemp);
			const durationHours =
				desiredRate > 0 ? tempChange / desiredRate : tempChange / 100;
			const durationSeconds = durationHours * 3600;

			const data: TrajectoryPoint[] = [
				{ time_hours: currentTime / 3600, temp: currentTemp },
				{
					time_hours: (currentTime + durationSeconds) / 3600,
					temp: targetTemp,
				},
			];

			const isHeating = targetTemp > currentTemp;
			segments.push({
				data,
				type: isHeating ? "ramp" : "cooling",
				color: isHeating ? "#ef4444" : "#3b82f6",
				step,
				desiredRate: step.desired_rate,
				minRate: step.min_rate,
			});

			currentTime += durationSeconds;
			currentTemp = targetTemp;
		}
	}

	return segments;
}

const DEFAULT_PROFILE: Profile = {
	name: "New Profile",
	temp_units: "c",
	description: "",
	steps: [
		{ type: "ramp", target_temp: 600, desired_rate: 100 },
		{ type: "hold", target_temp: 600, duration: 600 },
	],
};

export function ProfileEditor() {
	const [profile, setProfile] = useState<Profile>(DEFAULT_PROFILE);
	const [importMode, setImportMode] = useState<"file" | "pico">("file");
	const [exportMode, setExportMode] = useState<"download" | "upload">(
		"download",
	);
	const [selectedPicoProfile, setSelectedPicoProfile] = useState<string>("");
	const [uploadFilename, setUploadFilename] = useState<string>("");
	const { client, isConfigured } = usePico();
	const queryClient = useQueryClient();

	// Query to list profiles from Pico
	const { data: profilesData, isLoading: isLoadingProfiles } = useQuery({
		queryKey: ["files", "profiles"],
		queryFn: () => client!.listFiles("profiles"),
		enabled: importMode === "pico" && isConfigured && client !== null,
		retry: 1,
	});

	// Query to get profile content from Pico
	const { data: picoProfileContent } = useQuery({
		queryKey: ["file-content", "profiles", selectedPicoProfile],
		queryFn: () => client!.getFile("profiles", selectedPicoProfile),
		enabled:
			importMode === "pico" && selectedPicoProfile !== "" && client !== null,
		retry: 1,
	});

	// Mutation for uploading profile to Pico
	const uploadMutation = useMutation({
		mutationFn: async ({
			filename,
			content,
		}: {
			filename: string;
			content: string;
		}) => {
			if (!client) throw new Error("Pico client not configured");
			return client.uploadFile("profiles", filename, content);
		},
		onSuccess: (data) => {
			if (data.success) {
				// Invalidate profiles list to refresh
				queryClient.invalidateQueries({ queryKey: ["files", "profiles"] });
			}
		},
	});

	const segments = useMemo(() => {
		return calculateTrajectory(profile);
	}, [profile]);

	const stats = useMemo(() => {
		if (segments.length === 0) return null;
		const allTemps = segments.flatMap((s) => s.data.map((p) => p.temp));
		const maxTemp = Math.max(...allTemps);
		const lastSegment = segments[segments.length - 1];
		const duration = lastSegment.data[lastSegment.data.length - 1].time_hours;
		return { maxTemp, duration };
	}, [segments]);

	const updateProfile = (updates: Partial<Profile>) => {
		setProfile((prev) => ({ ...prev, ...updates }));
	};

	const updateStep = (index: number, updates: Partial<ProfileStep>) => {
		setProfile((prev) => ({
			...prev,
			steps: prev.steps.map((step, i) =>
				i === index ? { ...step, ...updates } : step,
			),
		}));
	};

	const addStep = () => {
		setProfile((prev) => ({
			...prev,
			steps: [...prev.steps, { type: "ramp", target_temp: 100 }],
		}));
	};

	const removeStep = (index: number) => {
		if (profile.steps.length <= 1) return; // Keep at least one step
		setProfile((prev) => ({
			...prev,
			steps: prev.steps.filter((_, i) => i !== index),
		}));
	};

	const moveStepUp = (index: number) => {
		if (index === 0) return;
		setProfile((prev) => {
			const steps = [...prev.steps];
			[steps[index - 1], steps[index]] = [steps[index], steps[index - 1]];
			return { ...prev, steps };
		});
	};

	const moveStepDown = (index: number) => {
		if (index === profile.steps.length - 1) return;
		setProfile((prev) => {
			const steps = [...prev.steps];
			[steps[index], steps[index + 1]] = [steps[index + 1], steps[index]];
			return { ...prev, steps };
		});
	};

	const downloadProfile = () => {
		const json = JSON.stringify(profile, null, 2);
		const blob = new Blob([json], { type: "application/json" });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = `${profile.name.replace(/\s+/g, "_").toLowerCase()}.json`;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);
	};

	const uploadToPico = () => {
		const filename =
			uploadFilename ||
			`${profile.name.replace(/\s+/g, "_").toLowerCase()}.json`;
		// Minify JSON for upload to save space on Pico
		const json = JSON.stringify(profile);
		uploadMutation.mutate({ filename, content: json });
	};

	const importFromFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
		const file = e.target.files?.[0];
		if (!file) return;

		try {
			const json = await readFileAsText(file);
			const imported = JSON.parse(json) as Profile;
			setProfile(imported);
		} catch (err) {
			alert("Failed to import profile: Invalid JSON");
		}
	};

	const importFromPico = () => {
		if (picoProfileContent?.success && picoProfileContent.content) {
			try {
				const imported = JSON.parse(picoProfileContent.content) as Profile;
				setProfile(imported);
			} catch (err) {
				alert("Failed to import profile: Invalid JSON");
			}
		}
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>Profile Editor</CardTitle>
				<CardDescription>
					Create and edit kiln firing profiles with live visualization
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
					<div>
						<Label htmlFor="profile-name">Profile Name</Label>
						<Input
							id="profile-name"
							value={profile.name}
							onChange={(e) => updateProfile({ name: e.target.value })}
						/>
					</div>
					<div>
						<Label htmlFor="temp-units">Temperature Units</Label>
						<Select
							value={profile.temp_units}
							onValueChange={(value) =>
								updateProfile({ temp_units: value as TempUnits })
							}
						>
							<SelectTrigger id="temp-units">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="c">Celsius (°C)</SelectItem>
								<SelectItem value="f">Fahrenheit (°F)</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</div>

				<div>
					<Label htmlFor="profile-description">Description</Label>
					<Textarea
						id="profile-description"
						value={profile.description}
						onChange={(e) => updateProfile({ description: e.target.value })}
						placeholder="Describe this firing profile..."
						rows={2}
					/>
				</div>

				{/* Import Section */}
				<div className="space-y-3">
					<div>
						<Label className="text-base font-semibold">Import Profile</Label>
						<p className="text-sm text-muted-foreground">
							Load an existing profile to edit
						</p>
					</div>

					<div className="flex gap-2">
						<Button
							variant={importMode === "file" ? "default" : "outline"}
							size="sm"
							onClick={() => setImportMode("file")}
						>
							<Upload className="w-4 h-4 mr-2" />
							From File
						</Button>
						<Button
							variant={importMode === "pico" ? "default" : "outline"}
							size="sm"
							onClick={() => setImportMode("pico")}
							disabled={!isConfigured}
						>
							<HardDrive className="w-4 h-4 mr-2" />
							From Pico
						</Button>
					</div>

					{importMode === "file" && (
						<label>
							<Button variant="outline" size="sm" className="w-full" asChild>
								<span>
									<Upload className="w-4 h-4 mr-2" />
									Choose File to Import
								</span>
							</Button>
							<input
								type="file"
								accept=".json"
								onChange={importFromFile}
								className="hidden"
							/>
						</label>
					)}

					{importMode === "pico" && (
						<div className="space-y-2">
							{!isConfigured && (
								<Alert>
									<AlertCircle className="h-4 w-4" />
									<AlertDescription>
										Please configure Pico connection to import profiles
									</AlertDescription>
								</Alert>
							)}
							{isConfigured && isLoadingProfiles && (
								<p className="text-sm text-muted-foreground">
									Loading profiles from Pico...
								</p>
							)}
							{isConfigured &&
								profilesData?.success &&
								profilesData.files.length === 0 && (
									<p className="text-sm text-muted-foreground">
										No profiles found on Pico
									</p>
								)}
							{isConfigured &&
								profilesData?.success &&
								profilesData.files.length > 0 && (
									<div className="flex gap-2">
										<Select
											value={selectedPicoProfile}
											onValueChange={setSelectedPicoProfile}
										>
											<SelectTrigger className="flex-1">
												<SelectValue placeholder="Choose a profile..." />
											</SelectTrigger>
											<SelectContent>
												{profilesData.files.map((file) => (
													<SelectItem key={file.name} value={file.name}>
														{file.name}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
										<Button
											onClick={importFromPico}
											disabled={
												!selectedPicoProfile || !picoProfileContent?.success
											}
											size="sm"
										>
											Import
										</Button>
									</div>
								)}
						</div>
					)}
				</div>

				{/* Export Section */}
				<div className="space-y-3 pt-4 border-t">
					<div>
						<Label className="text-base font-semibold">Export Profile</Label>
						<p className="text-sm text-muted-foreground">Save your profile</p>
					</div>

					<div className="flex gap-2">
						<Button
							variant={exportMode === "download" ? "default" : "outline"}
							size="sm"
							onClick={() => setExportMode("download")}
						>
							<Download className="w-4 h-4 mr-2" />
							Download
						</Button>
						<Button
							variant={exportMode === "upload" ? "default" : "outline"}
							size="sm"
							onClick={() => setExportMode("upload")}
							disabled={!isConfigured}
						>
							<FileUp className="w-4 h-4 mr-2" />
							Upload to Pico
						</Button>
					</div>

					{exportMode === "download" && (
						<Button onClick={downloadProfile} size="sm" className="w-full">
							<Download className="w-4 h-4 mr-2" />
							Download as JSON
						</Button>
					)}

					{exportMode === "upload" && (
						<div className="space-y-2">
							{!isConfigured && (
								<Alert>
									<AlertCircle className="h-4 w-4" />
									<AlertDescription>
										Please configure Pico connection to upload profiles
									</AlertDescription>
								</Alert>
							)}
							{isConfigured && (
								<>
									<div>
										<Label htmlFor="upload-filename">Filename (optional)</Label>
										<Input
											id="upload-filename"
											value={uploadFilename}
											onChange={(e) => setUploadFilename(e.target.value)}
											placeholder={`${profile.name.replace(/\s+/g, "_").toLowerCase()}.json`}
											className="mt-1"
										/>
										<p className="text-xs text-muted-foreground mt-1">
											Leave empty to use profile name
										</p>
									</div>
									<Button
										onClick={uploadToPico}
										disabled={uploadMutation.isPending}
										size="sm"
										className="w-full"
									>
										{uploadMutation.isPending ? (
											<>Uploading...</>
										) : (
											<>
												<FileUp className="w-4 h-4 mr-2" />
												Upload to Pico
											</>
										)}
									</Button>
									{uploadMutation.isSuccess && uploadMutation.data?.success && (
										<Alert>
											<CheckCircle className="h-4 w-4" />
											<AlertDescription>
												Profile uploaded successfully:{" "}
												{uploadMutation.data.filename}
											</AlertDescription>
										</Alert>
									)}
									{uploadMutation.isError && (
										<Alert variant="destructive">
											<AlertCircle className="h-4 w-4" />
											<AlertDescription>
												{uploadMutation.error instanceof Error
													? uploadMutation.error.message
													: "Upload failed"}
											</AlertDescription>
										</Alert>
									)}
									{uploadMutation.isSuccess &&
										!uploadMutation.data?.success && (
											<Alert variant="destructive">
												<AlertCircle className="h-4 w-4" />
												<AlertDescription>
													{uploadMutation.data?.error || "Upload failed"}
												</AlertDescription>
											</Alert>
										)}
								</>
							)}
						</div>
					)}
				</div>
			</CardContent>

			<div className="px-6 pb-6 space-y-6 border-t pt-6">
				<div className="flex items-center justify-between">
					<div>
						<h3 className="text-lg font-semibold">Profile Steps</h3>
					</div>
					<Button onClick={addStep} size="sm">
						<Plus className="w-4 h-4 mr-2" />
						Add Step
					</Button>
				</div>

				<div className="space-y-3">
					{profile.steps.map((step, index) => (
						<div
							key={index}
							className="p-4 rounded-lg border bg-muted/30 hover:bg-muted/50 transition-colors"
						>
							<div className="flex items-start gap-4">
								<div className="flex flex-col gap-1">
									<Button
										variant="ghost"
										size="sm"
										onClick={() => moveStepUp(index)}
										disabled={index === 0}
										className="h-8 w-8 p-0"
									>
										↑
									</Button>
									<Button
										variant="ghost"
										size="sm"
										onClick={() => moveStepDown(index)}
										disabled={index === profile.steps.length - 1}
										className="h-8 w-8 p-0"
									>
										↓
									</Button>
								</div>

								<div className="flex-1 space-y-3">
									<div className="flex items-center gap-4">
										<div className="font-semibold">Step {index + 1}</div>
										<Select
											value={step.type}
											onValueChange={(value) =>
												updateStep(index, { type: value as ProfileStepType })
											}
										>
											<SelectTrigger className="w-32">
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="ramp">Ramp</SelectItem>
												<SelectItem value="hold">Hold</SelectItem>
											</SelectContent>
										</Select>
									</div>

									<div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-4">
										<div>
											<Label>
												Target Temp (°{profile.temp_units.toUpperCase()})
											</Label>
											<Input
												type="number"
												value={step.target_temp ?? ""}
												onChange={(e) =>
													updateStep(index, {
														target_temp: parseFloat(e.target.value) || 0,
													})
												}
											/>
										</div>

										{step.type === "ramp" && (
											<>
												<div>
													<Label>Desired Rate (°C/h)</Label>
													<Input
														type="number"
														value={step.desired_rate ?? 100}
														onChange={(e) =>
															updateStep(index, {
																desired_rate: parseFloat(e.target.value) || 100,
															})
														}
													/>
												</div>
												<div>
													<Label>Min Rate (°C/h)</Label>
													<Input
														type="number"
														value={step.min_rate ?? ""}
														onChange={(e) =>
															updateStep(index, {
																min_rate:
																	parseFloat(e.target.value) || undefined,
															})
														}
														placeholder="Optional"
													/>
												</div>
											</>
										)}

										{step.type === "hold" && (
											<div>
												<Label>Duration (minutes)</Label>
												<Input
													type="number"
													value={
														step.duration ? (step.duration / 60).toFixed(0) : ""
													}
													onChange={(e) =>
														updateStep(index, {
															duration: (parseFloat(e.target.value) || 0) * 60,
														})
													}
												/>
											</div>
										)}
									</div>
								</div>

								<Button
									variant="ghost"
									size="sm"
									onClick={() => removeStep(index)}
									disabled={profile.steps.length <= 1}
									className="h-8 w-8 p-0"
								>
									<Trash2 className="w-4 h-4 text-destructive" />
								</Button>
							</div>
						</div>
					))}
				</div>
			</div>

			{segments.length > 0 && (
				<div className="px-6 pb-6 space-y-6 border-t pt-6">
					<div>
						<h3 className="text-lg font-semibold">Live Preview</h3>
						{stats && (
							<p className="text-sm text-muted-foreground mt-1">
								Duration: {stats.duration.toFixed(2)}h | Max Temp:{" "}
								{stats.maxTemp.toFixed(0)}°C
							</p>
						)}
					</div>

					<div className="space-y-4">
						<div className="flex gap-4 text-sm">
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
								<span>Cooling</span>
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

										const point = payload[0].payload as TrajectoryPoint;
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
					</div>
				</div>
			)}
		</Card>
	);
}
