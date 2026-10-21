//! Terminal find bar (Ctrl+Shift+F), driven by Ghostty's search binding actions.
//!
//! Mirrors Ghostty's own GTK search overlay (apprt/gtk/class/search_overlay.zig
//! and surface.zig): typing sends `search:<text>`, Enter / Shift+Enter send
//! `navigate_search:next|previous`, Escape sends `end_search`, and Ghostty
//! reports results back through the SEARCH_TOTAL / SEARCH_SELECTED actions.
//!
//! There is a single bar per window, overlaid on the top-right of the
//! workspace area. It targets one surface at a time (the pane that was focused
//! when it opened). The split engine reparents GLAreas, so a window-level
//! overlay is more robust than one widget per pane; when focus moves to a
//! different terminal pane, the bar ends the old search and hides.

use crate::ghostty::ffi;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Binding action that searches for `query` (an empty query cancels).
/// Ghostty splits the action at the first ':' only, so the query may itself
/// contain ':' and any UTF-8 text.
pub fn search_action(query: &str) -> String {
    format!("search:{query}")
}

/// Binding action that moves to the next or previous match.
pub fn navigate_action(previous: bool) -> &'static str {
    if previous {
        "navigate_search:previous"
    } else {
        "navigate_search:next"
    }
}

/// Binding action that ends the search and clears highlights.
pub const END_SEARCH_ACTION: &str = "end_search";

/// Ghostty reports counts as ssize_t where a negative value means "unknown".
pub fn count_from_ssize(v: isize) -> Option<usize> {
    usize::try_from(v).ok()
}

/// Counter text shown in the bar, matching Ghostty's overlay ("2/15").
/// `selected` is the 0-based index of the selected match.
pub fn format_counter(total: Option<usize>, selected: Option<usize>) -> String {
    match total {
        None | Some(0) => "0/0".to_string(),
        Some(total) => format!("{}/{}", selected.map(|s| s + 1).unwrap_or(0), total),
    }
}

struct FindBar {
    root: gtk4::Box,
    entry: gtk4::SearchEntry,
    counter: gtk4::Label,
    /// Surface the bar searches in (0 when closed).
    target: Cell<usize>,
    /// Terminal widget to give focus back to when the bar closes.
    target_widget: RefCell<Option<glib::WeakRef<gtk4::Widget>>>,
    /// Last needle sent to Ghostty, to avoid re-sending the same search when
    /// both a socket call and the entry's delayed search-changed fire.
    last_needle: RefCell<String>,
    total: Cell<Option<usize>>,
    selected: Cell<Option<usize>>,
}

