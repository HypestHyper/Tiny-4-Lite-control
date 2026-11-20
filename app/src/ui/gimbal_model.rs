//! A wireframe camera on a two-axis gimbal that follows the camera's live pan and tilt.
//!
//! The camera is drawn straight on from the front, a little above, like a product photo: the lens
//! and the front of the base face you. Click it to see it from behind instead. Positive pan turns
//! the camera to ITS right (your left, from the front) and positive tilt looks up; the readouts
//! beside it show the real numbers. It is only a drawing of the angles the camera
//! reports: it never sends anything to the camera.

use super::store::Store;
use super::theme;
use crate::backend::{current_config, update_config};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub use crate::backend::angles::{PAN_LIMIT, PAN_RANGE, TILT_LIMIT};

type V3 = (f64, f64, f64);

// ── Geometry (pure, tested) ───────────────────────────────────────────────

/// The direction the lens faces. x is right, y is up and the camera looks along -z at (0, 0).
pub fn forward(pan: f64, tilt: f64) -> V3 {
    let (p, t) = (pan.to_radians(), tilt.to_radians());
    (p.sin() * t.cos(), t.sin(), -p.cos() * t.cos())
}

/// Tilt about the x axis: looking up swings the lens up.
fn tilt_by(v: V3, degrees: f64) -> V3 {
    let (s, c) = degrees.to_radians().sin_cos();
    (v.0, v.1 * c - v.2 * s, v.1 * s + v.2 * c)
}

/// Pan about the vertical axis: a positive angle turns the lens to the right.
fn pan_by(v: V3, degrees: f64) -> V3 {
    let (s, c) = degrees.to_radians().sin_cos();
    (v.0 * c - v.2 * s, v.1, v.0 * s + v.2 * c)
}

/// The view looks straight on, a little from above. From the front the camera's right is on your left.
const VIEW_PITCH: f64 = 8.0;

/// Turn the world about the vertical so the front (or the back) faces the viewer.
fn view_yaw(behind: bool) -> f64 { if behind { 0.0 } else { 180.0 } }

/// The direction from the scene towards the viewer, in world coordinates.
fn toward_viewer(behind: bool) -> V3 {
    let (s, c) = VIEW_PITCH.to_radians().sin_cos();
    pan_by((0.0, s, c), view_yaw(behind))
}

/// World point to screen offset (right, down) in units of the model, with a little perspective.
pub fn project(p: V3, behind: bool) -> (f64, f64) {
    let (x, y, z) = pan_by(p, -view_yaw(behind));
    // Tilt the whole scene towards the viewer by the view pitch.
    let (s, c) = VIEW_PITCH.to_radians().sin_cos();
    let (y, z) = (y * c - z * s, y * s + z * c);
    let depth = 1.0 / (1.0 - z * 0.06);
    (x * depth, -y * depth)
}

struct Line { a: V3, b: V3, kind: Kind }

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Base, Moving, Detail, Lens, Aim, Limit,
    // Details on one side only. They are left out when that side faces away from the viewer.
    HeadFront, HeadBack, BaseFront, BaseBack, LensFront,
}

