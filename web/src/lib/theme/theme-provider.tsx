// Theme provider: light / dark / system with localStorage persistence.
// Applies the `.dark` class and `color-scheme` to <html> so Tailwind's
// `dark:` variant and native form controls follow the active theme.

import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";

export type Theme = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "pico-kiln-theme";

interface ThemeContextValue {
	/** User preference: light, dark, or follow the system setting. */
	theme: Theme;
	setTheme: (theme: Theme) => void;
	/** The theme actually applied right now (system resolved to light/dark). */
	resolvedTheme: ResolvedTheme;
}

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function getStoredTheme(): Theme {
	if (typeof window === "undefined") return "system";
	const stored = localStorage.getItem(STORAGE_KEY);
	if (stored === "light" || stored === "dark" || stored === "system") {
		return stored;
	}
	return "system";
}

function getSystemTheme(): ResolvedTheme {
	if (typeof window === "undefined") return "light";
	return window.matchMedia("(prefers-color-scheme: dark)").matches
		? "dark"
		: "light";
}

function applyTheme(resolved: ResolvedTheme) {
	if (typeof document === "undefined") return;
	const root = document.documentElement;
	root.classList.toggle("dark", resolved === "dark");
	root.style.colorScheme = resolved;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
	const [theme, setThemeState] = useState<Theme>(getStoredTheme);
	const [systemTheme, setSystemTheme] = useState<ResolvedTheme>(getSystemTheme);

	// Track the system preference while the user is on "system".
	useEffect(() => {
		const media = window.matchMedia("(prefers-color-scheme: dark)");
		const onChange = (e: MediaQueryListEvent) => {
			setSystemTheme(e.matches ? "dark" : "light");
		};
		media.addEventListener("change", onChange);
		return () => media.removeEventListener("change", onChange);
	}, []);

	const resolvedTheme: ResolvedTheme = theme === "system" ? systemTheme : theme;

	// Apply to <html> whenever the resolved theme changes.
	useEffect(() => {
		applyTheme(resolvedTheme);
	}, [resolvedTheme]);

	const setTheme = useCallback((next: Theme) => {
		setThemeState(next);
		if (typeof window !== "undefined") {
			localStorage.setItem(STORAGE_KEY, next);
		}
	}, []);

	const value = useMemo<ThemeContextValue>(
		() => ({ theme, setTheme, resolvedTheme }),
		[theme, setTheme, resolvedTheme],
	);

	return (
		<ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
	);
}

export function useTheme(): ThemeContextValue {
	const context = useContext(ThemeContext);
	if (!context) {
		throw new Error("useTheme must be used within a ThemeProvider");
	}
	return context;
}
