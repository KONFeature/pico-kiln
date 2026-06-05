//! Minimal DHCP server for the provisioning interfaces. Both the fallback SoftAP
//! and the always-on USB-NCM link act as the gateway on their own /24 and must
//! hand the client (a phone, or a USB host) an address — embassy-net is a DHCP
//! *client* only, so we run a server here.
//!
//! `leasehund` is instance-based (one `DhcpServer` value per stack), which lets
//! the AP and NCM servers coexist; a global-singleton server (e.g.
//! esp-hal-dhcp-server) could not. `pool_size = 2`: at most two interfaces serve
//! DHCP at once — unconfigured boot = SoftAP + USB-NCM; configured boot = USB-NCM
//! only (the STA interface is a DHCP *client*).

use core::net::Ipv4Addr;
use embassy_net::Stack;
use leasehund::DhcpServer;

/// Serve DHCP forever on `stack`. `gateway` is this device's IP on the link (also
/// advertised as router *and* DNS — harmless, the UI is reached by IP since there
/// is no captive portal); leases are handed from `pool_start`..=`pool_end`.
#[embassy_executor::task(pool_size = 2)]
pub async fn dhcp_server_task(
    stack: Stack<'static>,
    gateway: Ipv4Addr,
    pool_start: Ipv4Addr,
    pool_end: Ipv4Addr,
) -> ! {
    // leasehund 0.5.1: `DhcpServer::<MAX_CLIENTS, N_DNS>::new(server_ip, mask,
    // router, dns, pool_start, pool_end)`. One DNS slot (= the gateway). These are
    // setup-only links (a single phone/USB host at a time); a 4-lease table is
    // ample headroom over the one expected client (covers a reconnect with a stale
    // lease still outstanding) and saves the static RAM a 32-entry table cost ×2.
    let mut server: DhcpServer<4, 1> = DhcpServer::new(
        gateway,                          // server IP
        Ipv4Addr::new(255, 255, 255, 0),  // /24 subnet mask
        gateway,                          // router/gateway = this device
        gateway,                          // DNS = this device (unused; clients use IP)
        pool_start,
        pool_end,
    );
    // leasehund's `run(&mut self, stack: Stack<'_>) -> !` takes the (Copy) stack
    // by value and never returns, which satisfies this task's `-> !`.
    server.run(stack).await
}
