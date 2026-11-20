//! A wireframe camera on a two-axis gimbal that follows the camera's live pan and tilt.
//!
//! The picture is viewed from behind and a little above, so you see where the lens points.
//! Positive pan turns right and positive tilt looks up, the same signs as the joystick; the
//! readouts beside it show the real numbers. It is only a drawing of the angles the camera
//! reports: it never sends anything to the camera.

use super::store::Store;
use super::theme;
use crate::backend::current_config;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAN_LIMIT: f64 = 130.0;
pub const TILT_LIMIT: f64 = 90.0;

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

/// The fixed view: from behind and to the left, looking slightly down.
const VIEW_YAW: f64 = 32.0;
const VIEW_PITCH: f64 = 20.0;

/// World point to screen offset (right, down) in units of the model, with a little perspective.
pub fn project(p: V3) -> (f64, f64) {
    let (x, y, z) = pan_by(p, -VIEW_YAW);
    // Tilt the whole scene towards the viewer by the view pitch.
    let (s, c) = VIEW_PITCH.to_radians().sin_cos();
    let (y, z) = (y * c - z * s, y * s + z * c);
    let depth = 1.0 / (1.0 - z * 0.06);
    (x * depth, -y * depth)
}

struct Line { a: V3, b: V3, kind: Kind }

#[derive(Clone, Copy, PartialEq)]
enum Kind { Base, Moving, Lens, Aim, Limit }

/// The camera's size and shape, in millimetres. Only the overall size is published by OBSBOT
/// (48.37 × 46.46 × 64.2 mm, a 2-axis gimbal with a built-in stand, 79.4° diagonal field of view);
/// the rest is a close approximation drawn from that. Change a number here to refine the model.
mod shape {
    /// Overall footprint of the base, and the total height at rest.
    pub const BASE_HALF_W: f64 = 24.2;
    pub const BASE_HALF_D: f64 = 23.2;
    pub const BASE_HEIGHT: f64 = 14.0;
    pub const BASE_CORNER: f64 = 7.0;
    /// The turning hub between the base and the yoke.
    pub const HUB_RADIUS: f64 = 11.0;
    pub const HUB_TOP: f64 = 22.0;
    /// The U-shaped yoke: its outer half-width, arm thickness, depth and height of the arm tops.
    pub const YOKE_HALF_W: f64 = 24.2;
    pub const YOKE_ARM: f64 = 4.5;
    pub const YOKE_HALF_D: f64 = 8.0;
    pub const YOKE_FLOOR: f64 = 4.0;
    /// Height of the tilt axis, and the arm tops above it.
    pub const TILT_AXIS: f64 = 44.2;
    pub const ARM_TOP: f64 = 52.0;
    /// The head: half width, half height, half depth and corner radius.
    pub const HEAD_HALF_W: f64 = 18.5;
    pub const HEAD_HALF_H: f64 = 20.0;
    pub const HEAD_HALF_D: f64 = 17.0;
    pub const HEAD_CORNER: f64 = 9.0;
    /// Lens glass and the rings inside it: (radius, how far behind the front face).
    pub const LENS_RINGS: [(f64, f64); 4] = [(14.5, 0.0), (11.0, 1.2), (7.0, 2.6), (3.2, 3.6)];
    /// Radius of the floor ring that shows the pan range.
    pub const FLOOR_RING: f64 = 36.0;
    /// Field of view drawn as the cone: 79.4° diagonal (official spec) at 16:9.
    pub const FOV_DIAGONAL_DEGREES: f64 = 79.4;
    pub const CONE_REACH: f64 = 40.0;
    /// One drawing unit is this many millimetres.
    pub const UNIT: f64 = 32.0;
}

/// A rounded rectangle as a closed outline: each corner is `steps + 1` points, so the straight
/// edges run between the last point of one corner and the first of the next.
fn rounded_rect(half_w: f64, half_h: f64, radius: f64, steps: usize) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    // Corner centres, going round from top-right.
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

