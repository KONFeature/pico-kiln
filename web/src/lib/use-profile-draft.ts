import { useCallback, useEffect, useState } from "react";
import type { Profile } from "./pico/types";
import { validateProfile } from "./profile-schema";

const DRAFT_KEY = "pico-kiln-editor-draft";

function loadDraft(): Profile | null {
	if (typeof window === "undefined") {
		return null;
	}
	try {
		const raw = localStorage.getItem(DRAFT_KEY);
		if (!raw) {
			return null;
		}
		const result = validateProfile(JSON.parse(raw));
		return result.ok ? result.profile : null;
	} catch {
		return null;
	}
}

/**
 * Editor draft backed by localStorage so unsaved work survives tab switches
 * (Radix unmounts inactive tab content), reloads, back-navigation and
 * pull-to-refresh — the core data-loss fix for the profile editor.
 */
export function useProfileDraft(fallback: Profile) {
	const [profile, setProfile] = useState<Profile>(
		() => loadDraft() ?? fallback,
	);

	useEffect(() => {
		try {
			localStorage.setItem(DRAFT_KEY, JSON.stringify(profile));
		} catch {
			// localStorage unavailable/full — editing still works in-memory.
		}
	}, [profile]);

	const clearDraft = useCallback(() => {
		try {
			localStorage.removeItem(DRAFT_KEY);
		} catch {
			// Nothing to recover from; the next save will overwrite anyway.
		}
	}, []);

	return { profile, setProfile, clearDraft };
}
