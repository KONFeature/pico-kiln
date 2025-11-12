// CSV parsing utilities for kiln log files

export interface LogDataPoint {
	elapsed_seconds: number;
	current_temp_c: number;
	target_temp_c: number;
	ssr_output_percent: number;
	state: string;
	timestamp: string;
	step_name?: string;
	step_index?: number;
	total_steps?: number;
	current_rate_c_per_hour?: number;
}

export interface ParsedLogData {
	data: LogDataPoint[];
	headers: string[];
}

/**
 * Parse CSV log file content into structured data
 */
export function parseLogCSV(content: string): ParsedLogData {
	const lines = content.trim().split("\n");

	if (lines.length === 0) {
		throw new Error("Empty CSV file");
	}

	const headers = lines[0].split(",").map((h) => h.trim());
	const data: LogDataPoint[] = [];

	for (let i = 1; i < lines.length; i++) {
		const line = lines[i].trim();
		if (!line) continue;

		const values = line.split(",").map((v) => v.trim());
		if (values.length !== headers.length) {
			console.warn(`Line ${i + 1} has mismatched column count, skipping`);
			continue;
		}

		const row: Record<string, string> = {};
		headers.forEach((header, idx) => {
			row[header] = values[idx];
		});

		// Skip RECOVERY state entries
		if (row.state === "RECOVERY") {
			continue;
		}

		const dataPoint: LogDataPoint = {
			elapsed_seconds: parseFloat(row.elapsed_seconds || "0"),
			current_temp_c: parseFloat(row.current_temp_c || "0"),
			target_temp_c: parseFloat(row.target_temp_c || "0"),
			ssr_output_percent: parseFloat(row.ssr_output_percent || "0"),
			state: row.state || "UNKNOWN",
			timestamp: row.timestamp || "",
		};

		// Add optional fields if present
		if (row.step_name) dataPoint.step_name = row.step_name;
		if (row.step_index) dataPoint.step_index = parseInt(row.step_index, 10);
		if (row.total_steps) dataPoint.total_steps = parseInt(row.total_steps, 10);
		if (row.current_rate_c_per_hour) {
			dataPoint.current_rate_c_per_hour = parseFloat(
				row.current_rate_c_per_hour,
			);
		}

		data.push(dataPoint);
	}

	// Fallback: if all elapsed_seconds are 0, calculate from timestamps
	if (data.length > 0 && data.every((d) => d.elapsed_seconds === 0)) {
		console.warn("All elapsed_seconds are 0, calculating from timestamps");
		const startTime = new Date(data[0].timestamp).getTime();
		data.forEach((d) => {
			const dt = new Date(d.timestamp).getTime();
			d.elapsed_seconds = (dt - startTime) / 1000;
		});
	}

	return { data, headers };
}

/**
 * Convert elapsed seconds to hours for display
 */
export function secondsToHours(seconds: number): number {
	return seconds / 3600;
}

/**
 * Convert elapsed seconds to minutes for display
 */
export function secondsToMinutes(seconds: number): number {
	return seconds / 60;
}

/**
 * Detect if log data is from a tuning run or firing run
 */
export function detectRunType(data: LogDataPoint[]): "TUNING" | "FIRING" {
	return data.some((d) => d.state === "TUNING") ? "TUNING" : "FIRING";
}
