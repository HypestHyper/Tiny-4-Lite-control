//! One measurement for pan and tilt across the whole app: degrees.
//!
//! The camera speaks three dialects, and this file is the only place that knows about them:
//!
//! * the V4L2 controls `pan_absolute` and `tilt_absolute` count arcseconds (3600 to a degree) in
//!   steps of one whole degree, with pan in ±130° and tilt in ±90°;
//! * the live position reply (query 0x0043) gives pitch and yaw in hundredths of a degree, and the
//!   app's tilt is the negated pitch;
//! * the camera's own saved positions carry pan and pitch as floats, again with tilt as −pitch.
//!
//! Checked on a real camera: a +1° pan step and a +1° tilt step each raised the V4L2 value by 3600
//! and the live reading by 1° in the same direction, so pan and tilt agree in sign everywhere.
//! Positive pan turns the camera to its right and positive tilt looks up.

/// V4L2 raw units (arcseconds) in one degree.
pub const RAW_PER_DEGREE: f64 = 3600.0;
/// How far the gimbal can physically turn each way (OBSBOT's mechanical range), in degrees.
pub const PAN_LIMIT: f64 = 150.0;
pub const TILT_LIMIT: f64 = 90.0;

/// How far the camera will be driven (OBSBOT's published controllable range): pan ±140°, and tilt from
/// 30° down to 70° up. In this app tilt is positive looking up, so that is −30 to +70.
pub const PAN_RANGE: (f64, f64) = (-140.0, 140.0);
pub const TILT_RANGE: (f64, f64) = (-30.0, 70.0);

/// Keep a pan angle inside the range the camera is driven over.
pub fn clamp_pan(degrees: f64) -> f64 { degrees.clamp(PAN_RANGE.0, PAN_RANGE.1) }
/// Keep a tilt angle inside the range the camera is driven over.
pub fn clamp_tilt(degrees: f64) -> f64 { degrees.clamp(TILT_RANGE.0, TILT_RANGE.1) }

/// Within this many degrees of an end, driving further is refused.
pub const END_MARGIN: f64 = 2.0;
/// Within this many degrees of an end, the speed tapers off so it arrives gently.
pub const SLOW_ZONE: f64 = 25.0;
/// The slowest the taper goes before the margin, as a fraction of the speed asked for.
pub const CREEP: f64 = 0.15;

/// How much of the asked speed to allow, 0 to 1, for turning in `direction` (+1 or −1, 0 is still)
/// from `at` inside `range`: all of it far from the end, a tapering share near it, and none within
/// the margin. Turning away from an end is always allowed. A position outside the range may only
/// be driven back inside.
pub fn speed_allowance(at: f64, direction: f64, range: (f64, f64)) -> f64 {
    if direction == 0.0 { return 1.0; }
    let room = if direction > 0.0 { range.1 - at } else { at - range.0 };
    if room <= END_MARGIN { return 0.0; }
    if room >= SLOW_ZONE { return 1.0; }
    CREEP + (1.0 - CREEP) * (room - END_MARGIN) / (SLOW_ZONE - END_MARGIN)
}

/// A V4L2 pan or tilt value as degrees.
pub fn raw_to_degrees(raw: i64) -> f64 { raw as f64 / RAW_PER_DEGREE }

/// Degrees as a V4L2 value. The controls move in whole degrees, so this rounds to one.
pub fn degrees_to_raw(degrees: f64) -> i64 { (degrees.round() as i64) * RAW_PER_DEGREE as i64 }

/// The live position reply: pitch and yaw in hundredths of a degree, as (pan, tilt).
pub fn from_live(pitch_hundredths: i16, yaw_hundredths: i16) -> (f64, f64) {
    (f64::from(yaw_hundredths) / 100.0, -f64::from(pitch_hundredths) / 100.0)
}

/// An angle for people to read: whole degrees with a sign, and no "-0".
pub fn format(degrees: impl Into<f64>) -> String {
    let rounded = degrees.into().round();
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    format!("{rounded:+.0}°")
}

/// The live position wobbles a little, and the camera sometimes reports a value between two
/// degrees. A shown angle only changes once the reading is this far from it.
pub const SETTLE_DEGREES: f64 = 0.8;

