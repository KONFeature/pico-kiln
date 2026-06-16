# Web Config Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `/config` page to the `web/` React app that surfaces every Rust-firmware kiln config option, grouped into collapsible sections, with per-field validation, a draft/unsaved-changes system (sessionStorage + global badges + page-local sticky bar), a firing lock, and a reboot-to-apply flow.

**Architecture:** A single metadata schema (`lib/config/schema.ts`) drives a generic `<ConfigField>` renderer. TanStack Form owns field state + validation on the page. A `ConfigDraftContext` (mounted above `Header`) persists the working diff to sessionStorage and exposes `isDirty` globally so menu badges can render on any page. Save POSTs only the changed keys (matches the firmware's sparse PATCH + 2 KiB body cap), then offers a reboot.

**Tech Stack:** React 19, TanStack Router (file-based), TanStack Query, **TanStack Form (new dep)**, shadcn/Radix UI, Tailwind v4, Biome.

**Spec:** `docs/superpowers/specs/2026-06-04-web-config-page-design.md`

**Conventions:**
- Source files use **tabs**. After editing, every task runs `bunx @biomejs/biome check --write <files>` to normalize formatting/imports, then verifies clean.
- No automated tests (project convention for this UI). Verification = Biome clean + (final task) `bun run build` typecheck.
- All commands run from `web/`.
- Commit after each task. Branch already `feat/rust-kiln-core`.

---

## Task 1: Add TanStack Form dep + dep-free Switch primitive

**Files:**
- Modify: `web/package.json`
- Create: `web/src/components/ui/switch.tsx`

- [ ] **Step 1: Install TanStack Form**

Run (from `web/`):
```bash
bun add @tanstack/react-form
```
Expected: `package.json` gains `"@tanstack/react-form"` under dependencies; `bun.lock` updates.

- [ ] **Step 2: Create a dependency-free Switch primitive**

(`@radix-ui/react-switch` is NOT installed; this button-based switch avoids adding a dep and matches the app's Tailwind token style.)

`web/src/components/ui/switch.tsx`:
```tsx
import type * as React from "react";
import { cn } from "@/lib/utils";

interface SwitchProps {
	id?: string;
	checked: boolean;
	onCheckedChange: (checked: boolean) => void;
	disabled?: boolean;
	"aria-label"?: string;
}

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
```

- [ ] **Step 3: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/components/ui/switch.tsx && bunx @biomejs/biome check src/components/ui/switch.tsx
```
Expected: "Checked N files ... No fixes needed" / clean exit 0.

- [ ] **Step 4: Commit**

```bash
git add web/package.json web/bun.lock web/src/components/ui/switch.tsx
git commit -m "feat(web): add tanstack-form dep + Switch ui primitive"
```

---

## Task 2: Config types

**Files:**
- Modify: `web/src/lib/pico/types.ts`

- [ ] **Step 1: Append config types** (after the existing `TempUnits` usage / at end of file, before EOF)

Add to `web/src/lib/pico/types.ts`:
```ts
// === Kiln Configuration (GET/POST /api/config) ===

export type ThermocoupleType =
	| "B"
	| "E"
	| "J"
	| "K"
	| "N"
	| "R"
	| "S"
	| "T"
	| "G8"
	| "G32";

/**
 * Full kiln configuration as served by GET /api/config. Field names are
 * UPPER_SNAKE_CASE to match the firmware wire format exactly. LCD_* keys may be
 * absent in the response when the LCD is not configured; the client fills them
 * with firmware defaults for editing (see lib/config/schema.ts DEFAULTS).
 */
export interface KilnConfig {
	// Hardware (GPIO / SPI)
	MAX31856_SPI_ID: number;
	MAX31856_SCK_PIN: number;
	MAX31856_MOSI_PIN: number;
	MAX31856_MISO_PIN: number;
	MAX31856_CS_PIN: number;
	SSR_PIN: number[];

	// Network
	WIFI_SSID: string;
	WIFI_PASSWORD: string;
	WIFI_STATIC_IP: string | null;
	WIFI_SUBNET: string | null;
	WIFI_GATEWAY: string | null;
	WIFI_DNS: string | null;
	WEB_SERVER_HOST: string;
	WEB_SERVER_PORT: number;

	// Temperature & sensor
	THERMOCOUPLE_TYPE: ThermocoupleType;
	TEMP_UNITS: TempUnits;
	THERMOCOUPLE_OFFSET: number;
	MAINS_FREQUENCY: number;
	THERMOCOUPLE_AVERAGING: number;
	TEMP_MEDIAN_WINDOW: number;

	// Control loop timing
	TEMP_READ_INTERVAL: number;
	PID_UPDATE_INTERVAL: number;
	STATUS_UPDATE_INTERVAL: number;
	SSR_UPDATE_INTERVAL: number;

	// PID + thermal model
	PID_KP_BASE: number;
	PID_KI_BASE: number;
	PID_KD_BASE: number;
	THERMAL_H: number;
	THERMAL_T_AMBIENT: number;

	// SSR power
	SSR_CYCLE_TIME: number;
	SSR_STAGGER_DELAY: number;

	// Safety + rate/stall
	MAX_TEMP: number;
	STALL_CHECK_INTERVAL: number;
	STALL_CONSECUTIVE_FAILS: number;
	STALL_MIN_STEP_TIME: number;
	RATE_MEASUREMENT_WINDOW: number;
	RATE_RECORDING_INTERVAL: number;
	MAX_RECOVERY_TEMP_DELTA: number;

	// Logging + watchdog
	LOGGING_INTERVAL: number;
	ENABLE_WATCHDOG: boolean;
	WATCHDOG_TIMEOUT: number;

	// LCD (optional)
	LCD_I2C_ID: number;
	LCD_I2C_SCL: number;
	LCD_I2C_SDA: number;
	LCD_I2C_FREQ: number;
	LCD_I2C_ADDR: number;
}

/** A single config value as held by the form. */
export type ConfigValue = string | number | boolean | number[] | null;

export interface SaveConfigResponse {
	success: boolean;
	message?: string;
	error?: string;
}
```

- [ ] **Step 2: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/lib/pico/types.ts && bunx @biomejs/biome check src/lib/pico/types.ts
```
Expected: clean exit 0.

- [ ] **Step 3: Commit**

```bash
git add web/src/lib/pico/types.ts
git commit -m "feat(web): add KilnConfig + config wire types"
```

---

## Task 3: API client methods (getConfig / saveConfig)

**Files:**
- Modify: `web/src/lib/pico/client.ts`

`reboot()` already exists (lines 162-166) — do NOT re-add it.

- [ ] **Step 1: Extend the type import**

In `web/src/lib/pico/client.ts`, add `KilnConfig` and `SaveConfigResponse` to the existing `import type { ... } from "./types";` block (keep alphabetical-ish ordering consistent with the file; Biome will sort):
```ts
	KilnConfig,
	SaveConfigResponse,
```

- [ ] **Step 2: Add config methods** (insert a new section after the `reboot()` method, before `// === Tuning Endpoints ===`)

```ts
	// === Config Endpoints ===

	/**
	 * Fetch the full kiln configuration. LCD_* keys may be omitted by the
	 * firmware when the LCD is disabled; callers fill defaults for editing.
	 */
	async getConfig(): Promise<KilnConfig> {
		return this.request<KilnConfig>("/api/config");
	}

	/**
	 * Persist a sparse config PATCH (only the changed keys). The firmware merges
	 * it over the running config and applies it on the next reboot. Body must
	 * stay under the firmware's 2 KiB limit — a diff always does.
	 */
	async saveConfig(
		patch: Record<string, unknown>,
	): Promise<SaveConfigResponse> {
		return this.request<SaveConfigResponse>("/api/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(patch),
		});
	}
```

- [ ] **Step 3: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/lib/pico/client.ts && bunx @biomejs/biome check src/lib/pico/client.ts
```
Expected: clean exit 0.

- [ ] **Step 4: Commit**

```bash
git add web/src/lib/pico/client.ts
git commit -m "feat(web): add getConfig/saveConfig client methods"
```

---

## Task 4: Query/mutation hooks

**Files:**
- Modify: `web/src/lib/pico/hooks.ts`

- [ ] **Step 1: Add `config` to the query-key registry**

In `picoKeys` (lines 31-37) add:
```ts
	config: ["pico", "config"] as const,
```

- [ ] **Step 2: Extend the type import**

Add `KilnConfig` and `SaveConfigResponse` to the `import type { ... } from "./types";` block.

- [ ] **Step 3: Add config hooks** (append at end of file, after `useUploadFile`)

```ts
// === Config Hooks ===

/**
 * Fetch the kiln configuration. Changes rarely, so it is cached for a while and
 * only refetched on demand / after a save invalidation.
 */
export function useKilnConfig(
	options?: Partial<UseQueryOptions<KilnConfig, PicoAPIError>>,
) {
	const { client, isConfigured } = usePico();

	return useQuery<KilnConfig, PicoAPIError>({
		queryKey: picoKeys.config,
		queryFn: async () => {
			if (!client) {
				throw new PicoAPIError("Pico client not initialized");
			}
			return await client.getConfig();
		},
		enabled: isConfigured && Boolean(client),
		staleTime: 1000 * 60 * 5,
		refetchOnWindowFocus: false,
		retry: 2,
		...options,
	});
}

/**
 * Save a sparse config PATCH. On success the config query is invalidated so the
 * page re-seeds from the persisted values.
 */
export function useSaveConfig() {
	const { client } = usePico();
	const queryClient = useQueryClient();

	return useMutation<SaveConfigResponse, PicoAPIError, Record<string, unknown>>(
		{
			mutationFn: async (patch) => {
				if (!client) {
					throw new PicoAPIError("Pico client not initialized");
				}
				return unwrap(await client.saveConfig(patch), "Failed to save config");
			},
			onSuccess: () => {
				queryClient.invalidateQueries({ queryKey: picoKeys.config });
			},
		},
	);
}
```

- [ ] **Step 4: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/lib/pico/hooks.ts && bunx @biomejs/biome check src/lib/pico/hooks.ts
```
Expected: clean exit 0.

- [ ] **Step 5: Commit**

```bash
git add web/src/lib/pico/hooks.ts
git commit -m "feat(web): add useKilnConfig + useSaveConfig hooks"
```

---

## Task 5: Field schema + validators + diff helpers

**Files:**
- Create: `web/src/lib/config/validators.ts`
- Create: `web/src/lib/config/schema.ts`

- [ ] **Step 1: Create validators**

`web/src/lib/config/validators.ts`:
```ts
import type { ConfigValue } from "@/lib/pico/types";

export type Validator = (value: ConfigValue) => string | undefined;

/** Required, finite number within [min, max]. */
export function num(min: number, max: number, integer = false): Validator {
	return (value) => {
		if (value === "" || value === null || value === undefined) {
			return "Required";
		}
		const n = typeof value === "number" ? value : Number(value);
		if (!Number.isFinite(n)) return "Must be a number";
		if (integer && !Number.isInteger(n)) return "Must be a whole number";
		if (n < min || n > max) return `Must be between ${min} and ${max}`;
		return undefined;
	};
}

/** Non-empty string up to maxLen chars. */
export function str(maxLen: number, required = true): Validator {
	return (value) => {
		const s = typeof value === "string" ? value : "";
		if (required && s.length === 0) return "Required";
		if (s.length > maxLen) return `Must be at most ${maxLen} characters`;
		return undefined;
	};
}

/** Optional string up to maxLen chars (empty allowed). */
export function optionalStr(maxLen: number): Validator {
	return str(maxLen, false);
}

const IPV4 =
	/^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;

/** Empty (treated as DHCP/null) or a valid dotted-quad IPv4 address. */
export function ipv4Optional(): Validator {
	return (value) => {
		const s = typeof value === "string" ? value.trim() : "";
		if (s.length === 0) return undefined;
		if (!IPV4.test(s)) return "Must be a valid IPv4 address (e.g. 192.168.1.50)";
		return undefined;
	};
}

/** A host string: empty rejected, else IPv4 or a plausible hostname. */
export function host(): Validator {
	return (value) => {
		const s = typeof value === "string" ? value.trim() : "";
		if (s.length === 0) return "Required";
		if (s.length > 64) return "Must be at most 64 characters";
		return undefined;
	};
}

/**
 * 1..maxPins unique integer GPIO pins, each within [0, 29]. Value is a number[]
 * (parsed from the comma-separated input by ConfigField).
 */
export function pinList(maxPins = 10): Validator {
	return (value) => {
		if (!Array.isArray(value)) return "Enter one or more pins";
		if (value.length === 0) return "At least one pin is required";
		if (value.length > maxPins) return `At most ${maxPins} pins`;
		const seen = new Set<number>();
		for (const p of value) {
			if (!Number.isInteger(p)) return "Pins must be whole numbers";
			if (p < 0 || p > 29) return "Pins must be between 0 and 29";
			if (seen.has(p)) return `Duplicate pin: ${p}`;
			seen.add(p);
		}
		return undefined;
	};
}
```

- [ ] **Step 2: Create the schema + helpers**

`web/src/lib/config/schema.ts`:
```ts
import type { ConfigValue, KilnConfig } from "@/lib/pico/types";
import {
	host,
	ipv4Optional,
	num,
	optionalStr,
	pinList,
	str,
	type Validator,
} from "./validators";

export type FieldType =
	| "number"
	| "text"
	| "password"
	| "select"
	| "switch"
	| "pinlist"
	| "ip";

export interface ConfigFieldDef {
	key: keyof KilnConfig;
	label: string;
	help: string;
	type: FieldType;
	unit?: string;
	/** Hex hint shown next to a number input (e.g. "0x27"). */
	hexHint?: boolean;
	options?: { value: string; label: string }[];
	/** For ip fields: empty string is serialized as null. */
	nullable?: boolean;
	validate?: Validator;
}

export interface ConfigSectionDef {
	id: string;
	title: string;
	description?: string;
	advanced?: boolean;
	fields: ConfigFieldDef[];
}

/**
 * Firmware defaults. Used to fill keys the GET response may omit (LCD_*), so the
 * form can render every field and an untouched value produces no diff.
 */
export const DEFAULTS: KilnConfig = {
	MAX31856_SPI_ID: 0,
	MAX31856_SCK_PIN: 18,
	MAX31856_MOSI_PIN: 19,
	MAX31856_MISO_PIN: 16,
	MAX31856_CS_PIN: 28,
	SSR_PIN: [15],
	WIFI_SSID: "",
	WIFI_PASSWORD: "",
	WIFI_STATIC_IP: null,
	WIFI_SUBNET: null,
	WIFI_GATEWAY: null,
	WIFI_DNS: null,
	WEB_SERVER_HOST: "0.0.0.0",
	WEB_SERVER_PORT: 80,
	THERMOCOUPLE_TYPE: "K",
	TEMP_UNITS: "c",
	THERMOCOUPLE_OFFSET: 0,
	MAINS_FREQUENCY: 60,
	THERMOCOUPLE_AVERAGING: 8,
	TEMP_MEDIAN_WINDOW: 3,
	TEMP_READ_INTERVAL: 1,
	PID_UPDATE_INTERVAL: 1,
	STATUS_UPDATE_INTERVAL: 5,
	SSR_UPDATE_INTERVAL: 0.1,
	PID_KP_BASE: 25,
	PID_KI_BASE: 0.14,
	PID_KD_BASE: 160,
	THERMAL_H: 0,
	THERMAL_T_AMBIENT: 25,
	SSR_CYCLE_TIME: 20,
	SSR_STAGGER_DELAY: 0.01,
	MAX_TEMP: 1300,
	STALL_CHECK_INTERVAL: 60,
	STALL_CONSECUTIVE_FAILS: 3,
	STALL_MIN_STEP_TIME: 600,
	RATE_MEASUREMENT_WINDOW: 600,
	RATE_RECORDING_INTERVAL: 10,
	MAX_RECOVERY_TEMP_DELTA: 30,
	LOGGING_INTERVAL: 30,
	ENABLE_WATCHDOG: false,
	WATCHDOG_TIMEOUT: 8000,
	LCD_I2C_ID: 0,
	LCD_I2C_SCL: 21,
	LCD_I2C_SDA: 20,
	LCD_I2C_FREQ: 100000,
	LCD_I2C_ADDR: 39,
};

const pinHelp = "Raspberry Pi Pico GPIO number (0-29).";

export const SECTIONS: ConfigSectionDef[] = [
	{
		id: "temperature",
		title: "Temperature & Sensor",
		description: "Thermocouple readout, calibration and filtering.",
		fields: [
			{
				key: "THERMOCOUPLE_TYPE",
				label: "Thermocouple type",
				help: "Alloy type of your thermocouple. Must match the physical probe. K is the most common for kilns.",
				type: "select",
				options: [
					"B",
					"E",
					"J",
					"K",
					"N",
					"R",
					"S",
					"T",
					"G8",
					"G32",
				].map((v) => ({ value: v, label: v })),
			},
			{
				key: "TEMP_UNITS",
				label: "Display units",
				help: "Units shown in the UI. The control loop always runs in Celsius internally.",
				type: "select",
				options: [
					{ value: "c", label: "Celsius (°C)" },
					{ value: "f", label: "Fahrenheit (°F)" },
				],
			},
			{
				key: "THERMOCOUPLE_OFFSET",
				label: "Calibration offset",
				help: "Added to every temperature reading to correct sensor bias. Leave at 0 unless you've calibrated against a reference.",
				type: "number",
				unit: "°C",
				validate: num(-50, 50),
			},
			{
				key: "MAINS_FREQUENCY",
				label: "Mains frequency",
				help: "Your AC line frequency. Used by the MAX31856 noise rejection filter.",
				type: "number",
				unit: "Hz",
				options: [
					{ value: "50", label: "50 Hz" },
					{ value: "60", label: "60 Hz" },
				],
			},
			{
				key: "THERMOCOUPLE_AVERAGING",
				label: "Hardware averaging",
				help: "Number of samples the MAX31856 averages per reading. Higher = smoother but slower.",
				type: "number",
				unit: "samples",
				options: [1, 2, 4, 8, 16].map((v) => ({
					value: String(v),
					label: String(v),
				})),
			},
			{
				key: "TEMP_MEDIAN_WINDOW",
				label: "Median filter window",
				help: "Software median filter size used to reject single-sample spikes. 1 disables it.",
				type: "number",
				unit: "samples",
				validate: num(1, 15, true),
			},
		],
	},
	{
		id: "pid",
		title: "PID & Thermal Model",
		description: "Core control gains and the optional thermal feed-forward model.",
		fields: [
			{
				key: "PID_KP_BASE",
				label: "Proportional gain (Kp)",
				help: "Base proportional term. Drives output proportional to the current temperature error.",
				type: "number",
				validate: num(0, 10000),
			},
			{
				key: "PID_KI_BASE",
				label: "Integral gain (Ki)",
				help: "Base integral term. Eliminates steady-state error over time. Small values are normal.",
				type: "number",
				validate: num(0, 1000),
			},
			{
				key: "PID_KD_BASE",
				label: "Derivative gain (Kd)",
				help: "Base derivative term. Damps overshoot by reacting to the rate of error change.",
				type: "number",
				validate: num(0, 100000),
			},
			{
				key: "THERMAL_H",
				label: "Thermal efficiency (H)",
				help: "Feed-forward thermal-model coefficient. 0 disables the model and uses pure PID.",
				type: "number",
				validate: num(0, 10000),
			},
			{
				key: "THERMAL_T_AMBIENT",
				label: "Ambient temperature",
				help: "Room temperature assumed by the thermal model.",
				type: "number",
				unit: "°C",
				validate: num(-20, 60),
			},
		],
	},
	{
		id: "ssr",
		title: "SSR / Power Control",
		description: "Solid-state relay switching behaviour.",
		fields: [
			{
				key: "SSR_CYCLE_TIME",
				label: "Duty-cycle period",
				help: "Length of one SSR on/off PWM window. The PID output sets the on-fraction of this window.",
				type: "number",
				unit: "s",
				validate: num(1, 120),
			},
			{
				key: "SSR_STAGGER_DELAY",
				label: "Multi-relay stagger delay",
				help: "Delay between switching multiple SSRs on, to limit inrush current. Ignored with a single relay.",
				type: "number",
				unit: "s",
				validate: num(0, 5),
			},
		],
	},
	{
		id: "safety",
		title: "Safety & Stall Detection",
		description: "Hard limits and heating-rate sanity checks that trip the kiln to ERROR.",
		fields: [
			{
				key: "MAX_TEMP",
				label: "Maximum temperature",
				help: "Hard safety ceiling. Exceeding it immediately faults the kiln to ERROR and cuts power.",
				type: "number",
				unit: "°C",
				validate: num(100, 1400),
			},
			{
				key: "STALL_CHECK_INTERVAL",
				label: "Stall check interval",
				help: "How often the heating rate is evaluated against the target during a ramp.",
				type: "number",
				unit: "s",
				validate: num(10, 3600),
			},
			{
				key: "STALL_CONSECUTIVE_FAILS",
				label: "Stall fail count",
				help: "Number of consecutive failed rate checks before the kiln declares a stall error.",
				type: "number",
				unit: "checks",
				validate: num(1, 20, true),
			},
			{
				key: "STALL_MIN_STEP_TIME",
				label: "Stall grace period",
				help: "Minimum time into a step before stall detection is allowed to fire.",
				type: "number",
				unit: "s",
				validate: num(0, 7200),
			},
			{
				key: "RATE_MEASUREMENT_WINDOW",
				label: "Rate measurement window",
				help: "Rolling window used to compute the average heating/cooling rate.",
				type: "number",
				unit: "s",
				validate: num(60, 7200),
			},
			{
				key: "RATE_RECORDING_INTERVAL",
				label: "Rate sample interval",
				help: "How often a temperature sample is recorded for the rate calculation.",
				type: "number",
				unit: "s",
				validate: num(1, 600),
			},
			{
				key: "MAX_RECOVERY_TEMP_DELTA",
				label: "Max recovery temp drop",
				help: "After a crash/power loss, recovery is declined if the kiln cooled more than this.",
				type: "number",
				unit: "°C",
				validate: num(0, 200),
			},
		],
	},
	{
		id: "timing",
		title: "Control Loop Timing",
		description: "Cadence of the control loop's sub-tasks. Defaults suit most kilns.",
		fields: [
			{
				key: "TEMP_READ_INTERVAL",
				label: "Temperature read interval",
				help: "Outer control-tick period: how often a new temperature is read.",
				type: "number",
				unit: "s",
				validate: num(0.1, 10),
			},
			{
				key: "PID_UPDATE_INTERVAL",
				label: "PID update interval",
				help: "How often the PID controller recomputes its output.",
				type: "number",
				unit: "s",
				validate: num(0.1, 10),
			},
			{
				key: "STATUS_UPDATE_INTERVAL",
				label: "Status broadcast interval",
				help: "How often the controller publishes status to the web UI.",
				type: "number",
				unit: "s",
				validate: num(1, 60),
			},
			{
				key: "SSR_UPDATE_INTERVAL",
				label: "SSR update interval",
				help: "Sub-tick period at which the SSR duty cycle is refreshed.",
				type: "number",
				unit: "s",
				validate: num(0.05, 5),
			},
		],
	},
	{
		id: "logging",
		title: "Logging & Watchdog",
		description: "Run logging and the optional hardware watchdog.",
		fields: [
			{
				key: "LOGGING_INTERVAL",
				label: "Log row interval",
				help: "How often a row is written to the run CSV log.",
				type: "number",
				unit: "s",
				validate: num(1, 600, true),
			},
			{
				key: "ENABLE_WATCHDOG",
				label: "Enable hardware watchdog",
				help: "Resets the Pico automatically if the firmware ever hangs. Recommended on.",
				type: "switch",
			},
			{
				key: "WATCHDOG_TIMEOUT",
				label: "Watchdog timeout",
				help: "How long the firmware can be unresponsive before the watchdog resets it.",
				type: "number",
				unit: "ms",
				validate: num(1000, 30000, true),
			},
		],
	},
	{
		id: "lcd",
		title: "Display (LCD)",
		description:
			"Optional I²C LCD1602. If your kiln has no LCD, leave these untouched — editing any value enables the display.",
		fields: [
			{
				key: "LCD_I2C_ID",
				label: "I²C bus id",
				help: "Which I²C peripheral the LCD is wired to (0 or 1).",
				type: "number",
				validate: num(0, 1, true),
			},
			{
				key: "LCD_I2C_SCL",
				label: "SCL pin",
				help: `I²C clock pin. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "LCD_I2C_SDA",
				label: "SDA pin",
				help: `I²C data pin. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "LCD_I2C_FREQ",
				label: "I²C frequency",
				help: "I²C bus clock speed. 100000 (100 kHz) is standard.",
				type: "number",
				unit: "Hz",
				validate: num(10000, 1000000, true),
			},
			{
				key: "LCD_I2C_ADDR",
				label: "I²C address",
				help: "LCD backpack I²C address as a decimal number. Common: 39 (0x27) or 63 (0x3F).",
				type: "number",
				hexHint: true,
				validate: num(0, 127, true),
			},
		],
	},
	{
		id: "hardware",
		title: "Advanced — Hardware (GPIO)",
		advanced: true,
		description:
			"Thermocouple SPI wiring and SSR pins. Wrong values break temperature reads or relay control. Only change if you rewired the board — fixable only by re-flashing over USB.",
		fields: [
			{
				key: "MAX31856_SPI_ID",
				label: "SPI bus id",
				help: "Which SPI peripheral the MAX31856 is wired to (0 or 1).",
				type: "number",
				validate: num(0, 1, true),
			},
			{
				key: "MAX31856_SCK_PIN",
				label: "SCK pin",
				help: `SPI clock pin. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_MOSI_PIN",
				label: "MOSI pin",
				help: `SPI controller-to-sensor data pin. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_MISO_PIN",
				label: "MISO pin",
				help: `SPI sensor-to-controller data pin. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "MAX31856_CS_PIN",
				label: "Chip-select pin",
				help: `SPI chip-select for the MAX31856. ${pinHelp}`,
				type: "number",
				unit: "GPIO",
				validate: num(0, 29, true),
			},
			{
				key: "SSR_PIN",
				label: "SSR pin(s)",
				help: `Comma-separated GPIO pin(s) driving the relay(s), e.g. "15" or "15, 14". Up to 10. ${pinHelp}`,
				type: "pinlist",
				unit: "GPIO",
				validate: pinList(10),
			},
		],
	},
	{
		id: "network",
		title: "Advanced — Network",
		advanced: true,
		description:
			"WiFi credentials and web-server binding. A wrong SSID/password or static IP will drop the Pico off the network — recoverable only over USB.",
		fields: [
			{
				key: "WIFI_SSID",
				label: "WiFi network (SSID)",
				help: "Name of the WiFi network the Pico connects to.",
				type: "text",
				validate: str(64),
			},
			{
				key: "WIFI_PASSWORD",
				label: "WiFi password",
				help: "Password for the WiFi network. Stored in plaintext on the device.",
				type: "password",
				validate: optionalStr(64),
			},
			{
				key: "WIFI_STATIC_IP",
				label: "Static IP",
				help: "Optional fixed IPv4 for the Pico. Leave blank to use DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_SUBNET",
				label: "Subnet mask",
				help: "Subnet mask for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_GATEWAY",
				label: "Gateway",
				help: "Default gateway for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WIFI_DNS",
				label: "DNS server",
				help: "DNS server for the static IP. Leave blank for DHCP.",
				type: "ip",
				nullable: true,
				validate: ipv4Optional(),
			},
			{
				key: "WEB_SERVER_HOST",
				label: "Server bind address",
				help: "Address the HTTP server binds to. 0.0.0.0 listens on all interfaces.",
				type: "text",
				validate: host(),
			},
			{
				key: "WEB_SERVER_PORT",
				label: "Server port",
				help: "TCP port for the web server. Default 80.",
				type: "number",
				validate: num(1, 65535, true),
			},
		],
	},
];

