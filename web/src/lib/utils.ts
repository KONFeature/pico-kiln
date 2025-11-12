import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
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