/// The camera's size and shape in millimetres, traced from OBSBOT's product photos (front, back,
/// both sides and an angled view), calibrated to the published overall size of 48.37 × 46.46 × 64.2 mm
/// and the 79.4° diagonal field of view. Coordinates: x is the camera's right, y is up from the
/// desk, and the lens looks along -z. The pan axis is the vertical line through the middle of the base.
/// The head and tilt motor sit off to one side: the motor cylinder and the arm are on the camera's
/// LEFT (the right-hand side when you look into the lens). Change a number to refine the model.
mod shape {
    // Base: a rounded block, a little wider at the bottom than the top.
    pub const BASE_HALF_W_BOTTOM: f64 = 24.19;
    pub const BASE_HALF_D_BOTTOM: f64 = 23.23;
    pub const BASE_HALF_W_TOP: f64 = 23.65;
    pub const BASE_HALF_D_TOP: f64 = 22.7;
    pub const BASE_CORNER: f64 = 6.0;
    /// Height of the base, with the top edge rounded over the last few millimetres.
    pub const BASE_HEIGHT: f64 = 25.2;
    pub const BASE_TOP_ROUND: f64 = 3.0;
    /// The clip groove runs round the base this far above the desk.
    pub const CLIP_GROOVE: f64 = 3.6;
    /// Two microphone holes on the front, and the USB-C slot at the back: (x, height above the desk).
    pub const MIC_X: f64 = 11.2;
    pub const MIC_HEIGHT: f64 = 10.0;
    pub const MIC_RADIUS: f64 = 0.6;
    pub const USB_HEIGHT: f64 = 10.6;
    pub const USB_HALF_W: f64 = 4.8;
    pub const USB_HALF_H: f64 = 1.9;
    // Turntable: a flat round disc on top of the base.
    pub const DISC_RADIUS: f64 = 20.2;
    pub const DISC_THICKNESS: f64 = 4.0;
    // Arm: a plate under the motor, its outer face flush with the disc edge.
    pub const ARM_OUTER_X: f64 = -20.2;
    pub const ARM_WIDTH: f64 = 7.9;
    pub const ARM_HALF_D: f64 = 9.3;
    // Tilt motor: a cylinder along x on the camera's left. The head tilts about its axis.
    pub const TILT_AXIS: f64 = 50.2;
    pub const MOTOR_RADIUS: f64 = 12.4;
    pub const MOTOR_OUTER_X: f64 = -22.4;
    pub const MOTOR_INNER_X: f64 = -12.1;
    // Head: a rounded-square shell. It reaches from x = -12.1 to +21.1; the front plate is 3 mm
    // narrower on the motor side.
    pub const HEAD_X0: f64 = -12.1;
    pub const HEAD_X1: f64 = 21.1;
    pub const HEAD_HALF_H: f64 = 14.0;
    pub const HEAD_HALF_D: f64 = 14.4;
    pub const HEAD_CORNER: f64 = 6.5;
    pub const PLATE_X0: f64 = -9.1;
    pub const PLATE_INSET: f64 = 1.2;
    /// Lens: centre x, and (radius, depth behind the front face) for the glass edge, recess, barrel and glass.
    pub const LENS_X: f64 = 6.0;
    pub const LENS_RINGS: [(f64, f64); 4] = [(10.9, 0.0), (6.4, 0.8), (3.3, 2.0), (2.0, 2.6)];
    /// The fine ridges between the recess and the glass edge: radii, and how many.
    pub const RIDGE_INNER: f64 = 6.9;
    pub const RIDGE_OUTER: f64 = 10.2;
    pub const RIDGE_COUNT: usize = 56;
    /// The small status light on the front plate, from the lens centre.
    pub const LED_DX: f64 = 9.2;
    pub const LED_DY: f64 = 8.8;
    pub const LED_RADIUS: f64 = 0.6;
    /// Perforated panel on the back of the head: centre x, rings of dots (half size, dot radius).
    pub const DOT_PANEL_X: f64 = 5.9;
    pub const DOT_RINGS: [(f64, f64); 3] = [(12.0, 0.65), (10.3, 0.55), (8.6, 0.42)];
    pub const DOT_SPACING: f64 = 1.75;
    pub const DOT_CORNER: f64 = 5.2;
    /// Radius of the floor ring that shows the pan range.
    pub const FLOOR_RING: f64 = 38.0;
    /// Horizontal field of view at zoom 0: 67.2° (OBSBOT's published spec for the Tiny 2 Lite, which
    /// gives 79.4° diagonal at 4:3). The 16:9 picture is assumed to keep the full sensor width.
    pub const FOV_HORIZONTAL_DEGREES: f64 = 67.2;
    /// Zoom control position and the zoom factor measured there. The three presets are documented
    /// (0 → 1.00×, 5 → 1.15×, 15 → 1.47×); the top of the range (100) is OBSBOT's published 4× digital
    /// zoom, and the curve between 15 and 100 is a straight line (an assumption).
    pub const ZOOM_CURVE: [(f64, f64); 4] = [(0.0, 1.0), (5.0, 1.15), (15.0, 1.47), (100.0, 4.0)];
    pub const CONE_REACH: f64 = 40.0;
    /// One drawing unit is this many millimetres.
    pub const UNIT: f64 = 32.0;
}

/// Zoom factor at a zoom control position (0–100), straight between the documented points.
pub fn zoom_factor(zoom: f64) -> f64 {
    let curve = shape::ZOOM_CURVE;
    let z = zoom.clamp(curve[0].0, curve[curve.len() - 1].0);
    for pair in curve.windows(2) {
        let ((z0, f0), (z1, f1)) = (pair[0], pair[1]);
        if z <= z1 { return f0 + (f1 - f0) * (z - z0) / (z1 - z0); }
    }
    curve[curve.len() - 1].1
}

/// Horizontal field of view in degrees: the widest angle divided down by the zoom factor.
pub fn fov_horizontal(zoom: f64) -> f64 {
    let wide = (shape::FOV_HORIZONTAL_DEGREES / 2.0).to_radians().tan();
    2.0 * (wide / zoom_factor(zoom)).atan().to_degrees()
}

/// A rounded rectangle as a closed outline: each corner is `steps + 1` points, so the straight
/// edges run between the last point of one corner and the first of the next.
fn rounded_rect(half_w: f64, half_h: f64, radius: f64, steps: usize) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    for (k, (cx, cy)) in [(half_w - radius, half_h - radius), (-(half_w - radius), half_h - radius),
                          (-(half_w - radius), -(half_h - radius)), (half_w - radius, -(half_h - radius))].into_iter().enumerate() {
        let start = k as f64 * std::f64::consts::FRAC_PI_2;
        for i in 0..=steps {
            let a = start + i as f64 / steps as f64 * std::f64::consts::FRAC_PI_2;
            points.push((cx + radius * a.cos(), cy + radius * a.sin()));
        }
    }
    points
}

