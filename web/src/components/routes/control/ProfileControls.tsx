import {
	Calendar,
	Clock,
	Loader2,
	Play,
	Power,
	RotateCw,
	Square,
	X,
} from "lucide-react";
import { useState } from "react";
import { ErrorAlert } from "@/components/ErrorAlert";
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
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
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
import type { KilnStatus, Profile } from "@/lib/pico/types";
import {
	formatStepInfo,
	isStepControlledCooldown,
	StepIcon,
} from "@/lib/step-utils";
import { formatDuration } from "@/lib/utils";

interface ProfileControlsProps {
	status?: KilnStatus;
}

function ProfileDetails({ profile }: { profile: Profile }) {
	return (
		<div className="space-y-3 pt-4 border-t">
			{profile.description && (
				<div className="text-sm text-muted-foreground">
					{profile.description}
				</div>
			)}
			<div className="space-y-2">
				<div className="text-sm font-medium">
					Steps ({profile.steps.length})
				</div>
				<div className="space-y-1">
					{profile.steps.map((step, index) => {
						const prevStep = index > 0 ? profile.steps[index - 1] : undefined;
						const prevTargetTemp = prevStep?.target_temp;
						const controlledCooldown = isStepControlledCooldown(
							step,
							prevTargetTemp,
						);

						return (
							<div
								key={index}
								className="flex items-center gap-2 text-sm p-2 rounded bg-muted/50"
							>
								<Badge variant="outline" className="w-6 h-6 p-0 justify-center">
									{index + 1}
								</Badge>
								<StepIcon
									type={step.type}
									isControlledCooldown={controlledCooldown}
								/>
								<span className="flex-1">
									{formatStepInfo(step, profile.temp_units, prevTargetTemp)}
								</span>
							</div>
						);
					})}
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

	const selectedProfileData = selectedProfile
		? getProfile(selectedProfile)
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

	const handleRun = () => {
		if (!selectedProfile) return;
		runProfile.mutate(selectedProfile);
	};

	const handleStop = () => {
		stopProfile.mutate();
	};

	const handleShutdown = () => {
		shutdown.mutate(undefined, {
			onSuccess: () => setShowShutdownDialog(false),
		});
	};

	const handleReboot = () => {
		// useReboot resolves on a dropped connection (the expected reboot path),
		// so closing on success covers it; a real failure keeps the dialog open
		// and surfaces below.
		reboot.mutate(undefined, {
			onSuccess: () => setShowRebootDialog(false),
		});
	};

	const handleSchedule = () => {
		if (!selectedProfile || !scheduleDate || !scheduleTime) return;

		const startTime = Math.floor(
			new Date(`${scheduleDate}T${scheduleTime}`).getTime() / 1000,
		);

		scheduleProfile.mutate(
			{ profileName: selectedProfile, startTime },
			{
				onSuccess: () => {
					setShowScheduleDialog(false);
					setScheduleDate("");
					setScheduleTime("");
				},
			},
		);
	};

	const handleCancelScheduled = () => {
		cancelScheduled.mutate();
	};

	return (
		<div className="space-y-4">
			{hasScheduled && status?.scheduled_profile && (
				<Card className="border-warning/50">
					<CardHeader>
						<CardTitle className="flex items-center gap-2 text-warning">
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
								{formatDuration(status.scheduled_profile.seconds_until_start)}
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
							<ErrorAlert error={cancelScheduled.error} />
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
								<Label htmlFor="profile-select">Select Profile</Label>
								<Select
									value={selectedProfile}
									onValueChange={setSelectedProfile}
									disabled={status?.state === "TUNING"}
								>
									<SelectTrigger id="profile-select" className="w-full">
										<SelectValue placeholder="Choose a profile..." />
									</SelectTrigger>
									<SelectContent>
										{availableProfiles.map((profile) => (
											<SelectItem key={profile} value={profile}>
												{profile.replace(/_/g, " ")}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
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

							{runProfile.isError && <ErrorAlert error={runProfile.error} />}

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
							<Alert variant="info">
								<AlertDescription>
									Profile is currently running:{" "}
									<strong>{status?.profile_name}</strong>
								</AlertDescription>
							</Alert>

							<div className="space-y-1.5">
								<Button
									onClick={handleStop}
									disabled={!canStop || stopProfile.isPending}
									variant="outline"
									className="w-full border-destructive/50 text-destructive hover:bg-destructive/10 hover:text-destructive"
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
											Stop Firing
										</>
									)}
								</Button>
								<p className="text-xs text-muted-foreground">
									Ends the current firing in a controlled way — heating turns
									off and the kiln cools on its own.
								</p>
							</div>

							{stopProfile.isError && <ErrorAlert error={stopProfile.error} />}
						</>
					)}
				</CardContent>
			</Card>

			<Card className="border-destructive/50">
				<CardHeader>
					<CardTitle className="text-destructive">Emergency Controls</CardTitle>
					<CardDescription>
						Use with caution - immediately stops all heating
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-1.5">
						<Button
							onClick={() => setShowShutdownDialog(true)}
							variant="destructive"
							className="w-full"
							size="lg"
						>
							<Power className="w-4 h-4 mr-2" />
							Emergency Shutdown
						</Button>
						<p className="text-xs text-muted-foreground">
							Cuts power to the heating element right away. Use only if
							something looks wrong.
						</p>
					</div>
					<div className="space-y-1.5">
						<Button
							onClick={() => setShowRebootDialog(true)}
							variant="secondary"
							className="w-full"
							size="lg"
						>
							<RotateCw className="w-4 h-4 mr-2" />
							Reboot Controller
						</Button>
						<p className="text-xs text-muted-foreground">
							Restarts the controller software. Use if the interface stops
							responding.
						</p>
					</div>
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
					{shutdown.isError && <ErrorAlert error={shutdown.error} />}
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
							<p className="text-warning">
								Note: Any running profile will be stopped and the kiln will
								enter shutdown mode.
							</p>
						</DialogDescription>
					</DialogHeader>
					{reboot.isError && <ErrorAlert error={reboot.error} />}
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
						<ErrorAlert error={scheduleProfile.error} />
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
