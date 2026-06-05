# USB-NCM + SoftAP provisioning — design

**Date:** 2026-06-05
**Crates touched:** `kiln-firmware` (most), `kiln-app` (one helper)
**Status:** approved architecture, pending implementation

## Problem

Chicken-and-egg provisioning. The only way to set `config.json` (incl. WiFi
credentials) is the web API, but the web API is only reachable over WiFi, which
needs `config.json`. A fresh board is unreachable. We also want to browse/edit
all device files (config, profiles, run logs) over USB once plugged in.

## Decision summary

| Fork | Decision |
|---|---|
| USB-NCM (USB ethernet) | **Always on** whenever the cable is enumerated. Radio-independent. |
| SoftAP | **Fallback only** — active *only when unconfigured* (SSID empty or `"your_wifi_ssid"`). cyw43 cannot run STA + AP at once. |
| Configured but can't join saved WiFi | Retry STA **forever** (today's behaviour). USB-NCM is the wired escape hatch. No AP for configured boards (avoids AP-flapping). |
| Addressing | Tiny DHCP **server** (`edge-dhcp`) on AP + NCM stacks. Device at a fixed IP. No captive-portal DNS. |
| AP security | **Open** AP (`start_ap_open`). Documented exposure — see Risks. |
| Files over USB/AP | Reuse the **existing** router (`/api/config`, `/api/files/*`). No new endpoints. |

## Why this is safe where USB-MSC was not

Serving files over HTTP reuses `FlashStorage`, so every write still goes through
the existing `flash_handshake` (Core 1 de-energises the SSR and parks while Core 0
programs flash). No host-side block cache, no littlefs-readability problem, no
concurrent-write corruption — the three killers of the MSC approach.

## Architecture

At most **two embassy-net stacks are live at once** (the radio is one or the
other, never both):

```
                 Core 0 executor
  ┌────────────────────────────────────────────────┐
  │  cyw43 stack  ──  EITHER  STA   (configured)     │  same radio,
  │                   OR      SoftAP (unconfigured)  │  mutually exclusive
  │        └─ web_task workers ─┐                    │
  │                             ├── one shared       │
  │  usb-ncm stack (always on)  │     AppRouter      │
  │        └─ web_task workers ─┘   (make_app)       │
  │                                                  │
  │  dhcp_server_task on each non-STA stack          │
  │  (USB-NCM always; cyw43 stack only in AP mode)   │
  └────────────────────────────────────────────────┘
        file/config writes → flash_handshake → Core 1
```

### Boot state machine (`core0_main`)

```
mount flash, load config        (unchanged)
gate on Core 1 READY            (unchanged)
crash recovery + CSV logger     (unchanged, network-independent)
init_cyw43()  → (net_device, control)         [shared bring-up]
needs_setup = !config.wifi_is_configured()
if needs_setup:
    cyw_stack = init_softap(net_device, control, spawner)   # open AP, 192.168.4.1/24, DHCP
else:
    cyw_stack = init_sta(net_device, control, config)       # join-forever, DHCP/static (today)
usb_stack = init_usb_ncm(spawner, p.USB)                    # always; 192.168.7.1/24, DHCP
spawn web_task workers on cyw_stack AND usb_stack
if !needs_setup:
    spawn wifi_monitor_task + ntp_task          # STA only
spawn reboot_task                               (unchanged)
```

`wifi_is_configured()` ⇔ `wifi_ssid` is non-empty **and** ≠ `"your_wifi_ssid"`.

### Addressing

| Stack | Device IP | Subnet | DHCP pool (served) |
|---|---|---|---|
| USB-NCM | 192.168.7.1 | /24 | 192.168.7.2 – .15 |
| SoftAP | 192.168.4.1 | /24 | 192.168.4.2 – .15 |
| STA | DHCP client, or static (`WIFI_STATIC_IP`) | — | n/a (joins existing AP) |

No captive-portal DNS. User browses to the device IP directly (documented in
README). NCM gateway/DNS handed out = the device IP.

