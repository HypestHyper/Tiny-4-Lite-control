use gtk::prelude::*;

/// Capture the vertical position of the nearest scrolled window containing a widget.
pub fn capture_scroll_position(widget: &gtk::Widget) -> Option<(gtk::Adjustment, f64)> {
    let mut parent = widget.parent();
    while let Some(current) = parent {
        if let Ok(scrolled) = current.clone().downcast::<gtk::ScrolledWindow>() {
            let adjustment = scrolled.vadjustment();
            return Some((adjustment.clone(), adjustment.value()));
        }
        parent = current.parent();
    }
    None
}

/// Restore after GTK recalculates the rebuilt page's content allocation.
pub fn restore_scroll_position(position: Option<(gtk::Adjustment, f64)>) {
    if let Some((adjustment, value)) = position {
        gtk::glib::idle_add_local_once(move || {
            let lower = adjustment.lower();
            let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
            adjustment.set_value(value.clamp(lower, upper));
        });
    }
}



/// Prevent the mouse wheel from changing a slider or selection while the user
/// is scrolling the surrounding settings page. Pointer dragging and keyboard
/// navigation remain available.
pub fn disable_scroll_wheel_controls(widget: &gtk::Widget) {
    if !widget.has_css_class("scroll-wheel-locked")
        && (widget.is::<gtk::Scale>() || widget.is::<gtk::ComboBox>())
    {
        let controller = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL
                | gtk::EventControllerScrollFlags::DISCRETE,
        );
        controller.connect_scroll(|_, _, _| gtk::glib::Propagation::Stop);
        widget.add_controller(controller);
        widget.add_css_class("scroll-wheel-locked");
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        disable_scroll_wheel_controls(&current);
    }
}

/// Mark a control row or section as selectable in Compact mode.
pub fn mark_compact_item(widget: &gtk::Widget, key: &str) {
    let class = format!("compact-item-{}", key.replace(':', "--").replace('_', "-"));
    widget.add_css_class(&class);
}

/// Hide unselected quick controls while retaining section frames that contain
/// at least one selected control.
pub fn apply_compact_visibility(widget: &gtk::Widget, compact: bool, selected: &[String]) {
    let classes = widget.css_classes();
    let item_key = classes.iter().find_map(|class| {
        class.as_str().strip_prefix("compact-item-").map(|suffix| {
            suffix.replace("--", ":").replace('-', "_")
        })
    });
    if let Some(key) = item_key {
        let normalized = key.replace(':', "_");
        let selected_item = selected.iter().any(|item| item == &normalized || item.replace(':', "_") == normalized);
        widget.set_visible(!compact || selected_item);
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        apply_compact_visibility(&current, compact, selected);
    }

    if widget.has_css_class("compact-group") {
        widget.set_visible(!compact || has_visible_compact_item(widget));
    }
}

fn has_visible_compact_item(widget: &gtk::Widget) -> bool {
    if widget.is_visible() && widget.css_classes().iter()
        .any(|class| class.as_str().starts_with("compact-item-")) {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if has_visible_compact_item(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}
