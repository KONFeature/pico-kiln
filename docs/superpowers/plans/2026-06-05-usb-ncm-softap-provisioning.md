# USB-NCM + SoftAP Provisioning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a fresh/unreachable Pico-2-W kiln configurable without WiFi — always-on USB-ethernet (CDC-NCM) plus a fallback open SoftAP when unconfigured — both serving the existing web API/file routes.

**Architecture:** At most two `embassy-net` stacks live at once. The cyw43 radio is **either** STA (configured) **or** SoftAP (unconfigured) — never both (driver limit). A USB-CDC-NCM stack is **always** up when the cable is enumerated. The same `make_app()` router serves on every stack; file/config writes reuse the existing `flash_handshake`. A small DHCP server runs on each non-STA stack.

**Tech Stack:** Rust `no_std`, embassy 0.10 train (embassy-rp 0.10, embassy-net 0.9, embassy-usb), cyw43 0.7 SoftAP, `leasehund` DHCP server, picoserve 0.18, littlefs2.

**Spec:** `docs/superpowers/specs/2026-06-05-usb-ncm-softap-provisioning-design.md`

**Build/test commands (used throughout):**
- Host workspace test: `cd rust && cargo test --workspace`
- Firmware compile (the only firmware "test" — crate is excluded from host CI):
  `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build`
- Firmware lint: same env + `cargo clippy`
- Firmware size: `arm-none-eabi-size target/thumbv8m.main-none-eabihf/release/deps/kiln_firmware-*` (newest)

> **Note on TDD:** `kiln-firmware` cannot be host-tested (needs the thumbv8m target + cyw43 blobs). Per the crate's established practice, its "green" gate is **`cargo build` + `cargo clippy` clean for the target** plus the manual device checklist in Task 9. Only Task 1 (in host-testable `kiln-app`) uses real TDD.

---

## File Structure

- `rust/kiln-app/src/config.rs` — add `KilnConfig::wifi_is_configured()` (+ host test).
- `rust/kiln-app/src/server.rs` — bump the web-task pool budget; no router change.
- `rust/kiln-firmware/Cargo.toml` — add `embassy-usb`, `leasehund`.
- `rust/kiln-firmware/src/platform.rs` — USB-NCM bring-up, cyw43 STA/AP refactor, USB irq.
- `rust/kiln-firmware/src/dhcp.rs` — **new** — `dhcp_server_task` (isolated DHCP module).
- `rust/kiln-firmware/src/main.rs` — pass `p.USB`, boot mode selection, spawn pools.
- `rust/README.md` (or root README) — provisioning instructions + open-AP warning.

---

## Task 1: `wifi_is_configured()` helper (host-tested, TDD)

**Files:**
- Modify: `rust/kiln-app/src/config.rs` (impl block for `KilnConfig`, ~line 200; tests at ~line 890)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `config.rs`:

```rust
#[test]
fn wifi_is_configured_truth_table() {
    let mut c = KilnConfig::default();
    // default SSID is empty
    assert!(!c.wifi_is_configured(), "empty SSID must read as unconfigured");

    c.wifi_ssid = FixedStr::from_text("your_wifi_ssid").unwrap();
    assert!(
        !c.wifi_is_configured(),
        "the config.example placeholder must read as unconfigured"
    );

    c.wifi_ssid = FixedStr::from_text("home").unwrap();
    assert!(c.wifi_is_configured(), "a real SSID must read as configured");
}
```

