// React hooks over the native kiln monitor. No-ops on the plain web build.

import { useEffect, useState } from "react";
import type { TempSample } from "@/components/routes/control/LiveTempChart";
import {
	getKilnHistory,
	getMonitoringStatus,
	isTauri,
	type MonitoringStatus,
	onKilnMonitoring,
	onKilnSample,
} from "./kiln-monitor";

const MAX_SAMPLES = 240;

/**
 * Live temperature history sourced from the native rolling buffer: hydrates
 * with the accumulated (up to 4h) history on mount, then appends each new
 * sample event. Because the buffer persists in Rust, reopening the app shows
 * history gathered while it was backgrounded — not just since this mount.
 */
export function useKilnHistory(): TempSample[] {
	const [history, setHistory] = useState<TempSample[]>([]);

	useEffect(() => {
		if (!isTauri()) return;
		let alive = true;
		let unlisten: (() => void) | undefined;

		getKilnHistory()
			.then((points) => {
				if (!alive) return;
				setHistory(
					points
						.slice(-MAX_SAMPLES)
						.map((p) => ({ t: p.t, temp: p.temp, target: p.target })),
				);
			})
			.catch(() => {});

		onKilnSample((s) => {
			setHistory((prev) => {
				if (prev.length && prev[prev.length - 1].t === s.t) return prev;
				const next = [...prev, { t: s.t, temp: s.temp, target: s.target }];
				return next.length > MAX_SAMPLES
					? next.slice(next.length - MAX_SAMPLES)
					: next;
			});
		})
			.then((fn) => {
				if (alive) unlisten = fn;
				else fn();
			})
			.catch(() => {});

		return () => {
			alive = false;
			unlisten?.();
		};
	}, []);

	return history;
}

/**
 * Native monitor health, updated live. `null` on the web build or before the
 * first reading.
 */
export function useMonitoringStatus(): MonitoringStatus | null {
	const [status, setStatus] = useState<MonitoringStatus | null>(null);

	useEffect(() => {
		if (!isTauri()) return;
		let alive = true;
		let unlisten: (() => void) | undefined;

		getMonitoringStatus()
			.then((s) => {
				if (alive) setStatus(s);
			})
			.catch(() => {});

		onKilnMonitoring((s) => setStatus(s))
			.then((fn) => {
				if (alive) unlisten = fn;
				else fn();
			})
			.catch(() => {});

		return () => {
			alive = false;
			unlisten?.();
		};
	}, []);

	return status;
}
