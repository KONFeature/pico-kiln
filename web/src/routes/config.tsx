import { createFileRoute } from "@tanstack/react-router";
import { RequireConnection } from "@/components/RequireConnection";
import { ConfigPage } from "@/components/routes/config/ConfigPage";

export const Route = createFileRoute("/config")({
	component: ConfigRoute,
});

function ConfigRoute() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<RequireConnection>
				<ConfigPage />
			</RequireConnection>
		</div>
	);
}