(If `FixedStr`/`Str` construction differs, mirror the existing `parses_wifi_and_static_ip` test at `config.rs:932` for the exact constructor.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test -p kiln-app wifi_is_configured_truth_table`
Expected: FAIL — `no method named wifi_is_configured`.

- [ ] **Step 3: Write minimal implementation**

Add to `impl KilnConfig` in `config.rs`:

```rust
/// The WiFi placeholder that `config.example.json` ships (`config.rs` already
/// asserts this exact literal at `config_example_json_is_valid`).
const WIFI_SSID_PLACEHOLDER: &str = "your_wifi_ssid";

/// True when WiFi station credentials are usable: a non-empty SSID that is not
/// the shipped placeholder. Drives the boot choice between STA and the
/// provisioning SoftAP (firmware `core0_main`).
pub fn wifi_is_configured(&self) -> bool {
    let ssid = self.wifi_ssid.as_str();
    !ssid.is_empty() && ssid != Self::WIFI_SSID_PLACEHOLDER
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test -p kiln-app wifi_is_configured_truth_table`
Expected: PASS. Then `cargo test --workspace` — all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add rust/kiln-app/src/config.rs
git commit -m "feat(rust): KilnConfig::wifi_is_configured() for provisioning gate"
```

---

## Task 2: Web-task pool budget for multiple stacks

The macro `#[task(pool_size = N)]` caps total live `web_task` instances. Today
`N = WEB_TASK_POOL_SIZE = 2` (all on the WiFi stack). We will serve the primary
stack with `WEB_TASK_POOL_SIZE` workers and each secondary stack (NCM, AP) with
**one** worker. Worst case is configured mode: STA(2) + NCM(1) = 3.

**Files:**
- Modify: `rust/kiln-app/src/server.rs` (constants ~line 42; `web_task` attr ~line 1561)

- [ ] **Step 1: Add the pool constants**

Next to `pub const WEB_TASK_POOL_SIZE: usize = api::MAX_CONCURRENT_CONNECTIONS;` add:

```rust
/// Web workers per *secondary* interface (USB-NCM, fallback SoftAP). Provisioning
/// and USB file access are single-user, so one connection each is enough — and a
/// picoserve worker future is ~84 KB, so extra workers are the dominant RAM cost.
pub const SECONDARY_WEB_WORKERS: usize = 1;

/// Total `web_task` instances that can be live at once = the macro `pool_size`.
/// Worst case = configured boot: STA (`WEB_TASK_POOL_SIZE`) + USB-NCM
/// (`SECONDARY_WEB_WORKERS`). Unconfigured boot uses fewer (AP 1 + NCM 1).
pub const WEB_TASK_POOL_TOTAL: usize = WEB_TASK_POOL_SIZE + SECONDARY_WEB_WORKERS;
```

- [ ] **Step 2: Point the task macro at the new total**

Change the attribute on `web_task` (currently `#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]`) to:

```rust
    #[embassy_executor::task(pool_size = WEB_TASK_POOL_TOTAL)]
```

- [ ] **Step 3: Verify host build**

Run: `cd rust && cargo build -p kiln-app`
Expected: builds clean (firmware spawn-site updates land in Task 7).

- [ ] **Step 4: Commit**

```bash
git add rust/kiln-app/src/server.rs
git commit -m "feat(rust): size web_task pool for multiple net stacks"
```

---

## Task 3: Add firmware dependencies (USB + DHCP)

**Files:**
- Modify: `rust/kiln-firmware/Cargo.toml` (`[dependencies]`)

- [ ] **Step 1: Add the deps**

Append to `[dependencies]`:

```toml
# USB device stack for CDC-NCM (USB ethernet) provisioning + file access. Version
# must use the same `embassy-usb-driver` that embassy-rp 0.10 implements; if the
# solver complains, set this to match `cargo tree -p embassy-usb-driver`.
embassy-usb = "0.5"
# Instance-based DHCP server (one `DhcpServer` value per stack) so the SoftAP and
# the USB-NCM stack can each serve leases at the same time. (esp-hal-dhcp-server
# was rejected: it uses a *global* close signal = effectively a singleton.)
leasehund = "0.2"
```

> `embassy-net-driver-channel` is NOT added directly — it is re-exported via
> `embassy_usb::class::cdc_ncm::embassy_net` (`Device`/`Runner`/`State`).

- [ ] **Step 2: Verify it resolves + compiles**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build`
Expected: PASS (no code uses the deps yet, so this only proves version resolution).

If `embassy-usb` version conflicts: run `cargo tree -p embassy-usb-driver`, read the version embassy-rp 0.10 pins, and set `embassy-usb` to the matching release. If `leasehund` conflicts with embassy-net 0.9: switch to the hand-rolled fallback noted in Task 6 (and drop the `leasehund` line).

- [ ] **Step 3: Commit**

```bash
git add rust/kiln-firmware/Cargo.toml rust/kiln-firmware/Cargo.lock 2>/dev/null; git add rust/kiln-firmware/Cargo.toml
git commit -m "build(rust): add embassy-usb + leasehund for USB-NCM/SoftAP provisioning"
```

---

## Task 4: USB interrupt binding + module wiring

**Files:**
- Modify: `rust/kiln-firmware/src/platform.rs` (`bind_interrupts!` block ~line 892; imports)
- Modify: `rust/kiln-firmware/src/main.rs` (`mod` declarations ~line 39)

- [ ] **Step 1: Add the USB interrupt to the existing `Irqs`**

In `platform.rs`, extend the existing `bind_interrupts!` struct (it already has
`PIO0_IRQ_0` + `DMA_IRQ_0`) with the USB line:

```rust
bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>;
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
});
```

(Keep the existing two lines exactly as they are; only add the `USBCTRL_IRQ` line. If the existing lines use shorter imported names, match those.)

- [ ] **Step 2: Declare the new dhcp module**

In `main.rs`, next to `mod flash_handshake; mod lcd; mod platform;` add:

```rust
mod dhcp;
```

- [ ] **Step 3: Verify compile**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build`
Expected: FAIL — `file not found for module dhcp` (created in Task 5). This confirms the module is wired; proceed to Task 5 before re-running.

