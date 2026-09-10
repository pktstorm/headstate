//! Which processes are using the network (#718).
//!
//! The Network panel reports totals per interface. That answers "is the
//! machine using the network", never "what is using it" -- the gap the
//! CPU and Memory pages closed by naming processes (#710). This closes
//! it for network, on the one platform that will say without elevation.
//!
//! # This reading costs FIVE SECONDS, and that decides everything else
//!
//! `nettop -P -L 1 -x -J bytes_in,bytes_out` is the unprivileged route
//! on macOS. It was measured before anything was built on it, because
//! #661 exists for exactly the mistake of not measuring:
//!
//! | invocation                              | wall time   |
//! |-----------------------------------------|-------------|
//! | `nettop -P -L 1 -x -J bytes_in,bytes_out` | 5.06-5.25 s |
//! | the same, plus `-s 1`                   | 5.06 s      |
//! | the same, plus `-d` (deltas)            | 5.11 s      |
//!
//! Re-measured on the implementing machine: 5.156 s and 5.231 s wall,
//! against **0.08 s of CPU**. That ratio is the whole explanation:
//! `nettop` is not computing for five seconds, it is SAMPLING for a
//! full interval before it prints anything, and `-L 1` waits that
//! interval out rather than skipping it. There is no flag that makes it
//! return promptly -- asking for less data (`-s 1`) or for deltas
//! (`-d`) changes nothing, because the cost is the interval and not the
//! work.
//!
//! For scale against the two readings already on the shared timer:
//!
//! | source                    | cost      |
//! |---------------------------|-----------|
//! | process table (#710)      | 19-37 ms  |
//! | `ioreg` (#705)            | ~30 ms    |
//! | **`nettop`**              | **~5,100 ms** |
//!
//! Two orders of magnitude past `ioreg`.
//!
//! # So this is NOT on the shared health timer
//!
//! `HEALTH_POLL_MS` is five seconds. A 5.1-second subprocess on a
//! 5-second timer means every poll's `nettop` OUTLIVES the interval
//! that spawned it: they would overlap continuously and the machine
//! would permanently host one or more `nettop` processes for as long as
//! the app was open. That is the #661 failure -- a slow command on a
//! shared timer -- in its worst available form, so this reading gets
//! its own command and its own cadence on the Network detail page,
//! where the page is open or it is not and nothing pays when it is
//! closed. The cadence itself is set in `api/hooks.ts`
//! (`NET_PROCESSES_POLL_MS`), which is where the argument for the
//! number lives.
//!
//! # These counts are CUMULATIVE, which is not a rate
//!
//! [`NetProcess::bytes_in`] and [`NetProcess::bytes_out`] are totals
//! since each process started, exactly like [`super::Interface`]'s
//! counters are totals since boot. One reading is therefore a
//! standing, not a speed: a process that moved 6 GB last week and
//! nothing since outranks one saturating the link right now.
//!
//! Turning that into a rate needs TWO readings differenced, which the
//! view does in `netProcessRates` (`lib/health.ts`) -- the per-process
//! sibling of the `counterRates` it already uses for interfaces, which
//! differs in having to MATCH processes across the two readings before
//! anything can be differenced at all.
//!
//! That is why the page is roughly TWENTY seconds from opening to its
//! first meaningful rate rather than five: one ~5s reading, then the
//! 15s cadence, then a second ~5s reading. The view is required to say
//! so rather than looking broken for that long; see `SystemHealthPage`.
//!
//! Deltas are deliberately not asked of `nettop` itself (`-d`). It
//! costs the same five seconds, and a delta over an interval `nettop`
//! chose is not a delta over the interval between OUR readings -- so
//! the cumulative figure is both the cheaper and the more honest one,
//! since a consumer that wants a total does not have to add deltas up
//! and a consumer that wants a rate has to difference something either
//! way.
//!
//! # `name.pid`, kept as the platform writes it
//!
//! `nettop` labels each row `name.pid` -- the same identity the CPU and
//! Memory pages already list from the process table (#710), which is
//! what lets a reader follow one busy process across the three pages.
//! [`NetProcess`] therefore carries `name` and `pid` split apart, so
//! the view can match on either, and splitting them is the one genuinely
//! delicate part of this file: see [`parse_nettop`].
//!
//! # Absent is not zero, and the other two platforms
//!
//! An empty list means "we did not look" -- there is no unprivileged
//! way to on this platform -- and the view renders it as "Not measured"
//! with the reason, never as a table of zeroes. A machine on which
//! nothing is using the network is not a thing that happens while an
//! app is polling it, so an empty table would be read as a broken panel
//! rather than as an idle one. This is #705's precedent: a
//! well-evidenced "this platform cannot be read unprivileged" is a
//! legitimate outcome and strictly better than a silently blank panel.
//!
//! **Linux -- CANNOT, unprivileged.** Three routes were considered and
//! all three are shut:
//!
//! - `/proc/<pid>/net/dev` LOOKS per-process and is not. It is the
//!   per-NETWORK-NAMESPACE counter set, reached through that process's
//!   `/proc` entry; every process in the root namespace reads the same
//!   numbers, which are the machine's interface totals the Network
//!   panel already shows. Reporting them per process would give every
//!   row an identical, whole-machine figure -- numbers that contradict
//!   the OS's own tools, which the issue explicitly rules out.
//! - `nethogs` attributes traffic by packet capture, which needs
//!   `CAP_NET_ADMIN` (or `CAP_NET_RAW`). Not available to an
//!   unprivileged desktop app, and shipping a setcap binary is a much
//!   larger decision than a health panel.
//! - eBPF (`bpf(2)` with a socket or kprobe program) needs
//!   `CAP_BPF`/`CAP_SYS_ADMIN` on any ordinary kernel; the unprivileged
//!   path is disabled by default (`kernel.unprivileged_bpf_disabled`)
//!   across mainstream distributions.
//!
//! `/proc/<pid>/io` is not a fourth route: it counts bytes through
//! read/write syscalls of every kind, so a process reading a file and
//! one reading a socket are indistinguishable in it.
//!
//! **Windows -- possible, deliberately not built.**
//! `GetPerTcpConnectionEStats` can be read unprivileged, but it is per
//! TCP CONNECTION rather than per process: it needs the connection
//! table enumerated (`GetTcpTable2`) to map each connection to a PID,
//! stats enabled per connection before they collect anything, and it
//! sees no UDP or QUIC at all -- which is most of a browser's traffic
//! now. ETW sees everything and needs an administrator to start a
//! kernel session. Half-built, the first would report a confident
//! number that disagreed with Task Manager, which is the failure mode
//! #705 named. So it reports nothing, and says so.

