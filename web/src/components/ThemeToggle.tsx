import { Monitor, Moon, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import { type Theme, useTheme } from "@/lib/theme/theme-provider";

// Cycle order: light -> dark -> system -> light
const NEXT_THEME: Record<Theme, Theme> = {
	light: "dark",
	dark: "system",
	system: "light",
};

const THEME_LABEL: Record<Theme, string> = {
	light: "Light",
	dark: "Dark",
	system: "System",
};

export function ThemeToggle() {
	const { theme, setTheme } = useTheme();

	const Icon = theme === "light" ? Sun : theme === "dark" ? Moon : Monitor;
	const label = `Theme: ${THEME_LABEL[theme]} (tap to switch)`;

	return (
		<Button
			variant="ghost"
			size="icon"
			className="touch-target"
			onClick={() => setTheme(NEXT_THEME[theme])}
			aria-label={label}
			title={label}
		>
			<Icon className="w-5 h-5" />
		</Button>
	);
}
