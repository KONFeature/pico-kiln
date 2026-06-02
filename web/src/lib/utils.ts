import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

/** Format seconds as "Xh Ym" / "Xm Ys" / "Xs"; "N/A" when missing/NaN. */
export function formatDuration(seconds?: number): string {
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
}

/**
 * Read a file as text using FileReader
 * Returns a promise that resolves with the file content
 */
export function readFileAsText(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = (e) => {
			const result = e.target?.result;
			if (typeof result === "string") {
				resolve(result);
			} else {
				reject(new Error("Failed to read file as text"));
			}
		};
		reader.onerror = () =>
			reject(reader.error || new Error("Failed to read file"));
		reader.readAsText(file);
	});
}