use serde::{Deserialize, Serialize};

/// One process's network totals, as the UI consumes it.
///
/// Both counts are CUMULATIVE since the process started, not since the
/// last reading -- the same contract as [`super::Interface`]. A
/// consumer that wants a rate differences two of these; see the module
/// docs for why that makes the first rate the second reading rather
/// than the first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetProcess {
    /// The process as the platform names it, with the PID stripped off.
    ///
    /// Kept separate from `pid` so the view can match this against the
    /// names the CPU and Memory pages list, which come from the process
    /// table and carry no PID suffix.
    pub name: String,
    /// The PID `nettop` appended to the name.
    ///
    /// `None` when the label carried no parseable trailing PID. That is
    /// not expected from `nettop`, but a row whose identity cannot be
    /// established still has real byte counts worth showing, and
    /// inventing a PID for it would be worse than admitting there is
    /// none.
    pub pid: Option<u32>,
    /// Bytes received since the process started.
    pub bytes_in: u64,
    /// Bytes sent since the process started.
    pub bytes_out: u64,
}

/// Every process the platform will attribute traffic to.
///
/// Empty means "not measured on this platform", which the view renders
/// as a stated reason rather than as an empty table. See the module
/// docs for why Linux and Windows are both empty.
///
/// **Blocking for ~5 seconds on macOS.** Every caller must be on a
/// blocking worker, and no caller may be on the shared health timer.
pub fn read() -> Vec<NetProcess> {
    platform()
}

/// macOS: `nettop`, the one unprivileged per-process attribution.
///
/// `-P` sums per process rather than per connection, `-x` prints raw
/// byte counts instead of human units, `-J` selects just the two
/// columns, and `-L 1` prints one sample and exits. The five seconds
/// that costs is argued in the module docs.
#[cfg(target_os = "macos")]
fn platform() -> Vec<NetProcess> {
    let out = std::process::Command::new("nettop")
        .args(["-P", "-L", "1", "-x", "-J", "bytes_in,bytes_out"])
        .output();
    let Ok(out) = out else {
        // `nettop` missing or refusing to run is "we could not look",
        // and an empty list is how this module says that.
        return Vec::new();
    };
    parse_nettop(&String::from_utf8_lossy(&out.stdout))
}

