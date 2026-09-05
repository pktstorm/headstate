//! LAN advertisement of the listener: `_headstate._tcp` over mDNS.
//!
//! A phone remembers the addresses the QR code offered at pairing. When
//! the desktop moves -- a new DHCP lease, a different network -- those
//! go stale, and on the same LAN the phone can still find it by browsing
//! for this record and matching the fingerprint in its TXT data against
//! the one it pinned. The record is a hint only: mTLS on the connection
//! is what proves the address belongs to the paired desktop.
//!
//! # The record
//!
//! ```text
//! <instance name>._headstate._tcp.local.   SRV  port=<listener port>
//!                                          TXT  fp=<first 16 hex of the
//!                                                   certificate SHA256>
//!                                               v=1
//! ```
//!
//! The instance name is the display name the QR code carries, so what a
//! Bonjour browser shows matches what the phone shows. The mDNS host
//! name is `headstate-<fp prefix>.local.`, not the machine's hostname:
//! it must be a single DNS label, must not collide with another desktop
//! on the LAN, and nothing reads it -- the phone connects to the
//! resolved addresses. Deriving it from the fingerprint satisfies all
//! three without a hostname crate. Addresses are filled in by the daemon
//! from the host's interfaces and kept current as they change.
//!
//! # Best effort
//!
//! Advertising is started by `gate::start` after the listener is up and
//! stopped by `gate::stop`, and a failure to advertise never fails the
//! listener: the phone still has the stored addresses, and on Windows
//! and Linux the multicast setup varies enough (firewalls, no Avahi,
//! interfaces without multicast) that "could not advertise" is a warning
//! in the log, not a broken toggle. The daemon opens its sockets on its
//! own thread, so the errors that matter arrive later than
//! [`Advertisement::start`]; they are drained to the log.

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};
use std::time::Duration;

/// The service type, with the mDNS domain, as `mdns-sd` wants it.
pub const SERVICE_TYPE: &str = "_headstate._tcp.local.";
/// TXT key carrying the fingerprint prefix.
pub const TXT_FP: &str = "fp";
/// TXT key carrying the record version.
pub const TXT_VERSION: &str = "v";
/// The record version this desktop advertises.
pub const RECORD_VERSION: &str = "1";
/// How many hex characters of the fingerprint go on the wire. Sixteen
/// is 64 bits: plenty to tell desktops apart, and short enough that the
/// full fingerprint stays out of every multicast packet.
pub const FP_PREFIX_LEN: usize = 16;

/// How long `stop` waits for the goodbye packets and the shutdown.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// The first [`FP_PREFIX_LEN`] characters of a lowercase hex
/// fingerprint. A shorter input comes back whole.
pub fn fp_prefix(fingerprint_hex: &str) -> String {
    fingerprint_hex.chars().take(FP_PREFIX_LEN).collect()
}

/// The record for one desktop. Pure; separated from [`Advertisement`]
/// so the shape can be tested without a socket.
pub fn service_info(
    instance_name: &str,
    port: u16,
    fingerprint_hex: &str,
) -> Result<ServiceInfo, String> {
    let prefix = fp_prefix(fingerprint_hex);
    let host = format!("headstate-{prefix}.local.");
    let txt = [(TXT_FP, prefix.as_str()), (TXT_VERSION, RECORD_VERSION)];
    ServiceInfo::new(SERVICE_TYPE, instance_name, &host, "", port, &txt[..])
        .map(ServiceInfo::enable_addr_auto)
        .map_err(|e| e.to_string())
}

/// A running advertisement. Dropping it without [`stop`](Self::stop)
/// leaves the daemon thread to exit on its own with no goodbye packet,
/// so peers keep the record until its TTL runs out.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    /// Start advertising. Does not block: the daemon runs its own thread
    /// and registration is a message to it. Errors here are the daemon
    /// failing to start at all; socket-level failures show up in the log
    /// afterwards.
    pub fn start(instance_name: &str, port: u16, fingerprint_hex: &str) -> Result<Self, String> {
        let info = service_info(instance_name, port, fingerprint_hex)?;
        let fullname = info.get_fullname().to_string();
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        if let Ok(events) = daemon.monitor() {
            std::thread::spawn(move || drain_events(events));
        }
        daemon.register(info).map_err(|e| e.to_string())?;
        log::info!("advertising {fullname} on port {port}");
        Ok(Self { daemon, fullname })
    }

    /// The registered name, `<instance>._headstate._tcp.local.`.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Withdraw the record and shut the daemon down. Blocks for up to
    /// about twice [`STOP_TIMEOUT`]; call it off the async workers.
    pub fn stop(self) {
        match self.daemon.unregister(&self.fullname) {
            Ok(rx) => {
                // `Err` here is a timeout: the goodbye may not have gone
                // out, and the TTL covers that. Nothing more to do.
                let _ = rx.recv_timeout(STOP_TIMEOUT);
            }
            Err(e) => log::warn!("could not withdraw {}: {e}", self.fullname),
        }
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(STOP_TIMEOUT);
        }
        log::info!("stopped advertising {}", self.fullname);
    }
}