## Components

### `kiln-firmware/src/platform.rs`

- **`init_cyw43(spawner, p) -> (NetDevice, Control)`** — refactor: extract the
  blob load + PIO SPI + `cyw43::new` + `cyw43_task` spawn + `control.init(clm)` +
  `set_power_management` that `init_network` does today, so STA and AP share it.
- **`init_sta(net_device, control, config, spawner) -> Stack`** — today's
  `init_network` tail: build the embassy-net stack (DHCP or static), presence
  scan, join-forever loop, `wait_config_up`. Returns the stack. `control` is moved
  on to `wifi_monitor_task` by the caller as today.
- **`init_softap(net_device, control, spawner) -> Stack`** — static IPv4 config
  192.168.4.1/24, `embassy_net::new`, spawn net runner, `control.start_ap_open(
  AP_SSID, AP_CHANNEL)`, spawn `dhcp_server_task`. `AP_SSID = "pico-kiln-setup"`,
  `AP_CHANNEL` per device-verify (commonly 5 or 6).
- **`init_usb_ncm(spawner, usb) -> Stack`** — `embassy_rp::usb::Driver`,
  `embassy_usb::Builder`, `CdcNcmClass`, `into_embassy_net_device`, static
  192.168.7.1/24 embassy-net stack; spawn the USB device `run()` task, the NCM
  runner task, the net runner task, and `dhcp_server_task`. Always called.
- **`dhcp_server_task(stack, gateway, pool_start, pool_len)`** — `edge-dhcp`
  server over an `edge-nal-embassy` UDP socket, leasing addresses in the subnet.
  One instance per non-STA stack.
- Bind `USBCTRL_IRQ` in the existing `bind_interrupts!` block.

Exact crate versions + signatures (`CdcNcmClass::new`, `into_embassy_net_device`,
`start_ap_open`, `edge-dhcp` server loop, the USB `InterruptHandler`) are being
device-verified against the embassy 0.10 / embassy-net 0.9 tag — annotated in code
the same way the existing `sntpc`/`littlefs2` bindings are.

### `kiln-firmware/src/main.rs`

- Add `usb: p.USB` to the Core 0 peripheral hand-off (new field on a periph
  struct, taken before the executor starts).
- In `core0_main`: implement the boot state machine above. Spawn the `web_task`
  pool on both stacks (see pool sizing below). Spawn `wifi_monitor`/`ntp` only in
  STA mode.

### `kiln-app/src/server.rs` / `config.rs`

- `KilnConfig::wifi_is_configured(&self) -> bool` (host-testable, in `config.rs`).
- `web_task` is unchanged (already takes a `stack` arg). Pool sizing: it is
  `#[task(pool_size = WEB_TASK_POOL_SIZE)]` and the macro caps *total* live
  instances. To serve N workers on each of two stacks, raise the budget:
  `WEB_TASK_POOL_SIZE` stays the per-stack worker count (2); introduce
  `WEB_TASK_TOTAL = WEB_TASK_POOL_SIZE * MAX_STACKS` (2 × 2 = 4) as the macro's
  `pool_size`, and spawn `WEB_TASK_POOL_SIZE` instances per stack with unique
  `id`s. `NET_SOCKETS` accounting stays per-stack (in firmware).
- Router (`make_app`) unchanged — already serves config + files on any stack.

### New firmware dependencies

`embassy-usb`, `embassy-net-driver-channel`, `edge-dhcp`, `edge-nal-embassy`
(exact versions from the API-verification pass). Firmware crate only; `kiln-app`
gains no new deps.

## Data flow

1. **Plug USB (any mode):** host enumerates a USB-NCM NIC → device DHCP hands it
   192.168.7.x → browse `http://192.168.7.1` → full UI/API → edit config / browse
   `profiles/` + `logs/` via `/api/files/*` → writes go through `flash_handshake`.
