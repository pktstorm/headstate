//! Deciding when a battery is worth interrupting someone for (#720).
//!
//! Three conditions, one mechanism: low charge, unusually fast
//! discharge, and discharging while plugged in. The third is the reason
//! this module is worth having -- a machine draining on mains power
//! looks fine at every glance, because the plug is in, and nothing else
//! in the app would ever say so.
//!
//! # Everything here is pure
//!
//! [`evaluate`] takes a slice of samples and a threshold and returns
//! what is wrong. It touches no clock, no database and no notification
//! API, which is what lets the cases this module exists for -- an alert
//! that must NOT fire across a gap -- be tested as arithmetic rather
//! than by arranging a laptop to fall asleep.
//!
//! # Why a rate cannot be one subtraction
//!
//! The sampler writes once a minute and ONLY while the app runs, so the
//! series has holes by design. Three separate things make a naive
//! delta lie:
//!
//! 1. **Gaps.** Two samples either side of a closed lid are twelve
//!    hours apart, and the charge legitimately fell across them. A
//!    percent-per-minute computed over that pair is arithmetic over a
//!    period nobody measured. This mirrors `splitOnGaps` in
//!    `src/lib/health.ts`, which is the same rule for the charts -- see
//!    [`GAP_MS`].
//! 2. **Sleep looks exactly like a cliff.** Waking from suspend shows a
//!    large drop between two adjacent rows, and the timestamps show the
//!    gap. Rule 1 catches it, which is why rule 1 is not optional.
//! 3. **Noise.** Charge readings jump around near the extremes, and a
//!    battery reporting 44 then 42 then 44 has not discharged at
//!    2%/minute. So a single bad delta never fires: the trend must hold
//!    across [`SUSTAINED_SAMPLES`] consecutive, ungapped, discharging
//!    intervals.
//!
//! # Why "fired" is a set, not a bool
//!
//! A battery sitting at 24% must alert ONCE, not every sixty seconds
//! forever. [`Fired`] is the caller's memory of what it has already
//! said, and [`Alert::key`] is what it remembers -- the same
//! transition-only discipline as `poll::newly_broken`, which is the
//! established shape for "do not re-report a condition that is merely
//! still true".

use super::Sample;

/// The spacing above which two samples are not comparable.
///
/// MUST agree with `ABSOLUTE_GAP_MS` in `src/lib/health.ts`, which is
/// the same rule applied to the charts. The two are separate constants
/// in separate languages, so the agreement is asserted by
/// `src/lib/mirroredConstants.test.ts`, which reads THIS FILE's literal
/// via Vite's `?raw` and compares it to the TypeScript one.
///
/// That test is where the agreement lives because
/// `a_gap_here_is_a_gap_in_the_charts_too` below cannot hold it: a Rust
/// test can only compare `GAP_MS` to another Rust literal, which reads
/// one side twice and passes at any value the other side has taken
/// (#850).
///
/// Thirty minutes is comfortably above any legitimate spacing: the
/// sampler writes every sixty seconds, and the coarsest a stored series
/// is ever read back at is twelve minutes (`MAX_POINTS` across the
/// 24-hour window).
pub const GAP_MS: i64 = 30 * 60 * 1000;

/// How many consecutive ungapped intervals a discharge trend must hold.
///
/// Three intervals is four samples, so roughly three minutes at the
/// sampler's cadence. Chosen against the issue's own example -- "more
/// than 1% per 2 minutes sustained" -- as the smallest window that
/// spans it while still outlasting the one-sample wobble described in
/// the module docs. One interval would fire on noise; ten would take
/// ten minutes to notice a runaway process, by which point the user has
/// already watched the number fall.
pub const SUSTAINED_SAMPLES: usize = 3;

/// The discharge rate that counts as unusually fast, in percent per
/// minute.
///
/// The issue's figure: 1% per 2 minutes. Expressed per-minute because
/// that is the unit the intervals are measured in, and left as a
/// constant rather than a setting because -- unlike the low-charge
/// threshold, which is a personal preference about when to go looking
/// for a charger -- this one is a claim about a machine misbehaving,
/// and a user has no basis for tuning it.
pub const FAST_DISCHARGE_PER_MIN: f64 = 0.5;