(No commit yet — Task 5 completes this build.)

---

## Task 5: DHCP server module

A `#[task]` with `pool_size = 2` so two servers (AP + NCM) can run concurrently.

**Files:**
- Create: `rust/kiln-firmware/src/dhcp.rs`

- [ ] **Step 1: Write the module**

```rust
//! Minimal DHCP server for the provisioning interfaces. Both the fallback SoftAP
//! and the always-on USB-NCM link act as the gateway on their own /24 and must
//! hand the client (a phone, or a USB host) an address — embassy-net is a DHCP
//! *client* only, so we run a server here. `leasehund` is instance-based (one
//! `DhcpServer` value per stack), which lets the AP and NCM servers coexist; a
//! global-singleton server (e.g. esp-hal-dhcp-server) could not.
//!
//! `pool_size = 2`: at most two interfaces serve DHCP at once (unconfigured boot
//! = SoftAP + USB-NCM; configured boot = USB-NCM only, STA is a client).

use core::net::Ipv4Addr;
use embassy_net::Stack;
use leasehund::DhcpServer;

/// Serve DHCP forever on `stack`. `gateway` is this device's IP on the link (also
/// advertised as router); leases are handed from `pool_start`..=`pool_end`.
#[embassy_executor::task(pool_size = 2)]
pub async fn dhcp_server_task(
    stack: Stack<'static>,
    gateway: Ipv4Addr,
    pool_start: Ipv4Addr,
    pool_end: Ipv4Addr,
) -> ! {
    // No DNS advertised (no captive portal): clients reach the UI by IP.
    let mut server: DhcpServer<32, 4> = DhcpServer::new(
        gateway,                              // server IP
        Ipv4Addr::new(255, 255, 255, 0),      // /24 subnet mask
        gateway,                              // router/gateway = this device
        pool_start,
        pool_end,
    );
    server.run(stack).await;
    // `run` loops forever; satisfy the `-> !` return type.
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}
```

> **API verify:** confirm `leasehund`'s constructor name/arity against the
> installed version (`DhcpServer::new(...)` vs `new_with_dns(...)` /
> `DhcpConfigBuilder`) — see docs.rs/leasehund. Adjust the call only; keep the
> task signature stable. If `leasehund` is incompatible with embassy-net 0.9,
> replace the body with a hand-rolled responder over `embassy_net::udp::UdpSocket`
> bound to port 67 using `edge_dhcp::{Packet, Options, MessageType}` (compute-only
> core), keeping this exact `dhcp_server_task` signature.

