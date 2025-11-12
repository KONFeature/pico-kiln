// Component to require Pico connection for protected pages

import { WifiOff } from "lucide-react";
import type { ReactNode } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { usePico } from "@/lib/pico/context";
import { PicoConnectionConfig } from "./PicoConnectionConfig";

interface RequireConnectionProps {
	children: ReactNode;
}

export function RequireConnection({ children }: RequireConnectionProps) {
	const { isConfigured } = usePico();

	if (!isConfigured) {
		return (
			<div className="container max-w-2xl mx-auto py-8 px-4 space-y-6">
				<Alert>
					<WifiOff className="w-4 h-4" />
					<AlertTitle>Connection Required</AlertTitle>
					<AlertDescription>
						Please configure your Pico kiln connection to access this page.
					</AlertDescription>
				</Alert>

				<PicoConnectionConfig />
			</div>
		);
	}

	return <>{children}</>;
}