/// Join a list of points into a closed loop.
fn close_loop(points: &[V3], kind: Kind, out: &mut Vec<Line>) {
    for i in 0..points.len() {
        out.push(Line { a: points[i], b: points[(i + 1) % points.len()], kind });
    }
}

fn ring_xy(radius: f64, z: f64, steps: usize) -> Vec<V3> {
    (0..steps).map(|i| { let a = i as f64 / steps as f64 * std::f64::consts::TAU; (radius * a.cos(), radius * a.sin(), z) }).collect()
}

fn ring_xz(radius: f64, y: f64, steps: usize) -> Vec<V3> {
    (0..steps).map(|i| { let a = i as f64 / steps as f64 * std::f64::consts::TAU; (radius * a.cos(), y, radius * a.sin()) }).collect()
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
fn model(pan: f64, tilt: f64) -> Vec<Line> {
    use shape::*;
    let mut lines = Vec::new();

    // The base stays put: a rounded block, the floor ring, and marks for front and the pan limits.
    let base = rounded_rect(BASE_HALF_W, BASE_HALF_D, BASE_CORNER, 4);
    slab(&base, 4, |(x, z)| (x, 0.0, z), |(x, z)| (x, BASE_HEIGHT, z), Kind::Base, &mut lines);
    close_loop(&ring_xz(FLOOR_RING, 0.0, 40), Kind::Base, &mut lines);
    for (deg, kind, length) in [(0.0, Kind::Base, 9.0), (PAN_LIMIT, Kind::Limit, 6.0), (-PAN_LIMIT, Kind::Limit, 6.0)] {
        let f = forward(deg, 0.0);
        lines.push(Line { a: (f.0 * FLOOR_RING, 0.0, f.2 * FLOOR_RING), b: (f.0 * (FLOOR_RING + length), 0.0, f.2 * (FLOOR_RING + length)), kind });
    }

    // Everything above the base turns with pan.
    let mut turning: Vec<Line> = Vec::new();
    close_loop(&ring_xz(HUB_RADIUS, BASE_HEIGHT, 20), Kind::Moving, &mut turning);
    close_loop(&ring_xz(HUB_RADIUS, HUB_TOP, 20), Kind::Moving, &mut turning);
    for k in 0..4 {
        let a = k as f64 * std::f64::consts::FRAC_PI_2;
        turning.push(Line { a: (HUB_RADIUS * a.cos(), BASE_HEIGHT, HUB_RADIUS * a.sin()), b: (HUB_RADIUS * a.cos(), HUB_TOP, HUB_RADIUS * a.sin()), kind: Kind::Moving });
    }
    // The yoke is a U in the x-y plane, extruded front to back.
    let inner = YOKE_HALF_W - YOKE_ARM;
    let u: [(f64, f64); 8] = [
        (-YOKE_HALF_W, HUB_TOP), (YOKE_HALF_W, HUB_TOP), (YOKE_HALF_W, ARM_TOP), (inner, ARM_TOP),
        (inner, HUB_TOP + YOKE_FLOOR), (-inner, HUB_TOP + YOKE_FLOOR), (-inner, ARM_TOP), (-YOKE_HALF_W, ARM_TOP),
    ];
    let front: Vec<V3> = u.iter().map(|p| (p.0, p.1, -YOKE_HALF_D)).collect();
    let back: Vec<V3> = u.iter().map(|p| (p.0, p.1, YOKE_HALF_D)).collect();
    close_loop(&front, Kind::Moving, &mut turning);
    close_loop(&back, Kind::Moving, &mut turning);
    for i in 0..u.len() { turning.push(Line { a: front[i], b: back[i], kind: Kind::Moving }); }
    // The pivot caps on the outer faces of the arms.
    for side in [-1.0, 1.0] {
        let cap: Vec<V3> = (0..14).map(|i| { let a = i as f64 / 14.0 * std::f64::consts::TAU; (side * YOKE_HALF_W, TILT_AXIS + 3.4 * a.sin(), 3.4 * a.cos()) }).collect();
        close_loop(&cap, Kind::Moving, &mut turning);
    }

    // The head tilts about the axis through the arms, then turns with pan. Its own coordinates are
    // measured from the tilt axis; the front face is at -HEAD_HALF_D.
    let mut head: Vec<Line> = Vec::new();
    let outline = rounded_rect(HEAD_HALF_W, HEAD_HALF_H, HEAD_CORNER, 5);
    slab(&outline, 5, |(x, y)| (x, y, -HEAD_HALF_D), |(x, y)| (x, y, HEAD_HALF_D), Kind::Moving, &mut head);
    // A second outline just behind the front face gives the rounded front edge some depth.
    let bevel = rounded_rect(HEAD_HALF_W - 2.0, HEAD_HALF_H - 2.0, HEAD_CORNER - 1.5, 5);
    close_loop(&bevel.iter().map(|p| (p.0, p.1, -HEAD_HALF_D - 0.0)).collect::<Vec<_>>(), Kind::Moving, &mut head);
    // The lens: glass edge and the rings inside it.
    for (radius, depth) in LENS_RINGS {
        close_loop(&ring_xy(radius, -HEAD_HALF_D + depth, 28), Kind::Lens, &mut head);
    }
    // Pins joining the head to the arms.
    for side in [-1.0, 1.0] { head.push(Line { a: (side * HEAD_HALF_W, 0.0, 0.0), b: (side * (YOKE_HALF_W - YOKE_ARM), 0.0, 0.0), kind: Kind::Moving }); }
    // What the camera sees: a ray and a view frustum at the official field of view, 16:9.
    let front_z = -HEAD_HALF_D;
    let half_diagonal = (FOV_DIAGONAL_DEGREES / 2.0).to_radians().tan();
    let diagonal = (16.0f64 * 16.0 + 9.0 * 9.0).sqrt();
    let (half_w, half_h) = (half_diagonal * 16.0 / diagonal * CONE_REACH, half_diagonal * 9.0 / diagonal * CONE_REACH);
    head.push(Line { a: (0.0, 0.0, front_z), b: (0.0, 0.0, front_z - CONE_REACH * 1.2), kind: Kind::Aim });
    let far = [(-half_w, -half_h), (half_w, -half_h), (half_w, half_h), (-half_w, half_h)];
    for (x, y) in far { head.push(Line { a: (0.0, 0.0, front_z), b: (x, y, front_z - CONE_REACH), kind: Kind::Aim }); }
    for i in 0..4 {
        let (a, b) = (far[i], far[(i + 1) % 4]);
        head.push(Line { a: (a.0, a.1, front_z - CONE_REACH), b: (b.0, b.1, front_z - CONE_REACH), kind: Kind::Aim });
    }

    let pivot = (0.0, TILT_AXIS, 0.0);
    // Convert millimetres to drawing units, centred on the middle of the camera's height.
    let to_units = |v: V3| (v.0 / UNIT, (v.1 - 32.0) / UNIT, v.2 / UNIT);
    let pan_place = |v: V3| pan_by(v, pan);
    let mut out: Vec<Line> = lines.into_iter().map(|l| Line { a: to_units(l.a), b: to_units(l.b), kind: l.kind }).collect();
    for line in head {
        let place = |v: V3| {
            let r = tilt_by(v, tilt);
            pan_place((r.0 + pivot.0, r.1 + pivot.1, r.2 + pivot.2))
        };
        out.push(Line { a: to_units(place(line.a)), b: to_units(place(line.b)), kind: line.kind });
    }
    for line in turning {
        out.push(Line { a: to_units(pan_place(line.a)), b: to_units(pan_place(line.b)), kind: line.kind });
    }
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
    animating: Cell<bool>,
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
            area: area.clone(), target: Cell::new(None), shown: Cell::new((0.0, 0.0)),
            animating: Cell::new(false), last_frame: Cell::new(0), tick: RefCell::default(),
        });
        let model = GimbalModel(state);
        let drawer = model.clone();
        area.set_draw_func(move |_, cr, width, height| drawer.draw(cr, width, height));
        let weak = Rc::downgrade(&model.0);
        store.subscribe_gimbal(move |position| match weak.upgrade() {
            Some(state) => { GimbalModel(state).set_target(position); true }
            None => false,
        });
        model.set_target(store.gimbal());
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

    fn animate(&self) {
        let state = &self.0;
        if state.animating.replace(true) { return; }
        state.last_frame.set(0);
        let weak = Rc::downgrade(state);
        let id = state.area.add_tick_callback(move |_, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let Some((tp, tt)) = state.target.get() else { state.animating.set(false); return glib::ControlFlow::Break };
            let now = clock.frame_time();
            let dt = if state.last_frame.get() == 0 { 1.0 / 60.0 } else { ((now - state.last_frame.get()) as f64 / 1_000_000.0).clamp(0.001, 0.1) };
            state.last_frame.set(now);
            let (cp, ct) = state.shown.get();
            let (np, nt) = (approach(cp, tp, dt), approach(ct, tt, dt));
            let done = (np - tp).abs() < 0.05 && (nt - tt).abs() < 0.05;
            state.shown.set(if done { (tp, tt) } else { (np, nt) });
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
                Kind::Moving => (p.rgb("on_surface_variant"), 1.6, 0.95),
                Kind::Lens => (p.rgb("primary"), 1.8, 1.0),
                Kind::Aim => (p.rgb("primary"), 1.3, 0.55),
                Kind::Limit => (p.rgb("tertiary"), 2.0, 0.9),
            };
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            // Without a reading the model sits dim at centre instead of pretending.
            let fade = if known { 1.0 } else { 0.35 };
            for line in model(pan, tilt) {
                if !known && line.kind == Kind::Aim { continue; }
                let ((r, g, b), width, alpha) = colour(line.kind);
                cr.set_source_rgba(r, g, b, alpha * fade);
                cr.set_line_width(width);
                let (ax, ay) = project(line.a);
                let (bx, by) = project(line.b);
                cr.move_to(cx + ax * scale, cy + ay * scale);
                cr.line_to(cx + bx * scale, cy + by * scale);
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
            let lines = model(pan, tilt);
            let ray = lines.iter().filter(|l| l.kind == Kind::Aim).next().expect("an aim line");
            let d = (ray.b.0 - ray.a.0, ray.b.1 - ray.a.1, ray.b.2 - ray.a.2);
            let length = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
            let f = forward(pan, tilt);
            assert!(close((d.0 / length, d.1 / length, d.2 / length), f), "pan {pan} tilt {tilt}: {d:?} vs {f:?}");
        }
    }

    #[test]
    fn the_limits_are_the_ranges_the_camera_reports() {
        assert_eq!(PAN_LIMIT, 130.0);
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
        for line in model(0.0, 0.0) {
            for point in [line.a, line.b] {
                let (x, y) = project(point);
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
        let body: Vec<Line> = model(0.0, 0.0).into_iter().filter(|l| matches!(l.kind, Kind::Moving | Kind::Lens)
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

    #[test]
    fn the_view_cone_uses_the_official_field_of_view() {
        let lines = model(0.0, 0.0);
        let aims: Vec<&Line> = lines.iter().filter(|l| l.kind == Kind::Aim).collect();
        // Ray, four edges from the lens, four edges of the far rectangle.
        assert_eq!(aims.len(), 9);
        // Half the diagonal of the far rectangle, seen from the lens, is half the official 79.4°.
        let far = &aims[1].b;
        let lens = &aims[1].a;
        let (dx, dy, dz) = (far.0 - lens.0, far.1 - lens.1, far.2 - lens.2);
        let angle = (dx.hypot(dy)).atan2(dz.abs()).to_degrees();
        assert!((angle - 79.4 / 2.0).abs() < 0.2, "half diagonal {angle}");
    }
}
