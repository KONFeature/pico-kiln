import { createFileRoute } from "@tanstack/react-router";
import { TuningAnalyzer } from "@/components/routes/analyze/TuningAnalyzer";

export const Route = createFileRoute("/analyze")({
	component: AnalyzePage,
});

function AnalyzePage() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<TuningAnalyzer />
		</div>
	);
}
