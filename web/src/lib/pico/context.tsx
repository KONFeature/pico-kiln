// Context for managing Pico connection state and API client

import React, {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useState,
} from "react";
import { PicoAPIClient } from "./client";

const STORAGE_KEY = "pico-kiln-url";
const DEFAULT_URL = "";

interface PicoContextValue {
	// Connection state
	picoURL: string;
	setPicoURL: (url: string) => void;
	isConfigured: boolean;

	// API client
	client: PicoAPIClient | null;

	// Reset
	reset: () => void;
}

const PicoContext = createContext<PicoContextValue | undefined>(undefined);

interface PicoProviderProps {
	children: ReactNode;
}

export function PicoProvider({ children }: PicoProviderProps) {
	const [picoURL, setPicoURLState] = useState<string>(() => {
		// Load from localStorage on mount (only on client side)
		if (typeof window !== "undefined") {
			return localStorage.getItem(STORAGE_KEY) || DEFAULT_URL;
		}
		return DEFAULT_URL;
	});

	const [client, setClient] = useState<PicoAPIClient | null>(() => {
		const savedURL =
			typeof window !== "undefined"
				? localStorage.getItem(STORAGE_KEY) || DEFAULT_URL
				: DEFAULT_URL;
		return savedURL ? new PicoAPIClient(savedURL) : null;
	});

	// Update localStorage when URL changes
	const setPicoURL = useCallback((url: string) => {
		const trimmedURL = url.trim();
		setPicoURLState(trimmedURL);

		if (typeof window !== "undefined") {
			if (trimmedURL) {
				localStorage.setItem(STORAGE_KEY, trimmedURL);
			} else {
				localStorage.removeItem(STORAGE_KEY);
			}
		}

		// Update or create client
		if (trimmedURL) {
			setClient(new PicoAPIClient(trimmedURL));
		} else {
			setClient(null);
		}
	}, []);

	// Reset all state
	const reset = useCallback(() => {
		setPicoURL("");
	}, [setPicoURL]);

	const value: PicoContextValue = {
		picoURL,
		setPicoURL,
		isConfigured: Boolean(picoURL),
		client,
		reset,
	};

	return <PicoContext.Provider value={value}>{children}</PicoContext.Provider>;
}

export function usePico(): PicoContextValue {
	const context = useContext(PicoContext);
	if (!context) {
		throw new Error("usePico must be used within a PicoProvider");
	}
	return context;
}
