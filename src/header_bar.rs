use gtk4::prelude::*;

/// Build the header bar per D-04, D-05, D-06.
/// Returns None if config style is "none".
pub fn build_header_bar(config: &crate::config::Config) -> Option<gtk4::HeaderBar> {
    if config.ui.header_bar.style == "none" {
        return None;
    }

    let header = gtk4::HeaderBar::new();
    header.add_css_class("cmux-headerbar");

    // Left side: workspace actions (D-06)

    // [+] New Workspace (D-05)
    let new_ws_btn = gtk4::Button::from_icon_name("tab-new-symbolic");
    new_ws_btn.set_tooltip_text(Some("New Workspace (Ctrl+N)"));
    new_ws_btn.set_action_name(Some("win.new-workspace"));
    new_ws_btn.add_css_class("headerbar-btn");
    header.pack_start(&new_ws_btn);

    // [Browser] New Browser (D-05, D-07)
    let browser_btn = gtk4::Button::from_icon_name("web-browser-symbolic");
    browser_btn.set_tooltip_text(Some("New Browser (Ctrl+Shift+B)"));
    browser_btn.set_action_name(Some("win.browser-open"));
    browser_btn.add_css_class("headerbar-btn");
    header.pack_start(&browser_btn);

    // Right side: pane/view actions (D-06)
    // Note: pack_end adds right-to-left, so order is reversed from visual

    // Hamburger menu (D-11) — rightmost, so pack_end first
    let menu_model = crate::menus::build_hamburger_menu();
    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_tooltip_text(Some("Menu"));
    menu_btn.set_menu_model(Some(&menu_model));
    menu_btn.add_css_class("headerbar-btn");
    header.pack_end(&menu_btn);

    // Toggle Sidebar (D-05) — second from right
    let sidebar_btn = gtk4::Button::from_icon_name("sidebar-show-symbolic");
    sidebar_btn.set_tooltip_text(Some("Toggle Sidebar (Ctrl+B)"));
    sidebar_btn.set_action_name(Some("win.toggle-sidebar"));
    sidebar_btn.add_css_class("headerbar-btn");
    header.pack_end(&sidebar_btn);

    // Split Vertical / Split Down (D-05)
    let split_v_btn = gtk4::Button::from_icon_name("object-flip-vertical-symbolic");
    split_v_btn.set_tooltip_text(Some("Split Down (Ctrl+Shift+D)"));
    split_v_btn.set_action_name(Some("win.split-down"));
    split_v_btn.add_css_class("headerbar-btn");
    header.pack_end(&split_v_btn);

    // Split Horizontal / Split Right (D-05)
    let split_h_btn = gtk4::Button::from_icon_name("view-dual-symbolic");
    split_h_btn.set_tooltip_text(Some("Split Right (Ctrl+D)"));
    split_h_btn.set_action_name(Some("win.split-right"));
    split_h_btn.add_css_class("headerbar-btn");
    header.pack_end(&split_h_btn);

    Some(header)
}

// ── terminal-actions: window title from the focused pane ────────────────────
//
// Ghostty reports OSC 0/2 titles per surface (SET_TITLE). We remember the last
// title of every surface and show the focused pane's title as the window title
// (which the header bar displays). Switching panes or workspaces moves GTK
// focus to another GLArea, so following the window's focus widget is enough to
// show "that pane's last title".

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Default window title when the focused pane has not set one.
pub const DEFAULT_TITLE: &str = "cmux";

/// Last title reported by each surface, keyed by surface pointer.
#[derive(Default)]
pub struct TitleMap {
    titles: HashMap<usize, String>,
}

impl TitleMap {
    /// Record `title` for `surface`. Returns true if it changed.
    pub fn set(&mut self, surface: usize, title: &str) -> bool {
        if surface == 0 {
            return false;
        }
        match self.titles.get(&surface) {
            Some(old) if old == title => false,
            _ => {
                self.titles.insert(surface, title.to_string());
                true
            }
        }
    }

    pub fn get(&self, surface: usize) -> Option<&str> {
        self.titles.get(&surface).map(String::as_str)
    }

    /// Drop the entry of a closed surface (its pointer may be reused).
    pub fn remove(&mut self, surface: usize) {
        self.titles.remove(&surface);
    }

