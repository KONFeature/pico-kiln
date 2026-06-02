import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useBlocker } from "@tanstack/react-router";
import { curveLinear } from "@visx/curve";
import {
	CheckCircle,
	ChevronDown,
	ChevronUp,
	CircleAlert,
	Download,
	FileUp,
	Plus,
	RotateCcw,
	Trash2,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";
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
import { ErrorAlert } from "@/components/ErrorAlert";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
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
import { parseProfileText } from "@/lib/profile-schema";
import { buildProfileChart, calculateTrajectory } from "@/lib/profile-utils";
import {
	formatETA,
	formatStepInfo,
	isStepControlledCooldown,
	StepIcon,
} from "@/lib/step-utils";
import { useProfileDraft } from "@/lib/use-profile-draft";
import { FileSourceSelector } from "./FileSourceSelector";

const DEFAULT_PROFILE: Profile = {
	name: "New Profile",
	temp_units: "c",
	description: "",
	steps: [
		{ type: "ramp", target_temp: 600, desired_rate: 100 },
		{ type: "hold", target_temp: 600, duration: 600 },
	],
};

/** Minutes display for a duration stored in seconds, preserving fractional minutes. */
function minutesValue(seconds?: number): string {
	if (seconds === undefined || seconds === null) {
		return "";
	}
	return String(Math.round((seconds / 60) * 100) / 100);
}

function defaultFilename(profile: Profile): string {
	return `${profile.name.replace(/\s+/g, "_").toLowerCase()}.json`;
}

export function ProfileEditor() {
	const { profile, setProfile, clearDraft } = useProfileDraft(DEFAULT_PROFILE);
	const [savedSnapshot, setSavedSnapshot] = useState<string>(() =>
		JSON.stringify(profile),
	);
	const [exportMode, setExportMode] = useState<"download" | "upload">(
		"download",
	);
	const [uploadFilename, setUploadFilename] = useState<string>("");
	const [importError, setImportError] = useState<string | null>(null);
	const [stepToDelete, setStepToDelete] = useState<number | null>(null);
	const [pendingImport, setPendingImport] = useState<string | null>(null);
	const [pendingUpload, setPendingUpload] = useState<string | null>(null);
	const [showReset, setShowReset] = useState(false);
	const { client, isConfigured } = usePico();
	const queryClient = useQueryClient();

	const isDirty = JSON.stringify(profile) !== savedSnapshot;
	const isDirtyRef = useRef(isDirty);
	isDirtyRef.current = isDirty;

	// Warn before navigating away or reloading with unsaved (un-exported) work.
	const blocker = useBlocker({
		shouldBlockFn: () => isDirtyRef.current,
		enableBeforeUnload: () => isDirtyRef.current,
		withResolver: true,
	});

	// Existing profile filenames on the Pico, used to warn before overwriting.
	const { data: profilesData } = useQuery({
		queryKey: ["files", "profiles"],
		queryFn: () => client?.listFiles("profiles"),
		enabled: isConfigured && client !== null,
		retry: 1,
	});
	const existingNames = useMemo(
		() =>
			new Set(
				profilesData?.success ? profilesData.files.map((f) => f.name) : [],
			),
		[profilesData],
	);

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
				setSavedSnapshot(JSON.stringify(profile));
				queryClient.invalidateQueries({ queryKey: ["files", "profiles"] });
			}
		},
	});

	const segments = useMemo(() => calculateTrajectory(profile), [profile]);
	const chart = useMemo(() => buildProfileChart(segments), [segments]);

	// calculateTrajectory emits one segment per step in order, so index aligns.
	const stepDurations = useMemo(
		() =>
			segments.map((s) => {
				const points = s.data;
				return (
					(points[points.length - 1].time_hours - points[0].time_hours) * 3600
				);
			}),
		[segments],
	);

	const stats = useMemo(() => {
		if (segments.length === 0) return null;
		const allTemps = segments.flatMap((s) => s.data.map((p) => p.temp));
		const maxTemp = Math.max(...allTemps);
		const lastSegment = segments[segments.length - 1];
		const duration = lastSegment.data[lastSegment.data.length - 1].time_hours;
		return { maxTemp, duration };
	}, [segments]);

	const validateStep = (step: ProfileStep): string | null => {
		if (step.type === "ramp") {
			if (step.target_temp === undefined || step.target_temp === null) {
				return "Target temperature is required for ramp steps";
			}
			if (step.desired_rate === undefined || step.desired_rate === null) {
				return "Desired rate is required for ramp steps";
			}
		}
		if (step.type === "hold") {
			if (step.target_temp === undefined || step.target_temp === null) {
				return "Target temperature is required for hold steps";
			}
		}
		return null;
	};

	const stepErrors = profile.steps.map((step) => validateStep(step));
	const hasValidationErrors = stepErrors.some((error) => error !== null);

	const unit = profile.temp_units.toUpperCase();

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
			steps: [
				...prev.steps,
				{ type: "ramp", target_temp: 0, desired_rate: 100 },
			],
		}));
	};

	const removeStep = (index: number) => {
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

	const applyImport = (content: string) => {
		const result = parseProfileText(content);
		if (!result.ok) {
			setImportError(result.error);
			return;
		}
		setProfile(result.profile);
		setSavedSnapshot(JSON.stringify(result.profile));
		setImportError(null);
	};

	// Paste ("manual-input") is treated as live editing; a file/Pico load is a
	// discrete overwrite, so guard it behind a confirm when the draft is dirty.
	const handleFileSelected = (content: string, filename: string) => {
		if (filename === "manual-input") {
			applyImport(content);
			return;
		}
		if (isDirtyRef.current) {
			setPendingImport(content);
		} else {
			applyImport(content);
		}
	};

	const downloadProfile = () => {
		if (hasValidationErrors) return;
		const json = JSON.stringify(profile, null, 2);
		const blob = new Blob([json], { type: "application/json" });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = defaultFilename(profile);
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);
		setSavedSnapshot(JSON.stringify(profile));
	};

	const doUpload = (filename: string) => {
		const json = JSON.stringify(profile);
		uploadMutation.mutate({ filename, content: json });
	};

	const uploadToPico = () => {
		if (hasValidationErrors) return;
		const filename = uploadFilename || defaultFilename(profile);
		if (existingNames.has(filename)) {
			setPendingUpload(filename);
		} else {
			doUpload(filename);
		}
	};

	const resetDraft = () => {
		clearDraft();
		setProfile(DEFAULT_PROFILE);
		setSavedSnapshot(JSON.stringify(DEFAULT_PROFILE));
		setImportError(null);
		setShowReset(false);
	};

	return (
		<Card>
			<CardHeader>
				<div className="flex items-start justify-between gap-2">
					<div>
						<CardTitle>Profile Editor</CardTitle>
						<CardDescription>
							Create and edit kiln firing profiles with live visualization
						</CardDescription>
					</div>
					<div className="flex items-center gap-2">
						{isDirty && (
							<span className="text-xs text-muted-foreground whitespace-nowrap">
								Unsaved
							</span>
						)}
						<Button
							variant="ghost"
							size="sm"
							onClick={() => setShowReset(true)}
							aria-label="Start a new profile"
						>
							<RotateCcw className="w-4 h-4" />
						</Button>
					</div>
				</div>
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

				<Collapsible>
					<CollapsibleTrigger asChild>
						<Button
							variant="outline"
							size="sm"
							className="w-full justify-between"
						>
							<span>Import an existing profile</span>
							<ChevronDown className="w-4 h-4" />
						</Button>
					</CollapsibleTrigger>
					<CollapsibleContent className="pt-3">
						<FileSourceSelector
							directory="profiles"
							accept=".json"
							onFileSelected={handleFileSelected}
							label="Import Profile"
							description="Load a profile to edit (replaces the current draft)"
						/>
						{importError && (
							<Alert variant="destructive" className="mt-3">
								<CircleAlert className="h-4 w-4" />
								<AlertDescription>{importError}</AlertDescription>
							</Alert>
						)}
					</CollapsibleContent>
				</Collapsible>
			</CardContent>

			{chart.chartData.length > 0 && (
				<div className="sticky top-0 z-20 -mx-px border-y bg-background/95 px-6 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/80">
					<div className="flex items-center justify-between">
						<h3 className="text-sm font-semibold">Live Preview</h3>
						{stats && (
							<p className="text-xs text-muted-foreground">
								{stats.duration.toFixed(1)}h · max {stats.maxTemp.toFixed(0)}°
								{unit}
							</p>
						)}
					</div>
					<LineChart
						data={chart.chartData}
						xDataKey="t"
						aspectRatio="auto"
						className="h-44 mt-2"
						margin={{ top: 14, right: 12, bottom: 24, left: 40 }}
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
					<div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
						<LegendDot className="bg-chart-heating" label="Ramp" />
						<LegendDot className="bg-chart-hold" label="Hold" />
						<LegendDot
							className="bg-chart-cooling"
							label="Controlled cooling"
						/>
						<LegendDot
							className="bg-chart-natural-cooling"
							label="Natural cooling"
						/>
					</div>
				</div>
			)}

			<div className="px-6 pb-6 space-y-6 pt-6">
				<h3 className="text-lg font-semibold">Profile Steps</h3>

				<div className="space-y-3">
					{profile.steps.map((step, index) => {
						const error = stepErrors[index];
						const prevTargetTemp =
							index > 0 ? profile.steps[index - 1].target_temp : undefined;
						const controlledCooldown = isStepControlledCooldown(
							step,
							prevTargetTemp,
						);
						const durationSeconds = stepDurations[index] ?? 0;
						return (
							<div
								key={index}
								className={`p-4 rounded-lg border transition-colors ${
									error
										? "bg-destructive/10 border-destructive/40"
										: "bg-muted/30"
								}`}
							>
								<div className="flex items-start gap-3">
									<div className="flex flex-col gap-1">
										<Button
											variant="ghost"
											size="icon"
											onClick={() => moveStepUp(index)}
											disabled={index === 0}
											aria-label="Move step up"
											className="h-8 w-8"
										>
											<ChevronUp className="w-4 h-4" />
										</Button>
										<Button
											variant="ghost"
											size="icon"
											onClick={() => moveStepDown(index)}
											disabled={index === profile.steps.length - 1}
											aria-label="Move step down"
											className="h-8 w-8"
										>
											<ChevronDown className="w-4 h-4" />
										</Button>
									</div>

									<div className="flex-1 space-y-3">
										<div className="flex items-center gap-4">
											<div className="flex items-center gap-2">
												<StepIcon
													type={step.type}
													isControlledCooldown={controlledCooldown}
												/>
												<span className="font-semibold">Step {index + 1}</span>
											</div>
											<Select
												value={step.type}
												onValueChange={(value) =>
													updateStep(index, { type: value as ProfileStepType })
												}
											>
												<SelectTrigger className="w-40">
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													<SelectItem value="ramp">Ramp</SelectItem>
													<SelectItem value="hold">Hold</SelectItem>
													<SelectItem value="cooling">Cooling</SelectItem>
												</SelectContent>
											</Select>
										</div>

										{!error && (
											<p className="text-xs text-muted-foreground">
												{formatStepInfo(
													step,
													profile.temp_units,
													prevTargetTemp,
												)}
												{step.type !== "hold" && durationSeconds >= 60
													? ` · ≈ ${formatETA(durationSeconds)}`
													: ""}
											</p>
										)}

										<div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-4">
											{step.type !== "cooling" && (
												<div>
													<Label>Target Temp (°{unit})</Label>
													<Input
														type="number"
														value={step.target_temp ?? ""}
														onChange={(e) => {
															const value = e.target.value;
															updateStep(index, {
																target_temp:
																	value === "" ? undefined : parseFloat(value),
															});
														}}
														placeholder="0"
													/>
												</div>
											)}

											{step.type === "cooling" && (
												<div>
													<Label>Target Temp (°{unit}) - Optional</Label>
													<Input
														type="number"
														value={step.target_temp ?? ""}
														onChange={(e) => {
															const value = e.target.value;
															updateStep(index, {
																target_temp:
																	value === ""
																		? undefined
																		: parseFloat(value) || undefined,
															});
														}}
														placeholder="Leave empty for natural cooling"
													/>
													<p className="text-xs text-muted-foreground mt-1">
														If empty, cooling continues until manually stopped
													</p>
												</div>
											)}

											{step.type === "ramp" && (
												<>
													<div>
														<Label>Desired Rate (°{unit}/h)</Label>
														<Input
															type="number"
															value={step.desired_rate ?? ""}
															onChange={(e) => {
																const value = e.target.value;
																updateStep(index, {
																	desired_rate:
																		value === ""
																			? undefined
																			: parseFloat(value),
																});
															}}
															placeholder="100"
														/>
													</div>
													<div>
														<Label>Min Rate (°{unit}/h)</Label>
														<Input
															type="number"
															value={step.min_rate ?? ""}
															onChange={(e) => {
																const value = e.target.value;
																updateStep(index, {
																	min_rate:
																		value === ""
																			? undefined
																			: parseFloat(value),
																});
															}}
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
														step="any"
														value={minutesValue(step.duration)}
														onChange={(e) => {
															const value = e.target.value;
															updateStep(index, {
																duration:
																	value === ""
																		? undefined
																		: parseFloat(value) * 60,
															});
														}}
														placeholder="0"
													/>
												</div>
											)}
										</div>

										{error && (
											<Alert variant="destructive" className="mt-3">
												<CircleAlert className="h-4 w-4" />
												<AlertDescription>{error}</AlertDescription>
											</Alert>
										)}
									</div>

									<Button
										variant="ghost"
										size="icon"
										onClick={() => setStepToDelete(index)}
										disabled={profile.steps.length <= 1}
										aria-label="Delete step"
										className="h-8 w-8"
									>
										<Trash2 className="w-4 h-4 text-destructive" />
									</Button>
								</div>
							</div>
						);
					})}
				</div>

				<Button onClick={addStep} size="sm" className="w-full">
					<Plus className="w-4 h-4 mr-2" />
					Add Step
				</Button>
			</div>

			<div className="px-6 pb-6 space-y-3 border-t pt-6">
				<div>
					<Label className="text-base font-semibold">Export Profile</Label>
					<p className="text-sm text-muted-foreground">Save your profile</p>
				</div>

				{hasValidationErrors && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>
							Please fix all validation errors before exporting the profile
						</AlertDescription>
					</Alert>
				)}

				<div className="flex gap-2">
					<Button
						variant={exportMode === "download" ? "default" : "outline"}
						size="sm"
						onClick={() => setExportMode("download")}
						disabled={hasValidationErrors}
					>
						<Download className="w-4 h-4 mr-2" />
						Download
					</Button>
					<Button
						variant={exportMode === "upload" ? "default" : "outline"}
						size="sm"
						onClick={() => setExportMode("upload")}
						disabled={!isConfigured || hasValidationErrors}
					>
						<FileUp className="w-4 h-4 mr-2" />
						Upload to Pico
					</Button>
				</div>

				{exportMode === "download" && (
					<Button
						onClick={downloadProfile}
						size="sm"
						className="w-full"
						disabled={hasValidationErrors}
					>
						<Download className="w-4 h-4 mr-2" />
						Download as JSON
					</Button>
				)}

				{exportMode === "upload" && (
					<div className="space-y-2">
						{!isConfigured && (
							<Alert>
								<CircleAlert className="h-4 w-4" />
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
										placeholder={defaultFilename(profile)}
										className="mt-1"
									/>
									<p className="text-xs text-muted-foreground mt-1">
										Leave empty to use profile name
									</p>
								</div>
								<Button
									onClick={uploadToPico}
									disabled={uploadMutation.isPending || hasValidationErrors}
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
									<ErrorAlert error={uploadMutation.error} />
								)}
							</>
						)}
					</div>
				)}
			</div>

			{/* Delete-step confirmation */}
			<Dialog
				open={stepToDelete !== null}
				onOpenChange={(open) => !open && setStepToDelete(null)}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete step?</DialogTitle>
						<DialogDescription>
							{stepToDelete !== null &&
								`Step ${stepToDelete + 1} will be removed from the profile. This can't be undone.`}
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setStepToDelete(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								if (stepToDelete !== null) removeStep(stepToDelete);
								setStepToDelete(null);
							}}
						>
							<Trash2 className="w-4 h-4 mr-2" />
							Delete
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Import-overwrite confirmation */}
			<Dialog
				open={pendingImport !== null}
				onOpenChange={(open) => !open && setPendingImport(null)}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Replace current draft?</DialogTitle>
						<DialogDescription>
							You have unsaved changes. Importing will replace the profile
							you're editing.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setPendingImport(null)}>
							Keep editing
						</Button>
						<Button
							onClick={() => {
								if (pendingImport !== null) applyImport(pendingImport);
								setPendingImport(null);
							}}
						>
							Replace
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Upload-overwrite confirmation */}
			<Dialog
				open={pendingUpload !== null}
				onOpenChange={(open) => !open && setPendingUpload(null)}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Overwrite file on Pico?</DialogTitle>
						<DialogDescription>
							{pendingUpload &&
								`A profile named "${pendingUpload}" already exists on the Pico. Uploading will overwrite it.`}
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setPendingUpload(null)}>
							Cancel
						</Button>
						<Button
							onClick={() => {
								if (pendingUpload) doUpload(pendingUpload);
								setPendingUpload(null);
							}}
						>
							<FileUp className="w-4 h-4 mr-2" />
							Overwrite
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Reset / new profile confirmation */}
			<Dialog open={showReset} onOpenChange={setShowReset}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Start a new profile?</DialogTitle>
						<DialogDescription>
							This clears the current draft and starts from a fresh default
							profile.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setShowReset(false)}>
							Cancel
						</Button>
						<Button variant="destructive" onClick={resetDraft}>
							<RotateCcw className="w-4 h-4 mr-2" />
							New profile
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Unsaved-changes navigation guard */}
			<Dialog
				open={blocker.status === "blocked"}
				onOpenChange={(open) => {
					if (!open && blocker.status === "blocked") blocker.reset();
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Leave the editor?</DialogTitle>
						<DialogDescription>
							You have unsaved changes. Your draft is kept on this device, but
							it hasn't been downloaded or uploaded to the kiln.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => {
								if (blocker.status === "blocked") blocker.reset();
							}}
						>
							Stay
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								if (blocker.status === "blocked") blocker.proceed();
							}}
						>
							Leave
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</Card>
	);
}

function LegendDot({ className, label }: { className: string; label: string }) {
	return (
		<div className="flex items-center gap-1.5">
			<div className={`w-3 h-3 rounded ${className}`} />
			<span>{label}</span>
		</div>
	);
}