thread_local! {
    static BAR: RefCell<Option<Rc<FindBar>>> = const { RefCell::new(None) };
    static COUNTER_UPDATE_PENDING: Cell<bool> = const { Cell::new(false) };
    static RETARGET_CHECK_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Clone the bar out of the thread-local. Never hold the thread-local borrow
/// across a Ghostty call: binding actions re-enter action_cb synchronously
/// (e.g. end_search → END_SEARCH).
fn current_bar() -> Option<Rc<FindBar>> {
    BAR.with(|b| b.borrow().clone())
}

/// A surface is alive while it is in SURFACE_REGISTRY (both pane close and
/// workspace close remove it before/after ghostty_surface_free).
fn surface_alive(surface: usize) -> bool {
    surface != 0
        && crate::ghostty::callbacks::SURFACE_REGISTRY
            .lock()
            .map(|r| r.contains_key(&surface))
            .unwrap_or(false)
}

fn binding(surface: usize, action: &str) -> bool {
    if !surface_alive(surface) {
        return false;
    }
    unsafe {
        ffi::ghostty_surface_binding_action(
            surface as ffi::ghostty_surface_t,
            action.as_ptr() as *const _,
            action.len(),
        )
    }
}

impl FindBar {
    fn update_counter(&self) {
        self.counter
            .set_text(&format_counter(self.total.get(), self.selected.get()));
    }

    fn reset_results(&self) {
        self.total.set(None);
        self.selected.set(None);
        self.update_counter();
    }

    /// Send `query` to the target surface unless it is what we sent last.
    fn search(&self, query: &str) {
        if *self.last_needle.borrow() == query {
            return;
        }
        *self.last_needle.borrow_mut() = query.to_string();
        if query.is_empty() {
            self.reset_results();
        }
        binding(self.target.get(), &search_action(query));
    }

    fn navigate(&self, previous: bool) {
        // Make sure the current text is searched before navigating (the
        // entry's search-changed signal is delayed).
        let text = self.entry.text();
        self.search(text.as_str());
        binding(self.target.get(), navigate_action(previous));
    }

    /// Hide the bar. With `end_search`, also end the search in Ghostty; with
    /// `refocus`, give keyboard focus back to the terminal if the bar had it
    /// (focus is never taken from elsewhere, e.g. on socket-driven closes).
    fn hide(&self, end_search: bool, refocus: bool) {
        let target = self.target.replace(0);
        let widget = self
            .target_widget
            .borrow_mut()
            .take()
            .and_then(|w| w.upgrade());
        // Check before hiding: hiding drops focus from the entry.
        let bar_had_focus = bar_has_focus(self);
        self.root.set_visible(false);
        self.last_needle.borrow_mut().clear();
        self.reset_results();
        if end_search && target != 0 {
            binding(target, END_SEARCH_ACTION);
        }
        if refocus && bar_had_focus {
            if let Some(w) = widget {
                w.grab_focus();
            }
        }
    }
}

/// Build the bar and add it to `overlay` (which wraps the workspace stack).
pub fn install(window: &gtk4::ApplicationWindow, overlay: &gtk4::Overlay) {
    let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    root.add_css_class("cmux-find-bar");
    root.set_halign(gtk4::Align::End);
    root.set_valign(gtk4::Align::Start);
    root.set_margin_top(8);
    root.set_margin_end(16);
    root.set_visible(false);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Find"));
    entry.set_width_chars(24);
    root.append(&entry);

    let counter = gtk4::Label::new(Some("0/0"));
    counter.add_css_class("cmux-find-counter");
    counter.set_width_chars(7);
    root.append(&counter);

    let prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    prev_btn.set_tooltip_text(Some("Previous Match (Shift+Enter)"));
    prev_btn.add_css_class("headerbar-btn");
    root.append(&prev_btn);
    let next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    next_btn.set_tooltip_text(Some("Next Match (Enter)"));
    next_btn.add_css_class("headerbar-btn");
    root.append(&next_btn);
    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.set_tooltip_text(Some("Close (Esc)"));
    close_btn.add_css_class("headerbar-btn");
    root.append(&close_btn);

    overlay.add_overlay(&root);

    let new_bar = Rc::new(FindBar {
        root,
        entry: entry.clone(),
        counter,
        target: Cell::new(0),
        target_widget: RefCell::new(None),
        last_needle: RefCell::new(String::new()),
        total: Cell::new(None),
        selected: Cell::new(None),
    });
    BAR.with(|b| *b.borrow_mut() = Some(new_bar));

    // Typing (search-changed is debounced by GtkSearchEntry, like Ghostty's).
    entry.connect_search_changed(|e| {
        if let Some(bar) = current_bar() {
            if bar.target.get() != 0 {
                bar.search(e.text().as_str());
            }
        }
    });
    // Enter → next; Shift+Enter is handled by the key controller below.
    entry.connect_activate(|_| {
        if let Some(bar) = current_bar() {
            bar.navigate(false);
        }
    });
    // Ctrl+G / Ctrl+Shift+G (GtkSearchEntry keybindings).
    entry.connect_next_match(|_| {
        if let Some(bar) = current_bar() {
            bar.navigate(false);
        }
    });
    entry.connect_previous_match(|_| {
        if let Some(bar) = current_bar() {
            bar.navigate(true);
        }
    });
    // Escape.
    entry.connect_stop_search(|_| close(true));

    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, keyval, _, mods| {
        let is_enter = matches!(keyval, gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter);
        if is_enter && mods.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
            if let Some(bar) = current_bar() {
                bar.navigate(true);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    entry.add_controller(keys);

    prev_btn.connect_clicked(|_| {
        if let Some(bar) = current_bar() {
            bar.navigate(true);
        }
    });
    next_btn.connect_clicked(|_| {
        if let Some(bar) = current_bar() {
            bar.navigate(false);
        }
    });
    close_btn.connect_clicked(|_| close(true));

    // Retarget guard: when the user moves to a different pane or workspace
    // (terminal or not), end the search on the old pane and hide. Deferred so
    // it never runs in the middle of a split-engine mutation.
    window.connect_notify_local(Some("focus-widget"), |_, _| {
        if RETARGET_CHECK_PENDING.with(|p| p.replace(true)) {
            return;
        }
        glib::idle_add_local_once(|| {
            RETARGET_CHECK_PENDING.with(|p| p.set(false));
            let Some(bar) = current_bar() else { return };
            let target = bar.target.get();
            if target == 0 || bar_has_focus(&bar) {
                return;
            }
            match crate::header_bar::current_terminal() {
                Ok(Some((_, s))) if s == target => {}
                Ok(_) => bar.hide(true, false),
                Err(()) => {} // AppState busy; the next focus change re-checks
            }
        });
    });
}

/// True if keyboard focus is inside the bar (e.g. typing in the entry).
fn bar_has_focus(bar: &FindBar) -> bool {
    bar.root
        .root()
        .and_then(|r| r.focus())
        .map(|f| f.is_ancestor(&bar.root))
        .unwrap_or(false)
}

/// Open the bar on `surface`, whose terminal widget is `widget`. A non-empty
/// `needle` replaces the query (Ghostty's search_selection passes one).
/// `focus` moves keyboard focus into the entry (not for socket calls).
pub fn open(surface: usize, widget: Option<&gtk4::Widget>, needle: Option<&str>, focus: bool) {
    let Some(bar) = current_bar() else { return };
    if surface == 0 {
        return;
    }
    let old = bar.target.get();
    if old != surface {
        if old != 0 {
            bar.hide(true, false);
        }
        bar.target.set(surface);
        *bar.target_widget.borrow_mut() = widget.map(|w| w.downgrade());
        bar.reset_results();
        bar.last_needle.borrow_mut().clear();
    }
    bar.root.set_visible(true);
    if let Some(needle) = needle.filter(|n| !n.is_empty()) {
        bar.entry.set_text(needle);
        bar.search(needle);
    } else if old != surface && !bar.entry.text().is_empty() {
        // Re-run the previous query on the new target.
        let text = bar.entry.text();
        bar.search(text.as_str());
    }
    if focus {
        bar.entry.grab_focus();
        bar.entry.select_region(0, -1);
    }
}

/// Open the bar on the terminal the user is on (win.find, and socket
/// `surface.find` with `focus == false`). Returns false if there is none.
pub fn open_for_current(focus: bool) -> bool {
    let Ok(Some((widget, surface))) = crate::header_bar::current_terminal() else {
        return false;
    };
    open(surface, Some(&widget), None, focus);
    true
}

/// Search for `query` immediately (socket `surface.find`).
pub fn set_query(query: &str) {
    let Some(bar) = current_bar() else { return };
    if bar.target.get() == 0 {
        return;
    }
    bar.entry.set_text(query);
    bar.search(query);
}

/// Move to the next/previous match.
pub fn navigate(previous: bool) {
    if let Some(bar) = current_bar() {
        if bar.target.get() != 0 {
            bar.navigate(previous);
        }
    }
}

/// Close the bar and end the search. `refocus` gives keyboard focus back to
/// the terminal if the bar had it (UI paths only; socket closes pass false).
pub fn close(refocus: bool) {
    if let Some(bar) = current_bar() {
        bar.hide(true, refocus);
    }
}

/// START_SEARCH from Ghostty (its start_search / search_selection keybinds).
pub fn on_start_search(surface: usize, needle: &str) {
    let window_focus = current_bar()
        .and_then(|b| b.root.root())
        .and_then(|r| r.focus())
        .filter(|w| crate::header_bar::surface_for_widget(w) == Some(surface));
    open(surface, window_focus.as_ref(), Some(needle), true);
}

/// END_SEARCH from Ghostty (Escape in the terminal, or our own end_search).
pub fn on_end_search(surface: usize) {
    if let Some(bar) = current_bar() {
        if bar.target.get() == surface {
            // Ghostty already ended the search; just hide.
            bar.hide(false, true);
        }
    }
}

/// SEARCH_TOTAL/SELECTED come through ghostty_app_tick, which also runs inside
/// GLArea resize handlers (mid-layout), so the label is updated on idle.
fn schedule_counter_update() {
    if COUNTER_UPDATE_PENDING.with(|p| p.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        COUNTER_UPDATE_PENDING.with(|p| p.set(false));
        if let Some(bar) = current_bar() {
            bar.update_counter();
        }
    });
}

pub fn on_search_total(surface: usize, total: Option<usize>) {
    eprintln!("cmux: search total={total:?} surface={surface:#x}");
    if let Some(bar) = current_bar() {
        if bar.target.get() == surface {
            bar.total.set(total);
            schedule_counter_update();
        }
    }
}

pub fn on_search_selected(surface: usize, selected: Option<usize>) {
    eprintln!("cmux: search selected={selected:?} surface={surface:#x}");
    if let Some(bar) = current_bar() {
        if bar.target.get() == surface {
            bar.selected.set(selected);
            schedule_counter_update();
        }
    }
}

/// The surface is being freed: drop it as a target without calling Ghostty.
pub fn forget_surface(surface: usize) {
    if let Some(bar) = current_bar() {
        if bar.target.get() == surface {
            bar.hide(false, false);
        }
    }
}

/// Snapshot for socket queries.
pub fn status() -> serde_json::Value {
    match current_bar() {
        Some(bar) => serde_json::json!({
            "open": bar.target.get() != 0,
            "query": bar.entry.text().to_string(),
            "total": bar.total.get(),
            "selected": bar.selected.get(),
            "counter": format_counter(bar.total.get(), bar.selected.get()),
        }),
        None => serde_json::json!({"open": false}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_action_strings() {
        assert_eq!(search_action("needle"), "search:needle");
        assert_eq!(search_action(""), "search:");
        // Korean and colons are passed through verbatim (Ghostty splits on
        // the first ':' only).
        assert_eq!(search_action("needle-한글"), "search:needle-한글");
        assert_eq!(search_action("a:b c"), "search:a:b c");
        assert_eq!(navigate_action(false), "navigate_search:next");
        assert_eq!(navigate_action(true), "navigate_search:previous");
        assert_eq!(END_SEARCH_ACTION, "end_search");
    }

    #[test]
    fn test_counts_and_counter_text() {
        assert_eq!(count_from_ssize(-1), None);
        assert_eq!(count_from_ssize(0), Some(0));
        assert_eq!(count_from_ssize(42), Some(42));
        assert_eq!(format_counter(None, None), "0/0");
        assert_eq!(format_counter(Some(0), None), "0/0");
        assert_eq!(format_counter(Some(15), None), "0/15");
        assert_eq!(format_counter(Some(15), Some(0)), "1/15");
        assert_eq!(format_counter(Some(15), Some(14)), "15/15");
    }
}