/// Log what the daemon reports. Exits when the daemon shuts down and
/// closes the channel.
fn drain_events(events: mdns_sd::Receiver<DaemonEvent>) {
    while let Ok(event) = events.recv() {
        match event {
            DaemonEvent::Error(e) => log::warn!("mDNS: {e}"),
            DaemonEvent::NameChange(change) => log::info!("mDNS renamed: {change:?}"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::ServiceEvent;
    use std::time::{Instant, SystemTime};

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_prefix_is_the_first_sixteen_hex_characters() {
        assert_eq!(fp_prefix(FP), "0123456789abcdef");
        assert_eq!(fp_prefix("abc"), "abc");
        assert_eq!(fp_prefix(""), "");
    }

    #[test]
    fn the_record_carries_the_port_the_prefix_and_the_version() {
        let info = service_info("octocat's laptop", 41919, FP).unwrap();
        assert_eq!(
            info.get_fullname(),
            "octocat's laptop._headstate._tcp.local."
        );
        assert_eq!(info.get_type(), SERVICE_TYPE);
        assert_eq!(info.get_port(), 41919);
        assert_eq!(info.get_property_val_str(TXT_FP), Some("0123456789abcdef"));
        assert_eq!(info.get_property_val_str(TXT_VERSION), Some("1"));
        assert_eq!(info.get_properties().len(), 2);
        assert_eq!(info.get_hostname(), "headstate-0123456789abcdef.local.");
        assert!(info.is_addr_auto());
    }

    /// A dot in the display name must not become a label boundary.
    #[test]
    fn a_dotted_display_name_is_escaped_not_split() {
        let info = service_info("mac.local", 1, FP).unwrap();
        assert_eq!(info.get_fullname(), "mac\\.local._headstate._tcp.local.");
    }

    /// Whether a browsing daemon in this process can see multicast at
    /// all. A runner without it reports socket errors on the monitor;
    /// that is the case the round-trip test skips on, with a message.
    fn multicast_error(monitor: &mdns_sd::Receiver<DaemonEvent>) -> Option<String> {
        while let Ok(event) = monitor.try_recv() {
            if let DaemonEvent::Error(e) = event {
                return Some(e.to_string());
            }
        }
        None
    }

    /// Start, see the record from a second daemon, stop, see it go.
    #[test]
    fn start_registers_and_stop_withdraws_the_record() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros();
        let name = format!("headstate-test-{unique}");
        let port = 41919;

        let browser = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("SKIPPED: mDNS daemon could not start on this runner: {e}");
                return;
            }
        };
        let monitor = browser.monitor().unwrap();
        let events = browser.browse(SERVICE_TYPE).unwrap();

        let advert = match Advertisement::start(&name, port, FP) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("SKIPPED: advertisement could not start on this runner: {e}");
                return;
            }
        };
        let fullname = advert.fullname().to_string();
        assert_eq!(fullname, format!("{name}.{SERVICE_TYPE}"));

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut resolved = false;
        while Instant::now() < deadline {
            match events.recv_timeout(Duration::from_millis(200)) {
                Ok(ServiceEvent::ServiceResolved(info)) if info.fullname == fullname => {
                    assert_eq!(info.port, port);
                    assert_eq!(
                        info.txt_properties.get_property_val_str(TXT_FP),
                        Some("0123456789abcdef")
                    );
                    assert_eq!(
                        info.txt_properties.get_property_val_str(TXT_VERSION),
                        Some("1")
                    );
                    assert!(!info.addresses.is_empty(), "resolved with no address");
                    resolved = true;
                    break;
                }
                _ => {}
            }
        }
        if !resolved {
            if let Some(e) = multicast_error(&monitor) {
                eprintln!("SKIPPED: no multicast on this runner: {e}");
                advert.stop();
                return;
            }
            panic!("{fullname} was never resolved");
        }

        advert.stop();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut removed = false;
        while Instant::now() < deadline {
            if let Ok(ServiceEvent::ServiceRemoved(_, f)) =
                events.recv_timeout(Duration::from_millis(200))
            {
                if f == fullname {
                    removed = true;
                    break;
                }
            }
        }
        assert!(removed, "{fullname} was never withdrawn");
        let _ = browser.shutdown();
    }
}
