import { AlertTriangle, Loader2, Play, Square, Zap } from "lucide-react";
import { useState } from "react";
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
import { useStartTuning, useStopTuning } from "@/lib/pico/hooks";
import type { KilnStatus, TuningMode } from "@/lib/pico/types";

interface TuningControlsProps {
	status?: KilnStatus;
}

const TUNING_MODES: {
	value: TuningMode;
	label: string;
	description: string;
}[] = [
	{
		value: "safe",
		label: "Safe Mode",
		description: "Conservative tuning with lower temperatures (max 200°C)",
	},
	{
		value: "standard",
		label: "Standard Mode",
		description: "Balanced tuning for typical firing temperatures (max 350°C)",
	},
	{
		value: "thorough",
		label: "Thorough Mode",
		description: "Extended tuning for better accuracy (max 350°C)",
	},
	{
		value: "high_temp",
		label: "High Temperature Mode",
		description: "Tuning for high-fire ceramics (max 500°C)",
	},
];

export function TuningControls({ status }: TuningControlsProps) {
	const [selectedMode, setSelectedMode] = useState<TuningMode>("standard");
	const [maxTemp, setMaxTemp] = useState<string>("");

	const startTuning = useStartTuning();
	const stopTuning = useStopTuning();

	const isTuning = status?.state === "TUNING";

	const handleStart = async () => {
		try {
			const maxTempNum = maxTemp ? Number.parseInt(maxTemp, 10) : undefined;
			const result = await startTuning.mutateAsync({
				mode: selectedMode,
				maxTemp: maxTempNum,
			});
			if (!result.success) {
				console.error("Failed to start tuning:", result.error);
			}
		} catch (error) {
			console.error("Error starting tuning:", error);
		}
	};

	const handleStop = async () => {
		try {
			const result = await stopTuning.mutateAsync();
			if (!result.success) {
				console.error("Failed to stop tuning:", result.error);
			}
		} catch (error) {
			console.error("Error stopping tuning:", error);
		}
	};

	return (
		<div className="space-y-4">
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<Zap className="w-5 h-5 text-purple-500" />
						PID Auto-Tuning
					</CardTitle>
					<CardDescription>
						Automatically optimize PID parameters for your kiln
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{!isTuning ? (
						<>
							<div className="space-y-2">
								<Label>Tuning Mode</Label>
								{TUNING_MODES.map((mode) => (
									<label
										key={mode.value}
										className="flex items-start gap-3 p-3 border rounded-lg cursor-pointer hover:bg-accent transition-colors"
									>
										<input
											type="radio"
											name="tuning-mode"
											value={mode.value}
											checked={selectedMode === mode.value}
											onChange={(e) =>
												setSelectedMode(e.target.value as TuningMode)
											}
											className="mt-1"
										/>
										<div className="flex-1">
											<div className="font-medium">{mode.label}</div>
											<div className="text-sm text-muted-foreground">
												{mode.description}
											</div>
										</div>
									</label>
								))}
							</div>

							<div className="space-y-2">
								<Label htmlFor="max-temp">Maximum Temperature (optional)</Label>
								<Input
									id="max-temp"
									type="number"
									placeholder="Leave empty to use mode default"
									value={maxTemp}
									onChange={(e) => setMaxTemp(e.target.value)}
									min="50"
									max="500"
								/>
								<p className="text-xs text-muted-foreground">
									Override the mode's default maximum temperature (50-500°C)
								</p>
							</div>

							<Button
								onClick={handleStart}
								disabled={startTuning.isPending}
								className="w-full"
								size="lg"
							>
								{startTuning.isPending ? (
									<>
										<Loader2 className="w-4 h-4 mr-2 animate-spin" />
										Starting...
									</>
								) : (
									<>
										<Play className="w-4 h-4 mr-2" />
										Start Tuning
									</>
								)}
							</Button>

							{startTuning.isError && (
								<Alert variant="destructive">
									<AlertTriangle className="w-4 h-4" />
									<AlertDescription>
										{startTuning.error?.message || "Failed to start tuning"}
									</AlertDescription>
								</Alert>
							)}
						</>
					) : (
						<>
							<Alert className="border-purple-600 bg-purple-50">
								<Zap className="w-4 h-4 text-purple-600" />
								<AlertDescription className="text-purple-800">
									<div className="space-y-2">
										<div>
											<strong>Tuning in progress</strong>
											{status.tuning?.mode && ` (${status.tuning.mode} mode)`}
										</div>
										{status.tuning?.phase && (
											<div className="text-sm">
												Phase: {status.tuning.phase}
											</div>
										)}
										{status.tuning?.progress !== undefined && (
											<div className="space-y-1">
												<div className="text-sm">
													Progress: {status.tuning.progress.toFixed(1)}%
												</div>
												<div className="w-full bg-purple-200 rounded-full h-2">
													<div
														className="bg-purple-600 h-2 rounded-full transition-all duration-500"
														style={{ width: `${status.tuning.progress}%` }}
													/>
												</div>
											</div>
										)}
										{status.tuning?.estimated_time_remaining !== undefined && (
											<div className="text-sm">
												Estimated time remaining:{" "}
												{Math.ceil(status.tuning.estimated_time_remaining / 60)}{" "}
												minutes
											</div>
										)}
										{status.tuning?.oscillation_count !== undefined && (
											<div className="text-sm">
												Oscillations detected: {status.tuning.oscillation_count}
											</div>
										)}
									</div>
								</AlertDescription>
							</Alert>

							<Button
								onClick={handleStop}
								disabled={stopTuning.isPending}
								variant="destructive"
								className="w-full"
								size="lg"
							>
								{stopTuning.isPending ? (
									<>
										<Loader2 className="w-4 h-4 mr-2 animate-spin" />
										Stopping...
									</>
								) : (
									<>
										<Square className="w-4 h-4 mr-2" />
										Stop Tuning
									</>
								)}
							</Button>

							{stopTuning.isError && (
								<Alert variant="destructive">
									<AlertTriangle className="w-4 h-4" />
									<AlertDescription>
										{stopTuning.error?.message || "Failed to stop tuning"}
									</AlertDescription>
								</Alert>
							)}
						</>
					)}
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>About Auto-Tuning</CardTitle>
				</CardHeader>
				<CardContent className="space-y-2 text-sm text-muted-foreground">
					<p>
						Auto-tuning automatically determines the optimal PID parameters for
						your kiln by heating it up and observing its thermal behavior.
					</p>
					<p>
						The process typically takes 30-60 minutes depending on the mode and
						your kiln's characteristics. The kiln will heat up, cool down, and
						heat up again while measuring the response.
					</p>
					<p className="font-medium text-foreground">
						Do not open the kiln or interrupt the process once started.
					</p>
				</CardContent>
			</Card>
		</div>
	);
}
