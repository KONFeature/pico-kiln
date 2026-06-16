import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { useReboot } from "@/lib/pico/hooks";

interface RebootDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
}

export function RebootDialog({ open, onOpenChange }: RebootDialogProps) {
	const reboot = useReboot();

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>Configuration saved</DialogTitle>
					<DialogDescription>
						Your changes were written to the kiln, but they only take effect
						after a reboot. Reboot now? The connection will drop briefly while
						the Pico restarts.
					</DialogDescription>
				</DialogHeader>
				{reboot.isError && (
					<p className="text-sm text-destructive">
						Reboot failed: {reboot.error?.message}
					</p>
				)}
				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => onOpenChange(false)}
						disabled={reboot.isPending}
					>
						Later
					</Button>
					<Button
						onClick={() =>
							reboot.mutate(undefined, { onSuccess: () => onOpenChange(false) })
						}
						disabled={reboot.isPending}
					>
						{reboot.isPending ? "Rebooting…" : "Reboot now"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