/// Points spaced evenly along a closed outline.
fn spaced_along(outline: &[(f64, f64)], spacing: f64) -> Vec<(f64, f64)> {
    let n = outline.len();
    let lengths: Vec<f64> = (0..n).map(|i| { let (a, b) = (outline[i], outline[(i + 1) % n]); (b.0 - a.0).hypot(b.1 - a.1) }).collect();
    let total: f64 = lengths.iter().sum();
    let count = (total / spacing).round().max(3.0) as usize;
    let mut points = Vec::with_capacity(count);
    for k in 0..count {
        let mut distance = k as f64 / count as f64 * total;
        for i in 0..n {
            if distance <= lengths[i] || i == n - 1 {
                let t = if lengths[i] > 0.0 { distance / lengths[i] } else { 0.0 };
                let (a, b) = (outline[i], outline[(i + 1) % n]);
                points.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                break;
            }
            distance -= lengths[i];
        }
    }
    points
}

/// Join a list of points into a closed loop.
fn close_loop(points: &[V3], kind: Kind, out: &mut Vec<Line>) {
    for i in 0..points.len() {
        out.push(Line { a: points[i], b: points[(i + 1) % points.len()], kind });
    }
}

fn ring_xy(cx: f64, cy: f64, radius: f64, z: f64, steps: usize) -> Vec<V3> {
    (0..steps).map(|i| { let a = i as f64 / steps as f64 * std::f64::consts::TAU; (cx + radius * a.cos(), cy + radius * a.sin(), z) }).collect()
}

fn ring_xz(radius: f64, y: f64, steps: usize) -> Vec<V3> {
    (0..steps).map(|i| { let a = i as f64 / steps as f64 * std::f64::consts::TAU; (radius * a.cos(), y, radius * a.sin()) }).collect()
}

/// A circle in the y-z plane, for the ends of the motor cylinder.
fn ring_yz(x: f64, cy: f64, radius: f64, steps: usize) -> Vec<V3> {
    (0..steps).map(|i| { let a = i as f64 / steps as f64 * std::f64::consts::TAU; (x, cy + radius * a.sin(), radius * a.cos()) }).collect()
}

/// A rounded slab: two outlines joined at the corner tangent points.
fn slab(outline: &[(f64, f64)], steps: usize, front: impl Fn((f64, f64)) -> V3, back: impl Fn((f64, f64)) -> V3, kind: Kind, out: &mut Vec<Line>) {
    let front_pts: Vec<V3> = outline.iter().map(|p| front(*p)).collect();
    let back_pts: Vec<V3> = outline.iter().map(|p| back(*p)).collect();
    close_loop(&front_pts, kind, out);
    close_loop(&back_pts, kind, out);
    for corner in 0..4 {
        for i in [corner * (steps + 1), corner * (steps + 1) + steps] {
            out.push(Line { a: front_pts[i], b: back_pts[i], kind });
        }
    }
}

