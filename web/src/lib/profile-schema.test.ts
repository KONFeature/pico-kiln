import { describe, expect, it } from "vitest";
import { parseProfileText, validateProfile } from "./profile-schema";

const validProfile = {
	name: "Bisque",
	temp_units: "c",
	description: "Slow bisque",
	steps: [
		{ type: "ramp", target_temp: 600, desired_rate: 100 },
		{ type: "hold", target_temp: 600, duration: 600 },
		{ type: "cooling" },
	],
};

describe("validateProfile", () => {
	it("accepts a well-formed profile and returns typed data", () => {
		const result = validateProfile(validProfile);
		expect(result.ok).toBe(true);
		if (result.ok) {
			expect(result.profile.name).toBe("Bisque");
			expect(result.profile.steps).toHaveLength(3);
		}
	});

	it("defaults a missing description to an empty string", () => {
		const result = validateProfile({ ...validProfile, description: undefined });
		expect(result.ok && result.profile.description).toBe("");
	});

	it("rejects non-object input", () => {
		expect(validateProfile(null).ok).toBe(false);
		expect(validateProfile([]).ok).toBe(false);
		expect(validateProfile("nope").ok).toBe(false);
	});

	it("rejects a missing or blank name", () => {
		expect(validateProfile({ ...validProfile, name: "" }).ok).toBe(false);
		expect(validateProfile({ ...validProfile, name: 5 }).ok).toBe(false);
	});

	it("rejects invalid temperature units", () => {
		const result = validateProfile({ ...validProfile, temp_units: "kelvin" });
		expect(result.ok).toBe(false);
		if (!result.ok) expect(result.error).toMatch(/units/i);
	});

	it("rejects an empty or missing steps array", () => {
		expect(validateProfile({ ...validProfile, steps: [] }).ok).toBe(false);
		expect(validateProfile({ ...validProfile, steps: "x" }).ok).toBe(false);
	});

	it("rejects an unknown step type", () => {
		const result = validateProfile({
			...validProfile,
			steps: [{ type: "blast", target_temp: 600 }],
		});
		expect(result.ok).toBe(false);
		if (!result.ok) expect(result.error).toMatch(/invalid type/i);
	});

	it("requires target_temp and desired_rate on ramp steps", () => {
		expect(
			validateProfile({ ...validProfile, steps: [{ type: "ramp" }] }).ok,
		).toBe(false);
		expect(
			validateProfile({
				...validProfile,
				steps: [{ type: "ramp", target_temp: 600 }],
			}).ok,
		).toBe(false);
	});

	it("rejects non-positive rates and negative durations", () => {
		expect(
			validateProfile({
				...validProfile,
				steps: [{ type: "ramp", target_temp: 600, desired_rate: 0 }],
			}).ok,
		).toBe(false);
		expect(
			validateProfile({
				...validProfile,
				steps: [{ type: "hold", target_temp: 600, duration: -10 }],
			}).ok,
		).toBe(false);
	});

	it("allows cooling steps without a target", () => {
		const result = validateProfile({
			...validProfile,
			steps: [{ type: "cooling" }],
		});
		expect(result.ok).toBe(true);
	});
});

describe("parseProfileText", () => {
	it("parses valid JSON text", () => {
		const result = parseProfileText(JSON.stringify(validProfile));
		expect(result.ok).toBe(true);
	});

	it("reports invalid JSON distinctly from schema errors", () => {
		const result = parseProfileText("{not json");
		expect(result.ok).toBe(false);
		if (!result.ok) expect(result.error).toMatch(/valid JSON/i);
	});
});
