import { describe, expect, it } from "vitest";
import {
	ALL_FIELDS,
	buildPatch,
	FIRING_LOCKED_PREFIX,
	HARDWARE_KEYS,
	hardwarePatch,
	isFiringLocked,
} from "./schema";
import { fixed } from "./validators";

describe("firing lock", () => {
	it("detects the firmware's lock messages and nothing else", () => {
		// Verbatim from kiln-app errors::firing_lock_message.
		expect(
			isFiringLocked({
				error_message:
					"Firing locked: no config.json on the controller - save the configuration, then reboot",
			}),
		).toBe(true);
		expect(isFiringLocked({ error: `${FIRING_LOCKED_PREFIX} x` })).toBe(true);
		expect(
			isFiringLocked({ error_message: "Sensor emergency shutdown: too many" }),
		).toBe(false);
		expect(isFiringLocked({})).toBe(false);
		expect(isFiringLocked(undefined)).toBe(false);
	});

	it("always sends the hardware keys, in wire form, while locked", () => {
		const values = {
			THERMOCOUPLE_TYPE: "S",
			SSR_PIN: [15],
			MAINS_FREQUENCY: "50",
			MAX_TEMP: "1300",
			WIFI_SSID: "home",
		};
		expect(hardwarePatch(values)).toEqual({
			THERMOCOUPLE_TYPE: "S",
			SSR_PIN: [15],
			MAINS_FREQUENCY: 50,
			MAX_TEMP: 1300,
		});
	});

	it("only names keys that are real form fields", () => {
		const keys = new Set(ALL_FIELDS.map((f) => f.key as string));
		for (const k of HARDWARE_KEYS) expect(keys.has(k)).toBe(true);
	});

	it("leaves the unlocked save sparse", () => {
		const server = {
			THERMOCOUPLE_TYPE: "S",
			MAX_TEMP: 1300,
		} as unknown as Parameters<typeof buildPatch>[1];
		const patch = buildPatch(
			{ THERMOCOUPLE_TYPE: "S", MAX_TEMP: 1300 },
			server,
		);
		expect(patch.THERMOCOUPLE_TYPE).toBeUndefined();
		expect(patch.MAX_TEMP).toBeUndefined();
	});
});

describe("fixed wiring validator", () => {
	it("accepts only the hardwired value", () => {
		const v = fixed(21);
		expect(v(21)).toBeUndefined();
		expect(v("21")).toBeUndefined();
		expect(v(5)).toMatch(/fixed by the firmware/);
	});
});
