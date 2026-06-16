import { createFileRoute } from "@tanstack/react-router";
import { LiveLogs } from "@/components/routes/logs/LiveLogs";

export const Route = createFileRoute("/logs")({
	component: LogsPage,
});

function LogsPage() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<LiveLogs />
		</div>
	);
}
