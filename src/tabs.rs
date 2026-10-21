//! tabs: terminal tabs inside a split pane (macOS cmux parity).
//!
//! A pane (SplitNode::Leaf) holds 1..N terminals. The Leaf's own fields
//! (pane_id, gl_area, surface, uuid, has_attention) always describe the ACTIVE
//! tab, so every existing split/focus/notification path keeps acting on the
//! terminal the user sees. The other tabs live in `PaneTabs::others`, in tab
//! order with the active tab's slot left out:
//!
//!   tab order = others[..active] ++ [Leaf fields] ++ others[active..]
//!
//! Every tab keeps its own pane_id (unique within the workspace, like split
//! panes), so SURFACE_REGISTRY, notifications and socket lookups by surface
//! UUID resolve to the exact tab. Selecting a tab swaps its entry with the
//! Leaf fields.
//!
//! The pane widget becomes a vertical Box [tab bar, Stack of GLAreas] the
//! first time the pane gets a second tab (or at creation with
//! `[ui] always_show_tabs = true`); a single-tab pane otherwise stays a bare
//! GLArea, exactly as before tabs existed.

use crate::ghostty::ffi;
use gtk4::prelude::*;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

// ── config ───────────────────────────────────────────────────────────────────

static ALWAYS_SHOW_TABS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `[ui] always_show_tabs`: show the tab bar even on single-tab panes.
pub fn set_always_show_tabs(value: bool) {
    ALWAYS_SHOW_TABS.store(value, std::sync::atomic::Ordering::Relaxed);
}

pub fn always_show_tabs() -> bool {
    ALWAYS_SHOW_TABS.load(std::sync::atomic::Ordering::Relaxed)
}

// ── model ────────────────────────────────────────────────────────────────────

/// A terminal tab of a pane that is not its active tab.
#[derive(Clone)]
pub struct TabEntry {
    pub pane_id: u64,
    pub gl_area: gtk4::GLArea,
    pub surface: ffi::ghostty_surface_t,
    pub uuid: Uuid,
    pub has_attention: bool,
}

/// A pane's background tabs and its tab-bar widgets.
#[derive(Clone, Default)]
pub struct PaneTabs {
    /// Tabs other than the active one, in tab order.
    pub others: Vec<TabEntry>,
    /// Position of the active tab (the Leaf's own fields) in tab order.
    pub active: usize,
    /// Tab bar + Stack; None while the pane is a bare GLArea.
    pub ui: Option<PaneTabsUi>,
}

impl PaneTabs {
    pub fn count(&self) -> usize {
        self.others.len() + 1
    }

    /// Tab-order index of `others[i]`.
    pub fn order_index(&self, i: usize) -> usize {
        if i < self.active { i } else { i + 1 }
    }
}

// ── pure index math (unit tested) ────────────────────────────────────────────

/// Next (or previous) tab index, wrapping around.
pub fn cycle_index(current: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}

/// Tab to show when tab `closed` of `count` tabs is closed: the one to its
/// right, else the one to its left (index before the close). None if it was
/// the only tab.
pub fn neighbor_for_close(closed: usize, count: usize) -> Option<usize> {
    if count < 2 || closed >= count {
        None
    } else if closed + 1 < count {
        Some(closed + 1)
    } else {
        Some(closed - 1)
    }
}

/// Make tab-order position `target` the active one. `outgoing` is the current
/// active tab (at `active`); returns the entry that was at `target`, leaving
/// `others` = tab order without `target`.
pub fn swap_in<T>(others: &mut Vec<T>, active: usize, outgoing: T, target: usize) -> T {
    others.insert(active.min(others.len()), outgoing);
    others.remove(target)
}