/// All the lines of the model for a pan and tilt, in drawing units (see `shape::UNIT`).
fn model(pan: f64, tilt: f64, zoom: f64, behind: bool) -> Vec<Line> {
    use shape::*;
    let mut lines = Vec::new();

    // ── Base (never turns): a rounded block, slightly wider at the bottom, with a rounded top edge ──
    let bottom = rounded_rect(BASE_HALF_W_BOTTOM, BASE_HALF_D_BOTTOM, BASE_CORNER, 4);
    let shoulder_y = BASE_HEIGHT - BASE_TOP_ROUND;
    let upper = rounded_rect(BASE_HALF_W_TOP, BASE_HALF_D_TOP, BASE_CORNER, 4);
    let crown = rounded_rect(BASE_HALF_W_TOP - BASE_TOP_ROUND, BASE_HALF_D_TOP - BASE_TOP_ROUND, BASE_CORNER - 1.5, 4);
    let bottom_pts: Vec<V3> = bottom.iter().map(|&(x, z)| (x, 0.0, z)).collect();
    let upper_pts: Vec<V3> = upper.iter().map(|&(x, z)| (x, shoulder_y, z)).collect();
    let crown_pts: Vec<V3> = crown.iter().map(|&(x, z)| (x, BASE_HEIGHT, z)).collect();
    close_loop(&bottom_pts, Kind::Base, &mut lines);
    close_loop(&upper_pts, Kind::Base, &mut lines);
    close_loop(&crown_pts, Kind::Base, &mut lines);
    for corner in 0..4 {
        for i in [corner * 5, corner * 5 + 4] {
            lines.push(Line { a: bottom_pts[i], b: upper_pts[i], kind: Kind::Base });
            lines.push(Line { a: upper_pts[i], b: crown_pts[i], kind: Kind::Base });
        }
    }
    // The clip groove just above the desk.
    let groove: Vec<V3> = rounded_rect(BASE_HALF_W_BOTTOM - 0.1, BASE_HALF_D_BOTTOM - 0.1, BASE_CORNER, 4).iter().map(|&(x, z)| (x, CLIP_GROOVE, z)).collect();
    close_loop(&groove, Kind::Detail, &mut lines);
    // Two microphone holes on the front, and the USB-C slot on the back.
    for side in [-1.0, 1.0] {
        let hole: Vec<V3> = (0..8).map(|i| { let a = i as f64 / 8.0 * std::f64::consts::TAU; (side * MIC_X + MIC_RADIUS * a.cos(), MIC_HEIGHT + MIC_RADIUS * a.sin(), -BASE_HALF_D_TOP - 0.2) }).collect();
        close_loop(&hole, Kind::BaseFront, &mut lines);
    }
    let usb: Vec<V3> = rounded_rect(USB_HALF_W, USB_HALF_H, USB_HALF_H, 3).iter().map(|&(x, y)| (x, USB_HEIGHT + y, BASE_HALF_D_TOP + 0.2)).collect();
    close_loop(&usb, Kind::BaseBack, &mut lines);
    // The floor ring, with marks for front and the two pan limits.
    close_loop(&ring_xz(FLOOR_RING, 0.0, 44), Kind::Base, &mut lines);
    for (deg, kind, length) in [(0.0, Kind::Base, 9.0), (PAN_RANGE.1, Kind::Limit, 6.0), (PAN_RANGE.0, Kind::Limit, 6.0)] {
        let f = forward(deg, 0.0);
        lines.push(Line { a: (f.0 * FLOOR_RING, 0.0, f.2 * FLOOR_RING), b: (f.0 * (FLOOR_RING + length), 0.0, f.2 * (FLOOR_RING + length)), kind });
    }

    // ── Everything above the base turns with pan: the disc, the arm, the motor and the head's mount ──
    let mut turning: Vec<Line> = Vec::new();
    let disc_bottom = BASE_HEIGHT;
    let disc_top = BASE_HEIGHT + DISC_THICKNESS;
    close_loop(&ring_xz(DISC_RADIUS, disc_bottom, 36), Kind::Moving, &mut turning);
    close_loop(&ring_xz(DISC_RADIUS, disc_top, 36), Kind::Moving, &mut turning);
    close_loop(&ring_xz(DISC_RADIUS - 1.3, disc_top, 36), Kind::Detail, &mut turning);
    for k in 0..8 {
        let a = k as f64 / 8.0 * std::f64::consts::TAU;
        turning.push(Line { a: (DISC_RADIUS * a.cos(), disc_bottom, DISC_RADIUS * a.sin()), b: (DISC_RADIUS * a.cos(), disc_top, DISC_RADIUS * a.sin()), kind: Kind::Moving });
    }
    // The arm: a plate from the disc up to the underside of the motor.
    let arm_inner = ARM_OUTER_X + ARM_WIDTH;
    let arm_top = TILT_AXIS - MOTOR_RADIUS + 1.5;
    let arm_corners = [(ARM_OUTER_X, disc_top), (arm_inner, disc_top), (arm_inner, arm_top), (ARM_OUTER_X, arm_top)];
    let front: Vec<V3> = arm_corners.iter().map(|p| (p.0, p.1, -ARM_HALF_D)).collect();
    let back: Vec<V3> = arm_corners.iter().map(|p| (p.0, p.1, ARM_HALF_D)).collect();
    close_loop(&front, Kind::Moving, &mut turning);
    close_loop(&back, Kind::Moving, &mut turning);
    for i in 0..4 { turning.push(Line { a: front[i], b: back[i], kind: Kind::Moving }); }
    // The motor cylinder: end discs and the lines along it.
    let motor_ends = [MOTOR_OUTER_X, MOTOR_INNER_X];
    for x in motor_ends { close_loop(&ring_yz(x, TILT_AXIS, MOTOR_RADIUS, 32), Kind::Moving, &mut turning); }
    close_loop(&ring_yz(MOTOR_OUTER_X + 1.2, TILT_AXIS, MOTOR_RADIUS - 1.2, 32), Kind::Detail, &mut turning);
    for k in 0..8 {
        let a = k as f64 / 8.0 * std::f64::consts::TAU;
        turning.push(Line { a: (MOTOR_OUTER_X, TILT_AXIS + MOTOR_RADIUS * a.sin(), MOTOR_RADIUS * a.cos()), b: (MOTOR_INNER_X, TILT_AXIS + MOTOR_RADIUS * a.sin(), MOTOR_RADIUS * a.cos()), kind: Kind::Moving });
    }

    // ── The head tilts about the motor axis, then turns with pan. Its coordinates: x as in the
    // world, y and z measured from the tilt axis; the front face is at z = -HEAD_HALF_D ──
    let mut head: Vec<Line> = Vec::new();
    let head_centre_x = (HEAD_X0 + HEAD_X1) / 2.0;
    let shell = rounded_rect((HEAD_X1 - HEAD_X0) / 2.0, HEAD_HALF_H, HEAD_CORNER, 5);
    slab(&shell, 5, |(x, y)| (head_centre_x + x, y, -HEAD_HALF_D), |(x, y)| (head_centre_x + x, y, HEAD_HALF_D), Kind::Moving, &mut head);
    // The front plate: inset from the shell, and 3 mm narrower on the motor side.
    let plate_centre = (PLATE_X0 + HEAD_X1) / 2.0;
    let plate = rounded_rect((HEAD_X1 - PLATE_X0) / 2.0 - PLATE_INSET, HEAD_HALF_H - PLATE_INSET, HEAD_CORNER - 1.0, 5);
    close_loop(&plate.iter().map(|p| (plate_centre + p.0, p.1, -HEAD_HALF_D)).collect::<Vec<_>>(), Kind::HeadFront, &mut head);
    // The lens: glass edge, recess, barrel and glass, with fine ridges between recess and edge.
    // Zooming draws the inner glass further into the barrel, a sketch of the lens working.
    let zoom_t = (zoom_factor(zoom) - 1.0) / (ZOOM_CURVE[ZOOM_CURVE.len() - 1].1 - 1.0);
    for (radius, depth) in LENS_RINGS {
        let depth = depth * (1.0 + 0.9 * zoom_t * (depth / LENS_RINGS[3].1));
        close_loop(&ring_xy(LENS_X, 0.0, radius, -HEAD_HALF_D + depth, 32), Kind::LensFront, &mut head);
    }
    for k in 0..RIDGE_COUNT {
        let a = k as f64 / RIDGE_COUNT as f64 * std::f64::consts::TAU;
        head.push(Line {
            a: (LENS_X + RIDGE_INNER * a.cos(), RIDGE_INNER * a.sin(), -HEAD_HALF_D + 0.5),
            b: (LENS_X + RIDGE_OUTER * a.cos(), RIDGE_OUTER * a.sin(), -HEAD_HALF_D + 0.1),
            kind: Kind::HeadFront,
        });
    }
    // The status light.
    close_loop(&ring_xy(LENS_X + LED_DX, LED_DY, LED_RADIUS, -HEAD_HALF_D, 8), Kind::LensFront, &mut head);
    // The perforated panel on the back: three rings of dots round a plain middle.
    for (half, dot) in DOT_RINGS {
        let ring = rounded_rect(half, half, DOT_CORNER - (12.0 - half) * 0.3, 6);
        for (x, y) in spaced_along(&ring, DOT_SPACING) {
            close_loop(&ring_xy(DOT_PANEL_X + x, y, dot, HEAD_HALF_D, 8), Kind::HeadBack, &mut head);
        }
    }
    // What the camera sees: a ray and a view frustum at the official field of view, 16:9.
    let front_z = -HEAD_HALF_D;
    let half_w = (fov_horizontal(zoom) / 2.0).to_radians().tan() * CONE_REACH;
    let half_h = half_w * 9.0 / 16.0;
    head.push(Line { a: (LENS_X, 0.0, front_z), b: (LENS_X, 0.0, front_z - CONE_REACH * 1.2), kind: Kind::Aim });
    let far = [(-half_w, -half_h), (half_w, -half_h), (half_w, half_h), (-half_w, half_h)];
    for (x, y) in far { head.push(Line { a: (LENS_X, 0.0, front_z), b: (LENS_X + x, y, front_z - CONE_REACH), kind: Kind::Aim }); }
    for i in 0..4 {
        let (a, b) = (far[i], far[(i + 1) % 4]);
        head.push(Line { a: (LENS_X + a.0, a.1, front_z - CONE_REACH), b: (LENS_X + b.0, b.1, front_z - CONE_REACH), kind: Kind::Aim });
    }

    // Place everything: millimetres to drawing units, centred on the middle of the camera's height.
    let to_units = |v: V3| (v.0 / UNIT, (v.1 - 32.0) / UNIT, v.2 / UNIT);
    let pan_place = |v: V3| pan_by(v, pan);
    let mut out: Vec<Line> = lines.into_iter().map(|l| Line { a: to_units(l.a), b: to_units(l.b), kind: l.kind }).collect();
    for line in head {
        let place = |v: V3| {
            // Tilt about the x axis through the motor, then add the axis height and turn with pan.
            let r = tilt_by((v.0, v.1, v.2), tilt);
            pan_place((r.0, r.1 + TILT_AXIS, r.2))
        };
        out.push(Line { a: to_units(place(line.a)), b: to_units(place(line.b)), kind: line.kind });
    }
    for line in turning {
        out.push(Line { a: to_units(pan_place(line.a)), b: to_units(pan_place(line.b)), kind: line.kind });
    }
    // Leave out the details on a side that faces away, so the far side does not show through.
    let viewer = toward_viewer(behind);
    let facing = |normal: V3| normal.0 * viewer.0 + normal.1 * viewer.1 + normal.2 * viewer.2 > 0.02;
    let head_front = forward(pan, tilt);
    let visible = |kind: Kind| match kind {
        Kind::HeadFront | Kind::LensFront => facing(head_front),
        Kind::HeadBack => facing((-head_front.0, -head_front.1, -head_front.2)),
        Kind::BaseFront => facing((0.0, 0.0, -1.0)),
        Kind::BaseBack => facing((0.0, 0.0, 1.0)),
        _ => true,
    };
    out.retain(|line| visible(line.kind));
    out
}

