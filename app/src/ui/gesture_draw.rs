//! Pictures of the hand gestures, beside the gesture settings.
//!
//! Drawn the way Material Symbols are: on a 24-unit grid, from rounded shapes with even gaps, a solid
//! hand in the container's text colour and the part that explains the gesture (target, magnifier,
//! arrow) in the accent colour, inside a round tonal container like a Material list item's leading icon.

use super::theme;
use gtk::cairo::{Context, LineCap, LineJoin};
use gtk::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// An open palm: target selection.
    Target,
    /// An L-shaped hand (index up, thumb out): the zoom gesture.
    Zoom,
    /// Two L-shaped hands moving apart: dynamic zoom.
    Dynamic,
}

/// The picture for a gesture setting, if it has one.
pub fn for_setting(id: &str) -> Option<Gesture> {
    match id {
        "obsbot:gesture_target" => Some(Gesture::Target),
        "obsbot:gesture_zoom" => Some(Gesture::Zoom),
        "obsbot:gesture_dynamic_zoom" => Some(Gesture::Dynamic),
        _ => None,
    }
}

/// The round container with the gesture in it, `size` pixels across, following the theme.
pub fn picture(gesture: Gesture, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("gesture-picture");
    area.set_accessible_role(gtk::AccessibleRole::Presentation);
    area.set_draw_func(move |_, cr, width, height| {
        let side = f64::from(width.min(height));
        let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        theme::with(|p| {
            let container = p.rgb("secondary_container");
            let ink = p.rgb("on_secondary_container");
            let accent = p.rgb("primary");
            cr.set_source_rgb(container.0, container.1, container.2);
            cr.arc(cx, cy, side / 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            // The 24-unit icon takes the middle 60%, as a Material icon does in its container.
            let scale = side * 0.6 / 24.0;
            cr.translate(cx - 12.0 * scale, cy - 12.0 * scale);
            cr.scale(scale, scale);
            cr.set_line_cap(LineCap::Round);
            cr.set_line_join(LineJoin::Round);
            draw(cr, gesture, ink, accent, container);
        });
    });
    area
}

type Rgb = (f64, f64, f64);

fn colour(cr: &Context, c: Rgb) { cr.set_source_rgb(c.0, c.1, c.2); }

/// A finger: a straight stroke with round ends, `width` wide.
fn finger(cr: &Context, from: (f64, f64), to: (f64, f64), width: f64) {
    cr.set_line_width(width);
    cr.move_to(from.0, from.1);
    cr.line_to(to.0, to.1);
    let _ = cr.stroke();
}

/// A filled rectangle with rounded corners.
fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
    let _ = cr.fill();
}

/// A thin line in the container colour, to mark a gap between solid parts (between curled fingers).
fn cut(cr: &Context, container: Rgb, from: (f64, f64), to: (f64, f64)) {
    colour(cr, container);
    finger(cr, from, to, 0.9);
}

/// One L-shaped hand: the index finger straight up at `x`, the thumb out sideways towards `thumb` (+1
/// right, −1 left), and the fist with its curled fingers on the other side, its top at `fist_top`.
fn l_hand(cr: &Context, ink: Rgb, container: Rgb, x: f64, thumb: f64, top: f64, fist_top: f64, finger_w: f64, size: f64, thumb_len: f64) {
    let (fist_w, fist_h) = (7.4 * size, 7.6 * size);
    let half = finger_w / 2.0;
    // The fist starts under the index finger and goes away from the thumb.
    let fist_x = if thumb < 0.0 { x - half } else { x + half - fist_w };
    colour(cr, ink);
    rounded(cr, fist_x, fist_top, fist_w, fist_h, 2.4);
    finger(cr, (x, fist_top + 2.0), (x, top), finger_w);
    finger(cr, (x, fist_top + 3.4 * size), (x + thumb * thumb_len, fist_top + 2.2 * size), finger_w);
    // The curled fingers, as two lines across the fist.
    let (from, to) = if thumb < 0.0 { (x + half + 1.4, fist_x + fist_w - 1.3) } else { (x - half - 1.4, fist_x + 1.3) };
    for y in [fist_top + 2.6 * size, fist_top + 4.9 * size] { cut(cr, container, (from, y), (to, y)); }
}

