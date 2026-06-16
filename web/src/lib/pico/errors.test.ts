import { describe, expect, it } from "vitest";
import { PicoAPIError } from "./client";
import { getFriendlyError } from "./errors";

describe("getFriendlyError", () => {
	it("maps a timeout to a calm, recovery-oriented message", () => {
		const result = getFriendlyError(
			new PicoAPIError("Request timeout after 10000ms"),
		);
		expect(result.title).toBe("Kiln isn't responding");
		expect(result.message).toMatch(/powered on/i);
		expect(result.detail).toBe("Request timeout after 10000ms");
	});

	it("maps a network failure to a connection hint", () => {
		const result = getFriendlyError(new PicoAPIError("Network request failed"));
		expect(result.title).toBe("Can't reach the kiln");
		expect(result.detail).toBe("Network request failed");
	});

	it("maps an unconfigured client to a setup hint", () => {
		const result = getFriendlyError(
			new PicoAPIError("Pico client not initialized"),
		);
		expect(result.title).toBe("No kiln connected");
	});

	it("maps a non-JSON response to a wrong-address hint", () => {
		const result = getFriendlyError(
			new PicoAPIError(
				"Received an unexpected (non-JSON) response from the kiln.",
				200,
			),
		);
		expect(result.title).toBe("Unexpected response");
	});

	it("maps HTTP 404 and 500 distinctly", () => {
		expect(getFriendlyError(new PicoAPIError("HTTP 404: x", 404)).title).toBe(
			"Not found",
		);
		expect(
			getFriendlyError(new PicoAPIError("HTTP 500: boom", 500)).title,
		).toBe("Kiln reported an error");
	});

	it("surfaces the body text for other 4xx errors", () => {
		const result = getFriendlyError(
			new PicoAPIError("HTTP 409: kiln is busy", 409),
		);
		expect(result.message).toBe("kiln is busy");
	});

	it("falls back gracefully for unknown error shapes", () => {
		expect(getFriendlyError("boom").message).toBe("boom");
		expect(getFriendlyError(new Error("oops")).title).toBe(
			"Something went wrong",
		);
	});
});
