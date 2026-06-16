import { describe, expect, it } from "vitest";
import {
	LOG_LEVELS,
	type LogLevel,
	parseDiagLine,
	parseDiagText,
	uniqueTags,
} from "./diag-parser";

describe("parseDiagLine", () => {
	it("parses all four fields of a normal line", () => {
		const entry = parseDiagLine("01:02:03 INFO ctrl: temp=812");
		expect(entry).toEqual({
			raw: "01:02:03 INFO ctrl: temp=812",
			time: "01:02:03",
			level: "INFO",
			tag: "ctrl",
			message: "temp=812",
		});
	});

	it("parses every level value", () => {
		for (const level of LOG_LEVELS) {
			const entry = parseDiagLine(`12:00:00 ${level} app: hello`);
			expect(entry.level).toBe<LogLevel>(level);
			expect(entry.tag).toBe("app");
			expect(entry.message).toBe("hello");
		}
	});

	it("keeps an empty message when there is none after the tag", () => {
		const entry = parseDiagLine("00:00:01 WARN wifi: ");
		expect(entry.level).toBe("WARN");
		expect(entry.tag).toBe("wifi");
		expect(entry.message).toBe("");
	});

	it("treats a `# diag ...` header as a raw entry", () => {
		const line = "# diag 2026-06-06T12:00:00Z";
		const entry = parseDiagLine(line);
		expect(entry).toEqual({ raw: line, message: line });
		expect(entry.level).toBeUndefined();
		expect(entry.tag).toBeUndefined();
		expect(entry.time).toBeUndefined();
	});

	it("treats an unparseable line as a raw entry", () => {
		const line = "some raw library text without a tag";
		const entry = parseDiagLine(line);
		expect(entry).toEqual({ raw: line, message: line });
		expect(entry.level).toBeUndefined();
		expect(entry.tag).toBeUndefined();
	});
});

describe("parseDiagText", () => {
	it("splits multiple lines and drops trailing empty lines", () => {
		const text = "01:02:03 INFO ctrl: a\n01:02:04 WARN wifi: b\n\n";
		const entries = parseDiagText(text);
		expect(entries).toHaveLength(2);
		expect(entries[0].tag).toBe("ctrl");
		expect(entries[1].tag).toBe("wifi");
	});

	it("returns an empty array for empty text", () => {
		expect(parseDiagText("")).toEqual([]);
		expect(parseDiagText("\n\n")).toEqual([]);
	});
});

describe("uniqueTags", () => {
	it("returns distinct tags in first-appearance order", () => {
		const entries = parseDiagText(
			[
				"01:00:00 INFO net: up",
				"01:00:01 INFO ctrl: temp",
				"01:00:02 DEBUG net: retry",
				"# diag header",
				"01:00:03 INFO web: serve",
				"01:00:04 INFO ctrl: temp",
			].join("\n"),
		);
		expect(uniqueTags(entries)).toEqual(["net", "ctrl", "web"]);
	});

	it("excludes raw entries with no tag", () => {
		const entries = parseDiagText("raw line\nmore raw");
		expect(uniqueTags(entries)).toEqual([]);
	});
});
