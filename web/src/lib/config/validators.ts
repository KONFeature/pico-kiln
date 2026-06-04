import type { ConfigValue } from "@/lib/pico/types";

export type Validator = (value: ConfigValue) => string | undefined;

/** Required, finite number within [min, max]. */
export function num(min: number, max: number, integer = false): Validator {
	return (value) => {
		if (value === "" || value === null || value === undefined) {
			return "Required";
		}
		const n = typeof value === "number" ? value : Number(value);
		if (!Number.isFinite(n)) return "Must be a number";
		if (integer && !Number.isInteger(n)) return "Must be a whole number";
		if (n < min || n > max) return `Must be between ${min} and ${max}`;
		return undefined;
	};
}

/** Non-empty string up to maxLen chars. */
export function str(maxLen: number, required = true): Validator {
	return (value) => {
		const s = typeof value === "string" ? value : "";
		if (required && s.length === 0) return "Required";
		if (s.length > maxLen) return `Must be at most ${maxLen} characters`;
		return undefined;
	};
}

/** Optional string up to maxLen chars (empty allowed). */
export function optionalStr(maxLen: number): Validator {
	return str(maxLen, false);
}

const IPV4 =
	/^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;

/** Empty (treated as DHCP/null) or a valid dotted-quad IPv4 address. */
export function ipv4Optional(): Validator {
	return (value) => {
		const s = typeof value === "string" ? value.trim() : "";
		if (s.length === 0) return undefined;
		if (!IPV4.test(s)) {
			return "Must be a valid IPv4 address (e.g. 192.168.1.50)";
		}
		return undefined;
	};
}

/** A host string: empty rejected, max 64 chars. */
export function host(): Validator {
	return (value) => {
		const s = typeof value === "string" ? value.trim() : "";
		if (s.length === 0) return "Required";
		if (s.length > 64) return "Must be at most 64 characters";
		return undefined;
	};
}

/**
 * 1..maxPins unique integer GPIO pins, each within [0, 29]. Value is a number[]
 * (parsed from the comma-separated input by ConfigField).
 */
export function pinList(maxPins = 10): Validator {
	return (value) => {
		if (!Array.isArray(value)) return "Enter one or more pins";
		if (value.length === 0) return "At least one pin is required";
		if (value.length > maxPins) return `At most ${maxPins} pins`;
		const seen = new Set<number>();
		for (const p of value) {
			if (!Number.isInteger(p)) return "Pins must be whole numbers";
			if (p < 0 || p > 29) return "Pins must be between 0 and 29";
			if (seen.has(p)) return `Duplicate pin: ${p}`;
			seen.add(p);
		}
		return undefined;
	};
}
