//! Finding the paired desktop on the LAN: browse `_headstate._tcp`.
//!
//! The QR code gave the phone a list of addresses; those go stale when
//! the desktop's lease changes or it moves networks. While the desktop's
//! listener is up it advertises itself over mDNS (desktop
//! `remote/discovery.rs`) with the first sixteen hex characters of its
//! certificate fingerprint in the TXT record, so the phone can find the
//! current address of THE desktop it paired with, not just any Headstate
//! on the network. The result is only a hint about where to connect;
//! the pinned fingerprint on the TLS handshake is what proves it.
//!
//! # The record, as the desktop sends it
//!
//! ```text
//! <display name>._headstate._tcp.local.   SRV  port=41919
//!                                         TXT  fp=<16 hex>  v=1
//! ```
//!
//! `v` is carried for a future record shape and is not checked: a phone
//! that understands only `fp` should keep working against a newer
//! desktop that still sends it.
//!
//! # Where the client calls this
//!
//! `client.rs` (#514), before it tries the `addrs` stored for the paired
//! desktop: `browse(&fp_prefix(&paired.fp), Duration::from_secs(3))`,
//! and on `Some((ip, port))` that address goes to the front of the list.
//! It BLOCKS for up to `timeout` on a multicast channel, so the async
//! client runs it on a blocking task
//! (`tauri::async_runtime::spawn_blocking`). A `None` is not an error --
//! a different network, no multicast, a desktop whose listener is off --
//! and the stored addresses are tried exactly as they would have been.
//!
//! iOS gates all of this behind `NSLocalNetworkUsageDescription` and the
//! `_headstate._tcp` entry in `NSBonjourServiceTypes` (`Info.ios.plist`);
//! without them the browse sees nothing and never says why.

use mdns_sd::{ServiceDaemon, ServiceEvent, TxtProperties};
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// The service type, with the mDNS domain, as `mdns-sd` wants it.
pub const SERVICE_TYPE: &str = "_headstate._tcp.local.";
/// TXT key carrying the fingerprint prefix.
pub const TXT_FP: &str = "fp";
/// How many hex characters of the fingerprint the desktop puts on the
/// wire. Must match the desktop's `FP_PREFIX_LEN`.
pub const FP_PREFIX_LEN: usize = 16;

/// The prefix to browse for, from the fingerprint as the phone stores
/// it: the QR's `sha256:<hex>`, or bare hex. Lowercased, since the
/// desktop advertises lowercase and a fingerprint copied by hand may
/// not be.
pub fn fp_prefix(fingerprint: &str) -> String {
    fingerprint
        .strip_prefix("sha256:")
        .unwrap_or(fingerprint)
        .chars()
        .take(FP_PREFIX_LEN)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Look for the desktop whose TXT `fp` equals `fp_prefix`, for at most
/// `timeout`. Returns one address to try and the listener port, or
/// `None` when nothing matched in time or mDNS is unavailable here.
/// Blocking; see the module doc for how the client calls it.
pub fn browse(fp_prefix: &str, timeout: Duration) -> Option<(IpAddr, u16)> {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("mDNS unavailable: {e}");
            return None;
        }
    };
    let found = browse_with(&daemon, fp_prefix, timeout);
    // Best effort: the daemon thread exits when its channel drops even
    // if the shutdown message cannot be sent.
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    found
}

fn browse_with(
    daemon: &ServiceDaemon,
    fp_prefix: &str,
    timeout: Duration,
) -> Option<(IpAddr, u16)> {
    let events = match daemon.browse(SERVICE_TYPE) {
        Ok(rx) => rx,
        Err(e) => {
            log::warn!("mDNS browse failed: {e}");
            return None;
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match events.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                if !matches(&info.txt_properties, fp_prefix) {
                    continue;
                }
                match pick_addr(info.addresses.iter().map(|a| a.to_ip_addr())) {
                    Some(ip) => {
                        log::info!("found the paired desktop on the LAN at {ip}:{}", info.port);
                        return Some((ip, info.port));
                    }
                    // A record with no address is not one we can use;
                    // the daemon may resolve more of it later.
                    None => continue,
                }
            }
            Ok(_) => continue,
            // Timeout, or the daemon went away: either way, nothing.
            Err(_) => return None,
        }
    }
}

/// Exact match on the advertised prefix. Not `starts_with`: a desktop
/// advertising a shorter prefix must not match a longer one by luck.
fn matches(txt: &TxtProperties, fp_prefix: &str) -> bool {
    txt.get_property_val_str(TXT_FP)
        .is_some_and(|fp| fp.eq_ignore_ascii_case(fp_prefix))
}

