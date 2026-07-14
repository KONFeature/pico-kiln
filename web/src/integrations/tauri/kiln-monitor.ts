// Bridge to the native (Rust) background kiln monitor.
//
// In a Tauri build the Rust supervisor is the single poller of the kiln; the
// frontend reads status/history through these commands + live events instead
// of hitting the Pico directly. On the plain web build `isTauri()` is false and
// none of this is used (see lib/pico/hooks.ts for the source switch).

import { invoke, isTauri as tauriIsTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
	isServiceRunning,
	startService,
	stopService,
} from "tauri-plugin-background-service";
import type { KilnStatus } from "@/lib/pico/types";

/** True when running inside the Tauri shell (desktop or mobile). */
export function isTauri(): boolean {
	try {
		return tauriIsTauri();
	} catch {
		return false;
	}
}

/** A single temperature sample from the native rolling history buffer. */
export interface KilnHistoryPoint {
	/** Epoch milliseconds. */
	t: number;
	temp: number;
	target: number | null;
	state: string;
}

/** Native monitor health (drives the "not monitoring" toast). */
export interface MonitoringStatus {
	running: boolean;
	active: boolean;
	reachable: boolean;
	url?: string | null;
	lastError?: string | null;
	lastOk?: number | null;
}

// === Commands ===

/** Point the native poller at a kiln (or clear it with `null`). */
export async function setKilnUrl(url: string | null): Promise<void> {
	await invoke("set_kiln_url", { url: url || null });
}

/** Latest status snapshot the native monitor holds (null before first poll). */
export async function getKilnStatus(): Promise<KilnStatus | null> {
	return invoke<KilnStatus | null>("get_kiln_status");
}

/** Accumulated rolling temperature history (last 4h). */
export async function getKilnHistory(): Promise<KilnHistoryPoint[]> {
	return invoke<KilnHistoryPoint[]>("get_kiln_history");
}

/** Native monitor health snapshot. */
export async function getMonitoringStatus(): Promise<MonitoringStatus> {
	return invoke<MonitoringStatus>("monitoring_status");
}

/** Force an immediate poll (e.g. right after a control command). */
export async function refreshKiln(): Promise<void> {
	await invoke("refresh_kiln");
}

// === Events ===

export function onKilnStatus(
	handler: (status: KilnStatus) => void,
): Promise<UnlistenFn> {
	return listen<KilnStatus>("kiln://status", (e) => handler(e.payload));
}

export function onKilnSample(
	handler: (sample: KilnHistoryPoint) => void,
): Promise<UnlistenFn> {
	return listen<KilnHistoryPoint>("kiln://sample", (e) => handler(e.payload));
}

export function onKilnMonitoring(
	handler: (status: MonitoringStatus) => void,
): Promise<UnlistenFn> {
	return listen<MonitoringStatus>("kiln://monitoring", (e) =>
		handler(e.payload),
	);
}

// === Foreground service (promotion) ===

const ACTIVE_STATES = new Set(["RUNNING", "TUNING"]);

/** Whether a status warrants keeping the foreground service alive. */
export function isActiveStatus(status: KilnStatus | undefined | null): boolean {
	if (!status) return false;
	if (ACTIVE_STATES.has(status.state)) return true;
	return Boolean(status.scheduled_profile);
}

function tempTarget(status: KilnStatus): string {
	const temp = `${Math.round(status.current_temp)}\u00b0C`;
	return status.target_temp && status.target_temp > 0
		? `${temp} \u2192 ${Math.round(status.target_temp)}\u00b0C`
		: temp;
}

function countdown(seconds: number): string {
	if (seconds <= 0) return "moments";
	const h = Math.floor(seconds / 3600);
	const m = Math.floor((seconds % 3600) / 60);
	if (h > 0) return `${h}h ${m}m`;
	if (m > 0) return `${m}m`;
	return "less than a minute";
}

/**
 * Content text for the foreground-service notification. This is the snapshot
 * shown the instant the service starts; the Rust monitor then re-posts the same
 * notification each poll to keep the temperature live (including while the app
 * is backgrounded). Kept in sync with `notification_content` in monitor/mod.rs.
 */
function serviceLabel(status: KilnStatus): string {
	switch (status.state) {
		case "RUNNING": {
			const prefix = status.profile_name ? `${status.profile_name}: ` : "";
			let label = `${prefix}${tempTarget(status)}`;
			if (
				status.step_index !== undefined &&
				status.total_steps !== undefined &&
				status.total_steps > 0
			) {
				label += ` \u00b7 Step ${status.step_index + 1}/${status.total_steps}`;
			}
			return label;
		}
		case "TUNING":
			return `PID tuning \u00b7 ${tempTarget(status)}`;
		default:
			return status.scheduled_profile
				? `Firing starts in ${countdown(status.scheduled_profile.seconds_until_start)}`
				: "Monitoring kiln";
	}
}

/**
 * Ensure the foreground service matches the kiln's active state: start it when
 * the kiln becomes active (so monitoring survives the app being backgrounded),
 * and leave shutdown to the Rust service, which self-demotes once the kiln
 * settles. Safe to call on every status update. Start is the only side we drive
 * from JS; the native service owns stopping itself.
 */
export async function syncForegroundService(
	status: KilnStatus | null,
): Promise<void> {
	if (!isTauri()) return;
	if (!isActiveStatus(status)) return;
	try {
		if (await isServiceRunning()) return;
		await startService({
			// biome-ignore lint/style/noNonNullAssertion: isActiveStatus guards null
			serviceLabel: serviceLabel(status!),
			foregroundServiceType: "dataSync",
		});
	} catch {
		// Already running or platform rejected — non-fatal for monitoring.
	}
}

/** Explicitly stop the foreground service (used when clearing the kiln URL). */
export async function stopForegroundService(): Promise<void> {
	if (!isTauri()) return;
	try {
		if (await isServiceRunning()) await stopService();
	} catch {
		// Not running — ignore.
	}
}
