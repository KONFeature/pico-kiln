import { describe, expect, it } from "vitest";
import { elapsedSecondsToDate, formatElapsed } from "./chart-time";

describe("elapsedSecondsToDate", () => {
	it("maps elapsed seconds to an epoch-offset Date whose getTime() round-trips", () => {
		expect(elapsedSecondsToDate(0).getTime()).toBe(0);
		expect(elapsedSecondsToDate(3600).getTime()).toBe(3_600_000);
		expect(elapsedSecondsToDate(43_200).getTime()).toBe(43_200_000);
	});
});

describe("formatElapsed", () => {
	it("formats sub-minute values in seconds", () => {
		expect(formatElapsed(0)).toBe("0s");
		expect(formatElapsed(45)).toBe("45s");
	});

	it("formats sub-hour values in minutes", () => {
		expect(formatElapsed(60)).toBe("1m");
		expect(formatElapsed(600)).toBe("10m");
	});

	it("formats values under 10 hours with one decimal", () => {
		expect(formatElapsed(5400)).toBe("1.5h");
		expect(formatElapsed(3600)).toBe("1h");
	});

	it("rounds values of 10+ hours to whole hours", () => {
		expect(formatElapsed(46_800)).toBe("13h");
	});

	it("returns an empty string for non-finite input", () => {
		expect(formatElapsed(Number.NaN)).toBe("");
		expect(formatElapsed(Number.POSITIVE_INFINITY)).toBe("");
	});
});
