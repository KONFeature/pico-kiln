// Connection status indicator component

import { Loader2, Wifi, WifiOff } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { usePico } from "@/lib/pico/context";
import { useKilnStatus } from "@/lib/pico/hooks";

export function ConnectionStatus() {
	const { isConfigured } = usePico();
	const { data, error, isLoading } = useKilnStatus({ enabled: isConfigured });

	if (!isConfigured) {
		return (
			<Badge variant="outline" className="flex items-center gap-2">
				<WifiOff className="w-3 h-3" />
				<span>Not Connected</span>
			</Badge>
		);
	}

	if (isLoading && !data) {
		return (
			<Badge variant="outline" className="flex items-center gap-2">
				<Loader2 className="w-3 h-3 animate-spin" />
				<span>Connecting...</span>
			</Badge>
		);
	}

	if (!error && data) {
		return (
			<Badge
				variant="default"
				className="flex items-center gap-2 bg-green-600 hover:bg-green-700"
			>
				<Wifi className="w-3 h-3" />
				<span>Connected</span>
			</Badge>
		);
	}

	if (error) {
		return (
			<Badge variant="destructive" className="flex items-center gap-2">
				<WifiOff className="w-3 h-3" />
				<span>Connection Error</span>
			</Badge>
		);
	}

	return (
		<Badge variant="outline" className="flex items-center gap-2">
			<WifiOff className="w-3 h-3" />
			<span>Unknown</span>
		</Badge>
	);
}

export function ConnectionStatusDetailed() {
	const { isConfigured, picoURL } = usePico();
	const { error, dataUpdatedAt, failureCount } = useKilnStatus({
		enabled: isConfigured,
	});

	return (
		<div className="space-y-2">
			<div className="flex items-center justify-between">
				<span className="text-sm font-medium">Connection Status:</span>
				<ConnectionStatus />
			</div>

			{isConfigured && (
				<div className="text-xs text-muted-foreground space-y-1">
					<div>URL: {picoURL}</div>
					{dataUpdatedAt && (
						<div>
							Last success: {new Date(dataUpdatedAt).toLocaleTimeString()}
						</div>
					)}
					{failureCount > 0 && (
						<div className="text-destructive">
							Failed attempts: {failureCount}
						</div>
					)}
					{error && (
						<div className="text-destructive">Error: {error.message}</div>
					)}
				</div>
			)}
		</div>
	);
}
