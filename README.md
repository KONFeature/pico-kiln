# pico-kiln

A kiln controller for the Raspberry Pi Pico 2 (RP2350): MAX31856 thermocouple
input, PID control with thermal modeling / gain scheduling, SSR output, and a
web interface for monitoring, firing-profile management, and PID auto-tuning.

Greatly inspired by: https://github.com/jbruce12000/kiln-controller

## Repository layout

The firmware exists in two implementations. **Rust is the current/primary
firmware**; the original MicroPython implementation is kept under `python/`.

```
pico-kiln/
├── rust/        # Current firmware — Rust + Embassy (RP2350). See rust/ARCHITECTURE.md
├── python/      # Original firmware — MicroPython. See python/README.md
├── web/         # Web/desktop/mobile app — React + TypeScript + Tauri. See web/README.md
├── scripts/     # Offline analysis tools (Python 3) — shared by both firmwares
├── profiles/    # Firing profiles (JSON) — shared, deployed to either firmware
├── static/      # Embedded web assets served/baked by both firmwares
├── README.md    # This file
├── CLAUDE.md    # Guidance for AI assistants
└── AGENTS.md    # Agent/contributor notes
```

- `scripts/`, `profiles/`, and `static/` are **shared**: the analysis scripts
  read CSV logs produced by either firmware, the profiles fire on either, and
  both firmwares embed/serve the same `static/` HTML.

## Which firmware?

- **Rust** (`rust/`) — current target. Build/flash and architecture docs live in
  `rust/` (`rust/ARCHITECTURE.md`, `rust/TESTING.md`).
- **Python / MicroPython** (`python/`) — the original implementation and its
  tuning/thermal-model docs (`python/docs/`). See `python/README.md` for setup,
  deploy, and the full feature guide.

## Flash the firmware

**Quick (recommended).** Download `kiln-firmware.uf2` from the
[latest release](../../releases/latest), then:

1. Hold the **BOOTSEL** button while plugging the Pico 2 W into USB.
2. It appears as an `RPI-RP2` drive — drag the `.uf2` onto it.
3. It reboots running the firmware.

Each release ships two images: `kiln-firmware.uf2` (normal) and
`kiln-firmware-debug.uf2` (same image + on-device diagnostics — stack high-water
logging and fault capture; flash this when reporting a bug). They're
interchangeable; flash either the same way.

**From source.** Needs the Rust + Arm toolchain (see
[`rust/README.md`](rust/README.md#prerequisites)):

```bash
cd rust
./scripts/deploy.sh          # build + flash a Pico in BOOTSEL mode
```

Reflashing **preserves** your config, profiles, and run logs (separate flash
partition). On first boot there are no WiFi credentials yet — reach the UI over
USB (`http://192.168.7.1`) or join the open `pico-kiln-setup` Wi-Fi
(`http://192.168.4.1`), set your network, and reboot. Details:
[ARCHITECTURE.md §6](rust/ARCHITECTURE.md#6-provisioning-usb-ncm--fallback-softap).

## Install the app

The app monitors and controls the kiln (live temperature/SSR/progress, firing
profiles, PID auto-tune analysis) over the firmware's HTTP API.

- **Desktop / Android:** download the build for your platform from the
  [latest release](../../releases/latest) and point it at your kiln's IP address.
- **Browser:** the firmware also serves a built-in UI directly at the device IP —
  no install needed.

Run from source instead:

```bash
cd web
bun install
bun run dev          # http://localhost:3000
```

React + TanStack + Tailwind, packaged for desktop/Android with Tauri. See
`web/README.md`.

## Offline analysis (shared)

Python 3 tools in `scripts/` analyze CSV logs pulled from the kiln (matplotlib /
numpy / pandas). Run from the repo root:

```bash
python scripts/plot_run.py logs/run_*.csv
python scripts/analyze_pid_performance.py logs/run_*.csv
python scripts/analyze_heat_loss.py logs/firing.csv --volume 50 --power 5000
```

The MicroPython firmware's `dump_logs.sh` downloads logs into `scripts/logs/`
for these tools.

## Safety

⚠️ This controller drives high-temperature equipment that can start fires.

- Always supervise kiln operation and ensure proper ventilation
- Test thoroughly before any unattended operation; keep fire suppression nearby
- Follow local electrical/fire codes; never exceed your kiln's rated temperature
- Use a thermocouple rated for your maximum temperature

## License

PolyForm Noncommercial License 1.0.0 — personal, educational, and research use;
no commercial use. See [LICENSE](LICENSE). Contact the maintainer for commercial
licensing.

## Acknowledgments

- Inspired by [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller)
- PID methods from classical control theory (Ziegler-Nichols, Cohen-Coon, AMIGO)