/// Remove the background tab at tab-order position `order`, keeping `active`
/// pointing at the same tab. Returns None for the active position.
pub fn remove_background<T>(others: &mut Vec<T>, active: &mut usize, order: usize) -> Option<T> {
    let idx = match order.cmp(active) {
        std::cmp::Ordering::Less => order,
        std::cmp::Ordering::Equal => return None,
        std::cmp::Ordering::Greater => order - 1,
    };
    if idx >= others.len() {
        return None;
    }
    if order < *active {
        *active -= 1;
    }
    Some(others.remove(idx))
}

/// Move the tab at order position `from` to `to` (drag-to-reorder).
pub fn move_in_order<T>(others: &mut Vec<T>, active: &mut usize, from: usize, to: usize) -> bool {
    let count = others.len() + 1;
    if from >= count || to >= count || from == to {
        return false;
    }
    let mut all: Vec<Option<T>> = others.drain(..).map(Some).collect();
    all.insert((*active).min(all.len()), None);
    let item = all.remove(from);
    all.insert(to, item);
    *active = all.iter().position(Option::is_none).unwrap_or(0);
    others.extend(all.into_iter().flatten());
    true
}

/// Tab label: the terminal's OSC title, else "Terminal N" (1-based).
pub fn tab_label(title: Option<&str>, index: usize) -> String {
    match title.map(str::trim) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => format!("Terminal {}", index + 1),
    }
}

/// Tab bar visibility.
pub fn bar_visible(count: usize, always: bool) -> bool {
    count >= 2 || always
}

// ── session data ─────────────────────────────────────────────────────────────

/// One tab of a pane in session.json.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct TabData {
    pub surface_uuid: Uuid,
    #[serde(default)]
    pub cwd: String,
}

// ── widgets ──────────────────────────────────────────────────────────────────

/// What one tab button shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TabView {
    /// GLArea pointer: identifies the tab in click/drag requests.
    pub gl_ptr: usize,
    pub title: String,
    pub active: bool,
    pub attention: bool,
}

/// Tab bar on top of a Stack holding the pane's terminals.
#[derive(Clone)]
pub struct PaneTabsUi {
    pub container: gtk4::Box,
    pub bar: gtk4::Box,
    pub stack: gtk4::Stack,
    /// Views last rendered, to skip rebuilding an unchanged bar.
    rendered: Rc<RefCell<Vec<TabView>>>,
}

impl PaneTabsUi {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("cmux-tab-pane");
        container.set_hexpand(true);
        container.set_vexpand(true);
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        bar.add_css_class("cmux-tab-bar");
        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::None);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        // A Stack does not realize its hidden pages, and a terminal's surface
        // (and shell) is only created when its GLArea realizes: realize every
        // tab, so restored background tabs start their shells right away.
        stack.connect_realize(|stack| {
            let mut child = stack.first_child();
            while let Some(widget) = child {
                if !widget.is_realized() {
                    widget.realize();
                    // create_surface's realize marks every new surface focused;
                    // a background tab is not.
                    let hidden = stack.visible_child().as_ref() != Some(&widget);
                    let surface = widget
                        .downcast_ref::<gtk4::GLArea>()
                        .and_then(|area| live_surface(area, std::ptr::null_mut()));
                    if let (true, Some(surface)) = (hidden, surface) {
                        unsafe {
                            ffi::ghostty_surface_set_focus(surface, false);
                        }
                    }
                }
                child = widget.next_sibling();
            }
        });
        container.append(&bar);
        container.append(&stack);
        PaneTabsUi { container, bar, stack, rendered: Rc::new(RefCell::new(Vec::new())) }
    }

    /// Show `views` (tab order) in the bar.
    pub fn render(&self, views: &[TabView]) {
        self.bar.set_visible(bar_visible(views.len(), always_show_tabs()));
        if *self.rendered.borrow() == views {
            return;
        }
        while let Some(child) = self.bar.first_child() {
            self.bar.remove(&child);
        }
        for view in views {
            self.bar.append(&build_tab_item(view));
        }
        *self.rendered.borrow_mut() = views.to_vec();
    }
}

