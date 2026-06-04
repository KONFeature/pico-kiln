import { Link } from "@tanstack/react-router";
import {
	Edit,
	Flame,
	FolderOpen,
	LineChart,
	Menu,
	Settings,
	X,
} from "lucide-react";
import { useState } from "react";
import { useConfigDraft } from "@/lib/config/draft-context";
import { ConnectionStatus } from "./ConnectionStatus";
import { PicoConnectionDialog } from "./PicoConnectionDialog";
import { ThemeToggle } from "./ThemeToggle";

const navLinkClass =
	"flex items-center gap-3 p-3 rounded-lg hover:bg-sidebar-accent hover:text-sidebar-accent-foreground transition-colors mb-2";
const navLinkActiveClass =
	"flex items-center gap-3 p-3 rounded-lg bg-sidebar-primary text-sidebar-primary-foreground hover:bg-sidebar-primary/90 transition-colors mb-2";

export default function Header() {
	const [isOpen, setIsOpen] = useState(false);
	const [isSettingsOpen, setIsSettingsOpen] = useState(false);
	const closeMenu = () => setIsOpen(false);
	const { isDirty } = useConfigDraft();

	return (
		<>
			<header className="p-4 flex items-center justify-between bg-card text-card-foreground border-b border-border shadow-sm safe-top">
				<div className="flex items-center">
					<button
						onClick={() => setIsOpen(true)}
						className="touch-target p-2 hover:bg-accent hover:text-accent-foreground rounded-lg transition-colors"
						aria-label="Open menu"
					>
						<span className="relative">
							<Menu size={24} />
							{isDirty && (
								<span className="absolute -right-1 -top-1 size-2.5 rounded-full bg-primary ring-2 ring-card" />
							)}
						</span>
					</button>
					<h1 className="ml-4 text-xl font-semibold">
						<Link to="/" className="flex items-center gap-2">
							<Flame className="w-6 h-6 text-chart-ssr" />
							<span>Pico Kiln</span>
						</Link>
					</h1>
				</div>
				<div className="flex items-center gap-2">
					<ConnectionStatus />
					<ThemeToggle />
					<button
						onClick={() => setIsSettingsOpen(true)}
						className="touch-target p-2 hover:bg-accent hover:text-accent-foreground rounded-lg transition-colors"
						aria-label="Settings"
					>
						<Settings size={20} />
					</button>
				</div>
			</header>

			<PicoConnectionDialog
				open={isSettingsOpen}
				onOpenChange={setIsSettingsOpen}
			/>

			<aside
				className={`fixed top-0 left-0 h-full w-80 bg-sidebar text-sidebar-foreground border-r border-sidebar-border shadow-2xl z-50 transform transition-transform duration-300 ease-in-out flex flex-col ${
					isOpen ? "translate-x-0" : "-translate-x-full"
				}`}
			>
				<div className="flex items-center justify-between p-4 border-b border-sidebar-border">
					<h2 className="text-xl font-bold">Navigation</h2>
					<button
						onClick={() => setIsOpen(false)}
						className="p-2 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground rounded-lg transition-colors"
						aria-label="Close menu"
					>
						<X size={24} />
					</button>
				</div>

				<nav className="flex-1 p-4 overflow-y-auto">
					<Link
						to="/"
						onClick={closeMenu}
						className={navLinkClass}
						activeProps={{ className: navLinkActiveClass }}
					>
						<Flame size={20} />
						<span className="font-medium">Kiln Control</span>
					</Link>

					<Link
						to="/visualizer"
						onClick={closeMenu}
						className={navLinkClass}
						activeProps={{ className: navLinkActiveClass }}
					>
						<LineChart size={20} />
						<span className="font-medium">Visualizer</span>
					</Link>

					<Link
						to="/editor"
						onClick={closeMenu}
						className={navLinkClass}
						activeProps={{ className: navLinkActiveClass }}
					>
						<Edit size={20} />
						<span className="font-medium">Profile Editor</span>
					</Link>

					<Link
						to="/files"
						onClick={closeMenu}
						className={navLinkClass}
						activeProps={{ className: navLinkActiveClass }}
					>
						<FolderOpen size={20} />
						<span className="font-medium">Files</span>
					</Link>

					<Link
						to="/config"
						onClick={closeMenu}
						className={navLinkClass}
						activeProps={{ className: navLinkActiveClass }}
					>
						<Settings size={20} />
						<span className="font-medium">Configuration</span>
						{isDirty && (
							<span className="ml-auto size-2.5 rounded-full bg-primary" />
						)}
					</Link>
				</nav>
			</aside>
		</>
	);
}
