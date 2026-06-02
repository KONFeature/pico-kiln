import { createFileRoute } from "@tanstack/react-router";
import { ProfileEditor } from "@/components/routes/toolbox/ProfileEditor";

export const Route = createFileRoute("/editor")({
	component: EditorPage,
});

function EditorPage() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<ProfileEditor />
		</div>
	);
}