2. **Unconfigured boot (no cable):** SoftAP `pico-kiln-setup` (open) comes up →
   phone joins → device DHCP hands it 192.168.4.x → browse `http://192.168.4.1` →
   set `WIFI_SSID`/`WIFI_PASSWORD` via `/api/config` → reboot → STA mode.
3. **Configured boot:** STA join-forever exactly as today; USB-NCM also up if a
   cable is present.

## Error handling

- **USB unplugged / not plugged:** the USB device `run()` task idles; web workers
  on the USB stack block on accept. No effect on the kiln or the radio. Replug
  re-enumerates.
- **Host = Windows 10:** NCM has no in-box driver (Win11/macOS/Linux fine).
  Documented; SoftAP is the fallback for such hosts.
- **Android host MAC quirk:** USB-NCM fails if the host MAC has the
  locally-administered bit set → choose device/host MAC bytes accordingly
  (device-verify against the example's constants).
- **AP/unconfigured mode has no NTP:** NTP-gated runs stay gated. Acceptable —
  unconfigured mode is for provisioning, not firing. Control loop, recovery, and
  CSV logging run regardless (already network-independent).
- **DHCP server down:** client can still set a static IP in the subnet manually
  (documented fallback).
- **No new flash hazard:** all writes reuse the existing handshake path.

## Testing & verification

- **Host unit test (`kiln-app`):** `wifi_is_configured()` truth table — empty,
  `"your_wifi_ssid"` placeholder, real SSID. Runs in the host workspace.
- **Build:** `kiln-firmware` is excluded from the host workspace (needs
  thumbv8m + `memory.x` + cyw43 blobs + `arm-none-eabi-gcc`/`CC`). Verify it
  *compiles* for the target; no host test for the net glue (consistent with the
  crate's existing "not built in CI-on-host" status).
- **Manual device checklist (record results):**
  1. Configured board + USB on macOS/Linux → NIC appears → DHCP lease → UI at
     192.168.7.1 → edit a profile file → confirm persisted.
  2. Wipe SSID → reboot → `pico-kiln-setup` AP visible → phone joins → 192.168.4.1
     → set WiFi → reboot → board joins STA.
  3. Configured board, AP powered off → STA retries forever (LED blink) → plug USB
     → reachable at 192.168.7.1.

## Budget (RP2350: 2560 KiB flash region, 512 KiB RAM)

- **Flash:** image ~505 KiB today; region 2560 KiB → ~2 MiB headroom. `embassy-usb`
  + `edge-dhcp` add an estimated +60–150 KiB. Low risk. Measure with
  `arm-none-eabi-size` after.
- **RAM:** +1 `StackResources` (USB stack), +2 web workers (~4 KiB tcp/http
  each), USB endpoint buffers + NCM packet queues (a few KiB), DHCP buffers.
  Order ~30–40 KiB. Comfortable on 512 KiB.

## Risks / known trade-offs

1. **Open AP exposes the full control API** (incl. start-firing) to anyone in
   range *while unconfigured*. User-chosen. Mitigated by: fallback-only (gone once
   STA joins), short-lived (first boot), no NTP so runs are gated. README must
   call this out. (A `start_ap_wpa2` default-password upgrade is a one-line change
   later if desired.)
2. **embassy-usb / edge-dhcp version pinning** against the 0.10 train — the
   verification pass confirms compatible versions before coding; if `edge-dhcp` is
   incompatible, fall back to a hand-rolled minimal DHCP responder (as some embassy
   AP examples do).
3. **USB peripheral assumed free at runtime** (UF2 = BOOTSEL bootrom, defmt/probe
   = SWD). Verify no USB-serial logger is wired before claiming `p.USB`.

## Out of scope (YAGNI)

- Captive-portal DNS (auto-open setup page) — possible later enhancement.
- WPA2 AP / per-config AP password — open AP chosen.
- Background STA retry while in AP mode — USB is the escape hatch instead.
- USB Mass Storage (MSC) — rejected (host can't read littlefs; cache-coherency
  corruption; immature embassy MSC class).