- [ ] **Step 2: Verify compile**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build`
Expected: PASS (module compiles; not yet spawned).

- [ ] **Step 3: Commit**

```bash
git add rust/kiln-firmware/src/dhcp.rs rust/kiln-firmware/src/main.rs rust/kiln-firmware/src/platform.rs
git commit -m "feat(rust): DHCP server module + USB irq binding"
```

---

## Task 6: USB-CDC-NCM bring-up in `platform.rs`

**Files:**
- Modify: `rust/kiln-firmware/src/platform.rs` (imports; new tasks + `init_usb_ncm`)

- [ ] **Step 1: Add imports (top of `platform.rs`, near the other embassy imports)**

```rust
use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver as UsbDriver;
use embassy_usb::class::cdc_ncm::embassy_net::{Device as NcmDevice, Runner as NcmRunner, State as NcmNetState};
use embassy_usb::class::cdc_ncm::{CdcNcmClass, State as NcmState};
use embassy_usb::{Builder as UsbBuilder, Config as UsbConfig, UsbDevice};
```

- [ ] **Step 2: Add the constants + driver type alias**

```rust
/// USB-NCM ethernet MTU (standard frame; matches the embassy example).
const NCM_MTU: usize = 1514;
/// Device IP on the USB link; the host gets `192.168.7.2..=.15` via DHCP.
const NCM_GATEWAY: core::net::Ipv4Addr = core::net::Ipv4Addr::new(192, 168, 7, 1);
const NCM_POOL_START: core::net::Ipv4Addr = core::net::Ipv4Addr::new(192, 168, 7, 2);
const NCM_POOL_END: core::net::Ipv4Addr = core::net::Ipv4Addr::new(192, 168, 7, 15);
/// Sockets for the NCM stack: secondary web workers + 1 DHCP UDP socket + margin.
const NCM_NET_SOCKETS: usize = kiln_app::server::SECONDARY_WEB_WORKERS + 2;

/// Concrete USB driver type, named so the `#[task]`s below stay non-generic.
type NcmUsbDriver = UsbDriver<'static, USB>;

/// MAC addresses for the USB-NCM link. The HOST mac must NOT have the
/// locally-administered bit (bit 1 of byte 0) set or Android refuses the device;
/// `0x88` and `0xCC` both keep it clear.
const NCM_OUR_MAC: [u8; 6] = [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];
const NCM_HOST_MAC: [u8; 6] = [0x88, 0x88, 0x88, 0x88, 0x88, 0x88];
```

- [ ] **Step 3: Add the three USB tasks**

```rust
/// Drives the USB device (control transfers, enumeration).
#[embassy_executor::task]
async fn usb_device_task(mut device: UsbDevice<'static, NcmUsbDriver>) -> ! {
    device.run().await
}

/// Drives the CDC-NCM class (USB bulk RX/TX ↔ the embassy-net device).
#[embassy_executor::task]
async fn usb_ncm_task(runner: NcmRunner<'static, NcmUsbDriver, NCM_MTU>) -> ! {
    runner.run().await
}

