//! Pictures of the hand gestures, beside the gesture settings: Google's own Material Symbols for each
//! hand pose, in a round tonal container like a Material list item's leading icon.

use super::widgets;
use gtk::prelude::*;

/// The Material Symbol for a gesture setting, if it has one.
pub fn for_setting(id: &str) -> Option<&'static str> {
    match id {
        // An open palm picks who to follow.
        "obsbot:gesture_target" => Some("front_hand"),
        // The L-shaped hand, index finger up.
        "obsbot:gesture_zoom" => Some("pan_tool_alt"),
        // Fingers moving apart.
        "obsbot:gesture_dynamic_zoom" => Some("pinch_zoom_out"),
        _ => None,
    }
}

/// The round container with the gesture's symbol in it.
pub fn picture(glyph: &str, size: i32) -> gtk::Box {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("gesture-icon");
    holder.set_size_request(size, size);
    holder.set_valign(gtk::Align::Center);
    holder.set_halign(gtk::Align::Center);
    let icon = widgets::icon(glyph);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_vexpand(true);
    holder.append(&icon);
    holder.set_accessible_role(gtk::AccessibleRole::Presentation);
    holder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_gestures_have_symbols_that_are_in_the_font() {
        for id in ["obsbot:gesture_target", "obsbot:gesture_zoom", "obsbot:gesture_dynamic_zoom"] {
            let glyph = for_setting(id).expect("a symbol");
            assert!(super::super::icons::glyph(glyph).is_some(), "{glyph} is not in the bundled font");
        }
        assert_eq!(for_setting("obsbot:gesture_zoom_ratio"), None);
    }
}