fn build_tab_item(view: &TabView) -> gtk4::Box {
    let item = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    item.add_css_class("cmux-tab");
    if view.active {
        item.add_css_class("active");
    }
    item.set_tooltip_text(Some(&view.title));

    let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    dot.add_css_class("cmux-tab-dot");
    dot.set_visible(view.attention);
    dot.set_valign(gtk4::Align::Center);
    item.append(&dot);

    let label = gtk4::Label::new(Some(&view.title));
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_max_width_chars(24);
    label.set_hexpand(false);
    label.set_xalign(0.0);
    item.append(&label);

    let close = gtk4::Button::with_label("×");
    close.add_css_class("cmux-tab-close");
    close.set_focusable(false);
    close.set_focus_on_click(false);
    close.set_tooltip_text(Some("Close Tab"));
    let ptr = view.gl_ptr;
    close.connect_clicked(move |_| request(TabRequest::Close(ptr)));
    item.append(&close);

    // Left click selects, middle click closes. On release: selecting rebuilds
    // the bar, which would cancel a drag started on this tab.
    let click = gtk4::GestureClick::new();
    click.set_button(0);
    click.connect_released(move |gesture, _, _, _| match gesture.current_button() {
        1 => request(TabRequest::Select(ptr)),
        2 => request(TabRequest::Close(ptr)),
        _ => {}
    });
    item.add_controller(click);

    // Drag a tab onto another tab of the same pane to reorder.
    let drag = gtk4::DragSource::new();
    drag.set_actions(gtk4::gdk::DragAction::MOVE);
    drag.connect_prepare(move |_, _, _| {
        Some(gtk4::gdk::ContentProvider::for_value(&drag_payload(ptr).to_value()))
    });
    item.add_controller(drag);
    let drop = gtk4::DropTarget::new(glib::Type::STRING, gtk4::gdk::DragAction::MOVE);
    drop.connect_drop(move |_, value, _, _| {
        match value.get::<String>().ok().and_then(|s| parse_drag_payload(&s)) {
            Some(from) if from != ptr => {
                request(TabRequest::Move { from, to: ptr });
                true
            }
            _ => false,
        }
    });
    item.add_controller(drop);
    item
}

const DRAG_PREFIX: &str = "cmux-tab:";

fn drag_payload(gl_ptr: usize) -> String {
    format!("{DRAG_PREFIX}{gl_ptr}")
}

fn parse_drag_payload(s: &str) -> Option<usize> {
    s.strip_prefix(DRAG_PREFIX)?.parse().ok().filter(|&p| p != 0)
}

// ── UI requests (tab bar clicks/drags) ──────────────────────────────────────

thread_local! {
    static STATE: RefCell<Option<std::rc::Weak<RefCell<crate::app_state::AppState>>>> =
        const { RefCell::new(None) };
    static REFRESH_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Give the tab bars access to AppState (called once from build_ui).
pub fn install(state: &crate::app_state::AppStateRef) {
    STATE.with(|s| *s.borrow_mut() = Some(Rc::downgrade(state)));
}

fn app_state() -> Option<crate::app_state::AppStateRef> {
    STATE.with(|s| s.borrow().as_ref().and_then(|w| w.upgrade()))
}

/// A tab bar action, identified by the tab's GLArea pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TabRequest {
    Select(usize),
    Close(usize),
    Move { from: usize, to: usize },
}

/// Run a tab bar action on idle: it rebuilds the very bar whose gesture fired.
fn request(req: TabRequest) {
    glib::idle_add_local_once(move || handle_request(req));
}

