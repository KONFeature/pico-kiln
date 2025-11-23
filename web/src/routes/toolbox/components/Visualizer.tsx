import { Activity, LineChart, Zap } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { ProfileVisualizer } from "./ProfileVisualizer";
import { RunVisualizer } from "./RunVisualizer";
import { TuningPhasesVisualizer } from "./TuningPhasesVisualizer";

type VisualizerType = "profile" | "run" | "tuning";

const VISUALIZER_INFO = {
	profile: {
		title: "Profile Visualizer",
		description:
			"Visualize kiln firing profiles - see temperature trajectory over time",
		icon: LineChart,
	},
	run: {
		title: "Run Visualizer",
		description:
			"Visualize kiln firing or tuning runs - see temperature, SSR output, and rate data",
		icon: Activity,
	},
	tuning: {
		title: "Tuning Phases Visualizer",
		description: "Visualize PID tuning runs with physics-based phase detection",
		icon: Zap,
	},
};

export function Visualizer() {
	const [visualizerType, setVisualizerType] =
		useState<VisualizerType>("profile");

	const currentInfo = VISUALIZER_INFO[visualizerType];
	const Icon = currentInfo.icon;

	return (
		<div className="space-y-6">
			{/* Type Selector */}
			<Card>
				<CardHeader>
					<CardTitle>Visualizer</CardTitle>
					<CardDescription>
						Visualize profiles, runs, and tuning data from your kiln
					</CardDescription>
				</CardHeader>
				<CardContent>
					<div className="space-y-3">
						<div>
							<label className="text-sm font-medium">
								Select Visualization Type
							</label>
							<p className="text-sm text-muted-foreground mt-1">
								Choose what type of data you want to visualize
							</p>
						</div>
						<div className="grid grid-cols-1 md:grid-cols-3 gap-2">
							<Button
								variant={visualizerType === "profile" ? "default" : "outline"}
								onClick={() => setVisualizerType("profile")}
								className="flex items-center gap-2 h-auto py-3 px-4"
							>
								<LineChart className="w-5 h-5 flex-shrink-0" />
								<div className="text-left">
									<div className="font-semibold">Profile</div>
									<div className="text-xs opacity-80">
										Temperature trajectory
									</div>
								</div>
							</Button>
							<Button
								variant={visualizerType === "run" ? "default" : "outline"}
								onClick={() => setVisualizerType("run")}
								className="flex items-center gap-2 h-auto py-3 px-4"
							>
								<Activity className="w-5 h-5 flex-shrink-0" />
								<div className="text-left">
									<div className="font-semibold">Run</div>
									<div className="text-xs opacity-80">Firing & tuning logs</div>
								</div>
							</Button>
							<Button
								variant={visualizerType === "tuning" ? "default" : "outline"}
								onClick={() => setVisualizerType("tuning")}
								className="flex items-center gap-2 h-auto py-3 px-4"
							>
								<Zap className="w-5 h-5 flex-shrink-0" />
								<div className="text-left">
									<div className="font-semibold">Tuning Phases</div>
									<div className="text-xs opacity-80">PID phase detection</div>
								</div>
							</Button>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Visualizer Content */}
			{visualizerType === "profile" && <ProfileVisualizer />}
			{visualizerType === "run" && <RunVisualizer />}
			{visualizerType === "tuning" && <TuningPhasesVisualizer />}
		</div>
	);
}
