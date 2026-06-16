import { useForm, useStore } from "@tanstack/react-form";
import { useEffect, useMemo, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useConfigDraft } from "@/lib/config/draft-context";
import { buildPatch, SECTIONS, withDefaults } from "@/lib/config/schema";
import { useKilnConfig, useKilnStatus, useSaveConfig } from "@/lib/pico/hooks";
import type { ConfigValue, KilnConfig, KilnState } from "@/lib/pico/types";
import { ConfigSection } from "./ConfigSection";
import { RebootDialog } from "./RebootDialog";
import { UnsavedBar } from "./UnsavedBar";

type FormValues = Record<string, ConfigValue>;

function toFormValues(config: KilnConfig): FormValues {
	return { ...config } as unknown as FormValues;
}

/**
 * Outer page: owns the config query and renders gates. The form lives in a child
 * that only mounts once we have a config snapshot, so TanStack Form seeds its
 * defaults from real values (it does not re-seed after mount).
 */
export function ConfigPage() {
	const configQuery = useKilnConfig();
	const { data: status } = useKilnStatus();

	if (configQuery.isLoading) {
		return <p className="text-muted-foreground">Loading configuration…</p>;
	}

	if (configQuery.isError || !configQuery.data) {
		return (
			<Alert variant="destructive">
				<AlertTitle>Couldn't load configuration</AlertTitle>
				<AlertDescription>
					{configQuery.error?.message ?? "The kiln did not return its config."}{" "}
					<Button
						variant="link"
						className="h-auto p-0"
						onClick={() => configQuery.refetch()}
					>
						Retry
					</Button>
				</AlertDescription>
			</Alert>
		);
	}

	return (
		<ConfigForm
			serverConfig={withDefaults(configQuery.data)}
			state={status?.state}
		/>
	);
}

interface ConfigFormProps {
	serverConfig: KilnConfig;
	state?: KilnState;
}

function ConfigForm({ serverConfig, state }: ConfigFormProps) {
	const locked = state === "RUNNING" || state === "TUNING";
	const save = useSaveConfig();
	const { draft, isDirty, setDraft, clearDraft } = useConfigDraft();
	const [rebootOpen, setRebootOpen] = useState(false);

	// Seed once at mount from server values merged with any persisted draft.
	// Deliberately empty deps: later edits flow through the form, not a re-seed.
	// biome-ignore lint/correctness/useExhaustiveDependencies: seed once at mount.
	const initialValues = useMemo<FormValues>(
		() => ({ ...toFormValues(serverConfig), ...draft }),
		[],
	);

	const form = useForm({
		defaultValues: initialValues,
		onSubmit: async ({ value }: { value: FormValues }) => {
			const patch = buildPatch(value, serverConfig);
			if (Object.keys(patch).length === 0) return;
			await save.mutateAsync(patch);
			// Clear the draft immediately; the refetched serverConfig will confirm
			// the diff is empty. We intentionally do NOT reset the form, so it keeps
			// showing the values the user just saved.
			clearDraft();
			setRebootOpen(true);
		},
	});

	const values = useStore(form.store, (s) => s.values) as FormValues;
	const canSubmit = useStore(form.store, (s) => s.canSubmit);

	// Mirror the live form diff into the draft context (persists + drives badges).
	useEffect(() => {
		setDraft(buildPatch(values, serverConfig));
	}, [values, serverConfig, setDraft]);

	const changeCount = Object.keys(draft).length;

	return (
		<div className="space-y-6">
			<div className="space-y-1">
				<h1 className="text-2xl font-bold">Configuration</h1>
				<p className="text-sm text-muted-foreground">
					Device settings stored on the kiln. Changes are saved to flash and
					take effect after a reboot.
				</p>
			</div>

			{locked && (
				<Alert>
					<AlertTitle>Kiln is firing — configuration locked</AlertTitle>
					<AlertDescription>
						The kiln is currently {state?.toLowerCase()}. Settings are read-only
						until it finishes. Stop the run to make changes.
					</AlertDescription>
				</Alert>
			)}

			{save.isError && (
				<Alert variant="destructive">
					<AlertTitle>Save failed</AlertTitle>
					<AlertDescription>{save.error?.message}</AlertDescription>
				</Alert>
			)}

			<form
				onSubmit={(e) => {
					e.preventDefault();
					form.handleSubmit();
				}}
				className="space-y-4"
			>
				{SECTIONS.map((section) => (
					<ConfigSection
						key={section.id}
						section={section}
						form={form}
						disabled={locked}
					/>
				))}

				{!locked && isDirty && (
					<UnsavedBar
						changeCount={changeCount}
						canSave={canSubmit}
						saving={save.isPending}
						onSave={() => form.handleSubmit()}
						onDiscard={() => {
							clearDraft();
							form.reset(toFormValues(serverConfig));
						}}
					/>
				)}
			</form>

			<RebootDialog open={rebootOpen} onOpenChange={setRebootOpen} />
		</div>
	);
}
