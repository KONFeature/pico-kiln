import { type RefObject, useCallback, useEffect, useState } from "react";

type LockableOrientation = ScreenOrientation & {
	lock?: (orientation: string) => Promise<void>;
	unlock?: () => void;
};

export interface FullscreenLandscape {
	isFullscreen: boolean;
	supported: boolean;
	toggle: () => void;
}

/**
 * Drives the native Fullscreen API on `ref` and best-effort locks the device to
 * landscape while fullscreen (orientation lock is unsupported on desktop and
 * rejects there — that's expected and swallowed). The orientation is unlocked
 * on exit and unmount.
 */
export function useFullscreenLandscape(
	ref: RefObject<HTMLElement | null>,
): FullscreenLandscape {
	const [isFullscreen, setIsFullscreen] = useState(false);

	useEffect(() => {
		const onChange = () => setIsFullscreen(document.fullscreenElement != null);
		document.addEventListener("fullscreenchange", onChange);
		return () => document.removeEventListener("fullscreenchange", onChange);
	}, []);

	const unlockOrientation = useCallback(() => {
		const orientation = screen.orientation as LockableOrientation | undefined;
		try {
			orientation?.unlock?.();
		} catch {
			/* not supported */
		}
	}, []);

	useEffect(() => unlockOrientation, [unlockOrientation]);

	const enter = useCallback(async () => {
		const el = ref.current;
		if (!el?.requestFullscreen) return;
		try {
			await el.requestFullscreen();
			const orientation = screen.orientation as LockableOrientation | undefined;
			await orientation?.lock?.("landscape").catch(() => {});
		} catch {
			/* user gesture required or unsupported */
		}
	}, [ref]);

	const exit = useCallback(async () => {
		unlockOrientation();
		if (document.fullscreenElement) {
			try {
				await document.exitFullscreen();
			} catch {
				/* already exited */
			}
		}
	}, [unlockOrientation]);

	const toggle = useCallback(() => {
		if (document.fullscreenElement) {
			void exit();
		} else {
			void enter();
		}
	}, [enter, exit]);

	const supported =
		typeof document !== "undefined" &&
		typeof document.documentElement.requestFullscreen === "function";

	return { isFullscreen, supported, toggle };
}