fn handle_request(req: TabRequest) {
    let Some(state) = app_state() else { return };
    let gl_ptr = match req {
        TabRequest::Select(p) | TabRequest::Close(p) => p,
        TabRequest::Move { from, .. } => from,
    };
    let Ok(s) = state.try_borrow() else { return };
    let Some((ws_idx, tab_id)) = s
        .split_engines
        .iter()
        .enumerate()
        .find_map(|(i, e)| e.terminal_for_gl_area(gl_ptr).map(|id| (i, id)))
    else {
        return;
    };
    let app = s.gtk_app.clone();
    drop(s);
    match req {
        TabRequest::Select(_) => {
            with_attention_sync(&state, ws_idx, |engine| {
                engine.focus_pane(tab_id);
            });
        }
        TabRequest::Close(_) => {
            let closed_in_background = state
                .borrow_mut()
                .split_engines
                .get_mut(ws_idx)
                .map(|e| e.is_background_tab(tab_id) && e.close_tab(tab_id))
                .unwrap_or(false);
            if !closed_in_background {
                // The pane's visible tab: same as Ctrl+Shift+X on it (closes
                // the pane, or asks to close the workspace, on its last tab).
                if let Some(engine) = state.borrow_mut().split_engines.get_mut(ws_idx) {
                    engine.focus_pane(tab_id);
                }
                crate::shortcuts::handle_close_pane(&state, &app);
            }
        }
        TabRequest::Move { to, .. } => {
            let mut s = state.borrow_mut();
            let target = s.split_engines.get(ws_idx).and_then(|e| e.terminal_for_gl_area(to));
            if let (Some(engine), Some(to_id)) = (s.split_engines.get_mut(ws_idx), target) {
                let (from_loc, to_loc) = (engine.tab_location(tab_id), engine.tab_location(to_id));
                // Only within one pane: a drop on another pane's bar is ignored.
                if let (Some((from_leaf, _, _)), Some((to_leaf, index, _))) = (from_loc, to_loc) {
                    if from_leaf == to_leaf {
                        engine.move_tab(tab_id, index);
                    }
                }
            }
        }
    }
    save_session(&state);
}

fn save_session(state: &crate::app_state::AppStateRef) {
    if let Ok(s) = state.try_borrow() {
        s.trigger_session_save();
    }
}

/// A terminal's OSC title changed: relabel tab bars on idle (coalesced; this
/// runs inside ghostty_app_tick, where AppState may be borrowed).
pub fn on_title_changed() {
    if REFRESH_PENDING.with(|p| p.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        REFRESH_PENDING.with(|p| p.set(false));
        if let Some(state) = app_state() {
            if let Ok(s) = state.try_borrow() {
                for engine in &s.split_engines {
                    engine.refresh_tab_bars();
                }
            }
        }
    });
}

/// Surface pointer of a terminal GLArea, through GL_TO_SURFACE (leaf surface
/// fields are often still null).
pub fn live_surface(gl_area: &gtk4::GLArea, stored: ffi::ghostty_surface_t) -> Option<ffi::ghostty_surface_t> {
    let ptr = gl_area.as_ptr() as usize;
    crate::ghostty::callbacks::GL_TO_SURFACE
        .lock()
        .ok()
        .and_then(|m| m.get(&ptr).copied())
        .map(|s| s as ffi::ghostty_surface_t)
        .or_else(|| (!stored.is_null()).then_some(stored))
}

/// Label for a tab's terminal.
pub fn title_for(gl_area: &gtk4::GLArea, index: usize) -> String {
    let title = live_surface(gl_area, std::ptr::null_mut())
        .and_then(|s| crate::header_bar::title_for_surface(s as usize));
    tab_label(title.as_deref(), index)
}

// ── keyboard shortcuts ──────────────────────────────────────────────────────

/// New tab in the focused pane (Ctrl+Shift+T), in that terminal's cwd.
pub fn handle_new_tab(state: &crate::app_state::AppStateRef) -> Option<u64> {
    let id = state.borrow_mut().active_split_engine_mut()?.new_tab();
    save_session(state);
    id
}