/** Flat list of every field def, in section order. */
export const ALL_FIELDS: ConfigFieldDef[] = SECTIONS.flatMap((s) => s.fields);

const FIELD_BY_KEY = new Map<string, ConfigFieldDef>(
	ALL_FIELDS.map((f) => [f.key as string, f]),
);

/** Fill any keys missing from the GET response with firmware defaults. */
export function withDefaults(raw: Partial<KilnConfig>): KilnConfig {
	return { ...DEFAULTS, ...raw };
}

/**
 * Normalize a form value to its wire representation for a field: trims/【nulls】
 * empty ip strings, coerces numeric selects/numbers to numbers, leaves arrays
 * and booleans as-is.
 */
export function normalize(def: ConfigFieldDef, value: ConfigValue): ConfigValue {
	if (def.type === "ip") {
		const s = typeof value === "string" ? value.trim() : "";
		return s.length === 0 ? null : s;
	}
	if (def.type === "number") {
		if (value === "" || value === null || value === undefined) return value;
		return typeof value === "number" ? value : Number(value);
	}
	return value;
}

function equal(a: ConfigValue, b: ConfigValue): boolean {
	if (Array.isArray(a) || Array.isArray(b)) {
		return JSON.stringify(a) === JSON.stringify(b);
	}
	return a === b;
}

