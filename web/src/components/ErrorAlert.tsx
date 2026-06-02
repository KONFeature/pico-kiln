import { AlertTriangle, ChevronDown } from "lucide-react";
import type { ReactNode } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { getFriendlyError } from "@/lib/pico/errors";

interface ErrorAlertProps {
	/** Any thrown value — PicoAPIError, Error, string, etc. */
	error: unknown;
	variant?: "destructive" | "warning";
	className?: string;
	/** Optional action (e.g. a retry button) rendered under the message. */
	action?: ReactNode;
}

/**
 * Renders a thrown error as friendly, recovery-oriented copy with the raw
 * technical message tucked behind a "Technical details" disclosure so makers
 * can still debug without the jargon shouting at everyone else.
 */
export function ErrorAlert({
	error,
	variant = "destructive",
	className,
	action,
}: ErrorAlertProps) {
	const { title, message, detail } = getFriendlyError(error);
	const showDetail = Boolean(detail) && detail !== message;

	return (
		<Alert variant={variant} className={className}>
			<AlertTriangle className="h-4 w-4" />
			<AlertTitle>{title}</AlertTitle>
			<AlertDescription>
				<span>{message}</span>
				{showDetail && (
					<Collapsible className="w-full">
						<CollapsibleTrigger className="group mt-1 inline-flex items-center gap-1 text-xs font-medium opacity-70 hover:opacity-100">
							<ChevronDown className="h-3 w-3 transition-transform group-data-[state=open]:rotate-180" />
							Technical details
						</CollapsibleTrigger>
						<CollapsibleContent>
							<code className="mt-1 block w-full whitespace-pre-wrap break-words rounded bg-foreground/5 px-2 py-1 font-mono text-xs">
								{detail}
							</code>
						</CollapsibleContent>
					</Collapsible>
				)}
				{action && <div className="mt-2">{action}</div>}
			</AlertDescription>
		</Alert>
	);
}