    /// The title to display when `surface` is focused.
    pub fn display_title(&self, surface: usize) -> &str {
        match self.get(surface) {
            Some(t) if !t.trim().is_empty() => t,
            _ => DEFAULT_TITLE,
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.titles.len()
    }
}

thread_local! {
    static TITLES: RefCell<TitleMap> = RefCell::new(TitleMap::default());
    /// Surface of the terminal pane that most recently had GTK focus.
    static FOCUSED_SURFACE: Cell<usize> = const { Cell::new(0) };
    static TITLE_REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
    static WINDOW: RefCell<Option<glib::WeakRef<gtk4::ApplicationWindow>>> =
        const { RefCell::new(None) };
    static STATE: RefCell<Option<std::rc::Weak<RefCell<crate::app_state::AppState>>>> =
        const { RefCell::new(None) };
}

/// Surface pointer of the terminal GLArea `widget`, if it is one.
pub fn surface_for_widget(widget: &gtk4::Widget) -> Option<usize> {
    if !widget.is::<gtk4::GLArea>() {
        return None;
    }
    let ptr = widget.as_ptr() as usize;
    crate::ghostty::callbacks::GL_TO_SURFACE
        .lock()
        .ok()
        .and_then(|m| m.get(&ptr).copied())
}

fn apply_window_title() {
    let surface = FOCUSED_SURFACE.with(|f| f.get());
    let title = TITLES.with(|t| t.borrow().display_title(surface).to_string());
    let window = WINDOW.with(|w| w.borrow().as_ref().and_then(|w| w.upgrade()));
    if let Some(window) = window {
        if window.title().as_deref() != Some(title.as_str()) {
            eprintln!("cmux: window title -> {title:?} (surface {surface:#x})");
            window.set_title(Some(&title));
        }
    }
}

/// GLArea and surface of the active pane in the active workspace, if that
/// pane is a realized terminal. Uses the engine's active_pane_id: the
/// active-pane CSS class is not set until the first split.
pub fn active_terminal(state: &crate::app_state::AppState) -> Option<(gtk4::Widget, usize)> {
    let engine = state.active_split_engine()?;
    let area: gtk4::Widget = engine.gl_area_for_pane(engine.active_pane_id)?.upcast();
    let surface = surface_for_widget(&area)?;
    Some((area, surface))
}

/// The terminal the user is on: the window's focused GLArea if any (click-to-
/// focus updates active_pane_id only later), else the active pane of the
/// active workspace. `Err(())` if AppState is busy (mutably borrowed), so
/// callers can keep their previous answer instead of guessing.
pub fn current_terminal() -> Result<Option<(gtk4::Widget, usize)>, ()> {
    let window = WINDOW.with(|w| w.borrow().as_ref().and_then(|w| w.upgrade()));
    if let Some(focus) = window.and_then(|w| GtkWindowExt::focus(&w)) {
        if let Some(surface) = surface_for_widget(&focus) {
            return Ok(Some((focus, surface)));
        }
    }
    let Some(state) = STATE.with(|s| s.borrow().as_ref().and_then(|w| w.upgrade())) else {
        return Ok(None);
    };
    let s = state.try_borrow().map_err(|_| ())?;
    Ok(active_terminal(&s))
}

/// Re-resolve the focused pane and show its title, on idle (coalesced).
/// Deferred because SET_TITLE can arrive while a GLArea is being realized
/// mid-frame (retitling then makes GTK snapshot an unallocated header bar),
/// a fresh pane is registered in GL_TO_SURFACE only after it grabs focus, and
/// focus changes happen while AppState is borrowed.
fn schedule_title_refresh() {
    if TITLE_REFRESH_PENDING.with(|p| p.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        TITLE_REFRESH_PENDING.with(|p| p.set(false));
        if let Ok(current) = current_terminal() {
            // No terminal (e.g. a browser pane is active): default title.
            let surface = current.map(|(_, s)| s).unwrap_or(0);
            FOCUSED_SURFACE.with(|f| f.set(surface));
        }
        apply_window_title();
    });
}

/// Track the focused pane so the window title shows its last title. Focus
/// moves on every pane/workspace switch, so the window's focus-widget is the
/// trigger; the pane is resolved by current_terminal() (focus-widget alone
/// misses the initial pane on some setups).
pub fn install_title_tracking(
    window: &gtk4::ApplicationWindow,
    state: &crate::app_state::AppStateRef,
) {
    WINDOW.with(|w| *w.borrow_mut() = Some(window.downgrade()));
    STATE.with(|s| *s.borrow_mut() = Some(std::rc::Rc::downgrade(state)));
    window.connect_notify_local(Some("focus-widget"), |_, _| schedule_title_refresh());
}

/// SET_TITLE from Ghostty (main thread, inside ghostty_app_tick).
pub fn on_set_title(surface: usize, title: &str) {
    let changed = TITLES.with(|t| t.borrow_mut().set(surface, title));
    if changed {
        crate::tabs::on_title_changed(); // tabs: relabel tab bars
        eprintln!("cmux: set_title surface={surface:#x} title={title:?}");
        schedule_title_refresh();
    }
}

/// Last title reported by `surface` (for socket queries).
pub fn title_for_surface(surface: usize) -> Option<String> {
    TITLES.with(|t| t.borrow().get(surface).map(str::to_string))
}

/// The window title currently shown (for socket queries).
pub fn current_window_title() -> Option<String> {
    WINDOW
        .with(|w| w.borrow().as_ref().and_then(|w| w.upgrade()))
        .and_then(|w| w.title())
        .map(|t| t.to_string())
}

/// Forget a surface that is being freed.
pub fn forget_surface(surface: usize) {
    TITLES.with(|t| t.borrow_mut().remove(surface));
    FOCUSED_SURFACE.with(|f| {
        if f.get() == surface {
            f.set(0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_map_bookkeeping() {
        let mut m = TitleMap::default();
        assert_eq!(m.display_title(0x10), DEFAULT_TITLE);
        assert!(m.set(0x10, "vim 한글.txt"));
        assert!(!m.set(0x10, "vim 한글.txt"), "same title is not a change");
        assert!(m.set(0x20, "htop"));
        assert_eq!(m.display_title(0x10), "vim 한글.txt");
        assert_eq!(m.display_title(0x20), "htop");
        // Null surface pointers are never stored.
        assert!(!m.set(0, "x"));
        assert_eq!(m.len(), 2);
        // Closing a surface drops its entry; a reused pointer starts clean.
        m.remove(0x10);
        assert_eq!(m.get(0x10), None);
        assert_eq!(m.display_title(0x10), DEFAULT_TITLE);
        assert_eq!(m.len(), 1);
        // Blank titles fall back to the default.
        assert!(m.set(0x20, "  "));
        assert_eq!(m.display_title(0x20), DEFAULT_TITLE);
    }
}
