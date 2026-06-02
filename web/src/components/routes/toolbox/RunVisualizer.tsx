import { CircleAlert } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	detectRunType,
	type LogDataPoint,
	parseLogCSV,
} from "@/lib/csv-parser";
import { computeRunStats } from "@/lib/run-stats";
import { FileSourceSelector } from "./FileSourceSelector";
import { TrackingKpis } from "./run-chart/tracking-kpis";
import { UnifiedRunChart } from "./run-chart/unified-run-chart";

export function RunVisualizer() {
	const [logData, setLogData] = useState<LogDataPoint[] | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [runType, setRunType] = useState<"TUNING" | "FIRING">("FIRING");
	const [loadId, setLoadId] = useState(0);

	const handleFileSelected = useCallback((content: string) => {
		try {
			const { data } = parseLogCSV(content);
			if (data.length === 0) {
				throw new Error("No valid data points in log file");
			}
			setLogData(data);
			setRunType(detectRunType(data));
			setLoadId((n) => n + 1);
			setError(null);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to parse log file");
			setLogData(null);
		}
	}, []);

	const stats = useMemo(
		() => (logData ? computeRunStats(logData) : null),
		[logData],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle>Run Visualizer</CardTitle>
				<CardDescription>
					Overlay temperature, target, SSR output and heating rate on one chart
					— zoom into any moment, go fullscreen, and read the tracking quality.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					accept=".csv"
					description="Choose a log file to visualize"
					directory="logs"
					label="Select Log File"
					onFileSelected={handleFileSelected}
				/>

				{error && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{logData && stats && (
					<div className="space-y-5 border-t pt-6">
						<div>
							<h3 className="font-semibold text-lg">Kiln {runType} run</h3>
							<p className="mt-1 text-muted-foreground text-sm">
								{logData[0]?.timestamp
									? `Started ${logData[0].timestamp} · `
									: ""}
								{stats.holdCount} hold{stats.holdCount === 1 ? "" : "s"}{" "}
								detected
							</p>
						</div>

						<TrackingKpis stats={stats} />

						<UnifiedRunChart key={loadId} logData={logData} />
					</div>
				)}
			</CardContent>
		</Card>
	);
}
