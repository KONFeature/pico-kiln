// Mounted once at the app root. Bridges the native background monitor to the
// React Query cache and the foreground-service lifecycle. Renders nothing on
// the plain web build.

import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle } from "lucide-react";
import { useEffect } from "react";
import { usePico } from "@/lib/pico/context";
import { picoKeys } from "@/lib/pico/hooks";
import { useMonitoringStatus } from "./hooks";
import {
	isTauri,
	onKilnStatus,
	refreshKiln,
	setKilnUrl,
	stopForegroundService,
	syncForegroundService,
} from "./kiln-monitor";

export function KilnMonitorBridge() {
	const { picoURL, isConfigured } = usePico();
	const queryClient = useQueryClient();
	const monitoring = useMonitoringStatus();

	// Keep the native poller pointed at the configured kiln.
	useEffect(() => {
		if (!isTauri()) return;
		(async () => {
			await setKilnUrl(picoURL || null);
			if (picoURL) {
				// Poll the newly configured kiln now instead of waiting up to a
				// full cadence for the supervisor's next tick.
				void refreshKiln();
			} else {
				void stopForegroundService();
			}
		})();
	}, [picoURL]);

	// Push native status into the query cache and promote to the foreground
	// service when the kiln becomes active.
	useEffect(() => {
		if (!isTauri()) return;
		let unlisten: (() => void) | undefined;
		let alive = true;

		onKilnStatus((status) => {
			queryClient.setQueryData(picoKeys.status, status);
			void syncForegroundService(status);
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
	}, [queryClient]);

	if (!isTauri() || !isConfigured || !monitoring) return null;

	// Surface a problem only: poller not running, or an active firing we can no
	// longer reach.
	const notRunning = !monitoring.running;
	const activeUnreachable = monitoring.active && !monitoring.reachable;
	if (!notRunning && !activeUnreachable) return null;

	const message = notRunning
		? "Background monitoring is not running."
		: "Lost connection to the kiln while firing.";

	return (
		<div className="fixed inset-x-0 bottom-0 z-50 flex justify-center px-4 pb-4">
			<div className="flex items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 shadow-sm backdrop-blur dark:text-amber-300">
				<AlertTriangle className="h-4 w-4 flex-shrink-0" />
				<span>{message}</span>
			</div>
		</div>
	);
}