fn draw(cr: &Context, gesture: Gesture, ink: Rgb, accent: Rgb, container: Rgb) {
    match gesture {
        Gesture::Target => {
            colour(cr, ink);
            // Four fingers, the middle ones longest, then the palm and the thumb.
            for (x, top) in [(7.6, 4.6), (10.6, 2.6), (13.6, 3.4), (16.6, 6.2)] { finger(cr, (x, 12.0), (x, top), 2.3); }
            rounded(cr, 6.45, 9.8, 11.3, 12.2, 4.4);
            finger(cr, (7.5, 16.0), (3.4, 11.6), 2.3);
            // The gaps between the fingers reach a little into the palm.
            for x in [9.1, 12.1, 15.1] { cut(cr, container, (x, 9.0), (x, 10.4)); }
            // The target on the palm, with a ring of the container colour so it stands clear of the hand.
            let (tx, ty) = (12.1, 15.9);
            colour(cr, container);
            cr.arc(tx, ty, 3.4, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            colour(cr, accent);
            cr.set_line_width(1.2);
            cr.arc(tx, ty, 2.3, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.arc(tx, ty, 0.75, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                finger(cr, (tx + dx * 2.3, ty + dy * 2.3), (tx + dx * 3.3, ty + dy * 3.3), 1.2);
            }
        }
        Gesture::Zoom => {
            l_hand(cr, ink, container, 8.0, -1.0, 2.6, 13.6, 2.6, 1.0, 5.2);
            // A magnifier with a plus, beside the raised finger.
            colour(cr, accent);
            let (mx, my) = (17.0, 6.6);
            cr.set_line_width(1.6);
            cr.arc(mx, my, 3.3, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            finger(cr, (mx + 2.4, my + 2.4), (mx + 4.8, my + 4.8), 2.0);
            finger(cr, (mx - 1.5, my), (mx + 1.5, my), 1.3);
            finger(cr, (mx, my - 1.5), (mx, my + 1.5), 1.3);
        }
        Gesture::Dynamic => {
            // Two hands, thumbs towards each other, and an arrow between the raised fingers: moving the
            // hands apart zooms.
            l_hand(cr, ink, container, 6.6, 1.0, 4.0, 14.6, 2.3, 0.78, 2.6);
            l_hand(cr, ink, container, 17.4, -1.0, 4.0, 14.6, 2.3, 0.78, 2.6);
            colour(cr, accent);
            let y = 7.0;
            finger(cr, (9.4, y), (14.6, y), 1.5);
            for (tip, back) in [(9.4, 11.2), (14.6, 12.8)] {
                finger(cr, (tip, y), (back, y - 1.6), 1.4);
                finger(cr, (tip, y), (back, y + 1.6), 1.4);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_gestures_have_pictures_and_others_do_not() {
        assert_eq!(for_setting("obsbot:gesture_target"), Some(Gesture::Target));
        assert_eq!(for_setting("obsbot:gesture_zoom"), Some(Gesture::Zoom));
        assert_eq!(for_setting("obsbot:gesture_dynamic_zoom"), Some(Gesture::Dynamic));
        assert_eq!(for_setting("obsbot:gesture_zoom_ratio"), None);
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// With T4_GESTURE_PREVIEW=path, writes the three pictures large as raw RGBA (for looking at them).
    #[test]
    fn write_preview_when_asked() {
        let Some(path) = std::env::var_os("T4_GESTURE_PREVIEW") else { return };
        let (size, count) = (240, 3);
        let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, size * count, size).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            for (i, gesture) in [Gesture::Target, Gesture::Zoom, Gesture::Dynamic].into_iter().enumerate() {
                cr.save().unwrap();
                cr.translate(f64::from(size) * i as f64, 0.0);
                let (container, ink, accent) = ((0.86, 0.89, 0.95), (0.10, 0.15, 0.25), (0.20, 0.45, 0.85));
                colour(&cr, container);
                cr.arc(f64::from(size) / 2.0, f64::from(size) / 2.0, f64::from(size) / 2.0, 0.0, std::f64::consts::TAU);
                cr.fill().unwrap();
                let scale = f64::from(size) * 0.6 / 24.0;
                cr.translate(f64::from(size) * 0.2, f64::from(size) * 0.2);
                cr.scale(scale, scale);
                cr.set_line_cap(LineCap::Round);
                cr.set_line_join(LineJoin::Round);
                draw(&cr, gesture, ink, accent, container);
                cr.restore().unwrap();
            }
        }
        surface.flush();
        let data = surface.take_data().unwrap();
        std::fs::write(path, &data[..]).unwrap();
    }
}
