import { useQuery } from "@tanstack/react-query";
import { ChevronDown, ChevronUp, HardDrive, Upload } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Label } from "@/components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { usePico } from "@/lib/pico/context";
import type { FileDirectory } from "@/lib/pico/types";
import { readFileAsText } from "@/lib/utils";

type SourceMode = "upload" | "pico";

interface FileSourceSelectorProps {
	directory: FileDirectory;
	accept?: string;
	onFileSelected: (content: string, filename: string) => void;
	label?: string;
	description?: string;
}

export function FileSourceSelector({
	directory,
	accept,
	onFileSelected,
	label,
	description,
}: FileSourceSelectorProps) {
	const [sourceMode, setSourceMode] = useState<SourceMode>("upload");
	const [selectedFile, setSelectedFile] = useState<string>("");
	const [manualContent, setManualContent] = useState<string>("");
	const [showContent, setShowContent] = useState<boolean>(false);
	const { client } = usePico();

	// Query to list files from Pico
	const {
		data: filesData,
		isLoading: isLoadingFiles,
		error: filesError,
	} = useQuery({
		queryKey: ["files", directory],
		queryFn: () => client!.listFiles(directory),
		enabled: sourceMode === "pico" && client !== null,
		retry: 1,
	});

	// Query to get file content from Pico
	const { data: fileContent, isFetching: isFetchingContent } = useQuery({
		queryKey: ["file-content", directory, selectedFile],
		queryFn: () => client!.getFile(directory, selectedFile),
		enabled: sourceMode === "pico" && selectedFile !== "" && client !== null,
		retry: 1,
	});

	// Handle file upload
	const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
		const file = e.target.files?.[0];
		if (!file) return;

		try {
			const content = await readFileAsText(file);
			onFileSelected(content, file.name);
			setManualContent(content);
		} catch (error) {
			console.error("Failed to read file:", error);
		}
	};

	// Handle Pico file selection
	const handlePicoFileSelect = (filename: string) => {
		setSelectedFile(filename);
	};

	// Handle loading file from Pico
	const handleLoadFromPico = () => {
		if (fileContent?.success && fileContent.content) {
			onFileSelected(fileContent.content, fileContent.filename);
		}
	};

	return (
		<div className="space-y-4">
			<div>
				<Label className="text-base">{label || "Select File Source"}</Label>
				{description && (
					<p className="text-sm text-muted-foreground mt-1">{description}</p>
				)}
			</div>

			<div className="flex gap-2">
				<Button
					variant={sourceMode === "upload" ? "default" : "outline"}
					size="sm"
					onClick={() => setSourceMode("upload")}
					className="flex-1"
				>
					<Upload className="w-4 h-4 mr-2" />
					Upload File
				</Button>
				<Button
					variant={sourceMode === "pico" ? "default" : "outline"}
					size="sm"
					onClick={() => setSourceMode("pico")}
					className="flex-1"
				>
					<HardDrive className="w-4 h-4 mr-2" />
					From Pico
				</Button>
			</div>

			{sourceMode === "upload" && (
				<div className="space-y-2">
					<input
						type="file"
						accept={accept}
						onChange={handleFileUpload}
						className="block w-full text-sm text-slate-500
              file:mr-4 file:py-2 file:px-4
              file:rounded-md file:border-0
              file:text-sm file:font-semibold
              file:bg-blue-50 file:text-blue-700
              hover:file:bg-blue-100"
					/>
					<Collapsible open={showContent} onOpenChange={setShowContent}>
						<CollapsibleTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className="w-full justify-between"
							>
								<span>Or paste content directly</span>
								{showContent ? (
									<ChevronUp className="w-4 h-4" />
								) : (
									<ChevronDown className="w-4 h-4" />
								)}
							</Button>
						</CollapsibleTrigger>
						<CollapsibleContent className="mt-2">
							<Textarea
								id="manual-content"
								value={manualContent}
								onChange={(e) => {
									setManualContent(e.target.value);
									if (e.target.value) {
										onFileSelected(e.target.value, "manual-input");
									}
								}}
								placeholder="Paste file content here..."
								className="font-mono text-xs"
								rows={6}
							/>
						</CollapsibleContent>
					</Collapsible>
				</div>
			)}

			{sourceMode === "pico" && (
				<div className="space-y-2">
					{filesError && (
						<div className="text-sm text-destructive">
							Failed to load files from Pico. Make sure the kiln is IDLE.
						</div>
					)}
					{isLoadingFiles && (
						<div className="text-sm text-muted-foreground">
							Loading files from Pico...
						</div>
					)}
					{filesData?.success && filesData.files.length === 0 && (
						<div className="text-sm text-muted-foreground">
							No files found in {directory} directory.
						</div>
					)}
					{filesData?.success && filesData.files.length > 0 && (
						<div className="space-y-2">
							<Label>Select File</Label>
							<Select value={selectedFile} onValueChange={handlePicoFileSelect}>
								<SelectTrigger className="mt-1">
									<SelectValue placeholder="Choose a file..." />
								</SelectTrigger>
								<SelectContent>
									{filesData.files.map((file) => (
										<SelectItem key={file.name} value={file.name}>
											{file.name}
											<span className="text-xs text-muted-foreground ml-2">
												({(file.size / 1024).toFixed(1)} KB)
											</span>
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Button
								onClick={handleLoadFromPico}
								disabled={
									!selectedFile || isFetchingContent || !fileContent?.success
								}
								size="sm"
								className="w-full"
							>
								{isFetchingContent ? "Loading..." : "Load File"}
							</Button>
							{fileContent && !fileContent.success && (
								<div className="text-sm text-destructive">
									{fileContent.error || "Failed to load file content"}
								</div>
							)}
						</div>
					)}
				</div>
			)}
		</div>
	);
}
