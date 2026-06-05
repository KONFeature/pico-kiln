import type * as React from "react";
import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { ConfigFieldDef } from "@/lib/config/schema";
import type { ConfigValue } from "@/lib/pico/types";

// TanStack Form's generic API has 12 type params; typing it precisely here adds
// no safety for a metadata-driven renderer, so we accept it loosely.
// biome-ignore lint/suspicious/noExplicitAny: dynamic form over a flat record.
type AnyForm = any;

interface ConfigFieldProps {
	form: AnyForm;
	def: ConfigFieldDef;
	disabled?: boolean;
}

/** Render a config value for a text/number/select input: null/undefined -> "". */
function toInputStr(value: ConfigValue): string {
	return value === null || value === undefined ? "" : String(value);
}

/** Parse "15, 14" -> [15, 14]; non-numeric tokens become NaN (caught by validator). */
function parsePinList(raw: string): number[] {
	return raw
		.split(",")
		.map((s) => s.trim())
		.filter((s) => s.length > 0)
		.map((s) => Number(s));
}

export function ConfigField({ form, def, disabled }: ConfigFieldProps) {
	const [reveal, setReveal] = useState(false);
	const fieldId = `cfg-${String(def.key)}`;

	return (
		<form.Field
			name={String(def.key)}
			validators={
				def.validate
					? {
							onChange: ({ value }: { value: ConfigValue }) =>
								def.validate?.(value),
						}
					: undefined
			}
		>
			{(field: AnyForm) => {
				const errors = (field.state.meta.errors as unknown[]).filter(Boolean);
				const hasError = errors.length > 0;
				const value = field.state.value as ConfigValue;

				let control: React.ReactNode;

				if (def.options) {
					control = (
						<Select
							value={toInputStr(value)}
							onValueChange={(v) =>
								field.handleChange(def.type === "number" ? Number(v) : v)
							}
							disabled={disabled}
						>
							<SelectTrigger id={fieldId} className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{def.options.map((opt) => (
									<SelectItem key={opt.value} value={opt.value}>
										{opt.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					);
				} else if (def.type === "switch") {
					control = (
						<Switch
							id={fieldId}
							checked={Boolean(value)}
							onCheckedChange={field.handleChange}
							disabled={disabled}
							aria-label={def.label}
						/>
					);
				} else if (def.type === "pinlist") {
					control = (
						<Input
							id={fieldId}
							inputMode="numeric"
							value={Array.isArray(value) ? value.join(", ") : ""}
							onChange={(e) => field.handleChange(parsePinList(e.target.value))}
							onBlur={field.handleBlur}
							disabled={disabled}
							aria-invalid={hasError}
							placeholder="15, 14"
						/>
					);
				} else if (def.type === "number") {
					control = (
						<Input
							id={fieldId}
							type="number"
							inputMode="decimal"
							value={toInputStr(value)}
							onChange={(e) =>
								field.handleChange(
									e.target.value === "" ? "" : Number(e.target.value),
								)
							}
							onBlur={field.handleBlur}
							disabled={disabled}
							aria-invalid={hasError}
						/>
					);
				} else {
					// text | password | ip
					const isPassword = def.type === "password";
					control = (
						<div className="relative">
							<Input
								id={fieldId}
								type={isPassword && !reveal ? "password" : "text"}
								value={typeof value === "string" ? value : ""}
								onChange={(e) => field.handleChange(e.target.value)}
								onBlur={field.handleBlur}
								disabled={disabled}
								aria-invalid={hasError}
								className={isPassword ? "pr-16" : undefined}
								autoComplete={isPassword ? "new-password" : "off"}
							/>
							{isPassword && (
								<button
									type="button"
									onClick={() => setReveal((r) => !r)}
									disabled={disabled}
									className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground hover:text-foreground disabled:opacity-50"
								>
									{reveal ? "Hide" : "Show"}
								</button>
							)}
						</div>
					);
				}

				return (
					<div className="space-y-1.5">
						<div className="flex items-center justify-between gap-2">
							<Label htmlFor={fieldId} className="text-sm font-medium">
								{def.label}
							</Label>
							{def.unit && (
								<span className="shrink-0 text-xs text-muted-foreground">
									{def.hexHint && typeof value === "number"
										? `0x${value.toString(16).toUpperCase()} · ${def.unit}`
										: def.unit}
								</span>
							)}
						</div>
						{control}
						<p className="text-xs leading-snug text-muted-foreground">
							{def.help}
						</p>
						{hasError && (
							<p className="text-xs text-destructive">{String(errors[0])}</p>
						)}
					</div>
				);
			}}
		</form.Field>
	);
}
