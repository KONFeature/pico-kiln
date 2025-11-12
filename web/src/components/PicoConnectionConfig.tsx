// Pico connection configuration component

import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { usePico } from "@/lib/pico/context";
import { useTestConnection } from "@/lib/pico/hooks";
import { ConnectionStatusDetailed } from "./ConnectionStatus";

interface PicoConnectionConfigProps {
	onConnected?: () => void;
}

export function PicoConnectionConfig({
	onConnected,
}: PicoConnectionConfigProps) {
	const { picoURL, setPicoURL, isConfigured } = usePico();
	const testConnection = useTestConnection();

	const [inputURL, setInputURL] = useState(picoURL || "http://");

	const handleSave = () => {
		const trimmedURL = inputURL.trim();
		if (trimmedURL && trimmedURL !== "http://" && trimmedURL !== "https://") {
			setPicoURL(trimmedURL);
			testConnection.reset();
		}
	};

	const handleTest = async () => {
		await testConnection.mutateAsync();
	};

	// Call onConnected callback when test succeeds
	useEffect(() => {
		if (testConnection.isSuccess && testConnection.data && onConnected) {
			const timeout = setTimeout(onConnected, 1000);
			return () => clearTimeout(timeout);
		}
	}, [testConnection.isSuccess, testConnection.data, onConnected]);

	const handleReset = () => {
		setPicoURL("");
		setInputURL("http://");
		testConnection.reset();
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>Pico Kiln Connection</CardTitle>
				<CardDescription>
					Enter the IP address and port of your Pico kiln controller
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-2">
					<Label htmlFor="pico-url">Pico URL</Label>
					<div className="flex gap-2">
						<Input
							id="pico-url"
							type="text"
							placeholder="http://192.168.1.100:80"
							value={inputURL}
							onChange={(e) => setInputURL(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter") {
									handleSave();
								}
							}}
						/>
						<Button onClick={handleSave} variant="secondary">
							Save
						</Button>
					</div>
					<p className="text-xs text-muted-foreground">
						Example: http://192.168.1.100:80 or http://pico-kiln.local:80
					</p>
				</div>

				{isConfigured && (
					<>
						<div className="space-y-2">
							<ConnectionStatusDetailed />
						</div>

						<div className="flex gap-2">
							<Button
								onClick={handleTest}
								disabled={testConnection.isPending}
								className="flex-1"
							>
								{testConnection.isPending ? (
									<>
										<Loader2 className="w-4 h-4 mr-2 animate-spin" />
										Testing...
									</>
								) : (
									"Test Connection"
								)}
							</Button>
							<Button onClick={handleReset} variant="outline">
								Reset
							</Button>
						</div>

						{testConnection.isSuccess && testConnection.data && (
							<Alert className="border-green-600 bg-green-50">
								<CheckCircle2 className="w-4 h-4 text-green-600" />
								<AlertDescription className="text-green-800">
									Connection successful! You can now control your kiln.
								</AlertDescription>
							</Alert>
						)}

						{testConnection.isError ||
						(testConnection.isSuccess && !testConnection.data) ? (
							<Alert variant="destructive">
								<XCircle className="w-4 h-4" />
								<AlertDescription>
									Connection failed. Please check the URL and ensure the Pico is
									on the same network.
								</AlertDescription>
							</Alert>
						) : null}
					</>
				)}
			</CardContent>
		</Card>
	);
}
