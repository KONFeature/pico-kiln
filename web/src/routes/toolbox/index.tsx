import { createFileRoute } from "@tanstack/react-router";
import { Edit, FolderOpen, LineChart, Wrench } from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { FileManager } from "./components/FileManager";
import { ProfileEditor } from "./components/ProfileEditor";
import { Visualizer } from "./components/Visualizer";

export const Route = createFileRoute("/toolbox/")({
	component: ToolboxPage,
});

function ToolboxPage() {
	return (
		<div className="container max-w-7xl mx-auto py-8 px-4 space-y-6">
			<div className="flex items-center gap-3">
				<Wrench className="w-8 h-8 text-green-500" />
				<h1 className="text-3xl font-bold">Toolbox</h1>
			</div>

			<p className="text-muted-foreground">
				Visualize profiles and runs, analyze tuning data, and create custom
				firing profiles. Upload files or load them directly from your Pico when
				it's IDLE.
			</p>

			<Tabs defaultValue="visualizer" className="w-full">
				<TabsList className="grid w-full grid-cols-3">
					<TabsTrigger value="visualizer" className="flex items-center gap-2">
						<LineChart className="w-4 h-4" />
						Visualizer
					</TabsTrigger>
					<TabsTrigger value="editor" className="flex items-center gap-2">
						<Edit className="w-4 h-4" />
						Profile Editor
					</TabsTrigger>
					<TabsTrigger value="files" className="flex items-center gap-2">
						<FolderOpen className="w-4 h-4" />
						Files
					</TabsTrigger>
				</TabsList>

				<TabsContent value="visualizer" className="mt-6">
					<Visualizer />
				</TabsContent>

				<TabsContent value="editor" className="mt-6">
					<ProfileEditor />
				</TabsContent>

				<TabsContent value="files" className="mt-6">
					<FileManager />
				</TabsContent>
			</Tabs>
		</div>
	);
}