/// The whole degree to show for a reading, given the one shown now: it keeps the shown degree until
/// the reading is `SETTLE_DEGREES` or more away, then jumps to the nearest whole degree.
pub fn settle(shown: Option<f64>, reading: f64) -> f64 {
    match shown {
        Some(current) if (reading - current).abs() + 1e-9 < SETTLE_DEGREES => current,
        _ => { let nearest = reading.round(); if nearest == 0.0 { 0.0 } else { nearest } }
    }
}

/// Turns a stream of live readings into a steady whole-degree angle.
///
/// The camera's reading wobbles by a fraction of a degree, sometimes lands between two degrees, and
/// now and then gives one odd value. So: take the middle one of the last five readings (a single odd
/// reading cannot move it), follow the newest reading when it has clearly gone elsewhere twice in a
/// row (a real move, so it does not lag), and only change the shown degree once the result is
/// `SETTLE_DEGREES` or more away from it.
#[derive(Clone, Debug, Default)]
pub struct Steady {
    recent: std::collections::VecDeque<f64>,
    shown: Option<f64>,
}

/// How many readings the middle is taken from.
const WINDOW: usize = 5;
/// A reading this far from the middle is a move, not a wobble, if the one before agrees.
const MOVE_DEGREES: f64 = 3.0;

impl Steady {
    pub fn new() -> Steady { Steady::default() }

