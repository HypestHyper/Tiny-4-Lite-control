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

fn circle(center: V3, radius: f64, steps: usize, kind: Kind, out: &mut Vec<Line>) {
    for i in 0..steps {
        let (a0, a1) = (i as f64 / steps as f64 * std::f64::consts::TAU, (i + 1) as f64 / steps as f64 * std::f64::consts::TAU);
        out.push(Line {
            a: (center.0 + radius * a0.cos(), center.1, center.2 + radius * a0.sin()),
            b: (center.0 + radius * a1.cos(), center.1, center.2 + radius * a1.sin()),
            kind,
        });
    }
}

/// All the lines of the model for a pan and tilt, in world space.
fn model(pan: f64, tilt: f64) -> Vec<Line> {
    let mut lines = Vec::new();
    // The base and the floor ring stay put.
    circle((0.0, -1.25, 0.0), 1.05, 28, Kind::Base, &mut lines);
    circle((0.0, -1.05, 0.0), 1.05, 28, Kind::Base, &mut lines);
    for k in 0..4 {
        let a = k as f64 * std::f64::consts::FRAC_PI_2 + 0.4;
        lines.push(Line { a: (1.05 * a.cos(), -1.25, 1.05 * a.sin()), b: (1.05 * a.cos(), -1.05, 1.05 * a.sin()), kind: Kind::Base });
    }
    // Ticks on the ring: front (0°) and the two pan limits.
    for (deg, kind, length) in [(0.0, Kind::Base, 0.28), (PAN_LIMIT, Kind::Limit, 0.2), (-PAN_LIMIT, Kind::Limit, 0.2)] {
        let f = forward(deg, 0.0);
        lines.push(Line { a: (f.0 * 1.05, -1.25, f.2 * 1.05), b: (f.0 * (1.05 + length), -1.25, f.2 * (1.05 + length)), kind });
    }
    // Everything above the base turns with pan.
    let on_pan = |v: V3| pan_by(v, pan);
    let mut turning: Vec<Line> = Vec::new();
    circle((0.0, -1.05, 0.0), 0.5, 20, Kind::Moving, &mut turning);
    circle((0.0, -0.6, 0.0), 0.5, 20, Kind::Moving, &mut turning);
    for k in 0..4 {
        let a = k as f64 * std::f64::consts::FRAC_PI_2;
        turning.push(Line { a: (0.5 * a.cos(), -1.05, 0.5 * a.sin()), b: (0.5 * a.cos(), -0.6, 0.5 * a.sin()), kind: Kind::Moving });
    }
    // The yoke: a crossbar and two arms, each drawn as a thin pair of lines.
    for z in [-0.14, 0.14] {
        turning.push(Line { a: (-0.85, -0.6, z), b: (0.85, -0.6, z), kind: Kind::Moving });
        for x in [-0.85, 0.85] {
            turning.push(Line { a: (x, -0.6, z), b: (x, 0.1, z), kind: Kind::Moving });
        }
    }
    for x in [-0.85, 0.85] {
        turning.push(Line { a: (x, -0.6, -0.14), b: (x, -0.6, 0.14), kind: Kind::Moving });
        turning.push(Line { a: (x, 0.1, -0.14), b: (x, 0.1, 0.14), kind: Kind::Moving });
    }
    // The head turns with tilt about the pin through the arms (at y = 0.1), then with pan.
    let pivot = (0.0, 0.1, 0.0);
    let mut head: Vec<Line> = Vec::new();
    let (hx, hy, hz) = (0.62, 0.5, 0.3);
    let corner = |sx: f64, sy: f64, sz: f64| (sx * hx, sy * hy, sz * hz);
    for (a, b) in [
        ((-1., -1., -1.), (1., -1., -1.)), ((-1., 1., -1.), (1., 1., -1.)), ((-1., -1., 1.), (1., -1., 1.)), ((-1., 1., 1.), (1., 1., 1.)),
        ((-1., -1., -1.), (-1., 1., -1.)), ((1., -1., -1.), (1., 1., -1.)), ((-1., -1., 1.), (-1., 1., 1.)), ((1., -1., 1.), (1., 1., 1.)),
        ((-1., -1., -1.), (-1., -1., 1.)), ((1., -1., -1.), (1., -1., 1.)), ((-1., 1., -1.), (-1., 1., 1.)), ((1., 1., -1.), (1., 1., 1.)),
    ] {
        head.push(Line { a: corner(a.0, a.1, a.2), b: corner(b.0, b.1, b.2), kind: Kind::Moving });
    }
    // The lens is two rings on the front face (z = -hz), and the pins join the head to the arms.
    for (radius, kind) in [(0.34, Kind::Lens), (0.18, Kind::Lens)] {
        for i in 0..18 {
            let (a0, a1) = (i as f64 / 18.0 * std::f64::consts::TAU, (i + 1) as f64 / 18.0 * std::f64::consts::TAU);
            head.push(Line { a: (radius * a0.cos(), radius * a0.sin(), -hz), b: (radius * a1.cos(), radius * a1.sin(), -hz), kind });
        }
    }
    for x in [-1.0, 1.0] { head.push(Line { a: (x * hx, 0.0, 0.0), b: (x * 0.85, 0.0, 0.0), kind: Kind::Moving }); }
    // What the camera is looking at: a ray and a faint view frustum.
    let reach = 2.0;
    head.push(Line { a: (0.0, 0.0, -hz), b: (0.0, 0.0, -hz - reach), kind: Kind::Aim });
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        head.push(Line { a: (0.0, 0.0, -hz), b: (sx * 0.9, sy * 0.5, -hz - reach * 0.8), kind: Kind::Aim });
    }
    for (a, b) in [((-1.0, -1.0), (1.0, -1.0)), ((1.0, -1.0), (1.0, 1.0)), ((1.0, 1.0), (-1.0, 1.0)), ((-1.0, 1.0), (-1.0, -1.0))] {
        head.push(Line { a: (a.0 * 0.9, a.1 * 0.5, -hz - reach * 0.8), b: (b.0 * 0.9, b.1 * 0.5, -hz - reach * 0.8), kind: Kind::Aim });
    }
    for line in head {
        let place = |v: V3| {
            let r = tilt_by((v.0 - pivot.0, v.1 - pivot.1, v.2 - pivot.2), tilt);
            on_pan((r.0 + pivot.0, r.1 + pivot.1, r.2 + pivot.2))
        };
        lines.push(Line { a: place(line.a), b: place(line.b), kind: line.kind });
    }
    for line in turning { lines.push(Line { a: on_pan(line.a), b: on_pan(line.b), kind: line.kind }); }
    lines
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
        let scale = (f64::from(height) / 3.25).min(f64::from(width) / 3.9);
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
            let aim: Vec<&Line> = vec![];
            let _ = aim;
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
