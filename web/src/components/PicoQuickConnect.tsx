// Quick-connect presets for the Pico connection settings.
//
// The RP2350 firmware always exposes the device at two fixed gateway IPs (see
// rust/kiln-firmware/src/platform.rs): the USB-NCM link at 192.168.7.1 and the
// SoftAP at 192.168.4.1. Those never change, so they get one-click buttons.
// A LAN / mDNS address (once the kiln has joined another Wi-Fi) is site-specific,
// so it stays in the manual field rendered next to this component.

import { useQueryClient } from "@tanstack/react-query";
import {
	CircleCheckBig,
	CircleX,
	Loader2,
	type LucideIcon,
	Router,
	Usb,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { PicoAPIClient } from "@/lib/pico/client";
import { usePico } from "@/lib/pico/context";
import { picoKeys } from "@/lib/pico/hooks";

interface Preset {
	id: string;
	label: string;
	url: string;
	hint: string;
	note: string;
	icon: LucideIcon;
}

const PRESETS: Preset[] = [
	{
		id: "usb",
		label: "USB Cable",
		url: "http://192.168.7.1",
		hint: "192.168.7.1",
		note: "Plug the USB cable in",
		icon: Usb,
	},
	{
		id: "ap",
		label: "Access Point",
		url: "http://192.168.4.1",
		hint: "192.168.4.1",
		note: "Join the kiln's Wi-Fi first",
		icon: Router,
	},
];

// Reachability probe timeout — shorter than the client default (10s) so a wrong
// guess fails fast and the user can try the other preset.
const PROBE_TIMEOUT_MS = 6000;
// How long the success/failure glyph stays visible before we hand off via
// onConnected (e.g. closing the settings dialog), so the verdict is seen.
const VERDICT_LINGER_MS = 800;

interface PicoQuickConnectProps {
	/** Called shortly after a preset connects successfully (e.g. close dialog). */
	onConnected?: () => void;
	/** Called with the chosen URL so the parent can mirror it in its input. */
	onPick?: (url: string) => void;
}

export function PicoQuickConnect({
	onConnected,
	onPick,
}: PicoQuickConnectProps) {
	const { setPicoURL } = usePico();
	const queryClient = useQueryClient();

	const [pendingId, setPendingId] = useState<string | null>(null);
	const [result, setResult] = useState<{ id: string; ok: boolean } | null>(
		null,
	);

	const connect = async (preset: Preset) => {
		// Persist + swap the app-wide client so the rest of the UI follows this URL.
		setPicoURL(preset.url);
		onPick?.(preset.url);
		setResult(null);
		setPendingId(preset.id);

		// Probe with a throwaway client bound to the chosen URL: the context client
		// created by setPicoURL is not visible until the next render, so testing it
		// here would hit the stale one. testConnection never throws (returns false).
		const ok = await new PicoAPIClient(
			preset.url,
			PROBE_TIMEOUT_MS,
		).testConnection();

		setPendingId(null);
		setResult({ id: preset.id, ok });

		if (ok) {
			// Refresh every status-dependent view against the newly selected URL.
			queryClient.invalidateQueries({ queryKey: picoKeys.status });
			if (onConnected) {
				setTimeout(onConnected, VERDICT_LINGER_MS);
			}
		}
	};

	return (
		<div className="space-y-2">
			<p className="text-sm font-medium">Quick connect</p>
			<div className="grid grid-cols-2 gap-2">
				{PRESETS.map((preset) => {
					const Icon = preset.icon;
					const isPending = pendingId === preset.id;
					const verdict = result?.id === preset.id ? result.ok : null;
					return (
						<Button
							key={preset.id}
							type="button"
							variant="outline"
							onClick={() => connect(preset)}
							disabled={isPending}
							className="h-auto flex-col items-start gap-1 py-3 text-left"
						>
							<span className="flex w-full items-center gap-2">
								<Icon className="h-4 w-4 shrink-0" />
								<span className="font-medium">{preset.label}</span>
								<span className="ml-auto inline-flex">
									{isPending ? (
										<Loader2
											className="h-4 w-4 animate-spin"
											aria-label="Connecting"
										/>
									) : verdict === true ? (
										<CircleCheckBig
											className="h-4 w-4 text-success"
											aria-label="Connected"
										/>
									) : verdict === false ? (
										<CircleX
											className="h-4 w-4 text-destructive"
											aria-label="Failed"
										/>
									) : null}
								</span>
							</span>
							<span className="font-mono text-xs text-muted-foreground">
								{preset.hint}
							</span>
							<span className="text-xs text-muted-foreground">
								{preset.note}
							</span>
						</Button>
					);
				})}
			</div>
			{result && !result.ok && (
				<p className="text-xs text-destructive">
					Couldn't reach the kiln at that address. Check the cable / Wi-Fi, or
					enter the address manually below.
				</p>
			)}
		</div>
	);
}