/// One address out of what the record resolved to. IPv4 first: an IPv6
/// address from mDNS is usually link-local and needs a scope id that a
/// URL cannot carry. Then a routable IPv6, then whatever is left.
fn pick_addr(addrs: impl Iterator<Item = IpAddr>) -> Option<IpAddr> {
    let addrs: Vec<IpAddr> = addrs.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| {
            addrs
                .iter()
                .find(|a| matches!(a, IpAddr::V6(v6) if !v6.is_unicast_link_local()))
        })
        .or_else(|| addrs.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::{DaemonEvent, ServiceInfo};
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::time::SystemTime;

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_prefix_drops_the_sha256_label_and_takes_sixteen() {
        assert_eq!(fp_prefix(&format!("sha256:{FP}")), "0123456789abcdef");
        assert_eq!(fp_prefix(FP), "0123456789abcdef");
        assert_eq!(fp_prefix("sha256:ABCDEF"), "abcdef");
    }

    fn txt(pairs: &[(&str, &str)]) -> TxtProperties {
        ServiceInfo::new(SERVICE_TYPE, "x", "x.local.", "", 1, pairs)
            .unwrap()
            .get_properties()
            .clone()
    }

    #[test]
    fn matching_is_exact_on_the_fp_key() {
        assert!(matches(
            &txt(&[("fp", "0123456789abcdef"), ("v", "1")]),
            "0123456789abcdef"
        ));
        assert!(matches(
            &txt(&[("fp", "0123456789ABCDEF")]),
            "0123456789abcdef"
        ));
        assert!(!matches(
            &txt(&[("fp", "0123456789abcdef")]),
            "0123456789abcde"
        ));
        assert!(!matches(
            &txt(&[("fp", "0123456789abcde")]),
            "0123456789abcdef"
        ));
        assert!(!matches(&txt(&[("v", "1")]), "0123456789abcdef"));
        assert!(!matches(&txt(&[]), ""));
    }

    #[test]
    fn ipv4_is_preferred_then_routable_ipv6_then_anything() {
        let v4 = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
        let v6_ll = IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1));
        let v6 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert_eq!(pick_addr([v6_ll, v6, v4].into_iter()), Some(v4));
        assert_eq!(pick_addr([v6_ll, v6].into_iter()), Some(v6));
        assert_eq!(pick_addr([v6_ll].into_iter()), Some(v6_ll));
        assert_eq!(pick_addr(std::iter::empty()), None);
    }

    /// No desktop on the LAN: `None`, and no later than the timeout
    /// plus daemon teardown.
    #[test]
    fn browsing_for_nothing_times_out_to_none() {
        let started = Instant::now();
        assert_eq!(browse("ffffffffffffffff", Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// Register the desktop's record in-process and find it by prefix;
    /// a different prefix must not find it.
    #[test]
    fn browse_finds_the_desktop_with_the_matching_prefix() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros();
        let prefix = format!("{unique:016x}");
        let prefix = &prefix[prefix.len() - FP_PREFIX_LEN..];
        let port = 41919;

        let desktop = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("SKIPPED: mDNS daemon could not start on this runner: {e}");
                return;
            }
        };
        let monitor = desktop.monitor().unwrap();
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &format!("companion-test-{unique}"),
            &format!("headstate-{prefix}.local."),
            "",
            port,
            &[(TXT_FP, prefix), ("v", "1")][..],
        )
        .unwrap()
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        desktop.register(info).unwrap();

        let found = browse(prefix, Duration::from_secs(5));
        if found.is_none() {
            let mut errors = Vec::new();
            while let Ok(ev) = monitor.try_recv() {
                if let DaemonEvent::Error(e) = ev {
                    errors.push(e.to_string());
                }
            }
            if !errors.is_empty() {
                eprintln!(
                    "SKIPPED: no multicast on this runner: {}",
                    errors.join("; ")
                );
                let _ = desktop.shutdown();
                return;
            }
        }
        let (ip, found_port) = found.expect("the advertised desktop was not found");
        assert_eq!(found_port, port);
        assert!(!ip.is_unspecified());

        // Same LAN, a different desktop: not ours.
        assert_eq!(browse("0000000000000000", Duration::from_secs(1)), None);

        if let Ok(rx) = desktop.unregister(&fullname) {
            let _ = rx.recv_timeout(Duration::from_secs(1));
        }
        let _ = desktop.shutdown();
    }
}
