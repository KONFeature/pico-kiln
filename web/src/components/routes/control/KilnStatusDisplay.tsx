import {
	AlertTriangle,
	ChevronDown,
	CircleAlert,
	Clock,
	Flame,
	Gauge,
	Loader2,
	RefreshCw,
	TrendingUp,
	WifiOff,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { ErrorAlert } from "@/components/ErrorAlert";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import type { PicoAPIError } from "@/lib/pico/client";
import { STATE_DESCRIPTIONS, STATE_LABELS } from "@/lib/pico/errors";
import { useClearError } from "@/lib/pico/hooks";
import { useProfileCache } from "@/lib/pico/profile-cache";
import type { KilnStatus } from "@/lib/pico/types";
import {
	calculateETAs,
	formatETA,
	formatStepInfo,
	isStepControlledCooldown,
	StepIcon,
} from "@/lib/step-utils";
import { cn } from "@/lib/utils";
import { LiveTempChart, useTemperatureHistory } from "./LiveTempChart";
import { TemperatureGauge } from "./TemperatureGauge";

interface KilnStatusDisplayProps {
	status?: KilnStatus;
	isLoading: boolean;
	error: PicoAPIError | null;
	dataUpdatedAt?: number;
	onRefresh?: () => void;
}

function StatusHeader({
	onRefresh,
	isLoading,
	badge,
}: {
	onRefresh?: () => void;
	isLoading: boolean;
	badge?: ReactNode;
}) {
	return (
		<CardHeader>
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-3">
					<CardTitle>Kiln Status</CardTitle>
					{badge}
				</div>
				{onRefresh && (
					<Button
						variant="ghost"
						size="sm"
						onClick={onRefresh}
						disabled={isLoading}
						title="Refresh status"
					>
						<RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
					</Button>
				)}
			</div>
		</CardHeader>
	);
}

function LastUpdated({
	updatedAt,
	stale,
}: {
	updatedAt?: number;
	stale: boolean;
}) {
	const [, force] = useState(0);
	useEffect(() => {
		const id = setInterval(() => force((n) => n + 1), 1000);
		return () => clearInterval(id);
	}, []);

	if (!updatedAt) return null;
	const secs = Math.max(0, Math.round((Date.now() - updatedAt) / 1000));
	const ago =
		secs < 1
			? "just now"
			: secs < 60
				? `${secs}s ago`
				: `${Math.floor(secs / 60)}m ago`;

	return (
		<span
			className={cn(
				"inline-flex items-center gap-1.5 text-xs",
				stale ? "text-warning" : "text-muted-foreground",
			)}
		>
			<span
				className={cn(
					"inline-block h-1.5 w-1.5 rounded-full",
					stale ? "bg-warning animate-pulse" : "bg-success",
				)}
			/>
			Updated {ago}
		</span>
	);
}

export function KilnStatusDisplay({
	status,
	isLoading,
	error,
	dataUpdatedAt,
	onRefresh,
}: KilnStatusDisplayProps) {
	const {
		mutate: clearError,
		isPending: isClearingPending,
		isError: isClearingError,
		error: clearingError,
	} = useClearError();
	const { getProfile } = useProfileCache();
	const tempHistory = useTemperatureHistory(status, dataUpdatedAt);

	const runningProfile = status?.profile_name
		? getProfile(status.profile_name)
		: undefined;
	const currentStep =
		runningProfile && status?.step_index !== undefined
			? runningProfile.steps[status.step_index]
			: undefined;
	const prevStep =
		runningProfile && status?.step_index !== undefined && status.step_index > 0
			? runningProfile.steps[status.step_index - 1]
			: undefined;
	const currentStepIsControlledCooldown = currentStep
		? isStepControlledCooldown(currentStep, prevStep?.target_temp)
		: false;

	const etas = useMemo(() => {
		if (!runningProfile || !status) return null;
		return calculateETAs(status, runningProfile);
	}, [status, runningProfile]);

	if (error && !status) {
		return (
			<Card>
				<StatusHeader onRefresh={onRefresh} isLoading={isLoading} />
				<CardContent>
					<ErrorAlert
						error={error}
						action={
							onRefresh && (
								<Button variant="outline" size="sm" onClick={onRefresh}>
									<RefreshCw className="w-4 h-4 mr-2" />
									Try again
								</Button>
							)
						}
					/>
				</CardContent>
			</Card>
		);
	}

	if (isLoading && !status) {
		return (
			<Card>
				<StatusHeader onRefresh={onRefresh} isLoading={isLoading} />
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
				<StatusHeader onRefresh={onRefresh} isLoading={isLoading} />
				<CardContent>
					<p className="text-muted-foreground">No status data available</p>
				</CardContent>
			</Card>
		);
	}

	const getStateBadge = (state: KilnStatus["state"]) => {
		switch (state) {
			case "IDLE":
				return <Badge variant="outline">{STATE_LABELS.IDLE}</Badge>;
			case "RUNNING":
				return (
					<Badge className="bg-info text-info-foreground hover:bg-info/90">
						{STATE_LABELS.RUNNING}
					</Badge>
				);
			case "TUNING":
				return (
					<Badge className="bg-tuning text-tuning-foreground hover:bg-tuning/90">
						{STATE_LABELS.TUNING}
					</Badge>
				);
			case "ERROR":
				return <Badge variant="destructive">{STATE_LABELS.ERROR}</Badge>;
			case "COMPLETE":
				return (
					<Badge className="bg-success text-success-foreground hover:bg-success/90">
						{STATE_LABELS.COMPLETE}
					</Badge>
				);
			default:
				return <Badge variant="outline">{state}</Badge>;
		}
	};

	const formatTemp = (temp: number) => `${temp.toFixed(1)}°C`;

	const formatDuration = (seconds?: number) => {
		if (seconds === undefined || Number.isNaN(seconds)) {
			return "N/A";
		}
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

	const heating = (status.ssr_output ?? 0) > 0;
	const cooling =
		currentStepIsControlledCooldown || status.step_name === "cooling";

	let accentClassName = "text-muted-foreground";
	let caption: string | undefined = STATE_LABELS[status.state];
	switch (status.state) {
		case "ERROR":
			accentClassName = "text-destructive";
			caption = "Fault";
			break;
		case "TUNING":
			accentClassName = "text-tuning";
			caption = "Tuning";
			break;
		case "COMPLETE":
			accentClassName = "text-success";
			caption = "Complete";
			break;
		case "RUNNING":
			// Caption follows the firing step (stable) rather than instantaneous
			// SSR output, which can blip to 0% mid-ramp and cause flicker.
			if (cooling) {
				accentClassName = "text-chart-cooling";
				caption = "Cooling";
			} else if (status.step_name === "hold") {
				accentClassName = "text-info";
				caption = "Holding";
			} else if (status.step_name === "ramp" || heating) {
				accentClassName = "text-chart-heating";
				caption = "Heating";
			} else {
				accentClassName = "text-info";
				caption = "Running";
			}
			break;
		default:
			if (heating) accentClassName = "text-chart-heating";
	}

	return (
		<div className="space-y-6">
			<Card>
				<StatusHeader
					onRefresh={onRefresh}
					isLoading={isLoading}
					badge={getStateBadge(status.state)}
				/>
				<CardContent className="space-y-4">
					<p className="text-sm text-muted-foreground">
						{STATE_DESCRIPTIONS[status.state]}
					</p>

					{/* Stale data / offline banner — keep showing the last reading. */}
					{error && (
						<Alert variant="warning">
							<WifiOff className="h-4 w-4" />
							<AlertDescription>
								Connection lost — showing the last known reading while trying to
								reconnect.
							</AlertDescription>
						</Alert>
					)}

					{/* Hero temperature + live trend */}
					<div className="grid gap-4 sm:grid-cols-[auto_1fr] sm:items-center">
						<TemperatureGauge
							current={status.current_temp}
							target={status.target_temp}
							accentClassName={accentClassName}
							caption={caption}
						/>
						<div className="space-y-2">
							<LiveTempChart data={tempHistory} />
							<div className="flex items-center justify-between px-1">
								<LastUpdated updatedAt={dataUpdatedAt} stale={Boolean(error)} />
								<span className="inline-flex items-center gap-3 text-xs text-muted-foreground">
									<span className="inline-flex items-center gap-1">
										<span className="h-0.5 w-3 rounded bg-chart-heating" />
										Current
									</span>
									{status.target_temp !== undefined && (
										<span className="inline-flex items-center gap-1">
											<span className="h-0 w-3 border-t border-dashed border-muted-foreground" />
											Target
										</span>
									)}
								</span>
							</div>
						</div>
					</div>

					{/* SSR and Heating Rates */}
					<div className="grid grid-cols-2 gap-4 pt-2 border-t">
						<div className="space-y-2">
							<div className="flex items-center gap-2">
								<Flame
									className={cn(
										"w-5 h-5",
										heating ? "text-chart-ssr" : "text-muted-foreground",
									)}
								/>
								<span className="text-sm font-medium">Heat Output</span>
							</div>
							<div className="flex items-center gap-2">
								<Gauge className="w-4 h-4 text-muted-foreground" />
								<span className="text-sm font-bold">
									{status.ssr_output !== undefined
										? status.ssr_output > 0
											? `${status.ssr_output.toFixed(1)}%`
											: "Off"
										: "N/A"}
								</span>
							</div>
						</div>

						{status.measured_rate !== undefined && (
							<div className="space-y-2">
								<div className="flex items-center gap-2">
									<TrendingUp className="w-5 h-5 text-muted-foreground" />
									<span className="text-sm font-medium">Heating Rate</span>
								</div>
								<div className="text-sm">
									Measured:{" "}
									<strong>{status.measured_rate.toFixed(1)}°C/h</strong>
								</div>
								{status.desired_rate !== undefined &&
									status.state === "RUNNING" && (
										<div className="text-sm text-muted-foreground">
											Target: {status.desired_rate.toFixed(1)}°C/h
										</div>
									)}
							</div>
						)}
					</div>

					{/* Recovery Mode Warning */}
					{status.is_recovering && (
						<Alert variant="warning">
							<CircleAlert className="h-4 w-4" />
							<AlertDescription>
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
											disabled={isClearingPending || status.state !== "ERROR"}
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
													<CircleAlert className="w-4 h-4 mr-2" />
													Clear Error
												</>
											)}
										</Button>
									</div>
								</AlertDescription>
							</Alert>
							{isClearingError && <ErrorAlert error={clearingError} />}
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
										<StepIcon
											type={currentStep.type}
											isControlledCooldown={currentStepIsControlledCooldown}
										/>
									)}
									<span className="text-sm font-medium flex-1">
										{currentStep && runningProfile
											? formatStepInfo(
													currentStep,
													runningProfile.temp_units,
													prevStep?.target_temp,
												)
											: `Current Step: ${status.step_name}`}
									</span>
								</div>
								{status.desired_rate !== undefined &&
									status.step_name === "ramp" && (
										<div className="text-xs text-muted-foreground">
											Target rate: {status.desired_rate.toFixed(0)}°C/h
										</div>
									)}
								<div className="flex gap-4 text-xs text-muted-foreground">
									{status.step_elapsed !== undefined &&
										status.step_elapsed > 0 && (
											<span>
												Step time: {formatDuration(status.step_elapsed)}
											</span>
										)}
									{etas?.currentStepSeconds != null &&
										etas.currentStepSeconds > 0 && (
											<span>ETA: ~{formatETA(etas.currentStepSeconds)}</span>
										)}
								</div>
							</div>
						)}

						{(status.elapsed !== undefined || etas?.profileSeconds != null) && (
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
								{etas?.profileSeconds != null && (
									<div>
										<div className="text-muted-foreground">Est. Remaining</div>
										<div className="font-medium flex items-center gap-1 mt-1">
											<Clock className="w-4 h-4" />~
											{formatETA(etas.profileSeconds)}
										</div>
									</div>
								)}
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
										const stepPrev =
											index > 0 ? runningProfile.steps[index - 1] : undefined;
										const stepPrevTargetTemp = stepPrev?.target_temp;
										const stepControlledCooldown = isStepControlledCooldown(
											step,
											stepPrevTargetTemp,
										);

										return (
											<div
												key={index}
												className={`flex items-center gap-2 text-xs p-2 rounded ${
													isCurrent
														? "bg-info/10 border border-info/30"
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
												<StepIcon
													type={step.type}
													isControlledCooldown={stepControlledCooldown}
												/>
												<span className="flex-1 truncate">
													{formatStepInfo(
														step,
														runningProfile.temp_units,
														stepPrevTargetTemp,
													)}
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
									<Badge className="bg-tuning text-tuning-foreground hover:bg-tuning/90">
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
							<div className="p-3 rounded-lg bg-tuning/10 border border-tuning/30 space-y-2">
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

			{/* PID detail — tucked into an Advanced disclosure (progressive disclosure). */}
			{status.pid && (
				<Card>
					<Collapsible>
						<CardHeader className="pb-3">
							<CollapsibleTrigger className="group flex w-full items-center justify-between">
								<CardTitle className="text-base">
									Advanced — PID control
								</CardTitle>
								<ChevronDown className="h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
							</CollapsibleTrigger>
						</CardHeader>
						<CollapsibleContent>
							<CardContent>
								<div className="grid grid-cols-2 gap-4 text-sm">
									{status.pid.kp !== undefined && (
										<div>
											<div className="text-muted-foreground">Kp</div>
											<div className="font-mono">
												{status.pid.kp.toFixed(3)}
											</div>
										</div>
									)}
									{status.pid.ki !== undefined && (
										<div>
											<div className="text-muted-foreground">Ki</div>
											<div className="font-mono">
												{status.pid.ki.toFixed(3)}
											</div>
										</div>
									)}
									{status.pid.kd !== undefined && (
										<div>
											<div className="text-muted-foreground">Kd</div>
											<div className="font-mono">
												{status.pid.kd.toFixed(3)}
											</div>
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
						</CollapsibleContent>
					</Collapsible>
				</Card>
			)}
		</div>
	);
}
