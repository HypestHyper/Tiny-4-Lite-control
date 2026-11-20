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
/// How far the camera turns each way, in degrees.
pub const PAN_LIMIT: f64 = 130.0;
pub const TILT_LIMIT: f64 = 90.0;

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
        assert_eq!(raw_to_degrees(468_000), PAN_LIMIT);
        assert_eq!(raw_to_degrees(324_000), TILT_LIMIT);
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
