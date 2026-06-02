import { createFileRoute } from "@tanstack/react-router";
import { Visualizer } from "@/components/routes/toolbox/Visualizer";

export const Route = createFileRoute("/visualizer")({
	component: VisualizerPage,
});

function VisualizerPage() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<Visualizer />
		</div>
	);
}
