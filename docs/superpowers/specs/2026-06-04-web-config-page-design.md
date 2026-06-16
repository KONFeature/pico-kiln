# Web Config Page — Design Spec

**Date:** 2026-06-04
**Status:** Approved (design phase)
**Topic:** Surface the Rust firmware's kiln configuration in the `web/` React app.

## Problem

The Rust firmware (`rust/`) replaced the MicroPython runtime. The Pico is no longer
REPL-accessible over USB for editing files — the only I/O is over LAN. The firmware
exposes two routes for this:

- `GET /api/config` — returns the full running `KilnConfig` as JSON.
- `POST /api/config` — sparse PATCH merge of supplied keys, persisted to flash,
  applied on next reboot.

Neither is surfaced in the web app. This spec adds a **Config** page that surfaces
every configuration option with clear descriptions, validation, grouped collapsible
sections, a draft/unsaved-changes system, and a reboot-to-apply flow.

## Firmware facts (constraints we build against)

- **Wire format:** all field names are `UPPER_SNAKE_CASE` (e.g. `PID_KP_BASE`,
  `WIFI_SSID`). Verified in `rust/kiln-app/src/config.rs`.
- **GET `/api/config`** → 200, full config JSON. `LCD_I2C_*` keys are omitted when
  `lcd_enabled == false`.
- **POST `/api/config`** → sparse PATCH: only present keys are applied; absent keys
  unchanged. Body limit **2048 bytes** (`MAX_JSON_BODY`). Responses:
  - 200 `{"success":true,"message":"Config saved. Reboot to apply."}`
  - 400 `{"success":false,"error":"Invalid configuration"}` (bad JSON / bad value /
    string too long / >10 SSR pins)
  - 500 `{"success":false,"error":"Failed to save config"}`
- **POST `/api/reboot`** → 200 `{"success":true,"message":"Rebooting Pico..."}`; the
  firmware signals a one-shot reset.
- **No server-side firing guard on config writes** — the lock-while-firing behavior is
  a **client-side UX decision** in this spec.
- **Changes apply on reboot only.** The running config is read-only after boot.

## Decisions (from brainstorming)

1. **Read-only while firing** when kiln state is `RUNNING` **or** `TUNING`. Editable in
   `IDLE` / `COMPLETE` / `ERROR`.
2. **Post-save flow:** show success, then offer a **"Reboot now"** CTA (calls
   `POST /api/reboot`) plus a "Later" dismiss.
3. **Form library:** add **`@tanstack/react-form`**.
4. **Risky fields** (GPIO pins, WiFi creds, web host/port) live in **collapsed
   "Advanced" sections** with inline warnings; WiFi password is masked with a reveal
   toggle.
5. **Rendering:** **metadata-driven** — a declarative field schema drives a generic
   renderer (not 45 hand-coded fields).
6. **Sticky unsaved-changes bar:** rendered **on the Config page only**. Dirty-state
   **badges** appear globally (burger icon + Config nav item) so the user knows to
   return to the page.

## Architecture

### Metadata-driven schema (`web/src/lib/config/schema.ts`)

A single source of truth: an ordered array of **section** descriptors, each holding an
array of **field** descriptors.

```ts
type FieldType =
  | "number" | "text" | "password" | "select" | "switch" | "pinlist" | "ip";

interface ConfigFieldDef {
  key: string;            // UPPER_SNAKE wire name
  label: string;          // human label
  help: string;           // clear description shown under the field
  type: FieldType;
  unit?: string;          // e.g. "°C", "s", "Hz", "GPIO"
  min?: number; max?: number; step?: number;   // number/pinlist bounds
  options?: { value: string; label: string }[]; // select
  optional?: boolean;     // nullable (WiFi static net fields → null when blank)
  validate?: (value, allValues) => string | undefined; // extra rules
}

interface ConfigSectionDef {
  id: string;
  title: string;
  description?: string;
  advanced?: boolean;     // collapsed by default + warning banner
  defaultOpen?: boolean;
}
```

A generic `<ConfigField def={...}/>` maps `type` → the right shadcn control
(`Input`, `Select`, `Switch`, etc.) and wires it to a TanStack Form field, rendering
`label`, `help`, `unit`, and inline validation error.

### Sections & field assignment

Open by default:

1. **Temperature & Sensor** — `THERMOCOUPLE_TYPE`, `TEMP_UNITS`, `THERMOCOUPLE_OFFSET`,
   `MAINS_FREQUENCY`, `THERMOCOUPLE_AVERAGING`, `TEMP_MEDIAN_WINDOW`
