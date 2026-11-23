//! Pictures of the hand gestures, beside the gesture settings: the reference drawings the gestures were
//! designed from (an open palm with a target, an L-shaped hand with a magnifier, two L-shaped hands with an
//! arrow between them), traced into smooth outlines by tools/trace-gestures.py. Shown in a round tonal
//! container like a Material list item's leading icon; the hand takes the container's text colour and the
//! mark the accent colour.

use super::gesture_paths as paths;
use gtk::cairo;
use gtk::prelude::*;
use std::f64::consts::PI;

/// The hand and the mark for each picture.
fn outlines(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Target => (paths::TARGET_HAND, paths::TARGET_MARK),
        Kind::Zoom => (paths::ZOOM_HAND, paths::ZOOM_MARK),
        Kind::Apart => (paths::APART_HAND, paths::APART_MARK),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind { Target, Zoom, Apart }

/// The picture for a gesture setting, if it has one.
pub fn for_setting(id: &str) -> Option<Kind> {
    match id {
        "obsbot:gesture_target" => Some(Kind::Target),
        "obsbot:gesture_zoom" => Some(Kind::Zoom),
        "obsbot:gesture_dynamic_zoom" => Some(Kind::Apart),
        _ => None,
    }
}

/// Numbers and command letters of a path, in order.
enum Token { Command(char), Number(f64) }

fn tokens(path: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut Vec<Token>| {
        if !number.is_empty() { if let Ok(v) = number.parse() { out.push(Token::Number(v)); } number.clear(); }
    };
    for c in path.chars() {
        if c.is_ascii_alphabetic() { flush(&mut number, &mut out); out.push(Token::Command(c)); }
        else if c == '-' && !number.is_empty() { flush(&mut number, &mut out); number.push(c); }
        else if c.is_ascii_digit() || c == '.' || c == '-' { number.push(c); }
        else { flush(&mut number, &mut out); }
    }
    flush(&mut number, &mut out);
    out
}

/// An SVG arc (from the current point to x, y) as a cairo arc, from its centre.
#[allow(clippy::too_many_arguments)]
fn arc_to(cr: &cairo::Context, from: (f64, f64), r: f64, large: bool, sweep: bool, to: (f64, f64)) {
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let half = (dx * dx + dy * dy).sqrt();
    if half < 1e-9 { return; }
    let r = r.max(half);
    let mut along = ((r * r - half * half) / (half * half)).max(0.0).sqrt();
    if large == sweep { along = -along; }
    // The centre: the chord's middle, moved along its normal.
    let (mx, my) = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let (cx, cy) = (mx + along * dy, my - along * dx);
    let start = (from.1 - cy).atan2(from.0 - cx);
    let end = (to.1 - cy).atan2(to.0 - cx);
    // In SVG, sweep 1 runs the way cairo's arc does (angles growing, with y down).
    if sweep { cr.arc(cx, cy, r, start, end); } else { cr.arc_negative(cx, cy, r, start, end); }
}

/// Add a path to cairo's current path.
fn trace(cr: &cairo::Context, path: &str) {
    let tokens = tokens(path);
    let mut i = 0;
    let mut at = (0.0, 0.0);
    let mut command = 'M';
    let take = |i: &mut usize| -> Option<f64> {
        match tokens.get(*i) { Some(Token::Number(v)) => { *i += 1; Some(*v) } _ => None }
    };
    while i < tokens.len() {
        if let Token::Command(c) = tokens[i] { command = c; i += 1; }
        let before = i;
        match command {
            'M' => { let (Some(x), Some(y)) = (take(&mut i), take(&mut i)) else { break }; cr.move_to(x, y); at = (x, y); command = 'L'; }
            'L' => { let (Some(x), Some(y)) = (take(&mut i), take(&mut i)) else { break }; cr.line_to(x, y); at = (x, y); }
            'H' => { let Some(x) = take(&mut i) else { break }; cr.line_to(x, at.1); at.0 = x; }
            'V' => { let Some(y) = take(&mut i) else { break }; cr.line_to(at.0, y); at.1 = y; }
            'C' => {
                let v: Vec<f64> = (0..6).filter_map(|_| take(&mut i)).collect();
                if v.len() < 6 { break; }
                cr.curve_to(v[0], v[1], v[2], v[3], v[4], v[5]);
                at = (v[4], v[5]);
            }
            'A' => {
                let v: Vec<f64> = (0..7).filter_map(|_| take(&mut i)).collect();
                if v.len() < 7 { break; }
                arc_to(cr, at, v[0], v[3] != 0.0, v[4] != 0.0, (v[5], v[6]));
                at = (v[5], v[6]);
            }
            'Z' => { cr.close_path(); }
            'O' => {
                let (Some(x), Some(y), Some(r)) = (take(&mut i), take(&mut i), take(&mut i)) else { break };
                cr.new_sub_path();
                cr.arc(x, y, r, 0.0, 2.0 * PI);
                cr.close_path();
                at = (x + r, y);
            }
            _ => break,
        }
        if i == before && command != 'Z' { break; }
        if command == 'Z' { command = 'M'; }
    }
}

fn draw(cr: &cairo::Context, size: f64, path: &str, colour: gtk::gdk::RGBA) {
    cr.set_source_rgba(f64::from(colour.red()), f64::from(colour.green()), f64::from(colour.blue()), f64::from(colour.alpha()));
    cr.save().ok();
    cr.scale(size / 48.0, size / 48.0);
    cr.new_path();
    trace(cr, path);
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    let _ = cr.fill();
    cr.restore().ok();
}

/// The round container with the gesture's picture in it.
pub fn picture(kind: Kind, size: i32) -> gtk::Box {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("gesture-icon");
    holder.set_size_request(size, size);
    holder.set_valign(gtk::Align::Center);
    holder.set_halign(gtk::Align::Center);
    let art = (size as f64 * 0.8).round() as i32;
    let overlay = gtk::Overlay::new();
    overlay.set_halign(gtk::Align::Center);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_vexpand(true);
    let (hand, mark) = outlines(kind);
    let layer = |path: &'static str, class: &str| {
        let area = gtk::DrawingArea::new();
        area.add_css_class(class);
        area.set_content_width(art);
        area.set_content_height(art);
        area.set_draw_func(move |area, cr, w, h| {
            #[allow(deprecated)]
            let colour = area.style_context().color();
            draw(cr, f64::from(w.min(h)), path, colour);
        });
        area
    };
    overlay.set_child(Some(&layer(hand, "gesture-hand")));
    overlay.add_overlay(&layer(mark, "gesture-mark"));
    holder.append(&overlay);
    holder.set_accessible_role(gtk::AccessibleRole::Presentation);
    holder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_gestures_have_pictures_whose_outlines_read_whole() {
        for id in ["obsbot:gesture_target", "obsbot:gesture_zoom", "obsbot:gesture_dynamic_zoom"] {
            let (hand, mark) = outlines(for_setting(id).expect("a picture"));
            for path in [hand, mark] {
                let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 48, 48).unwrap();
                let cr = cairo::Context::new(&surface).unwrap();
                trace(&cr, path);
                let (x1, y1, x2, y2) = cr.path_extents().unwrap();
                assert!(x1 >= 0.0 && y1 >= 0.0 && x2 <= 48.0 && y2 <= 48.0, "{id}: outside the box");
                assert!(x2 - x1 > 5.0, "{id}: drew almost nothing");
            }
        }
        assert_eq!(for_setting("obsbot:gesture_zoom_ratio"), None);
    }
}