/// True if the focused pane has more than one tab. Tab shortcuts are left to
/// the terminal otherwise (Alt+digits, Ctrl+Tab, Ctrl+PageUp/Down are app keys).
pub fn active_pane_has_tabs(state: &crate::app_state::AppStateRef) -> bool {
    state
        .try_borrow()
        .ok()
        .and_then(|s| {
            let engine = s.active_split_engine()?;
            engine.tab_location(engine.active_pane_id)
        })
        .is_some_and(|(_, _, count)| count > 1)
}

/// Run a tab switch on workspace `ws_idx`; if it cleared the last tab
/// attention there, clear the workspace's sidebar dot too.
fn with_attention_sync<R>(
    state: &crate::app_state::AppStateRef,
    ws_idx: usize,
    f: impl FnOnce(&mut crate::split_engine::SplitEngine) -> R,
) -> Option<R> {
    let mut s = state.borrow_mut();
    let engine = s.split_engines.get_mut(ws_idx)?;
    let before = engine.root.any_attention();
    let result = f(engine);
    let after = engine.root.any_attention();
    if before && !after {
        s.clear_workspace_attention(ws_idx);
    }
    Some(result)
}

/// Next/previous tab of the focused pane (Ctrl+Tab / Ctrl+Shift+Tab).
pub fn handle_cycle_tab(state: &crate::app_state::AppStateRef, forward: bool) {
    let idx = state.borrow().active_index;
    if with_attention_sync(state, idx, |e| e.cycle_tab(forward)).unwrap_or(false) {
        save_session(state);
    }
}

/// Go to tab `n` (1-based) of the focused pane (Alt+1..9).
pub fn handle_goto_tab(state: &crate::app_state::AppStateRef, n: usize) {
    if n == 0 {
        return;
    }
    let idx = state.borrow().active_index;
    if with_attention_sync(state, idx, |e| e.select_tab(n - 1)).unwrap_or(false) {
        save_session(state);
    }
}

/// After a tab.* request touched the background workspace `ws_idx`: that
/// workspace's terminal must not keep Ghostty/GTK focus; give it back to the
/// visible workspace (SOCK-05).
fn restore_visible_focus(s: &mut crate::app_state::AppState, ws_idx: usize) {
    if ws_idx == s.active_index {
        return;
    }
    if let Some(engine) = s.split_engines.get(ws_idx) {
        if let Some(surface) = engine.find_surface(engine.active_pane_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(surface, false);
            }
        }
    }
    if let Some(page) = s.workspaces.get(s.active_index).map(|ws| ws.stack_page_name.clone()) {
        s.stack.set_visible_child_name(&page);
    }
    if let Some(engine) = s.split_engines.get(s.active_index) {
        engine.focus_active_surface();
    }
}

/// Tab number (1-9) for a GotoTabN action.
pub fn goto_tab_number(action: crate::config::ShortcutAction) -> Option<usize> {
    use crate::config::ShortcutAction as A;
    Some(match action {
        A::GotoTab1 => 1,
        A::GotoTab2 => 2,
        A::GotoTab3 => 3,
        A::GotoTab4 => 4,
        A::GotoTab5 => 5,
        A::GotoTab6 => 6,
        A::GotoTab7 => 7,
        A::GotoTab8 => 8,
        A::GotoTab9 => 9,
        _ => return None,
    })
}

// ── socket: tab.* ────────────────────────────────────────────────────────────