2. **PID & Thermal Model** — `PID_KP_BASE`, `PID_KI_BASE`, `PID_KD_BASE`, `THERMAL_H`,
   `THERMAL_T_AMBIENT`
3. **SSR / Power Control** — `SSR_CYCLE_TIME`, `SSR_STAGGER_DELAY`
4. **Safety & Stall Detection** — `MAX_TEMP`, `STALL_CHECK_INTERVAL`,
   `STALL_CONSECUTIVE_FAILS`, `STALL_MIN_STEP_TIME`, `RATE_MEASUREMENT_WINDOW`,
   `RATE_RECORDING_INTERVAL`, `MAX_RECOVERY_TEMP_DELTA`
5. **Control Loop Timing** — `TEMP_READ_INTERVAL`, `PID_UPDATE_INTERVAL`,
   `STATUS_UPDATE_INTERVAL`, `SSR_UPDATE_INTERVAL`
6. **Logging & Watchdog** — `LOGGING_INTERVAL`, `ENABLE_WATCHDOG`, `WATCHDOG_TIMEOUT`
7. **Display (LCD)** — `LCD_I2C_ID`, `LCD_I2C_SCL`, `LCD_I2C_SDA`, `LCD_I2C_FREQ`,
   `LCD_I2C_ADDR`

