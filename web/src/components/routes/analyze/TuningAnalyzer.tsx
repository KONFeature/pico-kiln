import { CircleAlert, Sparkles } from "lucide-react";
import { type ReactNode, useCallback, useState } from "react";
import { RebootDialog } from "@/components/routes/config/RebootDialog";
import { FileSourceSelector } from "@/components/routes/toolbox/FileSourceSelector";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { usePico } from "@/lib/pico/context";
import { useKilnStatus, useSaveConfig } from "@/lib/pico/hooks";
import {
	analyzeTuning,
	configPatchForMethod,
	type PidMethodKey,
	type TuningAnalysis,
} from "@/lib/tuning-analysis";

const METHOD_ORDER: PidMethodKey[] = ["amigo", "zieglerNichols", "cohenCoon"];

const QUALITY_INSIGHT: Record<TuningAnalysis["testQuality"], string> = {
	EXCELLENT: "Test quality is excellent — high confidence in these parameters.",
	GOOD: "Test quality is good — these should work well. For even better tuning, run a longer test covering a wider temperature range.",
	POOR: "Test quality is poor — parameters may need manual adjustment. Consider a longer test with a wider temperature range before trusting them.",
};

function qualityVariant(
	q: TuningAnalysis["testQuality"],
): "default" | "secondary" | "destructive" {
	if (q === "EXCELLENT") return "default";
	if (q === "GOOD") return "secondary";
	return "destructive";
}