/**
 * Build the sparse PATCH: only keys whose normalized form value differs from the
 * (default-filled) server config. LCD keys are included only when changed, so an
 * untouched LCD section never accidentally enables the display.
 */
export function buildPatch(
	values: Record<string, ConfigValue>,
	server: KilnConfig,
): Record<string, ConfigValue> {
	const patch: Record<string, ConfigValue> = {};
	for (const def of ALL_FIELDS) {
		const key = def.key as string;
		const next = normalize(def, values[key]);
		const base = normalize(def, server[def.key] as ConfigValue);
		if (!equal(next, base)) {
			patch[key] = next;
		}
	}
	return patch;
}

export function getFieldDef(key: string): ConfigFieldDef | undefined {
	return FIELD_BY_KEY.get(key);
}
```

> NOTE: in the `normalize` doc comment above, replace the placeholder `【nulls】`
> with the word `nulls` (the brackets are only here to flag it — do not ship
> bracket characters). Plain ASCII only.

- [ ] **Step 3: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/lib/config/validators.ts src/lib/config/schema.ts && bunx @biomejs/biome check src/lib/config/validators.ts src/lib/config/schema.ts
```
Expected: clean exit 0.

- [ ] **Step 4: Commit**

```bash
git add web/src/lib/config/validators.ts web/src/lib/config/schema.ts
git commit -m "feat(web): config field schema, validators and diff helpers"
```

