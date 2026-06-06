// Diagnostic log parsing utilities for kiln firmware diag output.

export type LogLevel = "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";

export interface LogEntry {
	raw: string; // the original line, verbatim
	time?: string; // "HH:MM:SS" when parsed
	level?: LogLevel; // when parsed
	tag?: string; // when parsed
	message: string; // parsed message, OR the full raw line when unparseable
}

/** All levels, most-severe first. */
export const LOG_LEVELS = [
	"ERROR",
	"WARN",
	"INFO",
	"DEBUG",
	"TRACE",
] as const satisfies readonly LogLevel[];

/** Matches a normal diag line: `HH:MM:SS LEVEL tag: message`. */
const LINE_RE =
	/^(\d{2}:\d{2}:\d{2})\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+(\S+?):\s?(.*)$/;

/** Parse one line into a LogEntry. Unparseable/header lines -> { raw, message: raw }. */
export function parseDiagLine(line: string): LogEntry {
	const match = LINE_RE.exec(line);
	if (!match) {
		return { raw: line, message: line };
	}
	return {
		raw: line,
		time: match[1],
		level: match[2] as LogLevel,
		tag: match[3],
		message: match[4],
	};
}

/** Split text on newlines (drop trailing empty lines) and parse each. */
export function parseDiagText(text: string): LogEntry[] {
	const lines = text.split("\n");
	while (lines.length > 0 && lines[lines.length - 1] === "") {
		lines.pop();
	}
	return lines.map(parseDiagLine);
}

/** Distinct tags present in entries (excludes undefined), stable order of first appearance. */
export function uniqueTags(entries: LogEntry[]): string[] {
	const seen = new Set<string>();
	const tags: string[] = [];
	for (const entry of entries) {
		if (entry.tag !== undefined && !seen.has(entry.tag)) {
			seen.add(entry.tag);
			tags.push(entry.tag);
		}
	}
	return tags;
}
