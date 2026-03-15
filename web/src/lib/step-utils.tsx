import { ArrowUp, Flame, Pause, Snowflake } from "lucide-react";
import type { KilnStatus, Profile, ProfileStep } from "@/lib/pico/types";

export function StepIcon({
	type,
	isControlledCooldown,
}: {
	type: ProfileStep["type"];
	isControlledCooldown?: boolean;
}) {
	if (type === "ramp" && isControlledCooldown) {
		return <Snowflake className="w-4 h-4 text-blue-500" />;
	}
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

// A "ramp" step with target below previous target is actually a controlled cooldown
export function isStepControlledCooldown(
	step: ProfileStep,
	prevTargetTemp?: number,
): boolean {
	return (
		step.type === "ramp" &&
		prevTargetTemp !== undefined &&
		step.target_temp !== undefined &&
		step.target_temp < prevTargetTemp
	);
}

export function formatStepInfo(
	step: ProfileStep,
	tempUnit: string,
	prevTargetTemp?: number,
): string {
	const unit = tempUnit === "f" ? "°F" : "°C";
	const rateUnit = tempUnit === "f" ? "°F/h" : "°C/h";

	switch (step.type) {
		case "ramp": {
			if (isStepControlledCooldown(step, prevTargetTemp)) {
				return `Cool to ${step.target_temp}${unit} at ${step.desired_rate}${rateUnit}`;
			}
			return `Ramp to ${step.target_temp}${unit} at ${step.desired_rate}${rateUnit}`;
		}
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

export interface ETAResult {
	/** null for hold steps — can't determine remaining time without step start time */
	currentStepSeconds: number | null;
	profileSeconds: number | null;
}

/**
 * Current step uses measured_rate; future steps use desired_rate from the profile.
 * Hold durations added as-is. Natural cooling steps (no rate) are skipped.
 */
export function calculateETAs(status: KilnStatus, profile: Profile): ETAResult {
	const { current_temp, measured_rate, step_index } = status;

	if (step_index === undefined || !profile.steps[step_index]) {
		return { currentStepSeconds: null, profileSeconds: null };
	}

	const currentStep = profile.steps[step_index];
	let currentStepSeconds: number | null = null;

	// Uses profile step target, NOT status.target_temp (which is a moving intermediate target)
	if (
		(currentStep.type === "ramp" || currentStep.type === "cooling") &&
		currentStep.target_temp !== undefined &&
		measured_rate !== undefined
	) {
		const remaining = Math.abs(currentStep.target_temp - current_temp);
		const absRate = Math.abs(measured_rate);

		if (absRate > 0 && remaining > 0) {
			currentStepSeconds = (remaining / absRate) * 3600;
		} else if (remaining <= 1) {
			currentStepSeconds = 0;
		}
	}

	let profileSeconds: number = currentStepSeconds ?? 0;

	for (let i = step_index + 1; i < profile.steps.length; i++) {
		const step = profile.steps[i];
		const prevStep = profile.steps[i - 1];
		const prevTarget = prevStep?.target_temp;

		if (
			(step.type === "ramp" || step.type === "cooling") &&
			step.target_temp !== undefined &&
			step.desired_rate !== undefined &&
			step.desired_rate > 0
		) {
			const fromTemp = prevTarget ?? current_temp;
			const tempDiff = Math.abs(step.target_temp - fromTemp);
			profileSeconds += (tempDiff / step.desired_rate) * 3600;
		} else if (step.type === "hold" && step.duration !== undefined) {
			profileSeconds += step.duration;
		}
	}

	return {
		currentStepSeconds,
		profileSeconds: profileSeconds > 0 ? profileSeconds : null,
	};
}

export function formatETA(seconds: number): string {
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.floor((seconds % 3600) / 60);

	if (hours > 0) {
		return `${hours}h ${minutes > 0 ? `${minutes}m` : ""}`.trim();
	}
	if (minutes > 0) {
		return `${minutes}m`;
	}
	return "< 1m";
}