---

## Task 6: Draft context (sessionStorage + global isDirty) + mount

**Files:**
- Create: `web/src/lib/config/draft-context.tsx`
- Modify: `web/src/router.tsx`

- [ ] **Step 1: Create the draft context**

`web/src/lib/config/draft-context.tsx`:
```tsx
import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { usePico } from "@/lib/pico/context";
import type { ConfigValue } from "@/lib/pico/types";

type Draft = Record<string, ConfigValue>;

interface ConfigDraftValue {
	/** Changed-keys-only diff, persisted to sessionStorage per kiln URL. */
	draft: Draft;
	isDirty: boolean;
	/** Replace the whole draft (ConfigPage pushes the computed diff here). */
	setDraft: (draft: Draft) => void;
	/** Clear the draft (after a successful save or a discard). */
	clearDraft: () => void;
}

const ConfigDraftContext = createContext<ConfigDraftValue | undefined>(
	undefined,
);

const KEY_PREFIX = "pico-kiln-config-draft:";

function storageKey(url: string): string {
	return `${KEY_PREFIX}${url}`;
}

export function ConfigDraftProvider({ children }: { children: ReactNode }) {
	const { picoURL } = usePico();
	const [draft, setDraftState] = useState<Draft>({});

	// Load any persisted draft for the current kiln on mount / URL change.
	useEffect(() => {
		if (typeof window === "undefined" || !picoURL) {
			setDraftState({});
			return;
		}
		try {
			const raw = sessionStorage.getItem(storageKey(picoURL));
			setDraftState(raw ? (JSON.parse(raw) as Draft) : {});
		} catch {
			setDraftState({});
		}
	}, [picoURL]);

	const persist = useCallback(
		(next: Draft) => {
			if (typeof window === "undefined" || !picoURL) return;
			const key = storageKey(picoURL);
			if (Object.keys(next).length === 0) {
				sessionStorage.removeItem(key);
			} else {
				sessionStorage.setItem(key, JSON.stringify(next));
			}
		},
		[picoURL],
	);

	const setDraft = useCallback(
		(next: Draft) => {
			setDraftState(next);
			persist(next);
		},
		[persist],
	);

	const clearDraft = useCallback(() => {
		setDraftState({});
		persist({});
	}, [persist]);

	const value = useMemo<ConfigDraftValue>(
		() => ({
			draft,
			isDirty: Object.keys(draft).length > 0,
			setDraft,
			clearDraft,
		}),
		[draft, setDraft, clearDraft],
	);

	return (
		<ConfigDraftContext.Provider value={value}>
			{children}
		</ConfigDraftContext.Provider>
	);
}

export function useConfigDraft(): ConfigDraftValue {
	const ctx = useContext(ConfigDraftContext);
	if (!ctx) {
		throw new Error("useConfigDraft must be used within a ConfigDraftProvider");
	}
	return ctx;
}
```