/// Linux and Windows: nothing readable without privileges we do not
/// have, so nothing is claimed. The evidence is in the module docs.
#[cfg(not(target_os = "macos"))]
fn platform() -> Vec<NetProcess> {
    Vec::new()
}

/// Pull the rows out of `nettop`'s CSV-ish output.
///
/// The shape, from a real run (names invented here, per
/// `CONTRIBUTING.md`):
///
/// ```text
/// ,bytes_in,bytes_out,
/// acme-sync.1,0,0
/// widget-daemon.376,6800258,14482767
/// ```
///
/// # The two ways a naive parse corrupts this
///
/// **A comma is the field separator and the label is first.** Splitting
/// on `.` first, or on the last `.` in the whole LINE, reaches into the
/// numbers: `acme.tool.88,120,340` split on its final `.` yields a name
/// of `acme.tool.88,120` and a PID of `340`. So the line is split on
/// commas FIRST and the label is only then taken apart.
///
/// **A process name can itself contain dots.** Four of the 56 rows on
/// the measuring machine had names with one to three dots in them --
/// this is the ordinary case on macOS, not an exotic one, since bundle
/// identifiers and versioned helpers are named that way. Splitting the
/// label on its FIRST `.` truncates every one of those names at its
/// first dot; the PID is after the LAST one.
///
/// Split out from [`platform`] because it is the only way to exercise
/// those shapes: a test cannot make the machine run a process whose
/// name contains three dots.
///
/// Rows that cannot be parsed are DROPPED rather than defaulted to
/// zero. A fabricated zero here would be a claim that a named process
/// moved no bytes, which is the "absent is not zero" rule this whole
/// module is built on.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_nettop(text: &str) -> Vec<NetProcess> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Commas first, always. See the doc comment: the label is field
        // 0 and everything about taking it apart has to happen after
        // the numbers are safely separated from it.
        let mut fields = line.split(',');
        let (Some(label), Some(bytes_in), Some(bytes_out)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // The header row is `,bytes_in,bytes_out,` -- an empty label
        // with the column names where the numbers go. It fails the
        // numeric parse below anyway, but skipping it explicitly means
        // a future header that happened to parse could not become a
        // row named "".
        let label = label.trim();
        if label.is_empty() {
            continue;
        }
        let (Ok(bytes_in), Ok(bytes_out)) = (
            bytes_in.trim().parse::<u64>(),
            bytes_out.trim().parse::<u64>(),
        ) else {
            continue;
        };
        // The PID is after the LAST dot, because the name may contain
        // its own. A label with no dot, or a trailing segment that is
        // not a number, keeps the whole label as the name and reports
        // no PID rather than guessing one.
        let (name, pid) = match label.rsplit_once('.') {
            Some((head, tail)) => match tail.parse::<u32>() {
                Ok(pid) if !head.is_empty() => (head.to_string(), Some(pid)),
                _ => (label.to_string(), None),
            },
            None => (label.to_string(), None),
        };
        out.push(NetProcess {
            name,
            pid,
            bytes_in,
            bytes_out,
        });
    }
    out
}

#[cfg(test)]
/// Every process name in these fixtures is INVENTED. This repo is
/// public and a real process list names what a person runs; per
/// `CONTRIBUTING.md`, fixtures carry synthetic names only.
mod tests {
    #![allow(unused_imports)]
    use super::*;

    /// The ordinary shape, header row and all.
    #[test]
    fn reads_the_rows_and_skips_the_header() {
        let text = ",bytes_in,bytes_out,\nacme-sync.1,0,0\nwidget-daemon.376,6800258,14482767\n";
        let rows = parse_nettop(text);
        assert_eq!(rows.len(), 2, "the header must not become a row");
        assert_eq!(rows[0].name, "acme-sync");
        assert_eq!(rows[0].pid, Some(1));
        assert_eq!(rows[0].bytes_in, 0);
        assert_eq!(rows[0].bytes_out, 0);
        assert_eq!(rows[1].name, "widget-daemon");
        assert_eq!(rows[1].pid, Some(376));
        assert_eq!(rows[1].bytes_in, 6_800_258);
        assert_eq!(rows[1].bytes_out, 14_482_767);
    }