/// Move `current` towards `target` by a time step, without overshoot (it is measured data).
fn approach(current: f64, target: f64, seconds: f64) -> f64 {
    let blend = 1.0 - (-seconds / 0.11).exp();
    current + (target - current) * blend
}

// ── Widget ────────────────────────────────────────────────────────────────

struct State {
    area: gtk::DrawingArea,
    /// Where the camera says it points, or `None` if it has not answered.
    target: Cell<Option<(f64, f64)>>,
    shown: Cell<(f64, f64)>,
    /// Zoom control position (0–100): where the camera says it is, and what is drawn now.
    zoom_target: Cell<f64>,
    zoom_shown: Cell<f64>,
    animating: Cell<bool>,
    behind: Cell<bool>,
    last_frame: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

#[derive(Clone)]
pub struct GimbalModel(Rc<State>);

impl GimbalModel {
    pub fn new(store: &Store) -> GimbalModel {
        let area = gtk::DrawingArea::new();
        area.add_css_class("model");
        area.set_content_height(185);
        area.set_content_width(200);
        area.set_hexpand(true);
        area.set_accessible_role(gtk::AccessibleRole::Img);
        area.update_property(&[gtk::accessible::Property::Label("Gimbal model showing where the camera points")]);
        let state = Rc::new(State {
            area: area.clone(), target: Cell::new(None), shown: Cell::new((0.0, 0.0)), zoom_target: Cell::new(0.0), zoom_shown: Cell::new(0.0),
            animating: Cell::new(false), behind: Cell::new(current_config().model_view_behind), last_frame: Cell::new(0), tick: RefCell::default(),
        });
        let model = GimbalModel(state);
        let drawer = model.clone();
        area.set_draw_func(move |_, cr, width, height| drawer.draw(cr, width, height));
        // Click to look from the other side.
        area.set_cursor_from_name(Some("pointer"));
        area.set_tooltip_text(Some("Click to flip between the front view and the view from behind"));
        let click = gtk::GestureClick::new();
        let flipper = Rc::downgrade(&model.0);
        click.connect_released(move |_, _, _, _| {
            let Some(state) = flipper.upgrade() else { return };
            let behind = !state.behind.get();
            state.behind.set(behind);
            update_config(|c| c.model_view_behind = behind);
            state.area.queue_draw();
        });
        area.add_controller(click);
        let weak = Rc::downgrade(&model.0);
        store.subscribe_gimbal(move |position| match weak.upgrade() {
            Some(state) => { GimbalModel(state).set_target(position); true }
            None => false,
        });
        model.set_target(store.gimbal());
        // The zoom comes from the snapshot the rest of the app already keeps.
        let zoom_of = |snapshot: &crate::backend::registry::Snapshot| snapshot.control("zoom_absolute").map(|c| c.value as f64);
        if let Some(z) = zoom_of(&store.snapshot()) { model.0.zoom_target.set(z); model.0.zoom_shown.set(z); }
        let weak = Rc::downgrade(&model.0);
        store.subscribe(move |snapshot| match weak.upgrade() {
            Some(state) => {
                if let Some(z) = zoom_of(snapshot) { GimbalModel(state).set_zoom(z); }
                true
            }
            None => false,
        });
        model
    }