- [ ] **Step 2: Mount the provider above Header**

In `web/src/router.tsx`, import and wrap inside `PicoProvider` (so it can read `picoURL`) and around `ProfileCacheProvider`:

Replace the import block + Wrap. New file content:
```tsx
import { createRouter } from "@tanstack/react-router";
import * as TanstackQuery from "./integrations/tanstack-query/root-provider";
import { ConfigDraftProvider } from "./lib/config/draft-context";
import { PicoProvider } from "./lib/pico/context";
import { ProfileCacheProvider } from "./lib/pico/profile-cache";
import { ThemeProvider } from "./lib/theme/theme-provider";

// Import the generated route tree
import { routeTree } from "./routeTree.gen";

// Create a new router instance
export const getRouter = () => {
	const rqContext = TanstackQuery.getContext();

	const router = createRouter({
		routeTree,
		context: { ...rqContext },
		defaultPreload: "intent",
		Wrap: (props: { children: React.ReactNode }) => {
			return (
				<ThemeProvider>
					<TanstackQuery.Provider {...rqContext}>
						<PicoProvider>
							<ConfigDraftProvider>
								<ProfileCacheProvider>{props.children}</ProfileCacheProvider>
							</ConfigDraftProvider>
						</PicoProvider>
					</TanstackQuery.Provider>
				</ThemeProvider>
			);
		},
	});

	return router;
};
```