/// The default low-charge threshold, in percent.
///
/// #720's default. Overridable in Settings, because when to worry about
/// charge depends entirely on how far the user is from a plug.
pub const DEFAULT_LOW_PERCENT: u32 = 25;

/// What the low-charge alert re-arms at, above the threshold.
///
/// Without this the alert flaps: a battery hovering at exactly the
/// threshold crosses it back and forth on measurement noise alone, and
/// each upward flicker would clear the fired-flag so the next downward
/// one re-notifies. Five points is comfortably wider than the wobble
/// and much narrower than any real recharge.
pub const LOW_CLEAR_MARGIN: f64 = 5.0;

/// Something about the battery worth interrupting the user for.
#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    /// Charge fell below the configured threshold.
    Low { percent: f64, threshold: u32 },
    /// Charge is falling faster than [`FAST_DISCHARGE_PER_MIN`], held
    /// across [`SUSTAINED_SAMPLES`] ungapped intervals.
    FastDischarge { percent_per_min: f64 },
    /// Charge is falling while `on_ac` is true.
    ///
    /// The valuable one. Either the machine is drawing more than the
    /// adapter supplies, or the adapter is not really charging -- and
    /// both look completely normal to someone glancing at a plugged-in
    /// laptop.
    DrainingOnAc { percent_per_min: f64 },
}

impl Alert {
    /// The identity the caller remembers, so a condition that is merely
    /// STILL TRUE does not notify again.
    ///
    /// Deliberately NOT including the numbers: keying on the percentage
    /// would make 24% and 23% different alerts, and a battery falling
    /// through the twenties would notify at every point on the way
    /// down. The condition is the identity; the number is only the
    /// wording.
    pub fn key(&self) -> &'static str {
        match self {
            Alert::Low { .. } => "low",
            Alert::FastDischarge { .. } => "fast_discharge",
            Alert::DrainingOnAc { .. } => "draining_on_ac",
        }
    }

    /// The notification title.
    pub fn title(&self) -> String {
        match self {
            Alert::Low { percent, .. } => format!("Battery at {:.0}%", percent),
            Alert::FastDischarge { .. } => "Battery draining fast".to_string(),
            Alert::DrainingOnAc { .. } => "Draining while plugged in".to_string(),
        }
    }

    /// The notification body: what was measured, and why it is being
    /// said.
    pub fn body(&self) -> String {
        match self {
            Alert::Low { threshold, .. } => {
                format!("Charge has fallen below {threshold}%.")
            }
            Alert::FastDischarge { percent_per_min } => format!(
                "Losing about {:.1}% a minute — faster than normal use, so something may be running away.",
                percent_per_min
            ),
            Alert::DrainingOnAc { percent_per_min } => format!(
                "The machine is on AC power but still losing about {:.1}% a minute. The adapter may not be keeping up, or may not be charging.",
                percent_per_min
            ),
        }
    }
}

/// Which alerts have already been announced.
///
/// The caller owns one of these across sampler ticks. An alert whose
/// key is present is not re-raised; a condition that clears removes its
/// key and re-arms.
#[derive(Debug, Default, Clone)]
pub struct Fired {
    keys: Vec<&'static str>,
    /// The threshold the standing low-charge alert fired at.
    ///
    /// Kept so [`LOW_CLEAR_MARGIN`] is measured against the number that
    /// actually fired: a user who raises the threshold in Settings
    /// while an alert stands would otherwise have it re-arm against a
    /// band it was never inside.
    low_threshold: Option<u32>,
}

impl Fired {
    fn has(&self, key: &str) -> bool {
        self.keys.contains(&key)
    }

    fn set(&mut self, key: &'static str) {
        if !self.has(key) {
            self.keys.push(key);
        }
    }

    fn clear(&mut self, key: &str) {
        self.keys.retain(|k| *k != key);
    }

