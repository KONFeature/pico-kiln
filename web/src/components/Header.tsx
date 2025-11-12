import { Link } from "@tanstack/react-router";
import { Flame, Menu, Settings, Wrench, X } from "lucide-react";
import { useState } from "react";
import { ConnectionStatus } from "./ConnectionStatus";
import { PicoConnectionDialog } from "./PicoConnectionDialog";

export default function Header() {
	const [isOpen, setIsOpen] = useState(false);
	const [isSettingsOpen, setIsSettingsOpen] = useState(false);

	return (
		<>
			<header className="p-4 flex items-center justify-between bg-gray-800 text-white shadow-lg">
				<div className="flex items-center">
					<button
						onClick={() => setIsOpen(true)}
						className="p-2 hover:bg-gray-700 rounded-lg transition-colors"
						aria-label="Open menu"
					>
						<Menu size={24} />
					</button>
					<h1 className="ml-4 text-xl font-semibold">
						<Link to="/" className="flex items-center gap-2">
							<Flame className="w-6 h-6 text-orange-500" />
							<span>Pico Kiln</span>
						</Link>
					</h1>
				</div>
				<div className="flex items-center gap-2">
					<ConnectionStatus />
					<button
						onClick={() => setIsSettingsOpen(true)}
						className="p-2 hover:bg-gray-700 rounded-lg transition-colors"
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
				className={`fixed top-0 left-0 h-full w-80 bg-gray-900 text-white shadow-2xl z-50 transform transition-transform duration-300 ease-in-out flex flex-col ${
					isOpen ? "translate-x-0" : "-translate-x-full"
				}`}
			>
				<div className="flex items-center justify-between p-4 border-b border-gray-700">
					<h2 className="text-xl font-bold">Navigation</h2>
					<button
						onClick={() => setIsOpen(false)}
						className="p-2 hover:bg-gray-800 rounded-lg transition-colors"
						aria-label="Close menu"
					>
						<X size={24} />
					</button>
				</div>

				<nav className="flex-1 p-4 overflow-y-auto">
					<Link
						to="/"
						onClick={() => setIsOpen(false)}
						className="flex items-center gap-3 p-3 rounded-lg hover:bg-gray-800 transition-colors mb-2"
						activeProps={{
							className:
								"flex items-center gap-3 p-3 rounded-lg bg-orange-600 hover:bg-orange-700 transition-colors mb-2",
						}}
					>
						<Flame size={20} />
						<span className="font-medium">Kiln Control</span>
					</Link>

					<Link
						to="/toolbox"
						onClick={() => setIsOpen(false)}
						className="flex items-center gap-3 p-3 rounded-lg hover:bg-gray-800 transition-colors mb-2"
						activeProps={{
							className:
								"flex items-center gap-3 p-3 rounded-lg bg-orange-600 hover:bg-orange-700 transition-colors mb-2",
						}}
					>
						<Wrench size={20} />
						<span className="font-medium">Toolbox</span>
					</Link>
				</nav>
			</aside>
		</>
	);
}
