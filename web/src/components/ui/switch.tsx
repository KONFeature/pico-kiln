import { cn } from "@/lib/utils";

interface SwitchProps {
	id?: string;
	checked: boolean;
	onCheckedChange: (checked: boolean) => void;
	disabled?: boolean;
	"aria-label"?: string;
}

/**
 * Minimal, dependency-free toggle switch. We avoid @radix-ui/react-switch (not
 * installed) and build on a plain button so we don't add a dependency for a
 * single control.
 */
function Switch({
	id,
	checked,
	onCheckedChange,
	disabled,
	...props
}: SwitchProps) {
	return (
		<button
			type="button"
			role="switch"
			id={id}
			aria-checked={checked}
			disabled={disabled}
			data-slot="switch"
			onClick={() => onCheckedChange(!checked)}
			className={cn(
				"inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-transparent transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50",
				checked ? "bg-primary" : "bg-input",
			)}
			{...props}
		>
			<span
				className={cn(
					"pointer-events-none inline-block size-4 rounded-full bg-background shadow-xs transition-transform",
					checked ? "translate-x-4" : "translate-x-0.5",
				)}
			/>
		</button>
	);
}

export { Switch };