    /// Feed one reading; get the degree to show.
    pub fn push(&mut self, reading: f64) -> f64 {
        if self.recent.len() == WINDOW { self.recent.pop_front(); }
        self.recent.push_back(reading);
        let mut sorted: Vec<f64> = self.recent.iter().copied().collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let middle = sorted[sorted.len() / 2];
        let n = self.recent.len();
        let moving = n >= 3 && {
            let (last, before) = (self.recent[n - 1] - middle, self.recent[n - 2] - middle);
            last.abs() > MOVE_DEGREES && before.abs() > MOVE_DEGREES && last.signum() == before.signum()
        };
        let value = if moving { reading } else { middle };
        let shown = settle(self.shown, value);
        self.shown = Some(shown);
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_values_and_degrees_agree() {
        assert_eq!(raw_to_degrees(3600), 1.0);
        assert_eq!(raw_to_degrees(-18_000), -5.0);
        assert_eq!(degrees_to_raw(12.4), 43_200);
        assert_eq!(degrees_to_raw(-0.4), 0);
        // The documented V4L2 ranges are exactly the limits.
        // The V4L2 pan range is narrower than the mechanical one; the driven range is narrower still.
        assert_eq!(raw_to_degrees(468_000), 130.0);
        assert_eq!(raw_to_degrees(324_000), TILT_LIMIT);
        assert!(PAN_RANGE.1 <= PAN_LIMIT && TILT_RANGE.1 <= TILT_LIMIT && -TILT_RANGE.0 <= TILT_LIMIT);
    }

    #[test]
    fn the_driven_range_is_the_published_one() {
        assert_eq!((PAN_RANGE.0, PAN_RANGE.1), (-140.0, 140.0));
        // Down 30 and up 70, with tilt positive looking up.
        assert_eq!((TILT_RANGE.0, TILT_RANGE.1), (-30.0, 70.0));
        assert_eq!(clamp_pan(200.0), 140.0);
        assert_eq!(clamp_tilt(-50.0), -30.0);
    }

    #[test]
    fn speed_tapers_towards_an_end_and_stops_before_it() {
        let r = PAN_RANGE;
        // Far from either end: full speed both ways.
        assert_eq!(speed_allowance(0.0, 1.0, r), 1.0);
        assert_eq!(speed_allowance(0.0, -1.0, r), 1.0);
        // Approaching +140 tapers, and never increases as it gets closer.
        let mut last = 1.0;
        for at in (100..=138).map(f64::from) {
            let now = speed_allowance(at, 1.0, r);
            assert!(now <= last + 1e-12, "{at}: {now} after {last}");
            last = now;
        }
        assert_eq!(speed_allowance(138.0, 1.0, r), 0.0);
        assert!(speed_allowance(137.0, 1.0, r) >= CREEP);
        // Turning away from the end is free, even right at it, or beyond it.
        assert_eq!(speed_allowance(140.0, -1.0, r), 1.0);
        assert_eq!(speed_allowance(145.0, -1.0, r), 1.0);
        assert_eq!(speed_allowance(145.0, 1.0, r), 0.0);
        // Tilt has different room each way.
        assert_eq!(speed_allowance(-28.0, -1.0, TILT_RANGE), 0.0);
        assert_eq!(speed_allowance(60.0, 1.0, TILT_RANGE) < 1.0, true);
        assert_eq!(speed_allowance(0.0, 0.0, r), 1.0);
    }

    #[test]
    fn the_live_reply_gives_pan_and_tilt_in_the_same_signs_as_v4l2() {
        // Looking up is a negative pitch in the reply and a positive tilt everywhere else.
        assert_eq!(from_live(-1000, 2500), (25.0, 10.0));
    }

    #[test]
    fn one_way_to_write_an_angle() {
        assert_eq!(format(0.0), "+0°");
        assert_eq!(format(-0.4), "+0°");
        assert_eq!(format(12.34), "+12°");
        assert_eq!(format(12.6), "+13°");
        assert_eq!(format(-5.0), "-5°");
    }

    #[test]
    fn a_wobbling_reading_does_not_change_the_shown_degree() {
        // Settling at 5° and wobbling between 4.3 and 5.7 never moves it.
        let mut shown = Some(5.0);
        for reading in [5.0, 5.5, 4.6, 5.7, 4.3, 5.2, 4.9] {
            shown = Some(settle(shown, reading));
            assert_eq!(shown, Some(5.0), "reading {reading}");
        }
        // A real move of 0.8° or more does change it, to the nearest degree.
        assert_eq!(settle(Some(5.0), 5.8), 6.0);
        assert_eq!(settle(Some(5.0), 4.2), 4.0);
        assert_eq!(settle(Some(5.0), 11.4), 11.0);
        // The first reading is just rounded, and zero is never negative.
        assert_eq!(settle(None, 7.6), 8.0);
        assert!(settle(None, -0.3).is_sign_positive());
    }

    fn run(readings: &[f64]) -> Vec<f64> {
        let mut steady = Steady::new();
        readings.iter().map(|r| steady.push(*r)).collect()
    }

    #[test]
    fn one_odd_reading_never_shows() {
        let shown = run(&[10.0, 10.2, 9.9, 10.1, 24.0, 10.0, 10.1, 9.8, 10.0]);
        assert!(shown.iter().all(|s| *s == 10.0), "{shown:?}");
        // Not even a spike on the very first reading after a settled run.
        let shown = run(&[-40.0, -40.1, -39.9, -40.0, -40.2, 90.0]);
        assert_eq!(*shown.last().unwrap(), -40.0);
    }

    #[test]
    fn a_wobble_between_degrees_stays_on_one_degree() {
        let shown = run(&[5.4, 5.6, 5.5, 5.45, 5.55, 5.5, 5.6, 5.4]);
        let first = shown[0];
        assert!(shown.iter().all(|s| *s == first), "{shown:?}");
    }

    #[test]
    fn a_real_move_is_followed_without_waiting() {
        // Still at 0, then turning quickly to 30: it follows within two readings of the move.
        let shown = run(&[0.0, 0.1, 0.0, 0.0, 8.0, 17.0, 26.0, 30.0, 30.1, 30.0]);
        assert_eq!(shown[3], 0.0);
        assert!(shown[5] >= 16.0, "{shown:?}");
        assert_eq!(shown[7], 30.0);
        assert_eq!(*shown.last().unwrap(), 30.0);
    }

    #[test]
    fn a_small_real_move_shows_once_it_is_large_enough() {
        // A 1° bump shows, then back again shows.
        let shown = run(&[5.0, 5.0, 5.0, 5.0, 6.0, 6.0, 6.0, 6.0, 5.0, 5.0, 5.0, 5.0]);
        assert_eq!(shown[3], 5.0);
        assert_eq!(shown[7], 6.0);
        assert_eq!(*shown.last().unwrap(), 5.0);
    }

    #[test]
    fn it_is_stable_when_the_gimbal_is_bumped_back_and_forth() {
        // Bumped 1° one way and back: shown goes 5 -> 6 -> 5, and nothing flickers in between.
        let mut shown = Some(5.0);
        let mut seen = Vec::new();
        for reading in [5.0, 5.4, 5.9, 6.0, 6.3, 5.7, 5.2, 4.9] {
            shown = Some(settle(shown, reading));
            seen.push(shown.unwrap());
        }
        assert_eq!(seen, vec![5.0, 5.0, 6.0, 6.0, 6.0, 6.0, 5.0, 5.0]);
    }
}
