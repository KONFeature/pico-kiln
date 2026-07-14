# Vendored cyw43 0.7.0 — pico-kiln patches

Verbatim copy of the crates.io `cyw43-0.7.0` sources, wired in via
`[patch.crates-io]` in `kiln-firmware/Cargo.toml`, with two cherry-picks from
upstream embassy `main` (both present there, neither released as of 0.7.0).
Search the sources for `PICO-KILN PATCH`.

## 1. `src/runner.rs` — ioctl error: warn, don't panic

Upstream 0.7.0 `panic!`s on any non-zero ioctl status in the runner's control
RX path (`runner.rs`, `rx()`, CHANNEL_TYPE_CONTROL). On a WPA3-SAE rekey/flap,
a `join()` ioctl can return a non-zero status; the panic halted Core 0 (USB +
WiFi both die) which is why `wifi_monitor_task` was disabled. Upstream main
downgraded it to `warn!` + `ioctl_done` (embassy-rs/embassy, runner.rs
"TODO: propagate error instead" → warn). The join caller observes the failure
via its event wait and retries.

## 2. `src/runner.rs` — DEAUTH_IND / DISASSOC_IND mark the link down

0.7.0 only marks the link down on `LINK(flag 0)`, `DEAUTH(SUCCESS)`, and the
WPA3 `AUTH FAIL/16/3` triple. AP-initiated deauth/disassoc indications
(`DEAUTH_IND`, `DISASSOC_IND` — AP reboot, idle-station inactivity timeout)
were ignored, leaving `is_link_up()` stale forever after such an event, so no
reconnect logic could trigger. Upstream main adds both events to the link-down
match; this vendored copy does the same.

## Removal

Delete this directory and the `[patch.crates-io]` section once a cyw43 release
(≥ 0.8) ships both fixes.