/// Drives the embassy-net stack riding on the USB-NCM device.
#[embassy_executor::task]
async fn usb_net_task(mut runner: embassy_net::Runner<'static, NcmDevice<'static, NCM_MTU>>) -> ! {
    runner.run().await
}
```

- [ ] **Step 4: Add `init_usb_ncm`**

```rust
/// Bring up USB-CDC-NCM → an always-on `embassy-net` `Stack` at a fixed IP, with
/// a DHCP server for the host. Independent of the radio, so it is up whenever the
/// cable is enumerated — the wired escape hatch for (re)configuring WiFi and
/// browsing files. Returns the stack the web workers serve on.
pub fn init_usb_ncm(
    spawner: &embassy_executor::Spawner,
    usb: Peri<'static, USB>,
) -> Stack<'static> {
    let driver = UsbDriver::new(usb, Irqs);

    let mut config = UsbConfig::new(0xc0de, 0xcafe);
    config.manufacturer = Some("pico-kiln");
    config.product = Some("pico-kiln (USB-NCM)");
    config.serial_number = Some("kiln-0001");
    config.max_power = 100;
    config.max_packet_size_0 = 64;
    // CDC-NCM enumerates as a composite device with an IAD.
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let mut builder = UsbBuilder::new(
        driver,
        config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        &mut [], // no Microsoft OS descriptors
        CONTROL_BUF.init([0; 128]),
    );

    static NCM_STATE: StaticCell<NcmState> = StaticCell::new();
    let class = CdcNcmClass::new(&mut builder, NCM_STATE.init(NcmState::new()), NCM_HOST_MAC, 64);

    let usb = builder.build();
    spawner.spawn(usb_device_task(usb).unwrap());

    static NET_STATE: StaticCell<NcmNetState<NCM_MTU, 4, 4>> = StaticCell::new();
    let (runner, device) =
        class.into_embassy_net_device::<NCM_MTU, 4, 4>(NET_STATE.init(NcmNetState::new()), NCM_OUR_MAC);
    spawner.spawn(usb_ncm_task(runner).unwrap());

    // Static IP: this device is the gateway on the USB /24. No DNS (no portal).
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 7, 1), 24),
        gateway: None,
        dns_servers: heapless_v09::Vec::new(),
    });
    let mut rng = RoscRng;
    let seed = rng.next_u64();
    static RES: StaticCell<embassy_net::StackResources<NCM_NET_SOCKETS>> = StaticCell::new();
    let (stack, net_runner) = embassy_net::new(
        device,
        net_config,
        RES.init(embassy_net::StackResources::new()),
        seed,
    );
    spawner.spawn(usb_net_task(net_runner).unwrap());

    // Lease addresses to the USB host.
    spawner.spawn(
        crate::dhcp::dhcp_server_task(stack, NCM_GATEWAY, NCM_POOL_START, NCM_POOL_END).unwrap(),
    );

    stack
}
```

> If `Ipv4Address::new`/`Ipv4Cidr::new`/`StaticConfigV4`/`heapless_v09` are not in
> scope, copy the exact import + `dns_servers` construction from the existing
> `static_config` helper in `platform.rs` (it already builds a `StaticConfigV4`).

- [ ] **Step 5: Verify compile + clippy**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo clippy`
Expected: PASS, zero warnings. (`init_usb_ncm` is `pub` and unused until Task 8 — clippy won't flag a `pub fn`.)

- [ ] **Step 6: Commit**

```bash
git add rust/kiln-firmware/src/platform.rs
git commit -m "feat(rust): USB-CDC-NCM net stack bring-up (init_usb_ncm)"
```

---

## Task 7: cyw43 STA/AP refactor in `platform.rs`

Split today's `init_network` into shared bring-up + an STA path (unchanged
behaviour) + a new SoftAP path.

**Files:**
- Modify: `rust/kiln-firmware/src/platform.rs` (`init_network` ~line 1000–1082)

- [ ] **Step 1: Extract shared cyw43 bring-up**

Refactor so a private helper builds the radio up to (but not including) the
embassy-net stack, returning the net device + control. Replace the head of the
current `init_network` body with a call to this:

```rust
/// Shared cyw43 bring-up: blobs, PIO SPI, `cyw43::new`, runner spawn,
/// `control.init` + power management. Used by both STA and SoftAP paths.
async fn init_cyw43(
    spawner: &embassy_executor::Spawner,
    p: Core0Periphs,
) -> (cyw43::NetDriver<'static>, Control<'static>) {
    let fw = cyw43::aligned_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = cyw43::aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let nvram = cyw43::aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");

    let pwr = Output::new(p.wl_pwr, Level::Low);
    let cs = Output::new(p.wl_cs, Level::High);
    let mut pio = Pio::new(p.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        RM2_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.wl_dio,
        p.wl_clk,
        Channel::new(p.dma, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());

    control.init(clm).await;
    control
        .set_power_management(PowerManagementMode::PowerSave)
        .await;

    (net_device, control)
}
```

- [ ] **Step 2: Make `init_network` the STA path on top of `init_cyw43`**

Rewrite `init_network` so its head delegates to `init_cyw43` and the rest (RNG
seed, net config STA via `static_config`/DHCP, `embassy_net::new`, `net_task`
spawn, scan, join-forever, `wait_config_up`, LED) is unchanged. Signature stays
`pub async fn init_network(spawner, p, config) -> (Stack<'static>, Control<'static>)`.

