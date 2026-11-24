//! The zoom control's 0 to 100 positions and the zoom factors (1× to 4×) they give.
//!
//! The camera's zoom is the standard V4L2 control, 0 to 100. Three positions are measured (the
//! presets: 0 is 1.00×, 5 is 1.15×, 15 is 1.47×) and OBSBOT publishes 1–4× digital zoom, so the top
//! of the range is 4×. The stretch between 15 and 100 is a straight line, which is an assumption.

/// (control position, zoom factor) points, joined by straight lines.
pub const CURVE: [(f64, f64); 4] = [(0.0, 1.0), (5.0, 1.15), (15.0, 1.47), (100.0, 4.0)];

/// The largest zoom factor.
pub const MAX_FACTOR: f64 = 4.0;

/// The zoom factor at a control position (0–100).
pub fn factor(position: f64) -> f64 {
    let p = position.clamp(CURVE[0].0, CURVE[CURVE.len() - 1].0);
    for pair in CURVE.windows(2) {
        let ((p0, f0), (p1, f1)) = (pair[0], pair[1]);
        if p <= p1 { return f0 + (f1 - f0) * (p - p0) / (p1 - p0); }
    }
    MAX_FACTOR
}

/// The control position (0–100) that gives a zoom factor, the other way round.
pub fn position(factor: f64) -> f64 {
    let f = factor.clamp(CURVE[0].1, MAX_FACTOR);
    for pair in CURVE.windows(2) {
        let ((p0, f0), (p1, f1)) = (pair[0], pair[1]);
        if f <= f1 { return p0 + (p1 - p0) * (f - f0) / (f1 - f0); }
    }
    CURVE[CURVE.len() - 1].0
}

static LAST_SENT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// The app just sent a zoom: the camera takes a moment to get there.
pub fn note_sent() { if let Ok(mut at) = LAST_SENT.lock() { *at = Some(std::time::Instant::now()); } }

/// Whether the camera may still be moving to a zoom the app sent (in the last 3 seconds).
pub fn settling() -> bool {
    LAST_SENT.lock().ok().and_then(|at| *at).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(3))
}

/// A factor for people: up to two decimals, none that are not needed, and the × sign.
pub fn label(factor: f64) -> String {
    let text = format!("{factor:.2}");
    format!("{}×", text.trim_end_matches('0').trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_measured_points_and_the_top_are_exact() {
        assert_eq!(factor(0.0), 1.0);
        assert!((factor(5.0) - 1.15).abs() < 1e-9 && (factor(15.0) - 1.47).abs() < 1e-9);
        assert_eq!(factor(100.0), 4.0);
        assert_eq!(factor(-5.0), 1.0);
        assert_eq!(factor(500.0), 4.0);
    }

    #[test]
    fn factor_and_position_undo_each_other() {
        for p in (0..=100).step_by(5) {
            let p = f64::from(p);
            assert!((position(factor(p)) - p).abs() < 1e-9, "{p}");
        }
        assert_eq!(position(1.0), 0.0);
        assert_eq!(position(4.0), 100.0);
    }

    #[test]
    fn labels_drop_what_is_not_needed() {
        assert_eq!(label(1.0), "1×");
        assert_eq!(label(1.15), "1.15×");
        assert_eq!(label(2.5), "2.5×");
        assert_eq!(label(4.0), "4×");
    }
}