Collapsed by default + ⚠️ warning ("Wrong values here can cut the Pico off the network
or break sensor reads — only fixable by re-flashing over USB"):

8. **Advanced — Hardware (GPIO)** — `MAX31856_SPI_ID`, `MAX31856_SCK_PIN`,
   `MAX31856_MOSI_PIN`, `MAX31856_MISO_PIN`, `MAX31856_CS_PIN`, `SSR_PIN` (pin-list)
9. **Advanced — Network** — `WIFI_SSID`, `WIFI_PASSWORD` (masked), `WIFI_STATIC_IP`,
   `WIFI_SUBNET`, `WIFI_GATEWAY`, `WIFI_DNS`, `WEB_SERVER_HOST`, `WEB_SERVER_PORT`

### Field reference (label / type / unit / bounds / help)

| Key | Type | Unit | Bounds | Help (summary) |
|-----|------|------|--------|----------------|
| THERMOCOUPLE_TYPE | select | — | B,E,J,K,N,R,S,T,G8,G32 | Thermocouple alloy type. Default K. |
| TEMP_UNITS | select | — | c,f | Display units; control loop is always Celsius. |
| THERMOCOUPLE_OFFSET | number | °C | -50..50 | Calibration offset added to readings. |
| MAINS_FREQUENCY | select | Hz | 50,60 | AC mains freq for SSR zero-cross / noise filter. |
| THERMOCOUPLE_AVERAGING | select | samples | 1,2,4,8,16 | MAX31856 hardware averaging (powers of two). |
| TEMP_MEDIAN_WINDOW | number | samples | 1..15 | Median filter window for smoothing. |
| PID_KP_BASE | number | — | ≥0 | Proportional gain (base). |
| PID_KI_BASE | number | — | ≥0 | Integral gain (base). |
| PID_KD_BASE | number | — | ≥0 | Derivative gain (base). |
| THERMAL_H | number | — | ≥0 | Thermal-model efficiency term (0 = off). |
| THERMAL_T_AMBIENT | number | °C | -20..60 | Ambient temp for thermal model. |
| SSR_CYCLE_TIME | number | s | 1..120 | SSR PWM duty-cycle period. |
| SSR_STAGGER_DELAY | number | s | 0..5 | Inrush delay between multi-relay activation. |
| MAX_TEMP | number | °C | 100..1400 | Hard safety ceiling → ERROR if exceeded. |
| STALL_CHECK_INTERVAL | number | s | ≥10 | How often heating rate is checked. |
| STALL_CONSECUTIVE_FAILS | number | count | 1..20 | Failed rate-checks before stall error. |
| STALL_MIN_STEP_TIME | number | s | ≥0 | Min time in step before stall logic applies. |
| RATE_MEASUREMENT_WINDOW | number | s | ≥60 | Window for avg heating/cooling rate. |
| RATE_RECORDING_INTERVAL | number | s | ≥1 | Sample interval for rate calc. |
| MAX_RECOVERY_TEMP_DELTA | number | °C | 0..200 | Max temp drop before crash-recovery declines. |
| TEMP_READ_INTERVAL | number | s | 0.1..10 | Outer control-tick / temp-read period. |
| PID_UPDATE_INTERVAL | number | s | 0.1..10 | PID update interval. |
| STATUS_UPDATE_INTERVAL | number | s | 1..60 | Status broadcast cadence. |
| SSR_UPDATE_INTERVAL | number | s | 0.05..5 | SSR sub-tick period. |
| LOGGING_INTERVAL | number | s | 1..600 | CSV log row write interval. |
| ENABLE_WATCHDOG | switch | — | bool | Enable hardware watchdog. |
| WATCHDOG_TIMEOUT | number | ms | 1000..30000 | Watchdog reset timeout. |
| LCD_I2C_ID | number | — | 0..1 | I2C peripheral id for LCD. |
| LCD_I2C_SCL | number | GPIO | 0..29 | I2C clock pin. |
| LCD_I2C_SDA | number | GPIO | 0..29 | I2C data pin. |
| LCD_I2C_FREQ | number | Hz | 10000..1000000 | I2C bus frequency. |
| LCD_I2C_ADDR | number | hex | 0..127 | LCD I2C slave address (e.g. 0x27=39). |
| MAX31856_SPI_ID | number | — | 0..1 | SPI peripheral id. |
| MAX31856_SCK_PIN | number | GPIO | 0..29 | SPI clock pin. |
| MAX31856_MOSI_PIN | number | GPIO | 0..29 | SPI MOSI pin. |
| MAX31856_MISO_PIN | number | GPIO | 0..29 | SPI MISO pin. |
| MAX31856_CS_PIN | number | GPIO | 0..29 | SPI chip-select pin. |
| SSR_PIN | pinlist | GPIO | 0..29, ≤10 entries | One or more SSR control pins. |
| WIFI_SSID | text | — | ≤64 chars | WiFi network name. |
| WIFI_PASSWORD | password | — | ≤64 chars | WiFi password (masked, reveal toggle). |
| WIFI_STATIC_IP | ip | — | optional | Static IPv4; blank → DHCP. |
| WIFI_SUBNET | ip | — | optional | Subnet mask; blank → DHCP. |
| WIFI_GATEWAY | ip | — | optional | Gateway; blank → DHCP. |
| WIFI_DNS | ip | — | optional | DNS server; blank → DHCP. |
| WEB_SERVER_HOST | text | — | IP/host | Server bind address (default 0.0.0.0). |
| WEB_SERVER_PORT | number | — | 1..65535 | HTTP server port. |

> `LCD_*` may be absent in the GET response. When absent, the Display section shows an
> "LCD not configured" enable toggle; enabling reveals the fields with firmware defaults
> (id 0, SCL 21, SDA 20, freq 100000, addr 39). The five LCD keys are sent together.

### Hex display for `LCD_I2C_ADDR`

Stored/sent as a decimal number (firmware expects a number). The field shows a hex hint
("0x27") next to the input; value is a plain integer 0–127.

## Data layer

### Types (`web/src/lib/pico/types.ts`)

Add `KilnConfig` interface with all keys above, typed:
`SSR_PIN: number[]`, WiFi static fields `string | null`, enums as string unions,
`ENABLE_WATCHDOG: boolean`, rest `number`/`string`.

### API client (`web/src/lib/pico/client.ts`)

- `getConfig(): Promise<KilnConfig>` → `GET /api/config`.
- `saveConfig(patch: Partial<KilnConfig>): Promise<{success:boolean;message?:string;error?:string}>`
  → `POST /api/config`, JSON body = **only changed keys**.
- `reboot(): Promise<{success:boolean;message?:string}>` → `POST /api/reboot`.

### Hooks (`web/src/lib/pico/hooks.ts`)

- `picoKeys.config = ["pico","config"]`.
- `useKilnConfig()` — query, `enabled: isConfigured`, `staleTime` modest (config rarely
  changes); used to seed the form.
- `useSaveConfig()` — mutation; on success invalidates `picoKeys.config` and clears the
  draft.
- `useReboot()` — mutation.

### Save = diff only

The save payload is the **draft diff** (changed keys vs. the server snapshot). Matches
the firmware's sparse PATCH and keeps the body far under the 2048-byte limit. A guard
warns if a serialized payload would exceed ~1900 bytes (defensive; realistically never
hit with a diff).

## Draft / unsaved-changes system

### `ConfigDraftContext` (`web/src/lib/config/draft-context.tsx`)

Follows the app's existing Context pattern (no zustand). State:

- `serverConfig: KilnConfig | null` — last fetched snapshot.
- `draft: Partial<KilnConfig>` — changed keys only.
- Derived `isDirty = Object.keys(draft).length > 0`.
- Actions: `setField(key, value)`, `resetField(key)`, `discardAll()`, `commit()` (clears
  draft after a successful save), `hydrate(serverConfig)`.

**Persistence:** `draft` is persisted to **`sessionStorage`** under a key scoped to the
current `picoURL` (e.g. `pico-kiln-config-draft:<url>`), so a draft for one kiln does not
leak to another and clears when the tab closes.

**Dirty semantics:** `setField` compares the new value to `serverConfig`. If equal, the
key is removed from the draft (returning a field to its saved value un-dirties it). Only
**valid** values are written to the draft (the field component validates first), so the
draft is always safe to POST.

Provider mounts in the root layout (alongside `PicoProvider`) so badges can read dirty
state from any page.

### Badges (global)

`web/src/components/Header.tsx`:
- Small dot on the **burger menu icon** when `isDirty`.
- Small dot on the **Config nav item** (with new `Settings` lucide icon) when `isDirty`.

### Sticky bar (Config page only)

`web/src/components/routes/config/UnsavedBar.tsx`, rendered inside the Config page when
`isDirty` and not firing:
- Text: "You have unsaved configuration changes."
- **Save** → `useSaveConfig().mutate(draft)` → on success show reboot dialog.
- **Discard** → `discardAll()`.
- Sticky to viewport bottom, respects mobile safe-area.

## Read-only while firing

Config page calls `useKilnStatus()`. If `state === "RUNNING" || state === "TUNING"`:
- Show an info banner: "Kiln is firing — configuration is locked until it finishes."
- All fields `disabled`; sticky bar hidden; Save/Discard unavailable.
- The form still renders current values (read-only view).

## Post-save reboot flow

On `useSaveConfig` success:
1. `commit()` (clear draft), invalidate config query.
2. Open a dialog: "Saved. A reboot is required to apply." with **Reboot now**
   (`useReboot().mutate()`) and **Later**.
3. On reboot: show "Rebooting…"; the connection will drop and re-poll via existing
   status query behavior.

## Validation

Client validators mirror firmware constraints (bounds table above) plus:
- Enums: `THERMOCOUPLE_TYPE`, `TEMP_UNITS`, `MAINS_FREQUENCY` constrained by select.
- Strings ≤ 64 chars (`WIFI_*`).
- `SSR_PIN`: 1–10 integer pins, each 0–29, no duplicates.
- `ip` fields: empty (→ `null`) or valid IPv4 dotted-quad.
- Numbers: range + integer-vs-float per field.
- A field failing validation blocks Save and shows an inline error; it is not written to
  the draft.

## Routing & navigation

- New route `web/src/routes/config.tsx` → `createFileRoute("/config")`, component wrapped
  in `RequireConnection` (needs a kiln URL).
- `Header.tsx`: add `Settings` nav `Link to="/config"` with badge.

## File inventory

**New**
- `web/src/routes/config.tsx`
- `web/src/components/routes/config/ConfigPage.tsx`
- `web/src/components/routes/config/ConfigSection.tsx`
- `web/src/components/routes/config/ConfigField.tsx`
- `web/src/components/routes/config/AdvancedWarning.tsx`
- `web/src/components/routes/config/UnsavedBar.tsx`
- `web/src/components/routes/config/RebootDialog.tsx`
- `web/src/lib/config/schema.ts`
- `web/src/lib/config/validators.ts`
- `web/src/lib/config/draft-context.tsx`

**Edited**
- `web/src/components/Header.tsx` (nav item + badges)
- `web/src/lib/pico/client.ts` (getConfig/saveConfig/reboot)
- `web/src/lib/pico/hooks.ts` (config + reboot hooks, query key)
- `web/src/lib/pico/types.ts` (`KilnConfig`)
- `web/src/router.tsx` (mount `ConfigDraftProvider`)
- `web/package.json` (+`@tanstack/react-form`)

May add shadcn `ui/switch.tsx` if not present (check before adding).

## Testing

**Skipped** — the web app UI has no existing test harness/convention for this area, so
no tests are added (per project convention and user decision). Verification is via
`bun run check` (Biome) + `bun run build` (typecheck) and manual smoke test against a
running Pico.

## Out of scope

- Editing the kiln connection URL (already handled by `PicoConnectionConfig`).
- Firmware changes (no server-side firing guard added; client-side lock only).
- Import/export of config files.
- Live (no-reboot) application of config.
