import {
	AlertTriangle,
	ArrowUp,
	Calendar,
	Clock,
	Flame,
	Loader2,
	Pause,
	Play,
	Power,
	RotateCw,
	Snowflake,
	Square,
	X,
} from "lucide-react";
import { useState } from "react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
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
	useCancelScheduled,
	useListFiles,
	useReboot,
	useRunProfile,
	useScheduleProfile,
	useShutdown,
	useStopProfile,
} from "@/lib/pico/hooks";
import { useProfileCache } from "@/lib/pico/profile-cache";
import type { KilnStatus, Profile, ProfileStep } from "@/lib/pico/types";

interface ProfileControlsProps {
	status?: KilnStatus;
}

// Helper function to format step details
function formatStepDetails(step: ProfileStep, tempUnit: string): string {
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

// Profile details display component
function ProfileDetails({ profile }: { profile: Profile }) {
	return (
		<div className="space-y-3 pt-4 border-t">
			{profile.description && (
				<div className="text-sm text-muted-foreground">
					{profile.description}
				</div>
			)}
			<div className="space-y-2">
				<div className="text-sm font-medium">Steps ({profile.steps.length})</div>
				<div className="space-y-1">
					{profile.steps.map((step, index) => (
						<div
							key={index}
							className="flex items-center gap-2 text-sm p-2 rounded bg-muted/50"
						>
							<Badge variant="outline" className="w-6 h-6 p-0 justify-center">
								{index + 1}
							</Badge>
							<StepIcon type={step.type} />
							<span className="flex-1">
								{formatStepDetails(step, profile.temp_units)}
							</span>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

export function ProfileControls({ status }: ProfileControlsProps) {
	const [selectedProfile, setSelectedProfile] = useState<string>("");
	const [showShutdownDialog, setShowShutdownDialog] = useState(false);
	const [showRebootDialog, setShowRebootDialog] = useState(false);
	const [showScheduleDialog, setShowScheduleDialog] = useState(false);
	const [scheduleDate, setScheduleDate] = useState("");
	const [scheduleTime, setScheduleTime] = useState("");

	// Profile cache for getting profile details
	const { getProfile, isPreloading, preloadProgress } = useProfileCache();

	// Dynamically load available profiles from the profiles directory
	const { data: profilesData } = useListFiles("profiles");
	const availableProfiles =
		profilesData?.files
			.filter((file) => file.name.endsWith(".json"))
			.map((file) => file.name.replace(".json", "")) || [];

	// Get the selected profile data from cache
	const selectedProfileData = selectedProfile
		? getProfile(selectedProfile)
		: undefined;

	// Get the running profile data from cache (when a profile is running)
	const runningProfileData = status?.profile_name
		? getProfile(status.profile_name)
		: undefined;

	const runProfile = useRunProfile();
	const stopProfile = useStopProfile();
	const shutdown = useShutdown();
	const reboot = useReboot();
	const scheduleProfile = useScheduleProfile();
	const cancelScheduled = useCancelScheduled();

	const isRunning = status?.state === "RUNNING";
	const hasScheduled = Boolean(status?.scheduled_profile);
	const canStart =
		!isRunning &&
		!hasScheduled &&
		selectedProfile &&
		status?.state !== "TUNING";
	const canSchedule =
		!isRunning &&
		!hasScheduled &&
		selectedProfile &&
		status?.state !== "TUNING";
	const canStop = isRunning;

	const handleRun = async () => {
		if (!selectedProfile) return;

		try {
			const result = await runProfile.mutateAsync(selectedProfile);
			if (!result.success) {
				console.error("Failed to start profile:", result.error);
			}
		} catch (error) {
			console.error("Error starting profile:", error);
		}
	};

	const handleStop = async () => {
		try {
			const result = await stopProfile.mutateAsync();
			if (!result.success) {
				console.error("Failed to stop profile:", result.error);
			}
		} catch (error) {
			console.error("Error stopping profile:", error);
		}
	};

	const handleShutdown = async () => {
		try {
			const result = await shutdown.mutateAsync();
			if (result.success) {
				setShowShutdownDialog(false);
			}
		} catch (error) {
			console.error("Error during shutdown:", error);
		}
	};

	const handleReboot = async () => {
		try {
			await reboot.mutateAsync();
			// Always close dialog and consider it successful
			// The Pico will disconnect immediately upon reboot
			setShowRebootDialog(false);
		} catch (error) {
			console.error("Error during reboot:", error);
			// Still close the dialog - the reboot might have worked
			setShowRebootDialog(false);
		}
	};

	const handleSchedule = async () => {
		if (!selectedProfile || !scheduleDate || !scheduleTime) return;

		try {
			// Combine date and time into Unix timestamp
			const dateTimeStr = `${scheduleDate}T${scheduleTime}`;
			const startTime = Math.floor(new Date(dateTimeStr).getTime() / 1000);

			const result = await scheduleProfile.mutateAsync({
				profileName: selectedProfile,
				startTime,
			});

			if (result.success) {
				setShowScheduleDialog(false);
				setScheduleDate("");
				setScheduleTime("");
			}
		} catch (error) {
			console.error("Error scheduling profile:", error);
		}
	};

	const handleCancelScheduled = async () => {
		try {
			await cancelScheduled.mutateAsync();
		} catch (error) {
			console.error("Error cancelling scheduled profile:", error);
		}
	};

	const formatCountdown = (seconds: number) => {
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
		<div className="space-y-4">
			{hasScheduled && status?.scheduled_profile && (
				<Card className="border-amber-600">
					<CardHeader>
						<CardTitle className="flex items-center gap-2 text-amber-600">
							<Clock className="w-5 h-5" />
							Scheduled Profile
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="space-y-2">
							<div className="text-sm">
								<strong>Profile:</strong>{" "}
								{status.scheduled_profile.profile_filename
									.replace(".json", "")
									.replace(/_/g, " ")}
							</div>
							<div className="text-sm">
								<strong>Start Time:</strong>{" "}
								{new Date(
									status.scheduled_profile.start_time * 1000,
								).toLocaleString()}
							</div>
							<div className="text-sm">
								<strong>Countdown:</strong>{" "}
								{formatCountdown(status.scheduled_profile.seconds_until_start)}
							</div>
						</div>

						<Button
							onClick={handleCancelScheduled}
							disabled={cancelScheduled.isPending}
							variant="outline"
							className="w-full"
						>
							{cancelScheduled.isPending ? (
								<>
									<Loader2 className="w-4 h-4 mr-2 animate-spin" />
									Cancelling...
								</>
							) : (
								<>
									<X className="w-4 h-4 mr-2" />
									Cancel Scheduled Profile
								</>
							)}
						</Button>

						{cancelScheduled.isError && (
							<Alert variant="destructive">
								<AlertTriangle className="w-4 h-4" />
								<AlertDescription>
									{cancelScheduled.error?.message ||
										"Failed to cancel scheduled profile"}
								</AlertDescription>
							</Alert>
						)}
					</CardContent>
				</Card>
			)}

			<Card>
				<CardHeader>
					<CardTitle>Profile Control</CardTitle>
					<CardDescription>Select and run a firing profile</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{!isRunning && !hasScheduled ? (
						<>
							<div className="space-y-2">
								<label className="text-sm font-medium">Select Profile</label>
								<select
									className="w-full px-3 py-2 border rounded-md bg-background"
									value={selectedProfile}
									onChange={(e) => setSelectedProfile(e.target.value)}
									disabled={status?.state === "TUNING"}
								>
									<option value="">-- Choose a profile --</option>
									{availableProfiles.map((profile) => (
										<option key={profile} value={profile}>
											{profile.replace(/_/g, " ")}
										</option>
									))}
								</select>
							</div>

							<div className="grid grid-cols-2 gap-2">
								<Button
									onClick={handleRun}
									disabled={!canStart || runProfile.isPending}
									size="lg"
								>
									{runProfile.isPending ? (
										<>
											<Loader2 className="w-4 h-4 mr-2 animate-spin" />
											Starting...
										</>
									) : (
										<>
											<Play className="w-4 h-4 mr-2" />
											Start Now
										</>
									)}
								</Button>

								<Button
									onClick={() => setShowScheduleDialog(true)}
									disabled={!canSchedule}
									variant="outline"
									size="lg"
								>
									<Calendar className="w-4 h-4 mr-2" />
									Schedule
								</Button>
							</div>

							{runProfile.isError && (
								<Alert variant="destructive">
									<AlertTriangle className="w-4 h-4" />
									<AlertDescription>
										{runProfile.error?.message || "Failed to start profile"}
									</AlertDescription>
								</Alert>
							)}

							{/* Profile details when a profile is selected */}
							{selectedProfile && selectedProfileData && (
								<ProfileDetails profile={selectedProfileData} />
							)}

							{/* Loading indicator when profile is being loaded */}
							{selectedProfile && !selectedProfileData && isPreloading && (
								<div className="flex items-center gap-2 text-sm text-muted-foreground pt-4 border-t">
									<Loader2 className="w-4 h-4 animate-spin" />
									Loading profile details ({preloadProgress.loaded}/
									{preloadProgress.total})...
								</div>
							)}
						</>
					) : (
						<>
							<Alert className="border-blue-600 bg-blue-50">
								<AlertDescription className="text-blue-800">
									Profile is currently running:{" "}
									<strong>{status?.profile_name}</strong>
								</AlertDescription>
							</Alert>

							<Button
								onClick={handleStop}
								disabled={!canStop || stopProfile.isPending}
								variant="destructive"
								className="w-full"
								size="lg"
							>
								{stopProfile.isPending ? (
									<>
										<Loader2 className="w-4 h-4 mr-2 animate-spin" />
										Stopping...
									</>
								) : (
									<>
										<Square className="w-4 h-4 mr-2" />
										Stop Profile
									</>
								)}
							</Button>

							{stopProfile.isError && (
								<Alert variant="destructive">
									<AlertTriangle className="w-4 h-4" />
									<AlertDescription>
										{stopProfile.error?.message || "Failed to stop profile"}
									</AlertDescription>
								</Alert>
							)}

							{/* Profile details during run */}
							{runningProfileData && (
								<ProfileDetails profile={runningProfileData} />
							)}
						</>
					)}
				</CardContent>
			</Card>

			<Card className="border-red-600">
				<CardHeader>
					<CardTitle className="text-destructive">Emergency Controls</CardTitle>
					<CardDescription>
						Use with caution - immediately stops all heating
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-2">
					<Button
						onClick={() => setShowShutdownDialog(true)}
						variant="destructive"
						className="w-full"
						size="lg"
					>
						<Power className="w-4 h-4 mr-2" />
						Emergency Shutdown
					</Button>
					<Button
						onClick={() => setShowRebootDialog(true)}
						variant="secondary"
						className="w-full"
						size="lg"
					>
						<RotateCw className="w-4 h-4 mr-2" />
						Reboot Pico
					</Button>
				</CardContent>
			</Card>

			<Dialog open={showShutdownDialog} onOpenChange={setShowShutdownDialog}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Confirm Emergency Shutdown</DialogTitle>
						<DialogDescription>
							This will immediately turn off the heating element and stop any
							running program. The kiln will begin cooling naturally.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowShutdownDialog(false)}
							disabled={shutdown.isPending}
						>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={handleShutdown}
							disabled={shutdown.isPending}
						>
							{shutdown.isPending ? (
								<>
									<Loader2 className="w-4 h-4 mr-2 animate-spin" />
									Shutting down...
								</>
							) : (
								"Confirm Shutdown"
							)}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<Dialog open={showRebootDialog} onOpenChange={setShowRebootDialog}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Confirm Reboot</DialogTitle>
						<DialogDescription className="space-y-2">
							<p>
								This will restart the Pico controller. The web interface will be
								unavailable for 10-15 seconds while the device reboots.
							</p>
							<p className="text-orange-600 dark:text-orange-400">
								Note: Any running profile will be stopped and the kiln will
								enter shutdown mode.
							</p>
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowRebootDialog(false)}
							disabled={reboot.isPending}
						>
							Cancel
						</Button>
						<Button
							variant="secondary"
							onClick={handleReboot}
							disabled={reboot.isPending}
						>
							{reboot.isPending ? (
								<>
									<Loader2 className="w-4 h-4 mr-2 animate-spin" />
									Rebooting...
								</>
							) : (
								"Confirm Reboot"
							)}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<Dialog open={showScheduleDialog} onOpenChange={setShowScheduleDialog}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Schedule Profile</DialogTitle>
						<DialogDescription>
							Set the date and time to start the profile:{" "}
							{selectedProfile.replace(/_/g, " ")}
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-4 py-4">
						<div className="space-y-2">
							<Label htmlFor="schedule-date">Date</Label>
							<Input
								id="schedule-date"
								type="date"
								value={scheduleDate}
								onChange={(e) => setScheduleDate(e.target.value)}
								min={new Date().toISOString().split("T")[0]}
							/>
						</div>
						<div className="space-y-2">
							<Label htmlFor="schedule-time">Time</Label>
							<Input
								id="schedule-time"
								type="time"
								value={scheduleTime}
								onChange={(e) => setScheduleTime(e.target.value)}
							/>
						</div>
					</div>
					{scheduleProfile.isError && (
						<Alert variant="destructive">
							<AlertTriangle className="w-4 h-4" />
							<AlertDescription>
								{scheduleProfile.error?.message || "Failed to schedule profile"}
							</AlertDescription>
						</Alert>
					)}
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setShowScheduleDialog(false)}
							disabled={scheduleProfile.isPending}
						>
							Cancel
						</Button>
						<Button
							onClick={handleSchedule}
							disabled={
								!scheduleDate || !scheduleTime || scheduleProfile.isPending
							}
						>
							{scheduleProfile.isPending ? (
								<>
									<Loader2 className="w-4 h-4 mr-2 animate-spin" />
									Scheduling...
								</>
							) : (
								"Schedule Profile"
							)}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
