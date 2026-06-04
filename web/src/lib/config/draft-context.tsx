import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { usePico } from "@/lib/pico/context";
import type { ConfigValue } from "@/lib/pico/types";

type Draft = Record<string, ConfigValue>;

interface ConfigDraftValue {
	/** Changed-keys-only diff, persisted to sessionStorage per kiln URL. */
	draft: Draft;
	isDirty: boolean;
	/** Replace the whole draft (ConfigForm pushes the computed diff here). */
	setDraft: (draft: Draft) => void;
	/** Clear the draft (after a successful save or a discard). */
	clearDraft: () => void;
}

const ConfigDraftContext = createContext<ConfigDraftValue | undefined>(
	undefined,
);

const KEY_PREFIX = "pico-kiln-config-draft:";

function storageKey(url: string): string {
	return `${KEY_PREFIX}${url}`;
}

function loadDraft(url: string): Draft {
	if (typeof window === "undefined" || !url) return {};
	try {
		const raw = sessionStorage.getItem(storageKey(url));
		return raw ? (JSON.parse(raw) as Draft) : {};
	} catch {
		return {};
	}
}

export function ConfigDraftProvider({ children }: { children: ReactNode }) {
	const { picoURL } = usePico();
	// Hydrate synchronously on first render so a reload directly on /config
	// restores the draft before the form seeds its defaults.
	const [draft, setDraftState] = useState<Draft>(() => loadDraft(picoURL));
	const lastURL = useRef(picoURL);

	// Reload the draft when the kiln URL changes (different device = different
	// draft). Skips the initial render, which already hydrated synchronously.
	useEffect(() => {
		if (lastURL.current === picoURL) return;
		lastURL.current = picoURL;
		setDraftState(loadDraft(picoURL));
	}, [picoURL]);

	const persist = useCallback(
		(next: Draft) => {
			if (typeof window === "undefined" || !picoURL) return;
			const key = storageKey(picoURL);
			if (Object.keys(next).length === 0) {
				sessionStorage.removeItem(key);
			} else {
				sessionStorage.setItem(key, JSON.stringify(next));
			}
		},
		[picoURL],
	);

	const setDraft = useCallback(
		(next: Draft) => {
			setDraftState(next);
			persist(next);
		},
		[persist],
	);

	const clearDraft = useCallback(() => {
		setDraftState({});
		persist({});
	}, [persist]);

	const value = useMemo<ConfigDraftValue>(
		() => ({
			draft,
			isDirty: Object.keys(draft).length > 0,
			setDraft,
			clearDraft,
		}),
		[draft, setDraft, clearDraft],
	);

	return (
		<ConfigDraftContext.Provider value={value}>
			{children}
		</ConfigDraftContext.Provider>
	);
}

export function useConfigDraft(): ConfigDraftValue {
	const ctx = useContext(ConfigDraftContext);
	if (!ctx) {
		throw new Error("useConfigDraft must be used within a ConfigDraftProvider");
	}
	return ctx;
}
