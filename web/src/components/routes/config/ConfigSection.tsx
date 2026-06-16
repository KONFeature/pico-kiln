import { ChevronDownIcon } from "lucide-react";
import { useState } from "react";
import { Card, CardContent } from "@/components/ui/card";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import type { ConfigSectionDef } from "@/lib/config/schema";
import { cn } from "@/lib/utils";
import { ConfigField } from "./ConfigField";

// See ConfigField for why the form is typed loosely.
// biome-ignore lint/suspicious/noExplicitAny: dynamic form over a flat record.
type AnyForm = any;

interface ConfigSectionProps {
	section: ConfigSectionDef;
	form: AnyForm;
	disabled?: boolean;
}

export function ConfigSection({ section, form, disabled }: ConfigSectionProps) {
	// Advanced sections start collapsed; everything else starts open.
	const [open, setOpen] = useState(!section.advanced);

	return (
		<Card className={cn("py-0", section.advanced && "border-destructive/40")}>
			<Collapsible open={open} onOpenChange={setOpen}>
				<CollapsibleTrigger className="flex w-full items-start justify-between gap-3 p-6 text-left">
					<div className="space-y-1">
						<h2
							className={cn(
								"text-lg font-semibold leading-none",
								section.advanced && "text-destructive",
							)}
						>
							{section.title}
						</h2>
						{section.description && (
							<p className="text-sm text-muted-foreground">
								{section.description}
							</p>
						)}
					</div>
					<ChevronDownIcon
						className={cn(
							"size-5 shrink-0 text-muted-foreground transition-transform",
							open && "rotate-180",
						)}
					/>
				</CollapsibleTrigger>
				<CollapsibleContent>
					<CardContent className="grid grid-cols-1 gap-x-6 gap-y-5 pb-6 sm:grid-cols-2">
						{section.fields.map((def) => (
							<ConfigField
								key={String(def.key)}
								form={form}
								def={def}
								disabled={disabled}
							/>
						))}
					</CardContent>
				</CollapsibleContent>
			</Collapsible>
		</Card>
	);
}