/// Error code + message for a tab.* request.
type SocketError = (&'static str, String);

/// Find the workspace + terminal for a surface UUID (any workspace), or the
/// focused terminal of the active workspace when `surface` is None.
fn resolve_terminal(
    s: &crate::app_state::AppState,
    surface: Option<&str>,
) -> Result<(usize, u64), SocketError> {
    match surface {
        Some(id) => s
            .split_engines
            .iter()
            .enumerate()
            .find_map(|(i, e)| e.find_pane_id_by_uuid(id).map(|p| (i, p)))
            .ok_or(("not_found", format!("surface {id} not found"))),
        None => s
            .split_engines
            .get(s.active_index)
            .map(|e| (s.active_index, e.active_pane_id))
            .ok_or(("not_found", "no active workspace".to_string())),
    }
}

/// JSON for every tab of the pane holding `tab_id`.
fn tab_list_json(s: &crate::app_state::AppState, ws_idx: usize, tab_id: u64) -> Value {
    let Some(engine) = s.split_engines.get(ws_idx) else {
        return json!({"tabs": []});
    };
    let focused_pane = ws_idx == s.active_index;
    let tabs = engine.pane_tabs(tab_id);
    let pane = tabs.iter().find(|t| t.active).map(|t| t.uuid.to_string());
    let list: Vec<Value> = tabs
        .iter()
        .map(|t| {
            json!({
                "index": t.index,
                "surface_id": t.uuid.to_string(),
                "title": t.title,
                "active": t.active,
                "focused": t.active && focused_pane && t.pane_active,
                "has_attention": t.has_attention,
            })
        })
        .collect();
    json!({
        "pane_id": pane,
        "workspace_uuid": s.workspaces.get(ws_idx).map(|w| w.uuid.to_string()),
        "tab_count": list.len(),
        "tabs": list,
    })
}

fn str_param<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| params.get(*k).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
}

/// Handle a `tab.*` socket method on the GTK main thread.
/// Focus policy (SOCK-05): tab.new/select/next/prev change the visible tab and
/// focus its pane (focus-intent, like surface.focus); tab.close/move/list do
/// not move focus to another pane.
pub fn handle_socket(
    state: &crate::app_state::AppStateRef,
    method: &str,
    params: &Value,
) -> Result<Value, SocketError> {
    let surface = str_param(params, &["surface_id", "surface", "id"]);
    match method {
        "tab.list" => {
            let s = state.borrow();
            let target = str_param(params, &["pane", "pane_id"]).or(surface);
            let (ws_idx, tab_id) = resolve_terminal(&s, target)?;
            Ok(tab_list_json(&s, ws_idx, tab_id))
        }
        "tab.new" => {
            let (ws_idx, tab_id) = resolve_terminal(&state.borrow(), surface)?;
            let new_id = {
                let mut s = state.borrow_mut();
                let engine = s.split_engines.get_mut(ws_idx).ok_or(("not_found", "workspace gone".to_string()))?;
                engine.focus_pane(tab_id);
                let id = engine.new_tab();
                restore_visible_focus(&mut s, ws_idx);
                id.ok_or(("new_tab_failed", "the pane is not a terminal".to_string()))?
            };
            save_session(state);
            let s = state.borrow();
            let uuid = s.split_engines[ws_idx].find_uuid_for_pane(new_id);
            let mut out = tab_list_json(&s, ws_idx, new_id);
            out["surface_id"] = json!(uuid);
            Ok(out)
        }
        "tab.close" => {
            let (ws_idx, tab_id) = resolve_terminal(&state.borrow(), surface)?;
            let result = {
                let mut s = state.borrow_mut();
                let engine = s.split_engines.get_mut(ws_idx).ok_or(("not_found", "workspace gone".to_string()))?;
                let orig = engine.active_pane_id;
                let result = if engine.close_tab(tab_id) {
                    Ok(())
                } else if engine.tab_location(tab_id).is_some_and(|(leaf, _, _)| leaf == orig) {
                    // Last tab of the focused pane: close the pane.
                    engine.close_active().map(|_| ()).ok_or(())
                } else {
                    // Last tab of another pane: close that pane, then put the
                    // user's focus back (as surface.close does).
                    engine.focus_pane(tab_id);
                    let r = engine.close_active().map(|_| ()).ok_or(());
                    if engine.tab_location(orig).is_some() {
                        engine.focus_pane(orig);
                    }
                    r
                };
                restore_visible_focus(&mut s, ws_idx);
                result.map_err(|_| ("close_failed", "cannot close the last pane of a workspace".to_string()))
            };
            result?;
            save_session(state);
            Ok(json!({"closed": surface.map(str::to_string)}))
        }
        "tab.select" | "tab.next" | "tab.prev" | "tab.previous" => {
            let (ws_idx, tab_id) = resolve_terminal(&state.borrow(), surface)?;
            {
                let mut s = state.borrow_mut();
                if ws_idx != s.active_index {
                    s.switch_to_index(ws_idx);
                }
                let engine = s.split_engines.get_mut(ws_idx).ok_or(("not_found", "workspace gone".to_string()))?;
                let attention_before = engine.root.any_attention();
                engine.focus_pane(tab_id);
                let ok = match method {
                    "tab.select" => match params.get("index").and_then(|v| v.as_u64()) {
                        Some(i) => engine.select_tab(i as usize),
                        // surface_id alone: focus_pane above brought it to the front.
                        None if surface.is_some() => true,
                        None => return Err(("invalid_params", "surface_id or index is required".to_string())),
                    },
                    "tab.next" => {
                        engine.cycle_tab(true);
                        true
                    }
                    _ => {
                        engine.cycle_tab(false);
                        true
                    }
                };
                if attention_before && !engine.root.any_attention() {
                    s.clear_workspace_attention(ws_idx);
                }
                if !ok {
                    return Err(("not_found", "no tab at that index".to_string()));
                }
            }
            save_session(state);
            let s = state.borrow();
            let active = s.split_engines[ws_idx].active_pane_id;
            Ok(tab_list_json(&s, ws_idx, active))
        }
        "tab.move" => {
            let Some(index) = params.get("index").and_then(|v| v.as_u64()) else {
                return Err(("invalid_params", "index is required".to_string()));
            };
            let (ws_idx, tab_id) = resolve_terminal(&state.borrow(), surface)?;
            let moved = state
                .borrow_mut()
                .split_engines
                .get_mut(ws_idx)
                .map(|e| e.move_tab(tab_id, index as usize))
                .unwrap_or(false);
            if !moved {
                return Err(("invalid_params", "index out of range".to_string()));
            }
            save_session(state);
            Ok(tab_list_json(&state.borrow(), ws_idx, tab_id))
        }
        _ => Err(("not_implemented", format!("{method} is not implemented"))),
    }
}