    /// **A process name containing dots keeps all of them.**
    ///
    /// Not an exotic case: four of the 56 rows on the measuring machine
    /// had one to three dots in the name, because bundle identifiers
    /// and versioned helpers are named that way. Splitting the label on
    /// its FIRST dot truncates every one of them -- `com.acme.helper`
    /// becomes `com`, and three unrelated helpers under one vendor
    /// prefix collapse into one indistinguishable row.
    #[test]
    fn a_dotted_process_name_is_not_truncated() {
        let text = "com.acme.widget.helper.4021,512,1024\n";
        let rows = parse_nettop(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].name, "com.acme.widget.helper",
            "the PID is after the LAST dot, not the first"
        );
        assert_eq!(rows[0].pid, Some(4021));
        assert_eq!(rows[0].bytes_in, 512);
        assert_eq!(rows[0].bytes_out, 1024);
    }

    /// **The comma is the field separator, so it is split on FIRST.**
    ///
    /// A parse that goes looking for the last dot in the whole LINE
    /// reaches past the label and into the numbers: here it would find
    /// the dot before `88` only after `,512,1024` had already been
    /// swept into the name. This is the mutation that produces a name
    /// with digits and commas glued to it and a PID taken from a byte
    /// count.
    #[test]
    fn the_numbers_are_never_swept_into_the_name() {
        let text = "acme.tool.88,512,1024\n";
        let rows = parse_nettop(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "acme.tool");
        assert_eq!(rows[0].pid, Some(88));
        assert_eq!(rows[0].bytes_in, 512, "the first number is bytes_in");
        assert_eq!(rows[0].bytes_out, 1024);
        assert!(
            !rows[0].name.contains(','),
            "a comma in a name means the numbers were parsed as part of it"
        );
    }

    /// A label with no trailing number reports NO pid rather than a
    /// made-up one, and keeps its whole label as the name.
    #[test]
    fn a_label_without_a_pid_reports_none_rather_than_guessing() {
        let rows = parse_nettop("acme-relay,7,9\nacme.helper.stub,1,2\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "acme-relay");
        assert_eq!(rows[0].pid, None);
        // The trailing segment is not a number, so it is part of the
        // name -- not a PID, and not silently dropped from the name.
        assert_eq!(rows[1].name, "acme.helper.stub");
        assert_eq!(rows[1].pid, None);
    }

    /// A row whose counts will not parse is DROPPED, never zeroed.
    ///
    /// A zero here would be a claim that a named process moved no
    /// bytes, which is the exact "absent is not zero" failure the rest
    /// of `health` exists to refuse. The good rows around it still come
    /// through.
    #[test]
    fn an_unparseable_row_is_dropped_rather_than_zeroed() {
        let text =
            ",bytes_in,bytes_out,\nacme-sync.1,100,200\nbroken-daemon.2,-,-\nacme-relay.3,1,2\n";
        let rows = parse_nettop(text);
        assert_eq!(rows.len(), 2, "the broken row must not become a zero row");
        assert!(
            !rows.iter().any(|r| r.name == "broken-daemon"),
            "a row whose bytes could not be read is absent, not zero"
        );
        assert_eq!(rows[1].name, "acme-relay");
    }

    /// Blank lines and a trailing newline do not produce empty rows.
    #[test]
    fn blank_lines_are_not_rows() {
        assert!(parse_nettop("\n\n   \n").is_empty());
    }

    /// The trailing comma `nettop` prints after `bytes_out` does not
    /// disturb the parse: the fourth field is simply never read.
    #[test]
    fn a_trailing_field_is_ignored() {
        let rows = parse_nettop("acme-sync.1,5,6,\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bytes_in, 5);
        assert_eq!(rows[0].bytes_out, 6);
    }

    /// On every platform but macOS this reports nothing at all, which
    /// the view renders as a stated reason. Pinned as a test because
    /// "empty" is a deliberate claim here -- see the module docs for
    /// why neither Linux nor Windows has an unprivileged route.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn platforms_without_an_unprivileged_route_report_nothing() {
        assert!(read().is_empty());
    }
}
