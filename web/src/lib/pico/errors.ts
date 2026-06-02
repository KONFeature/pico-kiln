// Friendly error mapping for the Pico API.
//
// Raw API/network errors are technical and scary ("HTTP 500: …", "Request
// timeout after 10000ms"). This module turns them into calm, recovery-oriented
// copy for makers while preserving the original message as `detail` so the
// technical truth is still one tap away (progressive disclosure).

import { PicoAPIError } from "./client";
import type { KilnState } from "./types";

export interface FriendlyError {
	/** Short headline, e.g. "Kiln isn't responding". */
	title: string;
	/** One or two sentences explaining what happened and how to recover. */
	message: string;
	/** Raw technical message, surfaced behind a disclosure for debugging. */
	detail?: string;
}

/**
 * A logical failure is an HTTP 200 response whose body is `{ success: false }`.
 * `unwrap()` (in hooks.ts) rethrows these as PicoAPIError with the original
 * response attached as `originalError`, so the device-provided message is
 * already human-readable and we can trust it.
 */
export function isLogicalFailure(err: PicoAPIError): boolean {
	const orig = err.originalError;
	return (
		typeof orig === "object" &&
		orig !== null &&
		"success" in orig &&
		(orig as { success?: unknown }).success === false
	);
}

/** Strip the "HTTP <code>: " prefix the client adds, returning the body text. */
function httpBodyText(raw: string): string | undefined {
	const match = raw.match(/^HTTP \d+:\s*([\s\S]*)$/);
	const body = match?.[1]?.trim();
	if (!body || body === "Unknown error") return undefined;
	return body;
}

export function getFriendlyError(err: unknown): FriendlyError {
	const raw =
		err instanceof Error
			? err.message
			: typeof err === "string"
				? err
				: "Unknown error";

	if (err instanceof PicoAPIError) {
		// Device said "no" with a reason — trust its message.
		if (isLogicalFailure(err)) {
			return {
				title: "The kiln declined that",
				message: raw || "The kiln couldn't complete that request.",
			};
		}

		const { statusCode } = err;

		// No status code => never reached the device (timeout / network / unconfigured).
		if (statusCode === undefined) {
			if (/timeout/i.test(raw)) {
				return {
					title: "Kiln isn't responding",
					message:
						"The kiln didn't reply in time. Make sure it's powered on and on the same Wi‑Fi network, then try again.",
					detail: raw,
				};
			}
			if (/not initialized|not configured/i.test(raw)) {
				return {
					title: "No kiln connected",
					message:
						"Add your kiln's address in connection settings to get started.",
					detail: raw,
				};
			}
			return {
				title: "Can't reach the kiln",
				message:
					"Couldn't connect to the kiln. Check your Wi‑Fi and confirm the kiln's address is correct.",
				detail: raw,
			};
		}

		// Reached an address, but the reply wasn't the kiln's JSON API.
		if (/non-?json|unexpected/i.test(raw)) {
			return {
				title: "Unexpected response",
				message:
					"That address replied, but not like a kiln would. Double‑check it points to your kiln controller.",
				detail: raw,
			};
		}

		if (statusCode === 404) {
			return {
				title: "Not found",
				message:
					"The kiln didn't recognize this request. Its firmware may be out of date.",
				detail: raw,
			};
		}

		if (statusCode >= 500) {
			return {
				title: "Kiln reported an error",
				message:
					"The controller hit an internal error. Wait a moment and try again; if it keeps happening, reboot the controller.",
				detail: raw,
			};
		}

		// Other 4xx — the device usually explains why in the body.
		return {
			title: "Request rejected",
			message: httpBodyText(raw) ?? "The kiln rejected the request.",
			detail: raw,
		};
	}

	return { title: "Something went wrong", message: raw };
}

/** Friendly, glanceable labels for kiln states (avoids raw enum jargon). */
export const STATE_LABELS: Record<KilnState, string> = {
	IDLE: "Idle",
	RUNNING: "Running",
	TUNING: "Tuning",
	COMPLETE: "Complete",
	ERROR: "Error",
};

/** One-line plain-language explanation of each state for progressive disclosure. */
export const STATE_DESCRIPTIONS: Record<KilnState, string> = {
	IDLE: "Ready — no program is running.",
	RUNNING: "A firing profile is running.",
	TUNING: "Measuring your kiln to calculate PID settings.",
	COMPLETE: "The last program finished.",
	ERROR: "Heating has stopped after a fault. Clear the error to continue.",
};
