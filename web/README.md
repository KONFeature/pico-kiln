# pico-kiln app

The control app for the [pico-kiln](../README.md) kiln controller. Runs as a web
page, a desktop app, or an Android app (same codebase, packaged with Tauri), and
talks to whichever firmware is running over its HTTP API.

What it does:

- **Live monitoring** — current temperature, SSR state, and firing progress.
- **Firing profiles** — create, edit, and upload ramp/hold schedules.
- **PID tuning** — run an auto-tune on the kiln and analyze the result in-app,
  with suggested PID values (`/analyze`).

## Install (end users)

- **Desktop (macOS / Windows / Linux) or Android:** download the build for your
  platform from the [latest release](../../../releases/latest), launch it, and enter
  your kiln's IP address.
- **No install:** the firmware serves a built-in UI directly at the device IP —
  just browse to it.

The kiln's IP is shown on the LCD (if fitted), or use the provisioning addresses
from first-time setup (see the [root README](../README.md#install-the-app)).

## Develop

```bash
bun install          # or npm install
bun run dev          # dev server on http://localhost:3000
```

| Command | Does |
|---------|------|
| `bun run test` | Vitest tests |
| `bun run check` | Biome lint + format check |
| `bun run lint` / `bun run format` | Biome lint / format |
| `bun run build` | production web build |
| `bun run tauri:build` | desktop app |
| `bun run tauri:android:build` | Android APK/AAB |

Packaging desktop and Android builds (signing, output paths) is documented in
[DESKTOP_MOBILE_BUILD.md](./DESKTOP_MOBILE_BUILD.md).

## Stack

React 19 + TypeScript, [TanStack Router](https://tanstack.com/router) +
[Query](https://tanstack.com/query), [Tailwind CSS v4](https://tailwindcss.com/),
[Biome](https://biomejs.dev/) for lint/format, and
[Tauri](https://tauri.app/) for desktop/mobile packaging. Source in `src/`,
native shell in `src-tauri/`.
