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

/// An angle for people to read: a sign, one decimal and no "-0.0".
pub fn format(degrees: impl Into<f64>) -> String {
    let degrees: f64 = degrees.into();
    let rounded = (degrees * 10.0).round() / 10.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    format!("{rounded:+.1}°")
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
        assert_eq!(format(0.0), "+0.0°");
        assert_eq!(format(-0.04), "+0.0°");
        assert_eq!(format(12.34), "+12.3°");
        assert_eq!(format(-5.0), "-5.0°");
    }
}
