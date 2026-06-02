# Pico-Kiln Mobile App — UX / UI Audit & Improvement Plan

> Scope: the `web/` React app (TanStack Router + Query, Tailwind v4, shadcn/ui,
> recharts, bundled to Android/desktop via Tauri). This is the primary mobile
> experience for controlling a physical kiln over Wi-Fi/HTTP to a Raspberry Pi Pico.
>
> Status: **Phase 1 implemented** ✅ (theming foundation + correctness). Phases 2–4 still pending.

## Decisions captured (from review)

- **Dark mode:** add a real theme provider **+ toggle** (respect system preference). Keep the existing `dark:` classes alive.
- **Visualization:** **full migration to [ui.bklit.com](https://ui.bklit.com/)** (visx + d3 + motion, shadcn-registry install).
- **Audience:** **makers / the author** — keep technical detail (PID, SSR, oscillations) available but tidier; progressive disclosure rather than hidden.
- **Now:** documentation only; implement in phases afterward.

---

## TL;DR

The app is **functionally complete and well-architected**, but it reads like a *desktop form ported to a phone*. Three systemic problems dominate:

1. **Dark mode is dead + theming is bypassed** by ~43 hardcoded color classes/hex values.
2. **Error handling is technical and half-silent** — raw `error.message` strings leak to users, and logical (`success:false`) failures are swallowed to `console.error`.
3. **Charts won't survive a real firing log** on a phone (no downsampling; animations on).

Plus mobile-ergonomics and data-loss issues (Profile Editor loses unsaved work; Fahrenheit half-broken). None require a rewrite.

---

## Stack snapshot

| Area | Detail |
|------|--------|
| Framework | React 19, TanStack Router (file-based), TanStack Query (+ persist) |
| Styling | Tailwind v4, shadcn/ui (new-york, zinc base), `tw-animate-css` |
| Charts | recharts v3 |
| Packaging | Tauri (Android min SDK 24, macOS), cleartext HTTP to Pico on LAN |
| Routes | `/` (control dashboard), `/toolbox` (visualizer / editor / files) |
| Data | `lib/pico/{client,hooks,context,profile-cache,types}`; smart polling by state |

Largest components: `ProfileEditor` (839), `KilnStatusDisplay` (606), `ProfileControls` (588), `hooks` (528), `FileManager` (523), `TuningPhasesVisualizer` (488).

---

## Findings

Grouped by severity. File:line references are approximate to the audited revision.

### 🔴 Critical — bugs, data loss, safety

| # | Issue | Where | Why it matters |
|---|-------|-------|----------------|
| C1 | **Dark mode is dead code.** Full `.dark` token set + ~30 `dark:` classes exist, but nothing ever applies the `.dark` class (no theme provider in `main.tsx`/`router.tsx`). App is permanently light; Header is hardcoded dark gray → inconsistent. | `styles.css:57`, `main.tsx`, `Header.tsx:13` | Half the styling is unused; inconsistent look |
| C2 | **`--destructive-foreground` equals `--destructive`** (red text on red bg). | `styles.css:37` | Destructive alerts/buttons can be unreadable |
| C3 | **Profile Editor loses unsaved work.** State is local `useState`; Radix Tabs unmount inactive content, so switching Toolbox tab (or back-nav / pull-to-refresh) wipes the profile. No nav guard, no confirm on import-overwrite, no confirm on step delete. | `ProfileEditor.tsx`, `routes/toolbox/index.tsx` | Data loss after long mobile editing |
| C4 | **Silent logical failures.** Mutations returning HTTP 200 + `{success:false}` only `console.error` — no UI feedback (start/stop profile, start/stop tuning); file download/read errors also silent. | `ProfileControls.tsx:147,158`, `TuningControls.tsx:64,75`, `FileManager` | User taps, nothing happens, no reason shown |
| C5 | **Charts have no downsampling + animations on.** A 12 h firing logged every 5 s ≈ 8,640 points rendered as raw SVG. | `csv-parser.ts:73`, `RunVisualizer.tsx:63`, `TuningPhasesVisualizer.tsx` | Real logs may freeze/crash the phone |
| C6 | **`return {} as T` on non-JSON responses.** Blank/non-JSON reply → object with no `state` → possible `Cannot read 'state' of undefined`, or a "Connected" badge with no data. | `client.ts:96` | Runtime fragility on a safety device |

### 🟠 Major — UX, correctness, trust

| # | Issue | Where | Why it matters |
|---|-------|-------|----------------|
| M1 | **Hardcoded colors instead of tokens (~43).** `bg-blue/purple/green-600`, `text-orange-500`, chart hex `#ef4444 #3b82f6 #eab308 #06b6d4`. Light-only combos (`bg-blue-50 text-blue-800`, `bg-purple-50`, `file:bg-blue-50`) break the moment dark mode is on. | `KilnStatusDisplay`, `ProfileControls.tsx:377`, `TuningControls.tsx:171`, `profile-utils.ts`, `RunVisualizer.tsx`, `FileSourceSelector.tsx:132` | Blocks dark mode; inconsistent palette |
| M2 | **Raw technical error strings to users.** `Failed to load kiln status: {error.message}` → `HTTP 500: …`, `Network request failed`, `Request timeout after 10000ms`; fallback `"Error mode"`; `Error: {error.message}` in connection details. | `KilnStatusDisplay.tsx:95,305`, `ConnectionStatus.tsx:86`, `client.ts:64,69,85` | No recovery guidance; scary jargon |
| M3 | **Fahrenheit half-broken.** Editor offers °F, but trajectory starts at `currentTemp=20` (°C room temp), rate labels hardcode `°C/h`, preview tooltip hardcodes `°C`; dashboard big temp readout always `°C`. | `profile-utils.ts:29`, `ProfileEditor.tsx:499,516,783`, `KilnStatusDisplay.tsx:177` | Contradictory units on a safety device |
| M4 | **`useReboot` treats *any* API error as success** ("Reboot initiated"), masking real failures (command never reached device). | `hooks.ts:188` | False confidence |
| M5 | **Current temperature isn't hero-sized.** `text-2xl` in a 2-col grid; no gauge/progress-to-target; no live temp-vs-time chart on the dashboard. | `KilnStatusDisplay.tsx:221-242` | The #1 glance value is buried |
| M6 | **No stale-data indicator.** Global `staleTime: 5min`; on disconnect the temp freezes at last value while only a small badge flips to "Connection Error". | `root-provider.tsx:26`, `ConnectionStatus.tsx` | Misleading "current" temp during dropout |
| M7 | **Editor default profile is born invalid** — Step 1 ramp has no `desired_rate`, which validation flags → red error on first open. | `ProfileEditor.tsx:52-60,129` | "App looks broken" first impression |
| M8 | **Native `alert()` for import errors** — blocking, off-brand vs the app's `<Alert>`. | `ProfileEditor.tsx:224,234` | Jarring on mobile |
| M9 | **Massive scroll editor.** 839-line single column; inputs are "miles" from the live preview chart; duration input `(/60).toFixed(0)` rounds away fractional minutes. | `ProfileEditor.tsx`, `:543` | Tedious iterative editing on a phone |
| M10 | **Mobile-hostile chart axes.** `position:"insideLeft"/"insideBottom"` labels overlap ticks at ~390px; three stacked charts (400+200+200px); no zoom/pan; no synchronized crosshair across temp/SSR/rate. | all visualizers | Hard to read & analyze on phone |

### 🟡 Minor — polish & consistency

| # | Issue | Where |
|---|-------|-------|
| m1 | **DevTools always bundled & rendered** (router + query), no `import.meta.env.DEV` gate. | `__root.tsx:21-32` |
| m2 | **Global `button{min-height/width:44px}`** overrides shadcn size variants and forces icon/inline buttons to ≥44px wide, distorting groups (e.g. ↑/↓ reorder). | `styles.css:146-150` |
| m3 | **Step reorder uses text `↑/↓` glyphs** in tiny ghost buttons rather than icons. | `ProfileEditor.tsx:409-427` |
| m4 | **Three scary red buttons when running** (Stop Profile / Emergency Shutdown / Reboot) with no inline distinction. | `ProfileControls.tsx` |
| m5 | **Component inconsistency:** native `<select>` in ProfileControls vs shadcn `Select` elsewhere; ProfileEditor reimplements import instead of reusing `FileSourceSelector` (losing its "paste content" feature). | `ProfileControls.tsx:307`, `ProfileEditor.tsx:288-387` |
| m6 | **Jargon shown raw:** "IDLE", "SSR Output", "Oscillations Detected", PID Kp/Ki/Kd always visible (vs progressive disclosure). | `KilnStatusDisplay.tsx:567-603`, `FileManager.tsx:156` |
| m7 | **No backdrop / tap-outside-to-close** on the nav drawer. | `Header.tsx:46` |
| m8 | **Inconsistent color semantics** — cooling = cyan in editor, blue elsewhere. | `profile-utils.ts:98`, `TuningPhasesVisualizer.tsx` |
| m9 | **`formatDuration(0)` returns "N/A"** (truthiness bug on `0`). | `KilnStatusDisplay.tsx:181` |
| m10 | **Weak profile validation** for a safety device — no max-temp ceiling, no rate>0 check, no out-of-range guards; import does `JSON.parse(...) as Profile` with no schema check. | `ProfileEditor.tsx:124-143,221` |
| m11 | **Dead variable** `_Icon` assigned but unused. | `Visualizer.tsx:42` |

### ✅ The Good (keep)

- Smart state-based polling (RUNNING 5 s / TUNING 2 s / IDLE 30 s / ERROR 15 s).
- File-cache persistence (`PersistQueryClientProvider`) → offline profile viewing.
- Confirm dialogs on Shutdown & Reboot; recovery-mode banner; ETA math (`step-utils.ts`).
- 44px touch targets + iOS/Android safe-area handling.

---

## Visualization — full bklit migration

**What bklit is:** an open-source **shadcn-registry** chart library (`bklit/bklit-ui`, ~771★, active, v2). Built on **visx + d3 + motion**, installed via `npx shadcn add …` (copy-paste components, not an npm dep wrapper). Uses **OKLCH + CSS variables** (ideal for Tailwind v4), responsive via `@visx/responsive`, with **touch tooltips, drag segment-selection, and event markers**.

**Why it fits a kiln app:**
- **Segment selection** → drag to zoom into a specific ramp/plateau to inspect PID stability.
- **Markers** → annotate kiln events ("target reached", "vent", "step change").
- **`var(--chart-*)` theming** → fixes the hardcoded-color problem and works with dark mode for free.
- **Live Line Chart** → ideal for a dashboard temp-vs-target view during firing.

**Costs / caveats:**
- Adds peer deps: `@visx/{shape,curve,scale,gradient,responsive,event,grid}`, `d3-array`, `motion`, `react-use-measure`.
- visx charts are more CPU-intensive than trivial SVGs — **still need downsampling** for long logs (migration does **not** remove this requirement).
- Confirm the license before shipping (registry is public; repo lists "Other/NOASSERTION").

**Target chart inventory after migration:**

| Current | Replace with | Add |
|---------|--------------|-----|
| Dashboard (none) | bklit **Live Line** | live temp + target, "now" marker |
| `ProfileVisualizer` | bklit Line/Area | segment colors via tokens, step markers |
| `RunVisualizer` (temp/SSR/rate stacked) | bklit composed/synced | shared crosshair, brush/segment-select |
| `TuningPhasesVisualizer` | bklit Area + reference regions | phase markers, animation off for large sets |

**Prerequisite for every chart:** add **LTTB downsampling** in `csv-parser.ts` (or a `useDownsample` hook) targeting ~500–1000 points; keep raw data for stats.

---

## Improvement plan (phased, no rewrite)

### Phase 1 — Foundation: theming & correctness (high value, low risk) — ✅ COMPLETED
1. ✅ Added a **theme provider + toggle** (`lib/theme/theme-provider.tsx`, `components/ThemeToggle.tsx`): light/dark/system, persists to `localStorage`, applies `.dark` to `documentElement` + sets `color-scheme`, follows the system preference live, and a pre-paint script in `index.html` prevents theme flash. `Header` re-themed to tokens (C1) and the toggle is mounted in it.
2. ✅ Replaced **all ~43 hardcoded colors** with **semantic tokens**. Added `--success/--warning/--info/--tuning` (+ `-foreground`) and kiln chart tokens `--chart-heating/hold/cooling/natural-cooling/ssr/rate` for both themes, registered via `@theme inline`; added `success/warning/info/tuning` variants to `ui/alert`. Fixed the `--destructive-foreground` contrast bug (C2). Light token lightness tuned to **pass WCAG AA** both as text-on-tint and as solid badge backgrounds.
3. ✅ **DevTools** (m1): production builds already strip all devtools via the `@tanstack/devtools-vite` plugin — verified the prod bundle contains **zero** devtools code. (An `import.meta.env.DEV` wrapper is incompatible with that plugin's transform, so it is intentionally not used.) **44px** rule scoped (m2): the global `button{min-height/width:44px}` override (which distorted icon/inline buttons) was replaced with an opt-in `.touch-target` utility; primary CTAs use `size="lg"` (44px) and header controls use `.touch-target`.
4. ✅ `return {} as T` → now throws a typed `PicoAPIError` on non-JSON responses (C6). `useReboot` now only treats a connection drop (`statusCode === undefined`, i.e. timeout/network) as success and rethrows real HTTP errors (M4). Verified against firmware: every endpoint returns `application/json`, so the strict path only triggers when the configured URL isn't the kiln.
5. ✅ **Fahrenheit** made consistent across the editor/preview/visualizers — trajectory room-temp baseline and natural-cooling fallback are unit-aware (68 °F / 20 °C), and all rate/temperature labels follow the profile's unit (M3). `DEFAULT_PROFILE` step 1 now has a valid `desired_rate` (M7). `formatDuration(0)` now returns `0s` instead of `N/A` (m9).

> **Notes / scope boundaries**
> - **Dashboard live readouts stay in °C.** `current_temp` comes from the MAX31856 (Celsius) and the `/api/status` payload has no unit field; the firmware also stores raw target values. A full Fahrenheit *display* on the live dashboard needs firmware coordination (unit field or server-side conversion) and is deferred.
> - **Charts still use recharts** (referencing the new `var(--chart-*)` tokens); the bklit migration remains Phase 4.

### Phase 2 — Error handling & trust
6. Central `getFriendlyError(err)` mapper with recovery hints; map states/jargon to friendly copy (makers still get detail) (M2, m6).
7. **Surface `success:false`** results + file errors as `<Alert>`s; replace `alert()` with `<Alert>` (C4, M8).
8. `staleTime: 0` for status + a **"last updated / stale"** indicator on the temperature readout; clear offline banner (M6).

### Phase 3 — Dashboard clarity (makers-tuned)
9. **Hero temperature** + progress-to-target ring/gauge; live mini temp-vs-time chart (M5).
10. De-duplicate `KilnStatusDisplay`'s four near-identical state blocks; move PID/SSR detail into an **"Advanced"** disclosure (open-by-default optional).
11. Clarify the running-state control cluster (Stop vs Emergency vs Reboot) (m4).

### Phase 4 — Editor for touch + bklit migration
12. Persist editor draft (context/localStorage) + **unsaved-changes guard**; confirm destructive actions (delete/import-overwrite); schema-validate imports; overwrite warning on upload (C3, m10).
13. Split editor into collapsible sections / **sticky live preview**; icon reorder buttons; fix duration rounding (M9, m3).
14. Reuse `FileSourceSelector` in the editor (m5); unify cooling color semantics (m8); remove dead code (m11).
15. **Migrate all charts to bklit**, add **LTTB downsampling**, token colors, mobile axis fixes, shared crosshair, markers, `isAnimationActive` off for large sets (C5, M1, M10).

---

## Suggested sequencing

Phase 1 first — it unblocks dark mode and every later color/chart change, and is the lowest-risk batch. Then 2 → 3 → 4. The bklit migration (Phase 4) depends on Phase 1 tokens being in place so charts inherit theme colors cleanly.