```rust
pub async fn init_network(
    spawner: &embassy_executor::Spawner,
    p: Core0Periphs,
    config: &'static KilnConfig,
) -> (Stack<'static>, Control<'static>) {
    let (net_device, mut control) = init_cyw43(spawner, p).await;

    let mut rng = RoscRng;
    let seed = rng.next_u64();
    let net_config = match static_config(config) {
        Some(static_v4) => embassy_net::Config::ipv4_static(static_v4),
        None => embassy_net::Config::dhcpv4(Default::default()),
    };
    static RES: StaticCell<embassy_net::StackResources<NET_SOCKETS>> = StaticCell::new();
    let (stack, net_runner) = embassy_net::new(
        net_device,
        net_config,
        RES.init(embassy_net::StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    // ---- unchanged from here: presence scan, join-forever, wait_config_up ----
    let ssid = config.wifi_ssid.as_str();
    let password = config.wifi_password.as_str();
    let mut led = false;
    for _ in 0..5 {
        if scan_visible(&mut control, ssid).await {
            break;
        }
        led = !led;
        control.gpio_set(STATUS_LED_GPIO, led).await;
        embassy_time::Timer::after(Duration::from_secs(1)).await;
    }
    while control
        .join(ssid, JoinOptions::new(password.as_bytes()))
        .await
        .is_err()
    {
        led = !led;
        control.gpio_set(STATUS_LED_GPIO, led).await;
        embassy_time::Timer::after(Duration::from_secs(2)).await;
    }
    stack.wait_config_up().await;
    control.gpio_set(STATUS_LED_GPIO, true).await;
    (stack, control)
}
```

- [ ] **Step 3: Add the SoftAP path**

```rust
/// Provisioning SoftAP, used when WiFi is unconfigured. Brings up an **open** AP
/// (`SECURITY WARNING`: anyone in range reaches the full control API while in this
/// mode — acceptable only because it is first-boot/unconfigured and short-lived,
/// see the spec) on a fixed /24 with a DHCP server. Returns the stack the web
/// workers serve on. cyw43 cannot run STA and AP at once, so this is mutually
/// exclusive with `init_network`.
pub async fn init_softap(
    spawner: &embassy_executor::Spawner,
    p: Core0Periphs,
) -> Stack<'static> {
    let (net_device, mut control) = init_cyw43(spawner, p).await;

    let mut rng = RoscRng;
    let seed = rng.next_u64();
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 4, 1), 24),
        gateway: None,
        dns_servers: heapless_v09::Vec::new(),
    });
    static RES: StaticCell<embassy_net::StackResources<{ kiln_app::server::SECONDARY_WEB_WORKERS + 2 }>> =
        StaticCell::new();
    let (stack, net_runner) = embassy_net::new(
        net_device,
        net_config,
        RES.init(embassy_net::StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    // Open AP on channel 6 (matches the embassy example's `start_ap_*` arity).
    control.start_ap_open("pico-kiln-setup", 6).await;
    control.gpio_set(STATUS_LED_GPIO, true).await; // solid: AP up

    spawner.spawn(
        crate::dhcp::dhcp_server_task(
            stack,
            core::net::Ipv4Addr::new(192, 168, 4, 1),
            core::net::Ipv4Addr::new(192, 168, 4, 2),
            core::net::Ipv4Addr::new(192, 168, 4, 15),
        )
        .unwrap(),
    );

    stack
}
```

- [ ] **Step 4: Verify compile + clippy**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo clippy`
Expected: PASS, zero warnings. (`init_softap` `pub`, unused until Task 8.)

> If `StackResources<{ ... }>` const-generic-expression fails to parse, define a
> `const AP_NET_SOCKETS: usize = kiln_app::server::SECONDARY_WEB_WORKERS + 2;` and
> use `StackResources<AP_NET_SOCKETS>` (same as NCM in Task 6).

- [ ] **Step 5: Commit**

```bash
git add rust/kiln-firmware/src/platform.rs
git commit -m "feat(rust): split cyw43 bring-up into STA + provisioning SoftAP paths"
```

---

## Task 8: Boot orchestration in `main.rs`

**Files:**
- Modify: `rust/kiln-firmware/src/main.rs` (`Core0Periphs` ~line 268; `main` ~line 116; `core0_main` ~line 293–390)

- [ ] **Step 1: Capture `p.USB` and pass it to `core0_main`**

`init_network`/`init_softap`/`init_cyw43` consume `Core0Periphs` by value, so USB
must travel separately (like `LcdPeriphs`). In `main()` capture it and add it to
the `core0_main` spawn:

```rust
    // (after the LcdPeriphs block, before EXECUTOR0)
    let usb = p.USB;