    /// Filter `alerts` down to those not already announced, and re-arm
    /// any condition that has stopped being true.
    ///
    /// One call does both halves on purpose: a caller that remembered
    /// to suppress but forgot to re-arm would announce a condition once
    /// per app lifetime, which is worse than announcing it every
    /// minute -- the user would conclude the feature does not work.
    ///
    /// `charge` is the current charge, used ONLY for the low-charge
    /// hysteresis described on [`LOW_CLEAR_MARGIN`]. `None` where the
    /// machine has no battery, in which case there is nothing to
    /// hold open.
    pub fn take_new(&mut self, alerts: &[Alert], charge: Option<f64>) -> Vec<Alert> {
        for key in ["low", "fast_discharge", "draining_on_ac"] {
            if alerts.iter().any(|a| a.key() == key) {
                continue;
            }
            // Low charge re-arms with HYSTERESIS, the other two
            // immediately.
            //
            // A battery hovering at the threshold crosses it back and
            // forth on measurement noise alone -- 24, 25, 24 -- and
            // clearing on the first reading at or above it would let
            // the next dip notify again. So the fired-flag is held
            // until the charge has climbed clear of the band, which
            // takes a real recharge rather than a flicker.
            //
            // The other two conditions need none of this: both are
            // already sustained trends across several intervals, so
            // they cannot flap on one noisy reading in the first place.
            if key == "low" && self.has("low") {
                if let (Some(c), Some(t)) = (charge, self.low_threshold) {
                    if c < f64::from(t) + LOW_CLEAR_MARGIN {
                        continue;
                    }
                }
            }
            self.clear(key);
        }
        let mut out = Vec::new();
        for a in alerts {
            if !self.has(a.key()) {
                if let Alert::Low { threshold, .. } = a {
                    // Remembered so the margin above is measured
                    // against the threshold that actually fired, not
                    // whatever Settings holds when it clears.
                    self.low_threshold = Some(*threshold);
                }
                self.set(a.key());
                out.push(a.clone());
            }
        }
        out
    }

    /// The same transition-only filter for keys this module does not
    /// own: `keys` is the complete set of keys the caller's rules can
    /// produce, and `present` the subset currently true.
    ///
    /// Added for the CPU runaway rules (#791) rather than giving them a
    /// second `Fired` type, because "announce a transition, not a
    /// standing condition" is one piece of state per app and two copies
    /// of it would be two places to forget to re-arm. What it does NOT
    /// share is [`take_new`]'s hysteresis: that is specific to a battery
    /// hovering at a user-set threshold, and the rules here are already
    /// sustained over many samples so they cannot flap on one reading
    /// -- the same reasoning that gives `fast_discharge` no margin.
    ///
    /// Returns the keys that are NEW, in `present`'s order, and re-arms
    /// every key in `keys` that is absent from it. `keys` is passed
    /// whole rather than inferred from `present` for the reason the
    /// doc on [`take_new`] gives: a caller that suppressed but never
    /// re-armed would announce a condition once per app lifetime, and
    /// the user would conclude the feature does not work.
    ///
    /// [`take_new`]: Fired::take_new
    pub fn take_new_keys(
        &mut self,
        keys: &[&'static str],
        present: &[&'static str],
    ) -> Vec<&'static str> {
        for key in keys {
            if !present.contains(key) {
                self.clear(key);
            }
        }
        let mut out = Vec::new();
        for key in present {
            if !self.has(key) {
                self.set(key);
                out.push(*key);
            }
        }
        out
    }
}

/// One measured interval between two consecutive samples.
struct Interval {
    /// Minutes elapsed. Always positive and never wider than
    /// [`GAP_MS`], because a wider one is not an interval at all.
    minutes: f64,
    /// Percentage points LOST. Negative while charging.
    dropped: f64,
    /// Whether the machine was on AC across the whole interval.
    ///
    /// Both endpoints, not just the newer one: a laptop unplugged
    /// halfway through the minute is not evidence of an adapter failing
    /// to keep up, and counting it would make "draining while plugged
    /// in" fire every time someone pulls the cable.
    on_ac: bool,
}

/// The ungapped intervals at the END of the series, newest last.
///
/// Stops at the first gap rather than skipping it. A trend is about
/// what has been happening continuously up to now, so a run of good
/// intervals from before a six-hour hole says nothing about the present
/// -- and stitching the two sides together is exactly the arithmetic
/// across an unmeasured period this module refuses to do.
fn recent_intervals(samples: &[Sample]) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for pair in samples.windows(2).rev() {
        let (a, b) = (&pair[0], &pair[1]);
        let (Some(ba), Some(bb)) = (a.battery.as_ref(), b.battery.as_ref()) else {
            // A sample with no battery reading is the same kind of
            // unknown as a gap: `splitOnGaps` breaks a run on a null
            // value for this reason, and so does this.
            break;
        };
        let (Ok(ta), Ok(tb)) = (
            chrono::DateTime::parse_from_rfc3339(&a.sampled_at),
            chrono::DateTime::parse_from_rfc3339(&b.sampled_at),
        ) else {
            break;
        };
        let ms = (tb.timestamp_millis() - ta.timestamp_millis()) as f64;
        // Non-positive spacing means the rows are out of order or share
        // an instant, and dividing by it produces an infinity that
        // would clear every threshold at once.
        if ms <= 0.0 || ms > GAP_MS as f64 {
            break;
        }
        out.push(Interval {
            minutes: ms / 60_000.0,
            dropped: ba.percent - bb.percent,
            on_ac: ba.on_ac && bb.on_ac,
        });
    }
    out.reverse();
    out
}