    pub fn widget(&self) -> &gtk::DrawingArea { &self.0.area }

    fn set_target(&self, position: Option<(f64, f64)>) {
        let state = &self.0;
        let first = state.target.get().is_none();
        // Whatever the camera sends, keep the drawing inside the range it can really reach.
        let position = position.map(|(p, t)| (p.clamp(-PAN_LIMIT, PAN_LIMIT), t.clamp(-TILT_LIMIT, TILT_LIMIT)));
        state.target.set(position);
        match position {
            // The first reading puts the model there at once; later ones glide.
            Some(p) if first || current_config().reduce_motion => { state.shown.set(p); state.area.queue_draw(); }
            Some(_) => self.animate(),
            None => state.area.queue_draw(),
        }
    }

    fn set_zoom(&self, zoom: f64) {
        let state = &self.0;
        if (state.zoom_target.get() - zoom).abs() < f64::EPSILON { return; }
        state.zoom_target.set(zoom);
        if current_config().reduce_motion { state.zoom_shown.set(zoom); state.area.queue_draw(); } else { self.animate(); }
    }

    fn animate(&self) {
        let state = &self.0;
        if state.animating.replace(true) { return; }
        state.last_frame.set(0);
        let weak = Rc::downgrade(state);
        let id = state.area.add_tick_callback(move |_, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let (tp, tt) = state.target.get().unwrap_or_else(|| state.shown.get());
            let now = clock.frame_time();
            let dt = if state.last_frame.get() == 0 { 1.0 / 60.0 } else { ((now - state.last_frame.get()) as f64 / 1_000_000.0).clamp(0.001, 0.1) };
            state.last_frame.set(now);
            let (cp, ct) = state.shown.get();
            let (np, nt) = (approach(cp, tp, dt), approach(ct, tt, dt));
            let (zt, zc) = (state.zoom_target.get(), state.zoom_shown.get());
            let nz = approach(zc, zt, dt);
            let done = (np - tp).abs() < 0.05 && (nt - tt).abs() < 0.05 && (nz - zt).abs() < 0.3;
            state.shown.set(if done { (tp, tt) } else { (np, nt) });
            state.zoom_shown.set(if done { zt } else { nz });
            state.area.queue_draw();
            if done { state.animating.set(false); glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
        *state.tick.borrow_mut() = Some(id);
    }

    fn draw(&self, cr: &gtk::cairo::Context, width: i32, height: i32) {
        let state = &self.0;
        let known = state.target.get().is_some();
        let (pan, tilt) = state.shown.get();
        let scale = (f64::from(height) / 3.15).min(f64::from(width) / 3.9);
        let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) * 0.58);
        theme::with(|p| {
            let colour = |kind: Kind| match kind {
                Kind::Base => (p.rgb("outline"), 1.4, 0.9),
                Kind::Moving => (p.rgb("on_surface_variant"), 1.5, 0.95),
                Kind::Detail | Kind::HeadFront | Kind::HeadBack | Kind::BaseFront | Kind::BaseBack => (p.rgb("outline"), 1.0, 0.75),
                Kind::Lens | Kind::LensFront => (p.rgb("primary"), 1.5, 1.0),
                Kind::Aim => (p.rgb("primary"), 1.3, 0.55),
                Kind::Limit => (p.rgb("tertiary"), 2.0, 0.9),
            };
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            // Without a reading the model sits dim at centre instead of pretending.
            let fade = if known { 1.0 } else { 0.35 };
            let behind = state.behind.get();
            let lines = model(pan, tilt, state.zoom_shown.get(), behind);
            // One stroke per style: the model has over a thousand short lines.
            let styles: [&[Kind]; 6] = [
                &[Kind::Base], &[Kind::Detail, Kind::HeadFront, Kind::HeadBack, Kind::BaseFront, Kind::BaseBack],
                &[Kind::Moving], &[Kind::Lens, Kind::LensFront], &[Kind::Limit], &[Kind::Aim],
            ];
            for kinds in styles {
                if !known && kinds[0] == Kind::Aim { continue; }
                let ((r, g, b), width, alpha) = colour(kinds[0]);
                cr.set_source_rgba(r, g, b, alpha * fade);
                cr.set_line_width(width);
                for line in lines.iter().filter(|l| kinds.contains(&l.kind)) {
                    let (ax, ay) = project(line.a, behind);
                    let (bx, by) = project(line.b, behind);
                    cr.move_to(cx + ax * scale, cy + ay * scale);
                    cr.line_to(cx + bx * scale, cy + by * scale);
                }
                let _ = cr.stroke();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool { (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9 }

    #[test]
    fn at_zero_the_camera_looks_straight_ahead() {
        assert!(close(forward(0.0, 0.0), (0.0, 0.0, -1.0)));
    }

    #[test]
    fn positive_pan_turns_right_and_positive_tilt_looks_up() {
        assert!(close(forward(90.0, 0.0), (1.0, 0.0, 0.0)));
        assert!(close(forward(-90.0, 0.0), (-1.0, 0.0, 0.0)));
        assert!(close(forward(0.0, 90.0), (0.0, 1.0, 0.0)));
        assert!(forward(0.0, -30.0).1 < 0.0);
    }

    #[test]
    fn the_drawn_lens_direction_matches_the_pan_and_tilt_maths() {
        // The model's aim ray starts at the lens and ends along `forward`.
        for (pan, tilt) in [(0.0, 0.0), (40.0, 15.0), (-75.0, -20.0), (130.0, 90.0)] {
            let lines = model(pan, tilt, 0.0, false);
            let ray = lines.iter().filter(|l| l.kind == Kind::Aim).next().expect("an aim line");
            let d = (ray.b.0 - ray.a.0, ray.b.1 - ray.a.1, ray.b.2 - ray.a.2);
            let length = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
            let f = forward(pan, tilt);
            assert!(close((d.0 / length, d.1 / length, d.2 / length), f), "pan {pan} tilt {tilt}: {d:?} vs {f:?}");
        }
    }

    #[test]
    fn the_marks_are_the_driven_range_and_the_clamp_is_the_mechanical_one() {
        assert_eq!(PAN_RANGE, (-140.0, 140.0));
        assert_eq!(PAN_LIMIT, 150.0);
        assert_eq!(TILT_LIMIT, 90.0);
    }

    #[test]
    fn motion_glides_to_the_target_without_overshoot() {
        let mut value = 0.0;
        for _ in 0..200 {
            let next = approach(value, 30.0, 1.0 / 60.0);
            assert!(next >= value && next <= 30.0, "{next}");
            value = next;
        }
        assert!((value - 30.0).abs() < 0.1);
        // A long frame never jumps past the target either.
        assert!(approach(0.0, 30.0, 10.0) <= 30.0);
    }

    #[test]
    fn projection_keeps_the_scene_inside_a_sensible_box() {
        for line in model(0.0, 0.0, 0.0, false) {
            for point in [line.a, line.b] {
                let (x, y) = project(point, false);
                assert!(x.abs() < 3.0 && y.abs() < 3.0, "{point:?} -> {x}, {y}");
            }
        }
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    fn extent(lines: &[Line], axis: fn(V3) -> f64) -> (f64, f64) {
        let values = lines.iter().flat_map(|l| [axis(l.a), axis(l.b)]);
        values.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }

    /// At rest (pan 0, tilt 0) the body, without the floor ring and the view cone, should be the
    /// size OBSBOT publishes: 48.37 wide, 46.46 deep and 64.2 high, in millimetres.
    #[test]
    fn the_camera_body_has_the_published_size() {
        let body: Vec<Line> = model(0.0, 0.0, 0.0, false).into_iter().filter(|l| !matches!(l.kind, Kind::Aim | Kind::Limit | Kind::Base)
            || (l.kind == Kind::Base && l.a.0.hypot(l.a.2) * shape::UNIT < shape::FLOOR_RING - 1.0)).collect();
        let mm = |units: f64| units * shape::UNIT;
        let (x0, x1) = extent(&body, |v| v.0);
        let (y0, y1) = extent(&body, |v| v.1);
        let (z0, z1) = extent(&body, |v| v.2);
        assert!((mm(x1 - x0) - 48.4).abs() < 1.0, "width {}", mm(x1 - x0));
        assert!((mm(y1 - y0) - 64.2).abs() < 1.5, "height {}", mm(y1 - y0));
        // Depth: the base footprint is 46.5; the head does not stick out behind or in front of it at rest.
        assert!((mm(z1 - z0) - 46.4).abs() < 1.5, "depth {}", mm(z1 - z0));
    }

    /// The published 67.2° horizontal at zoom 0, narrowing with the documented zoom factors, 4× at the top.
    #[test]
    fn zoom_narrows_the_published_angle() {
        assert!((zoom_factor(5.0) - 1.15).abs() < 1e-9 && (zoom_factor(15.0) - 1.47).abs() < 1e-9);
        assert!((zoom_factor(100.0) - 4.0).abs() < 1e-9);
        assert!((fov_horizontal(0.0) - 67.2).abs() < 0.01);
        assert!((fov_horizontal(5.0) - 60.0).abs() < 0.3, "{}", fov_horizontal(5.0));
        assert!((fov_horizontal(15.0) - 48.4).abs() < 0.3, "{}", fov_horizontal(15.0));
        assert!((fov_horizontal(100.0) - 18.9).abs() < 0.3, "{}", fov_horizontal(100.0));
    }

    /// OBSBOT gives 79.4° diagonal and 67.2° horizontal at 4:3; they agree with each other.
    #[test]
    fn the_published_horizontal_and_diagonal_angles_agree() {
        let half_width = (67.2f64 / 2.0).to_radians().tan();
        let half_diagonal = half_width * 5.0 / 4.0;
        assert!((2.0 * half_diagonal.atan().to_degrees() - 79.4).abs() < 0.1);
    }

    #[test]
    fn zooming_narrows_the_cone() {
        let reach = |zoom: f64| model(0.0, 0.0, zoom, false).iter().filter(|l| l.kind == Kind::Aim).map(|l| l.b.0.abs()).fold(0.0, f64::max);
        assert!(reach(15.0) < reach(0.0) && reach(100.0) < reach(15.0));
    }

    #[test]
    fn the_view_cone_uses_the_published_horizontal_field_of_view() {
        let lines = model(0.0, 0.0, 0.0, false);
        let aims: Vec<&Line> = lines.iter().filter(|l| l.kind == Kind::Aim).collect();
        // Ray, four edges from the lens, four edges of the far rectangle.
        assert_eq!(aims.len(), 9);
        // The far rectangle is 16:9, and its half-width seen from the lens is half the published 67.2°.
        let (lens, far) = (&aims[1].a, &aims[1].b);
        let (dx, dy, dz) = (far.0 - lens.0, far.1 - lens.1, far.2 - lens.2);
        let angle = dx.abs().atan2(dz.abs()).to_degrees();
        assert!((angle - 67.2 / 2.0).abs() < 0.2, "half width {angle}");
        assert!((dy.abs() / dx.abs() - 9.0 / 16.0).abs() < 1e-6);
    }
}
