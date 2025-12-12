import {
	AlertCircle,
	AlertTriangle,
	ArrowUp,
	Clock,
	Flame,
	Gauge,
	Loader2,
	Pause,
	RefreshCw,
	Snowflake,
	Thermometer,
	TrendingUp,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { PicoAPIError } from "@/lib/pico/client";
import { useClearError } from "@/lib/pico/hooks";
import { useProfileCache } from "@/lib/pico/profile-cache";
import type { KilnStatus, ProfileStep } from "@/lib/pico/types";

interface KilnStatusDisplayProps {
	status?: KilnStatus;
	isLoading: boolean;
	error: PicoAPIError | null;
	onRefresh?: () => void;
}

// Step icon component
function StepIcon({ type }: { type: ProfileStep["type"] }) {
	switch (type) {
		case "ramp":
			return <ArrowUp className="w-4 h-4 text-orange-500" />;
		case "hold":
			return <Pause className="w-4 h-4 text-yellow-500" />;
		case "cooling":
			return <Snowflake className="w-4 h-4 text-blue-500" />;
		default:
			return <Flame className="w-4 h-4" />;
	}
}

// Format step details for display
function formatStepInfo(step: ProfileStep, tempUnit: string): string {
	const unit = tempUnit === "f" ? "°F" : "°C";
	const rateUnit = tempUnit === "f" ? "°F/h" : "°C/h";

	switch (step.type) {
		case "ramp":
			return `Ramp to ${step.target_temp}${unit} at ${step.desired_rate}${rateUnit}`;
		case "hold":
			if (step.duration) {
				const hours = Math.floor(step.duration / 3600);
				const minutes = Math.floor((step.duration % 3600) / 60);
				const timeStr =
					hours > 0
						? `${hours}h ${minutes > 0 ? `${minutes}m` : ""}`
						: `${minutes}m`;
				return `Hold at ${step.target_temp}${unit} for ${timeStr}`;
			}
			return `Hold at ${step.target_temp}${unit}`;
		case "cooling":
			if (step.target_temp !== undefined && step.desired_rate !== undefined) {
				return `Cool to ${step.target_temp}${unit} at ${step.desired_rate}${rateUnit}`;
			}
			if (step.target_temp !== undefined) {
				return `Cool to ${step.target_temp}${unit}`;
			}
			return "Natural cooling";
		default:
			return "Unknown step";
	}
}

export function KilnStatusDisplay({
	status,
	isLoading,
	error,
	onRefresh,
}: KilnStatusDisplayProps) {
	const { mutate: clearError, isPending: isClearingPending, isError: isClearingError, error: clearingError } = useClearError();
	const { getProfile } = useProfileCache();

	// Get the running profile data from cache if available
	const runningProfile = status?.profile_name
		? getProfile(status.profile_name)
		: undefined;
	const currentStep =
		runningProfile && status?.step_index !== undefined
			? runningProfile.steps[status.step_index]
			: undefined;

	if (error) {
		return (
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<CardTitle>Kiln Status</CardTitle>
						{onRefresh && (
							<Button
								variant="ghost"
								size="sm"
								onClick={onRefresh}
								disabled={isLoading}
							>
								<RefreshCw
									className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`}
								/>
							</Button>
						)}
					</div>
				</CardHeader>
				<CardContent>
					<Alert variant="destructive">
						<AlertTriangle className="w-4 h-4" />
						<AlertDescription>
							Failed to load kiln status: {error.message}
						</AlertDescription>
					</Alert>
				</CardContent>
			</Card>
		);
	}

	if (isLoading && !status) {
		return (
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<CardTitle>Kiln Status</CardTitle>
						{onRefresh && (
							<Button
								variant="ghost"
								size="sm"
								onClick={onRefresh}
								disabled={isLoading}
							>
								<RefreshCw className="w-4 h-4 animate-spin" />
							</Button>
						)}
					</div>
				</CardHeader>
				<CardContent>
					<div className="flex items-center justify-center py-8">
						<Loader2 className="w-8 h-8 animate-spin text-muted-foreground" />
					</div>
				</CardContent>
			</Card>
		);
	}

	if (!status) {
		return (
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<CardTitle>Kiln Status</CardTitle>
						{onRefresh && (
							<Button
								variant="ghost"
								size="sm"
								onClick={onRefresh}
								disabled={isLoading}
							>
								<RefreshCw className="w-4 h-4" />
							</Button>
						)}
					</div>
				</CardHeader>
				<CardContent>
					<p className="text-muted-foreground">No status data available</p>
				</CardContent>
			</Card>
		);
	}

	const getStateBadge = (state: KilnStatus["state"]) => {
		switch (state) {
			case "IDLE":
				return <Badge variant="outline">Idle</Badge>;
			case "RUNNING":
				return <Badge className="bg-blue-600 hover:bg-blue-700">Running</Badge>;
			case "TUNING":
				return (
					<Badge className="bg-purple-600 hover:bg-purple-700">Tuning</Badge>
				);
			case "ERROR":
				return <Badge variant="destructive">Error</Badge>;
			default:
				return <Badge variant="outline">{state}</Badge>;
		}
	};

	const formatTemp = (temp: number) => {
		return `${temp.toFixed(1)}°C`;
	};

	const formatDuration = (seconds?: number) => {
		if (!seconds) return "N/A";
		const hours = Math.floor(seconds / 3600);
		const minutes = Math.floor((seconds % 3600) / 60);
		const secs = Math.floor(seconds % 60);

		if (hours > 0) {
			return `${hours}h ${minutes}m`;
		}
		if (minutes > 0) {
			return `${minutes}m ${secs}s`;
		}
		return `${secs}s`;
	};

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<div className="flex items-center gap-3">
							<CardTitle>Kiln Status</CardTitle>
							{getStateBadge(status.state)}
						</div>
						{onRefresh && (
							<Button
								variant="ghost"
								size="sm"
								onClick={onRefresh}
								disabled={isLoading}
								title="Refresh status"
							>
								<RefreshCw
									className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`}
								/>
							</Button>
						)}
					</div>
				</CardHeader>
				<CardContent className="space-y-4">
					{/* Temperature Display */}
					<div className="grid grid-cols-2 gap-4">
						<div className="space-y-1">
							<div className="flex items-center gap-2 text-sm text-muted-foreground">
								<Thermometer className="w-4 h-4" />
								Current Temperature
							</div>
							<div className="text-2xl font-bold">
								{formatTemp(status.current_temp)}
							</div>
						</div>

						{status.target_temp !== undefined && (
							<div className="space-y-1">
								<div className="text-sm text-muted-foreground">
									Target Temperature
								</div>
								<div className="text-2xl font-bold">
									{formatTemp(status.target_temp)}
								</div>
							</div>
						)}
					</div>

					{/* SSR and Heating Rates */}
					<div className="grid grid-cols-2 gap-4 pt-2 border-t">
						<div className="space-y-2">
							<div className="flex items-center gap-2">
								<Flame
									className={`w-5 h-5 ${(status.ssr_output ?? 0) > 0 ? "text-orange-500" : "text-gray-400"}`}
								/>
								<span className="text-sm font-medium">SSR Output</span>
							</div>
							<div className="flex items-center gap-2">
								<Gauge className="w-4 h-4 text-muted-foreground" />
								<span className="text-sm font-bold">
									{status.ssr_output !== undefined
										? status.ssr_output > 0
											? `${status.ssr_output.toFixed(1)}%`
											: "OFF"
										: "N/A"}
								</span>
							</div>
						</div>

						{(status.actual_rate !== undefined ||
							status.current_rate !== undefined) && (
								<div className="space-y-2">
									<div className="flex items-center gap-2">
										<TrendingUp className="w-5 h-5 text-muted-foreground" />
										<span className="text-sm font-medium">Heating Rate</span>
									</div>
									{status.actual_rate !== undefined && (
										<div className="text-sm">
											Actual: <strong>{status.actual_rate.toFixed(1)}°C/h</strong>
										</div>
									)}
									{status.current_rate !== undefined &&
										status.state === "RUNNING" && (
											<div className="text-sm text-muted-foreground">
												Target: {status.current_rate.toFixed(1)}°C/h
											</div>
										)}
									{status.adaptation_count !== undefined &&
										status.state === "RUNNING" && (
											<div className="text-xs text-muted-foreground">
												Adaptations: {status.adaptation_count}
											</div>
										)}
								</div>
							)}
					</div>

					{/* Recovery Mode Warning */}
					{status.is_recovering && (
						<Alert className="border-orange-500 bg-orange-50 dark:bg-orange-950/20">
							<AlertCircle className="h-4 w-4 text-orange-600" />
							<AlertDescription className="text-orange-800 dark:text-orange-400">
								<strong>Recovery Mode:</strong> Kiln is recovering from
								temperature drop.
								{status.recovery_target_temp !== undefined && (
									<> Target: {formatTemp(status.recovery_target_temp)}</>
								)}
							</AlertDescription>
						</Alert>
					)}

					{(status.error_message || status.state === "ERROR") && (
						<>
							<Alert variant="destructive">
								<AlertTriangle className="w-4 h-4" />
								<AlertDescription>
									<div className="flex flex-col gap-2">
										<span>{status.error_message ?? "Error mode"}</span>
										<Button
											onClick={() => clearError()}
											disabled={
												isClearingPending || status.state !== "ERROR"
											}
											variant="outline"
											size="sm"
											className="self-end"
										>
											{isClearingPending ? (
												<>
													<Loader2 className="w-4 h-4 mr-2 animate-spin" />
													Clearing...
												</>
											) : (
												<>
													<AlertCircle className="w-4 h-4 mr-2" />
													Clear Error
												</>
											)}
										</Button>
									</div>
								</AlertDescription>
							</Alert>
							{isClearingError && (
								<Alert variant="destructive">
									<AlertTriangle className="w-4 h-4" />
									<AlertDescription>
										Failed to clear error:{" "}
										{clearingError?.message || "Unknown error"}
									</AlertDescription>
								</Alert>
							)}
						</>
					)}
				</CardContent>
			</Card>

			{/* Profile Progress (for RUNNING state) */}
			{status.state === "RUNNING" && status.profile_name && (
				<Card>
					<CardHeader>
						<div className="flex items-center justify-between">
							<CardTitle>Profile Progress</CardTitle>
							{status.step_index !== undefined &&
								status.total_steps !== undefined && (
									<Badge variant="outline" className="text-sm">
										Step {status.step_index + 1} / {status.total_steps}
									</Badge>
								)}
						</div>
					</CardHeader>
					<CardContent className="space-y-4">
						{/* Profile Name */}
						<div className="space-y-1">
							<div className="text-sm text-muted-foreground">
								Active Profile
							</div>
							<div className="text-xl font-bold">{status.profile_name}</div>
						</div>

						{/* Current Step Info */}
						{status.step_name && (
							<div className="p-3 rounded-lg bg-muted/50 space-y-2">
								<div className="flex items-center gap-2">
									{currentStep && (
										<StepIcon type={currentStep.type} />
									)}
									<span className="text-sm font-medium flex-1">
										{currentStep && runningProfile
											? formatStepInfo(currentStep, runningProfile.temp_units)
											: `Current Step: ${status.step_name}`}
									</span>
								</div>
								{status.desired_rate !== undefined &&
									status.step_name === "ramp" && (
										<div className="text-xs text-muted-foreground">
											Target rate: {status.desired_rate.toFixed(0)}°C/h
										</div>
									)}
							</div>
						)}

						{/* Time Information */}
						{status.elapsed !== undefined && (
							<div className="grid grid-cols-1 gap-4 text-sm pt-2 border-t">
								<div>
									<div className="text-muted-foreground">Elapsed Time</div>
									<div className="font-medium flex items-center gap-1 mt-1">
										<Clock className="w-4 h-4" />
										{formatDuration(status.elapsed)}
									</div>
								</div>
							</div>
						)}

						{/* All Steps Overview (from cached profile) */}
						{runningProfile && (
							<div className="pt-2 border-t space-y-2">
								<div className="text-sm font-medium text-muted-foreground">
									Profile Steps
								</div>
								<div className="space-y-1">
									{runningProfile.steps.map((step, index) => {
										const isCurrent = index === status.step_index;
										const isCompleted =
											status.step_index !== undefined &&
											index < status.step_index;
										return (
											<div
												key={index}
												className={`flex items-center gap-2 text-xs p-2 rounded ${
													isCurrent
														? "bg-blue-100 dark:bg-blue-900/30 border border-blue-300 dark:border-blue-700"
														: isCompleted
															? "bg-muted/30 text-muted-foreground"
															: "bg-muted/50"
												}`}
											>
												<Badge
													variant={isCurrent ? "default" : "outline"}
													className="w-5 h-5 p-0 justify-center text-xs"
												>
													{index + 1}
												</Badge>
												<StepIcon type={step.type} />
												<span className="flex-1 truncate">
													{formatStepInfo(step, runningProfile.temp_units)}
												</span>
												{isCurrent && (
													<Badge variant="secondary" className="text-xs">
														Current
													</Badge>
												)}
											</div>
										);
									})}
								</div>
							</div>
						)}
					</CardContent>
				</Card>
			)}

			{/* Tuning Progress (for TUNING state) */}
			{status.state === "TUNING" && status.tuning && (
				<Card>
					<CardHeader>
						<div className="flex items-center justify-between">
							<CardTitle>Tuning Progress</CardTitle>
							{status.step_index !== undefined &&
								status.total_steps !== undefined && (
									<Badge className="bg-purple-600 hover:bg-purple-700">
										Step {status.step_index + 1} / {status.total_steps}
									</Badge>
								)}
						</div>
					</CardHeader>
					<CardContent className="space-y-4">
						{/* Tuning Mode */}
						<div className="space-y-1">
							<div className="text-sm text-muted-foreground">Tuning Mode</div>
							<div className="text-xl font-bold capitalize">
								{status.tuning.mode?.replace("_", " ")}
							</div>
						</div>

						{/* Current Phase/Step */}
						{status.step_name && (
							<div className="p-3 rounded-lg bg-purple-50 dark:bg-purple-950/20 border border-purple-200 dark:border-purple-800 space-y-2">
								<div className="flex items-center justify-between">
									<span className="text-sm font-medium">
										Current Phase: {status.step_name}
									</span>
								</div>
								{status.tuning.phase && (
									<div className="text-sm text-muted-foreground">
										{status.tuning.phase}
									</div>
								)}
							</div>
						)}

						{/* Tuning Information */}
						<div className="grid grid-cols-2 gap-4 text-sm pt-2 border-t">
							{status.elapsed !== undefined && (
								<div>
									<div className="text-muted-foreground">Elapsed Time</div>
									<div className="font-medium flex items-center gap-1 mt-1">
										<Clock className="w-4 h-4" />
										{formatDuration(status.elapsed)}
									</div>
								</div>
							)}
							{status.tuning.max_temp !== undefined && (
								<div>
									<div className="text-muted-foreground">Max Temperature</div>
									<div className="font-medium mt-1">
										{formatTemp(status.tuning.max_temp)}
									</div>
								</div>
							)}
						</div>

						{/* Oscillation Count (if available) */}
						{status.tuning.oscillation_count !== undefined && (
							<div className="text-sm">
								<span className="text-muted-foreground">
									Oscillations Detected:
								</span>{" "}
								<span className="font-medium">
									{status.tuning.oscillation_count}
								</span>
							</div>
						)}
					</CardContent>
				</Card>
			)}

			{status.pid && (
				<Card>
					<CardHeader>
						<CardTitle>PID Control</CardTitle>
					</CardHeader>
					<CardContent>
						<div className="grid grid-cols-2 gap-4 text-sm">
							{status.pid.kp !== undefined && (
								<div>
									<div className="text-muted-foreground">Kp</div>
									<div className="font-mono">{status.pid.kp.toFixed(3)}</div>
								</div>
							)}
							{status.pid.ki !== undefined && (
								<div>
									<div className="text-muted-foreground">Ki</div>
									<div className="font-mono">{status.pid.ki.toFixed(3)}</div>
								</div>
							)}
							{status.pid.kd !== undefined && (
								<div>
									<div className="text-muted-foreground">Kd</div>
									<div className="font-mono">{status.pid.kd.toFixed(3)}</div>
								</div>
							)}
							{status.pid.output !== undefined && (
								<div>
									<div className="text-muted-foreground">Output</div>
									<div className="font-mono">
										{status.pid.output.toFixed(1)}%
									</div>
								</div>
							)}
						</div>
					</CardContent>
				</Card>
			)}
		</div>
	);
}