/// The mean discharge rate across `intervals`, in percent per minute.
///
/// Weighted by elapsed time rather than a mean of per-interval rates:
/// the downsampled history can hand back intervals of different widths,
/// and averaging their rates would let one short noisy interval count
/// as much as a long quiet one.
fn rate(intervals: &[Interval]) -> f64 {
    let minutes: f64 = intervals.iter().map(|i| i.minutes).sum();
    if minutes <= 0.0 {
        return 0.0;
    }
    intervals.iter().map(|i| i.dropped).sum::<f64>() / minutes
}

/// Everything currently true of this series, worst-case first.
///
/// `samples` is oldest-first, as `store::health::history` returns it.
/// `low_percent` is the configured threshold.
///
/// Returns what IS true, not what is NEW -- deduplication is [`Fired`]'s
/// job, kept separate so this function stays a pure statement about the
/// data and the "have we said this already" state has exactly one home.
pub fn evaluate(samples: &[Sample], low_percent: u32) -> Vec<Alert> {
    let mut out = Vec::new();
    let Some(last) = samples.last() else {
        return out;
    };
    let Some(battery) = last.battery.as_ref() else {
        return out;
    };

    // Low charge needs no history at all: it is a fact about the
    // current reading, and requiring an interval would mean saying
    // nothing about a machine that has just been opened at 8%.
    if battery.percent < low_percent as f64 {
        out.push(Alert::Low {
            percent: battery.percent,
            threshold: low_percent,
        });
    }

    let intervals = recent_intervals(samples);
    // Fewer intervals than the trend needs is not "no discharge", it is
    // "not enough measured time to say" -- the app was just opened, or
    // the series resumes after a gap. Silence is the honest answer.
    if intervals.len() < SUSTAINED_SAMPLES {
        return out;
    }
    let window = &intervals[intervals.len() - SUSTAINED_SAMPLES..];

    // EVERY interval in the window must be a discharge, not merely the
    // average. A battery that fell four points and then gained three
    // back has an average that still looks like a discharge, and that
    // is the noise case this guard exists for.
    if !window.iter().all(|i| i.dropped > 0.0) {
        return out;
    }
    let per_min = rate(window);

    // Checked before the rate alert, and NOT gated on the rate: an
    // adapter that is not keeping up is worth saying at any speed. That
    // is the whole point of this one -- it needs no threshold tuning,
    // because a plugged-in machine should not be losing charge at all.
    if window.iter().all(|i| i.on_ac) {
        out.push(Alert::DrainingOnAc {
            percent_per_min: per_min,
        });
    } else if per_min > FAST_DISCHARGE_PER_MIN {
        // `else`: a machine draining on AC is already being reported,
        // and adding "and it is draining fast" would be two
        // notifications about one fault.
        out.push(Alert::FastDischarge {
            percent_per_min: per_min,
        });
    }

    out
}