> `Header` is rendered by `__root.tsx`, which is inside `Wrap`, so it can call
> `useConfigDraft()` for the badge.

- [ ] **Step 3: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/lib/config/draft-context.tsx src/router.tsx && bunx @biomejs/biome check src/lib/config/draft-context.tsx src/router.tsx
```
Expected: clean exit 0.

- [ ] **Step 4: Commit**

```bash
git add web/src/lib/config/draft-context.tsx web/src/router.tsx
git commit -m "feat(web): config draft context (sessionStorage) + provider mount"
```

---

## Task 7: ConfigField component

**Files:**
- Create: `web/src/components/routes/config/ConfigField.tsx`

This renders one field from a def, bound to a TanStack Form field. The form type is
`Record<string, ConfigValue>` so dynamic keys work.

- [ ] **Step 1: Create the component**

`web/src/components/routes/config/ConfigField.tsx`:
```tsx
import type { ReactFormExtendedApi } from "@tanstack/react-form";
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

// The form holds a flat record of config values keyed by wire name.
// biome-ignore lint/suspicious/noExplicitAny: TanStack Form generics over a dynamic record.
type ConfigForm = ReactFormExtendedApi<Record<string, ConfigValue>, any>;

interface ConfigFieldProps {
	form: ConfigForm;
	def: ConfigFieldDef;
	disabled?: boolean;
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
				def.validate ? { onChange: ({ value }) => def.validate?.(value) } : undefined
			}
		>
			{(field) => {
				const errors = field.state.meta.errors.filter(Boolean);
				const hasError = errors.length > 0;
				const value = field.state.value as ConfigValue;

				let control: React.ReactNode;

				if (def.options) {
					control = (
						<Select
							value={value === null || value === undefined ? "" : String(value)}
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
							onCheckedChange={(c) => field.handleChange(c)}
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
							value={value === null || value === undefined ? "" : String(value)}
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
								<span className="text-xs text-muted-foreground shrink-0">
									{def.hexHint && typeof value === "number"
										? `0x${value.toString(16).toUpperCase()} · ${def.unit}`
										: def.unit}
								</span>
							)}
						</div>
						{control}
						<p className="text-xs text-muted-foreground leading-snug">
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
```

> If TanStack Form's exported generic type is not `ReactFormExtendedApi` in the
> installed version, fall back to typing `form` as
> `{ Field: (props: any) => React.ReactNode }` via a local interface, or
> `ReturnType<typeof useForm>`. The runtime usage (`form.Field`, `field.state`,
> `field.handleChange`, `field.handleBlur`, `field.state.meta.errors`) is stable
> across TanStack Form v1.

- [ ] **Step 2: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/components/routes/config/ConfigField.tsx && bunx @biomejs/biome check src/components/routes/config/ConfigField.tsx
```
Expected: clean exit 0.

- [ ] **Step 3: Commit**

```bash
git add web/src/components/routes/config/ConfigField.tsx
git commit -m "feat(web): generic metadata-driven ConfigField"
```

---

## Task 8: ConfigSection component

**Files:**
- Create: `web/src/components/routes/config/ConfigSection.tsx`

- [ ] **Step 1: Create the collapsible section**

`web/src/components/routes/config/ConfigSection.tsx`:
```tsx
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

// biome-ignore lint/suspicious/noExplicitAny: form generic, see ConfigField.
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
```

- [ ] **Step 2: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/components/routes/config/ConfigSection.tsx && bunx @biomejs/biome check src/components/routes/config/ConfigSection.tsx
```
Expected: clean exit 0.

- [ ] **Step 3: Commit**

```bash
git add web/src/components/routes/config/ConfigSection.tsx
git commit -m "feat(web): collapsible ConfigSection with advanced styling"
```

---

## Task 9: RebootDialog + UnsavedBar

**Files:**
- Create: `web/src/components/routes/config/RebootDialog.tsx`
- Create: `web/src/components/routes/config/UnsavedBar.tsx`

- [ ] **Step 1: RebootDialog**

`web/src/components/routes/config/RebootDialog.tsx`:
```tsx
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
						Your changes were written to the kiln, but they only take effect after
						a reboot. Reboot now? The connection will drop briefly while the Pico
						restarts.
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
```

- [ ] **Step 2: UnsavedBar**

`web/src/components/routes/config/UnsavedBar.tsx`:
```tsx
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
```

- [ ] **Step 3: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/components/routes/config/RebootDialog.tsx src/components/routes/config/UnsavedBar.tsx && bunx @biomejs/biome check src/components/routes/config/RebootDialog.tsx src/components/routes/config/UnsavedBar.tsx
```
Expected: clean exit 0.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/routes/config/RebootDialog.tsx web/src/components/routes/config/UnsavedBar.tsx
git commit -m "feat(web): reboot dialog + sticky unsaved-changes bar"
```

---

## Task 10: ConfigPage (compose) + route

**Files:**
- Create: `web/src/components/routes/config/ConfigPage.tsx`
- Create: `web/src/routes/config.tsx`

- [ ] **Step 1: Create the page component**

`web/src/components/routes/config/ConfigPage.tsx`:
```tsx
import { useForm } from "@tanstack/react-form";
import { useEffect, useMemo, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useConfigDraft } from "@/lib/config/draft-context";
import { buildPatch, SECTIONS, withDefaults } from "@/lib/config/schema";
import { useKilnConfig, useKilnStatus, useSaveConfig } from "@/lib/pico/hooks";
import type { ConfigValue, KilnConfig } from "@/lib/pico/types";
import { ConfigSection } from "./ConfigSection";
import { RebootDialog } from "./RebootDialog";
import { UnsavedBar } from "./UnsavedBar";

type FormValues = Record<string, ConfigValue>;

function toFormValues(config: KilnConfig): FormValues {
	return { ...config } as unknown as FormValues;
}

export function ConfigPage() {
	const configQuery = useKilnConfig();
	const { data: status } = useKilnStatus();
	const save = useSaveConfig();
	const { draft, isDirty, setDraft, clearDraft } = useConfigDraft();
	const [rebootOpen, setRebootOpen] = useState(false);

	const locked = status?.state === "RUNNING" || status?.state === "TUNING";

	// Default-filled server snapshot (the diff baseline).
	const serverConfig = useMemo<KilnConfig | null>(
		() => (configQuery.data ? withDefaults(configQuery.data) : null),
		[configQuery.data],
	);

	// Seed the form from server values merged with any persisted draft.
	const defaultValues = useMemo<FormValues>(() => {
		if (!serverConfig) return {};
		return { ...toFormValues(serverConfig), ...draft };
		// Only re-seed when the server snapshot changes; draft edits flow through
		// the form itself afterwards.
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [serverConfig]);

	const form = useForm({
		defaultValues,
		onSubmit: async ({ value }) => {
			if (!serverConfig) return;
			const patch = buildPatch(value as FormValues, serverConfig);
			if (Object.keys(patch).length === 0) return;
			await save.mutateAsync(patch);
			clearDraft();
			form.reset();
			setRebootOpen(true);
		},
	});

	// Mirror the live form diff into the draft context (persists + drives badges).
	const values = form.useStore((s) => s.values) as FormValues;
	useEffect(() => {
		if (!serverConfig) return;
		const patch = buildPatch(values, serverConfig);
		setDraft(patch);
	}, [values, serverConfig, setDraft]);

	const canSubmit = form.useStore((s) => s.canSubmit);
	const changeCount = Object.keys(draft).length;

	if (configQuery.isLoading) {
		return <p className="text-muted-foreground">Loading configuration…</p>;
	}

	if (configQuery.isError || !serverConfig) {
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
		<div className="space-y-6">
			<div className="space-y-1">
				<h1 className="text-2xl font-bold">Configuration</h1>
				<p className="text-sm text-muted-foreground">
					Device settings stored on the kiln. Changes are saved to flash and take
					effect after a reboot.
				</p>
			</div>

			{locked && (
				<Alert>
					<AlertTitle>Kiln is firing — configuration locked</AlertTitle>
					<AlertDescription>
						The kiln is currently {status?.state?.toLowerCase()}. Settings are
						read-only until it finishes. Stop the run to make changes.
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
							form.reset();
						}}
					/>
				)}
			</form>

			<RebootDialog open={rebootOpen} onOpenChange={setRebootOpen} />
		</div>
	);
}
```

> `form.reset()` after `clearDraft()` resets to the form's `defaultValues`. Since
> `defaultValues` re-seeds when `serverConfig` changes (after the save invalidates
> the query), the reset baseline updates correctly on the next render.

- [ ] **Step 2: Create the route**

`web/src/routes/config.tsx`:
```tsx
import { createFileRoute } from "@tanstack/react-router";
import { ConfigPage } from "@/components/routes/config/ConfigPage";
import { RequireConnection } from "@/components/RequireConnection";

export const Route = createFileRoute("/config")({
	component: ConfigRoute,
});

function ConfigRoute() {
	return (
		<div className="container max-w-7xl mx-auto py-4 sm:py-8 px-4">
			<RequireConnection>
				<ConfigPage />
			</RequireConnection>
		</div>
	);
}
```

- [ ] **Step 3: Generate route tree + format + verify**

Run (the dev/build step regenerates `routeTree.gen.ts`; do a one-off build to pick up the new route, or rely on the final task's build):
```bash
bunx @biomejs/biome check --write src/components/routes/config/ConfigPage.tsx src/routes/config.tsx && bunx @biomejs/biome check src/components/routes/config/ConfigPage.tsx src/routes/config.tsx
```
Expected: clean exit 0. (Type errors involving the new route's `routeTree.gen` entry are resolved in Task 12's build.)

- [ ] **Step 4: Commit**

```bash
git add web/src/components/routes/config/ConfigPage.tsx web/src/routes/config.tsx
git commit -m "feat(web): ConfigPage compose + /config route"
```

---

## Task 11: Header nav item + dirty badges

**Files:**
- Modify: `web/src/components/Header.tsx`

- [ ] **Step 1: Add the draft hook + a badge dot, and the Config nav link**

Edit `web/src/components/Header.tsx`:

1. Add import near the top (after the existing imports):
```tsx
import { useConfigDraft } from "@/lib/config/draft-context";
```

2. Inside `Header()`, after `const closeMenu = ...`, add:
```tsx
	const { isDirty } = useConfigDraft();
```

3. On the burger button (the `<button onClick={() => setIsOpen(true)} ...>` with `<Menu size={24} />`), add a relative wrapper dot. Replace the `<Menu size={24} />` line with:
```tsx
						<span className="relative">
							<Menu size={24} />
							{isDirty && (
								<span className="absolute -right-1 -top-1 size-2.5 rounded-full bg-primary ring-2 ring-card" />
							)}
						</span>
```

4. Add a Config nav link inside `<nav>`, after the Files `<Link>` (before `</nav>`):
```tsx
						<Link
							to="/config"
							onClick={closeMenu}
							className={navLinkClass}
							activeProps={{ className: navLinkActiveClass }}
						>
							<Settings size={20} />
							<span className="font-medium">Configuration</span>
							{isDirty && (
								<span className="ml-auto size-2.5 rounded-full bg-primary" />
							)}
						</Link>
```

> `Settings` is already imported in `Header.tsx`. `Link` is already imported.

- [ ] **Step 2: Format + verify**

Run:
```bash
bunx @biomejs/biome check --write src/components/Header.tsx && bunx @biomejs/biome check src/components/Header.tsx
```
Expected: clean exit 0.

- [ ] **Step 3: Commit**

```bash
git add web/src/components/Header.tsx
git commit -m "feat(web): Config nav item + unsaved-changes badges"
```

---

## Task 12: Full typecheck/build + manual smoke

**Files:** none (verification only)

- [ ] **Step 1: Biome check the whole project**

Run (from `web/`):
```bash
bunx @biomejs/biome check src
```
Expected: clean exit 0. Fix any reported issues, re-run.

- [ ] **Step 2: Type-check + build (regenerates route tree)**

Run:
```bash
bun run build
```
Expected: build succeeds with no TypeScript errors. The `/config` route now appears in `routeTree.gen.ts`. Common fixes if it fails:
- TanStack Form type import name (see Task 7 note).
- `form.useStore` selector typing — cast values as shown.

- [ ] **Step 3: Manual smoke (against a running Pico or `bun run dev`)**

Verify by hand (no automated tests per project convention):
1. Open `/config` — all 9 sections render; Advanced (Hardware, Network) start collapsed with red-tinted headers + warnings; others open.
2. Each field shows label + help; units/hex hint render; password masked with Show/Hide.
3. Change a field (e.g. `PID_KP_BASE`) → sticky bar appears with "1 unsaved change"; burger + Config nav show a dot.
4. Navigate to `/` then back → badge persists; edit restored from sessionStorage.
5. Enter an invalid value (e.g. `MAX_TEMP` = 99999, or a bad IP) → inline error; Save disabled.
6. Reload the tab → draft restored from sessionStorage.
7. Discard → form resets, badge clears, sessionStorage entry removed.
8. Save (valid diff) → POST contains only changed keys → reboot dialog appears → "Later" dismisses; "Reboot now" calls `/api/reboot`.
9. While kiln RUNNING/TUNING → lock banner shows, all fields disabled, no sticky bar.

- [ ] **Step 4: Final commit (if any fixes were made in steps 1-2)**

```bash
git add -A web
git commit -m "fix(web): config page typecheck/lint cleanup"
```

---

## Self-Review (completed during planning)

**Spec coverage:**
- New `/config` page → Tasks 10-11. ✓
- All options, clear text, sections, collapsible, grouped → Task 5 schema (9 sections, help per field) + Task 8. ✓
- Save → Tasks 3/4/10 (diff PATCH). ✓
- Draft in sessionStorage → Task 6. ✓
- Badge on burger + config icon when unsaved → Task 11. ✓
- Sticky bottom bar w/ Save + Discard CTAs, Config-page-only → Task 9 + Task 10. ✓
- Read-only while firing (RUNNING/TUNING) → Task 10 `locked`. ✓
- Form lib (TanStack Form) → Task 1 + usage Tasks 7/10. ✓
- Risky fields → Advanced collapsible + warnings + masked password → Tasks 5/7/8. ✓
- Reboot-to-apply CTA → Task 9 RebootDialog + Task 10. ✓

**Placeholder scan:** One intentional flag in Task 5 normalize() doc comment (bracketed `nulls`) with an explicit instruction to de-bracket. No other TBDs.

**Type consistency:** `KilnConfig`/`ConfigValue`/`SaveConfigResponse` defined in Task 2, used consistently in Tasks 3-10. `buildPatch`/`withDefaults`/`normalize`/`SECTIONS`/`ALL_FIELDS` defined in Task 5, used in Task 10. `useConfigDraft`/`ConfigDraftProvider` defined Task 6, used Tasks 10-11. `useKilnConfig`/`useSaveConfig` defined Task 4, used Task 10. `useReboot` pre-existing, used Task 9.