```

Add `usb` as a parameter to the `core0_main(...)` call (place it right after `lcd`):

```rust
            core0_main(
                core0_periphs,
                lcd_periphs,
                usb,
                storage,
                config,
                commands.sender(),
                status,
                reboot,
            )
```

- [ ] **Step 2: Add the param + USB import to `core0_main`**

In `main.rs` imports add `use embassy_rp::peripherals::USB; use embassy_rp::Peri;`
(if `Peri` is not already imported). Change the `core0_main` signature to insert:

```rust
    usb: Peri<'static, USB>,
```

immediately after `lcd: LcdPeriphs,`.

- [ ] **Step 3: Replace the network bring-up block with mode selection**

Replace the current single `let (stack, control) = platform::init_network(...)`
call and the WiFi-dependent spawns with the state machine. The block from
`// --- Network bring-up ---` down to the end of `core0_main` becomes:

```rust
    // --- USB-NCM: always on (radio-independent), the wired escape hatch -------
    let usb_stack = platform::init_usb_ncm(&spawner, usb);

    // --- Radio: STA when configured, else the provisioning SoftAP ------------
    // cyw43 cannot do both at once. A configured board retries STA forever (USB
    // is the recovery path); an unconfigured board serves the setup AP.
    let configured = config.wifi_is_configured();

    static APP: StaticCell<kiln_app::server::AppRouter> = StaticCell::new();
    let app: &'static _ = APP.init(kiln_app::server::make_app(state));
    let web_cfg = platform::web_config();
    let port = config.web_server_port;

    if configured {
        let (sta_stack, control) = platform::init_network(&spawner, p, config).await;
        // Primary interface: full worker pool.
        for id in 0..kiln_app::server::WEB_TASK_POOL_SIZE {
            spawner.spawn(kiln_app::server::web_task(id, sta_stack, app, web_cfg, port).unwrap());
        }
        // STA-only background tasks.
        spawner.spawn(
            platform::wifi_monitor_task(
                control,
                sta_stack,
                config.wifi_ssid.as_str(),
                config.wifi_password.as_str(),
            )
            .unwrap(),
        );
        spawner.spawn(platform::ntp_task(clock, sta_stack).unwrap());
    } else {
        let ap_stack = platform::init_softap(&spawner, p).await;
        for i in 0..kiln_app::server::SECONDARY_WEB_WORKERS {
            spawner.spawn(kiln_app::server::web_task(i, ap_stack, app, web_cfg, port).unwrap());
        }
    }

    // USB-NCM workers, always (ids offset past the primary pool to stay unique).
    for i in 0..kiln_app::server::SECONDARY_WEB_WORKERS {
        let id = kiln_app::server::WEB_TASK_POOL_SIZE + i;
        spawner.spawn(kiln_app::server::web_task(id, usb_stack, app, web_cfg, port).unwrap());
    }

    // LCD status line (optional), unchanged — works in either radio mode.
    if config.lcd_enabled {
        if let Some(display) = platform::init_display(lcd, config) {
            spawner.spawn(kiln_app::server::lcd_task(status, display).unwrap());
        }
    }
    spawner.spawn(platform::reboot_task(reboot).unwrap());
```

> Notes for the worker: `state` (the `AppState`) is `Copy` and built earlier in
> `core0_main`; `clock`, `storage`, recovery, and the CSV logger spawn are all
> **before** this block and stay unchanged. NTP/wifi-monitor are intentionally
> skipped in AP mode (no upstream network → NTP-gated runs stay gated, per spec).

