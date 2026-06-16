import { Button } from "@/components/ui/button";

interface UnsavedBarProps {
	changeCount: number;
	canSave: boolean;
	saving: boolean;
	onSave: () => void;
	onDiscard: () => void;
}

export function UnsavedBar({
	changeCount,
	canSave,
	saving,
	onSave,
	onDiscard,
}: UnsavedBarProps) {
	return (
		<div className="sticky bottom-0 z-30 -mx-4 mt-6 border-t border-border bg-card/95 px-4 py-3 shadow-[0_-2px_8px_rgba(0,0,0,0.06)] backdrop-blur supports-[backdrop-filter]:bg-card/80 [padding-bottom:calc(0.75rem+env(safe-area-inset-bottom))]">
			<div className="container mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-3">
				<p className="text-sm">
					You have{" "}
					<span className="font-semibold">
						{changeCount} unsaved change{changeCount === 1 ? "" : "s"}
					</span>
					. Save them to the kiln, or discard.
				</p>
				<div className="flex items-center gap-2">
					<Button variant="ghost" onClick={onDiscard} disabled={saving}>
						Discard
					</Button>
					<Button onClick={onSave} disabled={!canSave || saving}>
						{saving ? "Saving…" : "Save changes"}
					</Button>
				</div>
			</div>
		</div>
	);
}
