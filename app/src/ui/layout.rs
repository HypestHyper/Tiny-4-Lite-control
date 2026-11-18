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