- [ ] **Step 4: Verify compile + clippy**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo clippy`
Expected: PASS, zero warnings.

- [ ] **Step 5: Release build + size check**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf="$HOME/opt/armgnu/Payload/bin/arm-none-eabi-gcc" cargo build --release && arm-none-eabi-size $(ls -t target/thumbv8m.main-none-eabihf/release/deps/kiln_firmware-* | grep -v '\.' | head -1)`
Expected: text fits well under the 2560 KiB FLASH region; `.bss` increase ≈ one extra web worker (~84 KB) + USB/AP stack resources + USB buffers. Record the before/after `bss` (target was ~207 KB; expect ~300 KB). If `.bss` is uncomfortably high, set `SECONDARY_WEB_WORKERS` stays 1 (already minimal) and re-measure.

- [ ] **Step 6: Commit**

```bash
git add rust/kiln-firmware/src/main.rs
git commit -m "feat(rust): boot into STA or provisioning SoftAP + always-on USB-NCM"
```

---

## Task 9: Docs + manual device verification

**Files:**
- Modify: `rust/README.md` (or root `README.md` — whichever documents deployment)
- Modify: `docs/superpowers/specs/2026-06-05-usb-ncm-softap-provisioning-design.md` (tick the verification checklist results)

- [ ] **Step 1: Document provisioning**

Add a "First-time setup / provisioning" section covering:
- **USB:** plug the Pico into a Mac/Linux/Win11 host (Windows 10 has no in-box
  NCM driver — use the SoftAP instead). A USB ethernet NIC appears; browse to
  `http://192.168.7.1` for the full UI, set WiFi under config, reboot.
- **SoftAP (no cable):** an unconfigured board broadcasts the **open** network
  `pico-kiln-setup`. Join from a phone, browse to `http://192.168.4.1`, set WiFi,
  reboot. **Security note:** the AP is open and exposes the full control API while
  unconfigured — provision somewhere you trust, and the AP disappears once the
  board joins your WiFi.
- **Recovery:** a configured board that can't reach its saved WiFi retries
  forever (status LED blinks); plug in USB to fix the credentials.

- [ ] **Step 2: Flash + run the device checklist**

Deploy the release build (existing `probe-rs`/UF2 flow) and record results in the
spec's "Testing & verification" checklist:
1. Configured board + USB on macOS/Linux → NIC + DHCP lease → UI at 192.168.7.1 →
   edit a profile via the UI → confirm it persists after reboot.
2. Wipe `WIFI_SSID` (set to `""` via the USB UI) → reboot → `pico-kiln-setup`
   visible → phone joins → 192.168.4.1 → set WiFi → reboot → board joins STA.
3. Configured board, AP powered off → STA blink/retry → plug USB → reachable at
   192.168.7.1.

- [ ] **Step 3: Commit**

```bash
git add rust/README.md README.md docs/superpowers/specs/2026-06-05-usb-ncm-softap-provisioning-design.md
git commit -m "docs(rust): USB-NCM + SoftAP provisioning guide + verification results"
```

---

## Final integration commit (spec + plan)

- [ ] After all tasks pass, add the spec + plan docs if not already committed:

```bash
git add docs/superpowers/specs/2026-06-05-usb-ncm-softap-provisioning-design.md docs/superpowers/plans/2026-06-05-usb-ncm-softap-provisioning.md
git commit -m "docs(rust): provisioning design spec + implementation plan"
```

---

## Self-review notes (filled by plan author)

- **Spec coverage:** every spec section maps to a task — `wifi_is_configured` (T1),
  pool sizing (T2), deps (T3), USB irq + dhcp module (T4/T5), `init_usb_ncm` (T6),
  STA/SoftAP split (T7), boot state machine + spawn (T8), docs + the spec's
  verification checklist (T9).
- **Open-AP risk** is surfaced in code (`SECURITY WARNING` doc-comment, T7) and
  README (T9), matching the spec's Risks section.
- **DHCP two-instance requirement** drove the `leasehund` choice with `pool_size = 2`
  (T5) and a hand-rolled fallback documented in-task.
- **RAM correction:** secondary interfaces use 1 worker (T2) because each worker
  future is ~84 KB; budget re-measured in T8 step 5.
- **Unknowns are executable steps, not placeholders:** exact `embassy-usb` version
  (T3 step 2 resolution command) and `leasehund` constructor arity (T5 verify note)
  are confirmed against `cargo`/docs during execution, with concrete fallbacks.