/// The configured low-charge threshold, clamped to something sane.
///
/// Mirrors `poll::stale_venv_days`: 0 means "never set", not "alert at
/// zero percent", so a stored default does not silently disable the
/// feature. The ceiling stops a typo'd 95 from alerting on a nearly
/// full battery every minute the app is open.
pub fn low_percent(configured: u32) -> u32 {
    match configured {
        0 => DEFAULT_LOW_PERCENT,
        d => d.clamp(5, 90),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::{Battery, Memory, Sample};

    /// A series at the sampler's own cadence, ending now.
    ///
    /// Each entry is `(percent, on_ac)`, oldest first, one minute
    /// apart -- so a test only says the thing it is about.
    fn series(points: &[(f64, bool)]) -> Vec<Sample> {
        let end = chrono::Utc::now();
        points
            .iter()
            .enumerate()
            .map(|(i, (percent, on_ac))| {
                let at = end - chrono::Duration::minutes((points.len() - 1 - i) as i64);
                let mut s = bare(&at.to_rfc3339());
                s.battery = Some(Battery {
                    percent: *percent,
                    on_ac: *on_ac,
                    capacity_percent: Some(84.0),
                    cycle_count: Some(413),
                    power: None,
                });
                s
            })
            .collect()
    }

    fn bare(at: &str) -> Sample {
        Sample {
            sampled_at: at.to_string(),
            load: None,
            cpu_percent: None,
            cpu_per_core: vec![],
            memory: Memory {
                total: 1,
                used: 1,
                available: 0,
                swap_total: 0,
                swap_used: 0,
            },
            gpus: vec![],
            disks: vec![],
            battery: None,
            thermal: None,
            networks: vec![],
            uptime_secs: 1,
        }
    }

    #[test]
    fn a_battery_below_the_threshold_is_low() {
        let s = series(&[(30.0, false), (26.0, false), (24.0, false)]);
        let alerts = evaluate(&s, 25);
        assert!(alerts.iter().any(|a| a.key() == "low"));
    }

    #[test]
    fn a_battery_above_the_threshold_is_not() {
        let s = series(&[(90.0, false), (89.0, false), (88.0, false)]);
        assert!(!evaluate(&s, 25).iter().any(|a| a.key() == "low"));
    }

    /// **The mutation test for #720's central requirement.**
    ///
    /// A closed laptop wakes with much less charge than it had, and the
    /// two samples either side are hours apart. Drawn naively that is a
    /// catastrophic discharge rate -- 40 points in one "interval" --
    /// and it is arithmetic over a period nobody measured.
    ///
    /// If this test fails, the rate alert is computing across gaps and
    /// every user who closes their lid gets a false alarm on waking.
    #[test]
    fn no_rate_alert_is_computed_across_a_gap() {
        let end = chrono::Utc::now();
        let at = |mins: i64| (end - chrono::Duration::minutes(mins)).to_rfc3339();
        let mut samples = Vec::new();
        for (mins, percent) in [(722, 95.0), (721, 95.0), (720, 95.0)] {
            let mut s = bare(&at(mins));
            s.battery = Some(Battery {
                percent,
                on_ac: false,
                capacity_percent: None,
                cycle_count: None,
                power: None,
            });
            samples.push(s);
        }
        // Twelve hours asleep, then awake at 55%: a 40-point fall that
        // the app did not watch happen.
        for (mins, percent) in [(2, 55.0), (1, 54.0), (0, 53.0)] {
            let mut s = bare(&at(mins));
            s.battery = Some(Battery {
                percent,
                on_ac: false,
                capacity_percent: None,
                cycle_count: None,
                power: None,
            });
            samples.push(s);
        }

        let alerts = evaluate(&samples, 25);
        assert!(
            !alerts.iter().any(|a| a.key() == "fast_discharge"),
            "a rate must never be computed across a period nobody measured: {alerts:?}"
        );
        // And the intervals that WERE measured are the only ones used:
        // three post-wake intervals is two, one short of the trend, so
        // there is nothing to report yet either way.
        assert!(
            !alerts.iter().any(|a| a.key() == "draining_on_ac"),
            "the same rule applies to the AC alert: {alerts:?}"
        );
    }

    /// A genuinely fast discharge, measured entirely within one
    /// unbroken run, does fire. The other half of the gap test: a rule
    /// that never fires is as useless as one that always does.
    #[test]
    fn a_sustained_fast_discharge_fires() {
        let s = series(&[
            (80.0, false),
            (79.0, false),
            (78.0, false),
            (77.0, false),
            (76.0, false),
        ]);
        let alerts = evaluate(&s, 25);
        assert!(
            alerts.iter().any(|a| a.key() == "fast_discharge"),
            "1% a minute is faster than the 1%-per-2-minutes rule: {alerts:?}"
        );
    }

    /// A single bad delta is noise, not a trend. Charge readings wobble
    /// near the extremes, and one 3-point drop followed by a recovery
    /// must not interrupt anyone.
    #[test]
    fn one_bad_delta_does_not_fire() {
        let s = series(&[
            (44.0, false),
            (41.0, false),
            (44.0, false),
            (44.0, false),
            (44.0, false),
        ]);
        let alerts = evaluate(&s, 25);
        assert!(
            !alerts.iter().any(|a| a.key() == "fast_discharge"),
            "a wobble is not a discharge: {alerts:?}"
        );
    }

    /// The valuable one: the plug is in and the charge is still going
    /// down.
    #[test]
    fn draining_while_plugged_in_fires_at_any_speed() {
        // Deliberately SLOW -- well under the fast-discharge rate.
        // Losing charge on mains is worth saying however gently it
        // happens, which is why this alert has no threshold.
        let s = series(&[
            (60.0, true),
            (59.9, true),
            (59.8, true),
            (59.7, true),
            (59.6, true),
        ]);
        let alerts = evaluate(&s, 25);
        assert!(
            alerts.iter().any(|a| a.key() == "draining_on_ac"),
            "a plugged-in machine losing charge must be reported: {alerts:?}"
        );
        assert!(
            !alerts.iter().any(|a| a.key() == "fast_discharge"),
            "and not reported twice: {alerts:?}"
        );
    }

    /// Charging on AC is the ordinary case and says nothing.
    #[test]
    fn charging_on_ac_is_silent() {
        let s = series(&[(60.0, true), (61.0, true), (62.0, true), (63.0, true)]);
        assert!(evaluate(&s, 25).is_empty());
    }

    /// Unplugging halfway through does not count as draining on AC.
    /// Otherwise pulling the cable would alert every single time.
    #[test]
    fn unplugging_is_not_an_adapter_fault() {
        let s = series(&[
            (60.0, true),
            (59.0, true),
            (58.0, false),
            (57.0, false),
            (56.0, false),
        ]);
        let alerts = evaluate(&s, 25);
        assert!(
            !alerts.iter().any(|a| a.key() == "draining_on_ac"),
            "{alerts:?}"
        );
    }

    /// Too little history is silence, not a verdict. A freshly opened
    /// app has no measured interval to draw a trend from.
    #[test]
    fn a_short_series_says_nothing_about_rate() {
        let s = series(&[(80.0, false), (70.0, false)]);
        let alerts = evaluate(&s, 25);
        assert!(
            !alerts.iter().any(|a| a.key() == "fast_discharge"),
            "{alerts:?}"
        );
    }

    /// A machine with no battery is not a flat battery.
    #[test]
    fn no_battery_is_no_alert() {
        let s = vec![bare(&chrono::Utc::now().to_rfc3339())];
        assert!(evaluate(&s, 25).is_empty());
    }

    /// The re-notify guard. A battery sitting at 24% alerts once, not
    /// once a minute forever.
    #[test]
    fn a_standing_condition_is_announced_once() {
        let mut fired = Fired::default();
        let alert = vec![Alert::Low {
            percent: 24.0,
            threshold: 25,
        }];
        assert_eq!(
            fired.take_new(&alert, Some(24.0)).len(),
            1,
            "the first one is news"
        );
        assert!(
            fired.take_new(&alert, Some(24.0)).is_empty(),
            "the second is not"
        );
        assert!(fired.take_new(&alert, Some(24.0)).is_empty());
    }

    /// ...and re-arms once the condition clears, so a battery that is
    /// charged and then drains again is reported the second time too.
    #[test]
    fn a_cleared_condition_re_arms() {
        let mut fired = Fired::default();
        let low = vec![Alert::Low {
            percent: 24.0,
            threshold: 25,
        }];
        assert_eq!(fired.take_new(&low, Some(24.0)).len(), 1);
        // Charged well clear of the threshold band: the condition has
        // genuinely ended, not merely flickered.
        assert!(
            fired.take_new(&[], Some(80.0)).is_empty(),
            "clearing announces nothing"
        );
        assert_eq!(
            fired.take_new(&low, Some(24.0)).len(),
            1,
            "after clearing, the condition is news again"
        );
    }

    /// A battery hovering AT the threshold must not re-notify on every
    /// flicker across it.
    ///
    /// This is the failure `LOW_CLEAR_MARGIN` exists for, and it is the
    /// realistic one: charge readings wobble by a point, so a battery
    /// sitting at 25% produces 24, 25, 24, 25 -- and without the margin
    /// each upward reading would clear the flag so the next downward one
    /// notifies again. The user gets an alert a minute from a battery
    /// that is doing nothing.
    #[test]
    fn a_battery_hovering_at_the_threshold_alerts_once() {
        let mut fired = Fired::default();
        let low = |percent: f64| {
            vec![Alert::Low {
                percent,
                threshold: 25,
            }]
        };

        assert_eq!(fired.take_new(&low(24.0), Some(24.0)).len(), 1);
        // Flickers just above the threshold: no longer "below 25", so
        // `evaluate` reports nothing -- but still well inside the band,
        // so the flag is HELD.
        assert!(fired.take_new(&[], Some(25.0)).is_empty());
        assert!(fired.take_new(&[], Some(26.0)).is_empty());
        // And back under: still the same standing condition, not news.
        assert!(
            fired.take_new(&low(24.0), Some(24.0)).is_empty(),
            "a flicker across the threshold must not re-notify"
        );

        // A real recharge clears the band, and the next genuine
        // discharge is genuinely news again.
        assert!(fired.take_new(&[], Some(90.0)).is_empty());
        assert_eq!(
            fired.take_new(&low(24.0), Some(24.0)).len(),
            1,
            "after a real recharge, low charge is news again"
        );
    }

    /// The other two conditions re-arm immediately, with no margin.
    ///
    /// They are already sustained trends across several intervals, so
    /// they cannot flap on one noisy reading -- and holding them open
    /// would delay a second, genuinely separate episode.
    #[test]
    fn the_trend_alerts_re_arm_without_hysteresis() {
        let mut fired = Fired::default();
        let fast = vec![Alert::FastDischarge {
            percent_per_min: 1.0,
        }];
        assert_eq!(fired.take_new(&fast, Some(50.0)).len(), 1);
        assert!(fired.take_new(&[], Some(50.0)).is_empty());
        assert_eq!(
            fired.take_new(&fast, Some(50.0)).len(),
            1,
            "a second episode is news"
        );
    }

    /// The key is the CONDITION, not the number. A battery falling
    /// through the twenties must not notify at every point on the way
    /// down.
    #[test]
    fn a_falling_percentage_is_still_one_alert() {
        let mut fired = Fired::default();
        // Enumerated rather than matched on the value. What the assertion
        // below is actually about is the FIRST crossing versus every one
        // after it, and `percent == 24.0` said that by comparing an f64
        // against a literal -- which is what clippy::float_cmp now
        // rejects repo-wide (#892). The index says the same thing without
        // the float equality, and says it more directly.
        for (i, percent) in [24.0, 23.0, 22.0, 21.0].into_iter().enumerate() {
            let alert = vec![Alert::Low {
                percent,
                threshold: 25,
            }];
            let new = fired.take_new(&alert, Some(percent));
            if i == 0 {
                assert_eq!(new.len(), 1);
            } else {
                assert!(new.is_empty(), "{percent}% re-notified");
            }
        }
    }

    /// The threshold is clamped the way `stale_venv_days` is: 0 means
    /// unset, not "never alert".
    #[test]
    fn an_unset_threshold_is_the_default() {
        assert_eq!(low_percent(0), DEFAULT_LOW_PERCENT);
        assert_eq!(low_percent(15), 15);
        assert_eq!(low_percent(99), 90, "clamped");
        assert_eq!(low_percent(1), 5, "clamped");
    }

    /// The key-only filter the CPU rules use (#791): the same
    /// announce-once-then-re-arm discipline, with no hysteresis.
    #[test]
    fn a_standing_condition_on_a_borrowed_key_is_announced_once() {
        let mut fired = Fired::default();
        let all = ["diffuse_cpu"];
        assert_eq!(
            fired.take_new_keys(&all, &["diffuse_cpu"]),
            vec!["diffuse_cpu"],
            "the first one is news"
        );
        assert!(
            fired.take_new_keys(&all, &["diffuse_cpu"]).is_empty(),
            "the second is not"
        );
        // Cleared, then true again: a second episode is news.
        assert!(fired.take_new_keys(&all, &[]).is_empty());
        assert_eq!(
            fired.take_new_keys(&all, &["diffuse_cpu"]),
            vec!["diffuse_cpu"]
        );
    }

    /// The two filters share one `Fired` without interfering: a battery
    /// alert must not re-arm a CPU one, or either would be announced
    /// twice.
    #[test]
    fn the_two_filters_do_not_clear_each_others_keys() {
        let mut fired = Fired::default();
        let low = vec![Alert::Low {
            percent: 24.0,
            threshold: 25,
        }];
        assert_eq!(fired.take_new(&low, Some(24.0)).len(), 1);
        assert_eq!(
            fired.take_new_keys(&["diffuse_cpu"], &["diffuse_cpu"]),
            vec!["diffuse_cpu"]
        );
        // Neither call re-announces the other's standing condition.
        assert!(fired.take_new(&low, Some(24.0)).is_empty());
        assert!(fired
            .take_new_keys(&["diffuse_cpu"], &["diffuse_cpu"])
            .is_empty());
    }

    /// This side of the gap rule is thirty minutes, deliberately.
    ///
    /// NOT the cross-language assertion, which this test cannot make and
    /// used to claim it did (#850): comparing `GAP_MS` to a Rust literal
    /// reads the Rust side twice, and passed unchanged while
    /// `ABSOLUTE_GAP_MS` in `src/lib/health.ts` could have held any value
    /// at all. `src/lib/mirroredConstants.test.ts` is where the two are
    /// actually compared -- it reads this file's literal via `?raw`.
    ///
    /// Kept because it is still worth pinning the VALUE here: thirty
    /// minutes is chosen against the sampler's 60s cadence and the
    /// 12-minute coarsest bucketing, and a change to it should be a
    /// deliberate edit in both a test and a constant. What the agreement
    /// protects is the inversion: a series the chart draws as broken must
    /// not be one the alerts compute a rate across, or the user is told a
    /// rate the picture refuses to draw.
    #[test]
    fn a_gap_here_is_a_gap_in_the_charts_too() {
        assert_eq!(
            GAP_MS,
            30 * 60_000,
            "must match ABSOLUTE_GAP_MS in lib/health.ts"
        );
    }

    /// Wording exists for all three, since a notification with an empty
    /// body is a notification that says nothing.
    #[test]
    fn every_alert_can_say_what_it_means() {
        for a in [
            Alert::Low {
                percent: 24.0,
                threshold: 25,
            },
            Alert::FastDischarge {
                percent_per_min: 1.2,
            },
            Alert::DrainingOnAc {
                percent_per_min: 0.2,
            },
        ] {
            assert!(!a.title().is_empty());
            assert!(!a.body().is_empty());
            assert!(!a.key().is_empty());
        }
    }
}