/// Socket methods handled here (for system.capabilities).
pub const SOCKET_METHODS: &[&str] = &[
    "tab.new", "tab.close", "tab.select", "tab.next", "tab.prev", "tab.list", "tab.move",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Tab order as seen by the user: others with the active tab at `active`.
    fn order(others: &[char], active: usize, active_tab: char) -> String {
        let mut all: Vec<char> = others.to_vec();
        all.insert(active, active_tab);
        all.into_iter().collect()
    }

    #[test]
    fn cycle_wraps_both_ways() {
        assert_eq!(cycle_index(0, 3, true), 1);
        assert_eq!(cycle_index(2, 3, true), 0);
        assert_eq!(cycle_index(0, 3, false), 2);
        assert_eq!(cycle_index(1, 3, false), 0);
        assert_eq!(cycle_index(0, 1, true), 0);
        assert_eq!(cycle_index(0, 0, true), 0);
    }

    #[test]
    fn close_picks_right_then_left_neighbor() {
        assert_eq!(neighbor_for_close(0, 3), Some(1));
        assert_eq!(neighbor_for_close(1, 3), Some(2));
        assert_eq!(neighbor_for_close(2, 3), Some(1));
        assert_eq!(neighbor_for_close(0, 1), None);
        assert_eq!(neighbor_for_close(5, 3), None);
    }

    #[test]
    fn swap_in_selects_any_position() {
        // Tabs A B [C] D: select A, then D.
        let mut others = vec!['A', 'B', 'D'];
        let incoming = swap_in(&mut others, 2, 'C', 0);
        assert_eq!(incoming, 'A');
        assert_eq!(order(&others, 0, incoming), "ABCD");
        let incoming = swap_in(&mut others, 0, 'A', 3);
        assert_eq!(incoming, 'D');
        assert_eq!(order(&others, 3, incoming), "ABCD");
        // Selecting the active position is a no-op round trip.
        let incoming = swap_in(&mut others, 3, 'D', 3);
        assert_eq!(incoming, 'D');
        assert_eq!(others, vec!['A', 'B', 'C']);
    }

    #[test]
    fn new_tab_opens_right_of_the_active_one() {
        // Tabs A [B] C: a new tab N goes between B and C and becomes active.
        let mut others = vec!['A', 'C'];
        let mut active = 1;
        others.insert(active, 'B');
        active += 1;
        assert_eq!(order(&others, active, 'N'), "ABNC");
    }

    #[test]
    fn closing_the_middle_tab_keeps_indices_consistent() {
        // A [B] C: closing B shows C (right neighbor), leaving A [C].
        let mut others = vec!['A', 'C'];
        let mut active = 1;
        let next = neighbor_for_close(active, others.len() + 1).unwrap();
        let incoming = swap_in(&mut others, active, 'B', next);
        active = next;
        assert_eq!(incoming, 'C');
        let closed = remove_background(&mut others, &mut active, 1).unwrap();
        assert_eq!(closed, 'B');
        assert_eq!(order(&others, active, incoming), "AC");
        assert_eq!(active, 1);
        // Closing a background tab left of the active one shifts it left.
        let mut others = vec!['A', 'B', 'D'];
        let mut active = 3; // A B D [E]
        assert_eq!(remove_background(&mut others, &mut active, 1), Some('B'));
        assert_eq!(order(&others, active, 'E'), "ADE");
        assert_eq!(active, 2);
        // Right of it: active unchanged. The active position itself: refused.
        let mut others = vec!['B', 'C'];
        let mut active = 0; // [A] B C
        assert_eq!(remove_background(&mut others, &mut active, 2), Some('C'));
        assert_eq!(active, 0);
        assert_eq!(remove_background(&mut others, &mut active, 0), None);
        assert_eq!(remove_background(&mut others, &mut active, 9), None);
    }

    #[test]
    fn move_reorders_and_tracks_the_active_tab() {
        // A [B] C D: drag A to the end.
        let mut others = vec!['A', 'C', 'D'];
        let mut active = 1;
        assert!(move_in_order(&mut others, &mut active, 0, 3));
        assert_eq!(order(&others, active, 'B'), "BCDA");
        assert_eq!(active, 0);
        // Drag the active tab itself.
        assert!(move_in_order(&mut others, &mut active, 0, 2));
        assert_eq!(order(&others, active, 'B'), "CDBA");
        assert_eq!(active, 2);
        // Out of range / no-op moves are refused.
        assert!(!move_in_order(&mut others, &mut active, 0, 4));
        assert!(!move_in_order(&mut others, &mut active, 1, 1));
        assert_eq!(order(&others, active, 'B'), "CDBA");
    }

    #[test]
    fn labels_fall_back_to_terminal_n() {
        assert_eq!(tab_label(Some("vim 한글.txt"), 0), "vim 한글.txt");
        assert_eq!(tab_label(Some("   "), 1), "Terminal 2");
        assert_eq!(tab_label(None, 2), "Terminal 3");
    }

    #[test]
    fn bar_shows_for_two_tabs_or_always() {
        assert!(!bar_visible(1, false));
        assert!(bar_visible(1, true));
        assert!(bar_visible(2, false));
    }

    #[test]
    fn drag_payload_round_trips() {
        assert_eq!(parse_drag_payload(&drag_payload(0x5555_1234)), Some(0x5555_1234));
        assert_eq!(parse_drag_payload("cmux-tab:0"), None);
        assert_eq!(parse_drag_payload("other:12"), None);
        assert_eq!(parse_drag_payload("cmux-tab:abc"), None);
    }
}