export function TuningAnalyzer() {
	const [analysis, setAnalysis] = useState<TuningAnalysis | null>(null);
	const [filename, setFilename] = useState<string>("");
	const [method, setMethod] = useState<PidMethodKey>("amigo");
	const [error, setError] = useState<string | null>(null);
	const [confirmOpen, setConfirmOpen] = useState(false);
	const [rebootOpen, setRebootOpen] = useState(false);

	const { isConfigured } = usePico();
	const { data: status } = useKilnStatus();
	const save = useSaveConfig();

	const firing = status?.state === "RUNNING" || status?.state === "TUNING";
	const canApply = isConfigured && !firing;

	const handleFileSelected = useCallback((content: string, name: string) => {
		try {
			const result = analyzeTuning(content);
			if (result.phases.length === 0) {
				throw new Error(
					"No usable phases detected — is this a tuning/firing log?",
				);
			}
			setAnalysis(result);
			setMethod(result.recommended);
			setFilename(name);
			setError(null);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to analyze log");
			setAnalysis(null);
		}
	}, []);

	const patch = analysis ? configPatchForMethod(analysis, method) : null;

	const applyNow = async () => {
		if (!patch) return;
		await save.mutateAsync(patch as Record<string, unknown>);
		setConfirmOpen(false);
		setRebootOpen(true);
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>Analyze Tuning Run</CardTitle>
				<CardDescription>
					Load a tuning log to fit a thermal model and get suggested PID values
					— then apply them straight to the kiln. No laptop required.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-6">
				<FileSourceSelector
					accept=".csv"
					description="Choose a tuning log to analyze"
					directory="logs"
					label="Select Tuning Log"
					onFileSelected={handleFileSelected}
				/>

				{error && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				)}

				{analysis && patch && (
					<div className="space-y-6 border-t pt-6">
						{/* Summary */}
						<div className="flex flex-wrap items-center gap-3">
							<h3 className="font-semibold text-lg">
								{filename || "Tuning run"}
							</h3>
							<Badge variant={qualityVariant(analysis.testQuality)}>
								{analysis.testQuality}
							</Badge>
						</div>
						<p className="text-muted-foreground text-sm">
							{QUALITY_INSIGHT[analysis.testQuality]}
						</p>

						{/* Test info + thermal model */}
						<div className="grid gap-4 sm:grid-cols-2">
							<InfoCard title="Test">
								<Row
									label="Duration"
									value={`${(analysis.testInfo.durationS / 60).toFixed(1)} min`}
								/>
								<Row
									label="Data points"
									value={analysis.testInfo.dataPoints.toLocaleString()}
								/>
								<Row
									label="Temperature"
									value={`${analysis.testInfo.tempMin}–${analysis.testInfo.tempMax} °C`}
								/>
								<Row
									label="Phases detected"
									value={String(analysis.testInfo.phasesDetected)}
								/>
							</InfoCard>

							<InfoCard title="Thermal model">
								<Row
									label="Dead time (L)"
									value={`${analysis.thermalModel.deadTimeS} s`}
								/>
								<Row
									label="Time constant (τ)"
									value={`${analysis.thermalModel.timeConstantS} s`}
								/>
								<Row
									label="Base gain (K)"
									value={`${analysis.thermalModel.steadyStateGain} °C/% (${analysis.thermalModel.gainMethod}, ${analysis.thermalModel.gainConfidence})`}
								/>
								<Row
									label="Heat-loss (h)"
									value={`${analysis.thermalModel.heatLossCoeff} (${analysis.thermalModel.heatLossMethod})`}
								/>
								<Row
									label="Ambient"
									value={`${analysis.thermalModel.ambientTemp} °C`}
								/>
							</InfoCard>
						</div>

						{/* PID methods */}
						<div className="space-y-3">
							<h4 className="font-medium text-sm">
								Suggested PID — pick a method to apply
							</h4>
							<div className="grid gap-3 sm:grid-cols-3">
								{METHOD_ORDER.map((key) => {
									const m = analysis.pidMethods[key];
									const selected = method === key;
									const recommended = analysis.recommended === key;
									return (
										<button
											type="button"
											key={key}
											onClick={() => setMethod(key)}
											className={`rounded-lg border p-3 text-left transition-colors ${
												selected
													? "border-primary bg-primary/5 ring-1 ring-primary"
													: "hover:bg-accent"
											}`}
										>
											<div className="flex items-center justify-between gap-2">
												<span className="font-medium text-sm">{m.method}</span>
												{recommended && (
													<Badge variant="default" className="gap-1">
														<Sparkles className="h-3 w-3" /> Rec.
													</Badge>
												)}
											</div>
											<dl className="mt-2 space-y-0.5 font-mono text-xs">
												<Row label="Kp" value={m.kp} />
												<Row label="Ki" value={m.ki} />
												<Row label="Kd" value={m.kd} />
											</dl>
											<p className="mt-2 text-[11px] text-muted-foreground leading-snug">
												{m.characteristics}
											</p>
										</button>
									);
								})}
							</div>
						</div>

						{/* Gain scheduling table */}
						{analysis.thermalModel.gainVsTemp.length > 0 && (
							<InfoCard title="Gain vs temperature (heat-loss scaling)">
								<table className="w-full text-sm">
									<thead className="text-muted-foreground text-xs">
										<tr>
											<th className="text-left font-normal">Temp °C</th>
											<th className="text-right font-normal">Eff. gain</th>
											<th className="text-right font-normal">SSR %</th>
										</tr>
									</thead>
									<tbody className="font-mono">
										{analysis.thermalModel.gainVsTemp.map((g) => (
											<tr key={g.temp}>
												<td>{g.temp}</td>
												<td className="text-right">{g.gain}</td>
												<td className="text-right">{g.ssr}</td>
											</tr>
										))}
									</tbody>
								</table>
							</InfoCard>
						)}

						{/* Apply */}
						<div className="space-y-3 rounded-lg border bg-muted/30 p-4">
							<h4 className="font-medium text-sm">Apply to kiln</h4>
							<p className="text-muted-foreground text-xs">
								Writes these 5 values to the kiln config (gain scheduling
								included). Takes effect after a reboot.
							</p>
							<dl className="grid grid-cols-2 gap-x-6 gap-y-1 font-mono text-xs sm:grid-cols-3">
								{Object.entries(patch).map(([k, v]) => (
									<Row key={k} label={k} value={v as number} />
								))}
							</dl>
							{!isConfigured && (
								<p className="text-amber-600 text-xs dark:text-amber-500">
									Connect to a kiln to apply.
								</p>
							)}
							{firing && (
								<p className="text-amber-600 text-xs dark:text-amber-500">
									Kiln is firing — config is locked until it finishes.
								</p>
							)}
							{save.isError && (
								<p className="text-destructive text-xs">
									Save failed: {save.error?.message}
								</p>
							)}
							<Button disabled={!canApply} onClick={() => setConfirmOpen(true)}>
								Apply {analysis.pidMethods[method].method}
							</Button>
						</div>
					</div>
				)}
			</CardContent>

			{/* Confirm dialog */}
			<Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Apply PID values?</DialogTitle>
						<DialogDescription>
							This overwrites the kiln's PID and gain-scheduling config with the{" "}
							{analysis?.pidMethods[method].method} values. The change is saved
							to flash and takes effect after a reboot.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setConfirmOpen(false)}
							disabled={save.isPending}
						>
							Cancel
						</Button>
						<Button onClick={applyNow} disabled={save.isPending}>
							{save.isPending ? "Applying…" : "Apply"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<RebootDialog open={rebootOpen} onOpenChange={setRebootOpen} />
		</Card>
	);
}

function InfoCard({ title, children }: { title: string; children: ReactNode }) {
	return (
		<div className="rounded-lg border p-4">
			<h4 className="mb-2 font-medium text-sm">{title}</h4>
			<dl className="space-y-1">{children}</dl>
		</div>
	);
}

function Row({ label, value }: { label: string; value: string | number }) {
	return (
		<div className="flex items-center justify-between gap-3">
			<dt className="text-muted-foreground text-xs">{label}</dt>
			<dd className="text-sm">{value}</dd>
		</div>
	);
}
