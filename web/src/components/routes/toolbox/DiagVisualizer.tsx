import { useCallback, useState } from "react";
import { LogView } from "@/components/logs/LogView";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { type LogEntry, parseDiagText } from "@/lib/diag-parser";
import { FileSourceSelector } from "./FileSourceSelector";

export function DiagVisualizer() {
	const [entries, setEntries] = useState<LogEntry[] | null>(null);
	const [filename, setFilename] = useState<string>("");

	const handleFileSelected = useCallback((content: string, name: string) => {
		setEntries(parseDiagText(content));
		setFilename(name);
	}, []);

	return (
		<Card>
			<CardHeader>
				<CardTitle>Diag Log Viewer</CardTitle>
				<CardDescription>
					Inspect a firmware diagnostic log — filter by level/tag and search.
					Load from the kiln (diag directory, requires IDLE) or upload a .log
					file.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					accept=".log"
					description="Choose a diagnostic log file to inspect"
					directory="diag"
					label="Select Diag Log"
					onFileSelected={handleFileSelected}
				/>

				{entries && (
					<div className="space-y-4 border-t pt-6">
						<div className="text-sm text-muted-foreground">
							{filename ? `${filename} · ` : ""}
							{entries.length} lines
						</div>
						<LogView
							entries={entries}
							emptyMessage="This diag file has no log lines."
						/>
					</div>
				)}
			</CardContent>
		</Card>
	);
}
