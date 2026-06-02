import { createFileRoute } from "@tanstack/react-router";
import { FileManager } from "@/components/routes/toolbox/FileManager";

export const Route = createFileRoute("/files")({
	component: FilesPage,
});

function FilesPage() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<FileManager />
		</div>
	);
}
