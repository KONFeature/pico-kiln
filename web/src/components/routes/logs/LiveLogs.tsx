import { CircleAlert, Download, Pause, Play, RefreshCw } from "lucide-react";
import { useMemo, useState } from "react";
import { ErrorAlert } from "@/components/ErrorAlert";
import { LogView } from "@/components/logs/LogView";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { parseDiagText } from "@/lib/diag-parser";
import { usePico } from "@/lib/pico/context";
import { useLiveLogs } from "@/lib/pico/hooks";

export function LiveLogs() {
	const [paused, setPaused] = useState(false);
	const [intervalMs, setIntervalMs] = useState(3000);
	const { isConfigured } = usePico();
	const { data, error, isFetching, refetch } = useLiveLogs({
		paused,
		intervalMs,
	});

	const entries = useMemo(() => parseDiagText(data ?? ""), [data]);

	const handleDownload = () => {
		if (!data) return;
		const blob = new Blob([data], { type: "text/plain" });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = "live-logs.txt";
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>Live Logs</CardTitle>
				<CardDescription>
					Live tail of the firmware RAM log ring. Updates while the kiln runs.
					For archived diagnostics, use the Diag views in Files or the
					Visualizer.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{!isConfigured && (
					<Alert>
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>
							Please configure Pico connection to view logs
						</AlertDescription>
					</Alert>
				)}

				{isConfigured && (
					<>
						<div className="flex flex-wrap gap-2 items-center">
							<Button
								variant="outline"
								size="sm"
								onClick={() => setPaused((prev) => !prev)}
							>
								{paused ? (
									<>
										<Play className="w-4 h-4 mr-2 flex-shrink-0" />
										<span className="truncate">Resume</span>
									</>
								) : (
									<>
										<Pause className="w-4 h-4 mr-2 flex-shrink-0" />
										<span className="truncate">Pause</span>
									</>
								)}
							</Button>

							<div className="flex items-center gap-2">
								<span className="text-sm text-muted-foreground">Refresh</span>
								<Select
									value={String(intervalMs)}
									onValueChange={(v) => setIntervalMs(Number(v))}
								>
									<SelectTrigger size="sm" className="w-20">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="2000">2s</SelectItem>
										<SelectItem value="3000">3s</SelectItem>
										<SelectItem value="5000">5s</SelectItem>
										<SelectItem value="10000">10s</SelectItem>
									</SelectContent>
								</Select>
							</div>

							<Button variant="outline" size="sm" onClick={() => refetch()}>
								<RefreshCw
									className={`w-4 h-4 mr-2 flex-shrink-0 ${isFetching ? "animate-spin" : ""}`}
								/>
								<span className="truncate">Refresh</span>
							</Button>

							<Button
								variant="outline"
								size="sm"
								onClick={handleDownload}
								disabled={!data}
							>
								<Download className="w-4 h-4 mr-2 flex-shrink-0" />
								<span className="truncate">Download</span>
							</Button>
						</div>

						{error && <ErrorAlert error={error} />}

						<LogView
							entries={entries}
							autoScroll
							emptyMessage="No log lines yet."
						/>
					</>
				)}
			</CardContent>
		</Card>
	);
}
