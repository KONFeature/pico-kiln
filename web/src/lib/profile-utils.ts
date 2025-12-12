/**
 * Profile utility functions for trajectory calculation and formatting
 */

import type { Profile, ProfileStep } from "./pico/types";

export interface TrajectoryPoint {
	time_hours: number;
	temp: number;
}

export interface Segment {
	data: TrajectoryPoint[];
	type: "ramp" | "hold" | "cooling";
	color: string;
	step: ProfileStep;
	desiredRate?: number;
	minRate?: number;
	duration?: number; // for holds, in minutes
}

/**
 * Calculate temperature trajectory from profile steps
 * Returns separate data arrays for each segment to enable different colors in charts
 */
export function calculateTrajectory(profile: Profile): Segment[] {
	const segments: Segment[] = [];
	let currentTime = 0;
	let currentTemp = 20; // Start at room temperature

	for (const step of profile.steps) {
		const targetTemp = step.target_temp ?? currentTemp;

		if (step.type === "hold") {
			// Hold: constant temperature for duration
			const duration = step.duration ?? 0;
			const data: TrajectoryPoint[] = [
				{ time_hours: currentTime / 3600, temp: currentTemp },
				{ time_hours: (currentTime + duration) / 3600, temp: currentTemp },
			];

			segments.push({
				data,
				type: "hold",
				color: "#eab308", // yellow-500
				step,
				duration: duration / 60, // convert to minutes
			});

			currentTime += duration;
		} else if (step.type === "ramp") {
			// Ramp: linear temperature change at desired rate
			const desiredRate = step.desired_rate ?? 100; // Default 100°C/h
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
				color: isHeating ? "#ef4444" : "#3b82f6", // red-500 for heating, blue-500 for cooling
				step,
				desiredRate: step.desired_rate,
				minRate: step.min_rate,
			});

			currentTime += durationSeconds;
			currentTemp = targetTemp;
		} else if (step.type === "cooling") {
			// Natural cooling: estimate at 100°C/h if target specified
			const coolingTarget = step.target_temp ?? 20; // Default to room temp
			const tempChange = Math.abs(currentTemp - coolingTarget);
			const naturalCoolingRate = 100; // Estimated natural cooling rate
			const durationHours = tempChange / naturalCoolingRate;
			const durationSeconds = durationHours * 3600;

			const data: TrajectoryPoint[] = [
				{ time_hours: currentTime / 3600, temp: currentTemp },
				{
					time_hours: (currentTime + durationSeconds) / 3600,
					temp: coolingTarget,
				},
			];

			segments.push({
				data,
				type: "cooling",
				color: "#06b6d4", // cyan-500 for natural cooling
				step,
			});

			currentTime += durationSeconds;
			currentTemp = coolingTarget;
		}
	}

	return segments;
}
