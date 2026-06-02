import type {
	Profile,
	ProfileStep,
	ProfileStepType,
	TempUnits,
} from "./pico/types";

export type ProfileParseResult =
	| { ok: true; profile: Profile }
	| { ok: false; error: string };

const STEP_TYPES: ProfileStepType[] = ["ramp", "hold", "cooling"];

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}

function validateStep(
	raw: unknown,
	index: number,
): { step: ProfileStep } | { error: string } {
	const where = `Step ${index + 1}`;

	if (!isObject(raw)) {
		return { error: `${where} is not a valid object.` };
	}

	const type = raw.type;
	if (
		typeof type !== "string" ||
		!STEP_TYPES.includes(type as ProfileStepType)
	) {
		return {
			error: `${where} has an invalid type "${String(type)}" (expected ramp, hold, or cooling).`,
		};
	}

	const step: ProfileStep = { type: type as ProfileStepType };

	if (raw.target_temp !== undefined && raw.target_temp !== null) {
		if (!isFiniteNumber(raw.target_temp)) {
			return { error: `${where}: target temperature must be a number.` };
		}
		step.target_temp = raw.target_temp;
	}

	if (raw.desired_rate !== undefined && raw.desired_rate !== null) {
		if (!isFiniteNumber(raw.desired_rate) || raw.desired_rate <= 0) {
			return { error: `${where}: desired rate must be a positive number.` };
		}
		step.desired_rate = raw.desired_rate;
	}

	if (raw.min_rate !== undefined && raw.min_rate !== null) {
		if (!isFiniteNumber(raw.min_rate) || raw.min_rate <= 0) {
			return { error: `${where}: min rate must be a positive number.` };
		}
		step.min_rate = raw.min_rate;
	}

	if (raw.duration !== undefined && raw.duration !== null) {
		if (!isFiniteNumber(raw.duration) || raw.duration < 0) {
			return { error: `${where}: duration must be zero or a positive number.` };
		}
		step.duration = raw.duration;
	}

	if (type === "ramp") {
		if (step.target_temp === undefined) {
			return { error: `${where} (ramp): target temperature is required.` };
		}
		if (step.desired_rate === undefined) {
			return { error: `${where} (ramp): desired rate is required.` };
		}
	}

	if (type === "hold" && step.target_temp === undefined) {
		return { error: `${where} (hold): target temperature is required.` };
	}

	return { step };
}

/** Validate an unknown value as a kiln Profile, returning typed data or a reason. */
export function validateProfile(input: unknown): ProfileParseResult {
	if (!isObject(input)) {
		return { ok: false, error: "Profile must be a JSON object." };
	}

	const { name, temp_units, description, steps } = input;

	if (typeof name !== "string" || name.trim() === "") {
		return { ok: false, error: "Profile name is required." };
	}
	if (temp_units !== "c" && temp_units !== "f") {
		return { ok: false, error: 'Temperature units must be "c" or "f".' };
	}
	if (description !== undefined && typeof description !== "string") {
		return { ok: false, error: "Description must be text." };
	}
	if (!Array.isArray(steps) || steps.length === 0) {
		return { ok: false, error: "Profile must have at least one step." };
	}

	const validatedSteps: ProfileStep[] = [];
	for (let i = 0; i < steps.length; i++) {
		const result = validateStep(steps[i], i);
		if ("error" in result) {
			return { ok: false, error: result.error };
		}
		validatedSteps.push(result.step);
	}

	return {
		ok: true,
		profile: {
			name,
			temp_units: temp_units as TempUnits,
			description: typeof description === "string" ? description : "",
			steps: validatedSteps,
		},
	};
}

/** Parse + schema-validate profile JSON text in one step. */
export function parseProfileText(text: string): ProfileParseResult {
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return { ok: false, error: "This file isn't valid JSON." };
	}
	return validateProfile(parsed);
}

/**
 * Lenient structural parse for a persisted editor draft. Must NOT enforce
 * validateProfile's required-field rules: a mid-edit draft is legitimately
 * incomplete, and rejecting it would discard in-progress work on remount.
 */
export function parseDraftProfile(input: unknown): Profile | null {
	if (!isObject(input)) {
		return null;
	}

	const { name, temp_units, description, steps } = input;

	if (typeof name !== "string") {
		return null;
	}
	if (temp_units !== "c" && temp_units !== "f") {
		return null;
	}
	if (!Array.isArray(steps)) {
		return null;
	}

	const draftSteps: ProfileStep[] = [];
	for (const raw of steps) {
		if (!isObject(raw)) {
			return null;
		}
		if (
			typeof raw.type !== "string" ||
			!STEP_TYPES.includes(raw.type as ProfileStepType)
		) {
			return null;
		}
		const step: ProfileStep = { type: raw.type as ProfileStepType };
		if (isFiniteNumber(raw.target_temp)) {
			step.target_temp = raw.target_temp;
		}
		if (isFiniteNumber(raw.desired_rate)) {
			step.desired_rate = raw.desired_rate;
		}
		if (isFiniteNumber(raw.min_rate)) {
			step.min_rate = raw.min_rate;
		}
		if (isFiniteNumber(raw.duration)) {
			step.duration = raw.duration;
		}
		draftSteps.push(step);
	}

	return {
		name,
		temp_units,
		description: typeof description === "string" ? description : "",
		steps: draftSteps,
	};
}
