//! Developer tools panel for the browser preview pane (Console / Network /
//! Elements), toggled by the `{ }` nav button or F12 while the preview has focus.
//!
//! Data comes from the agent-browser daemon actions that also back the CLI
//! (`console`, `errors`, `requests`, `evaluate`, `snapshot`, `inspect`,
//! `reload`). Polling runs every 500ms on the tokio blocking pool; results are
//! applied on the GTK main thread, so the UI never waits on the daemon socket.
//!
//! Clearing is client-side (entries before a watermark are hidden), so the
//! CLI keeps seeing the daemon's full buffers.

use gtk4::prelude::*;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use crate::app_state::AppState;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Console rows kept in the panel; older rows are dropped first.
const MAX_CONSOLE_ROWS: usize = 2000;
const DEFAULT_PANEL_HEIGHT: i32 = 260;

/// Which tab of the panel is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Console,
    Network,
    Elements,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Console => "console",
            Tab::Network => "network",
            Tab::Elements => "elements",
        }
    }
}

/// Mutable bookkeeping for one panel (never borrowed across an await).
#[derive(Default)]
struct PollState {
    in_flight: bool,
    /// Daemon's monotonic console/error totals already shown (or skipped).
    seen_console: u64,
    seen_errors: u64,
    /// Requests before this index were cleared by the user.
    net_offset: usize,
    /// Length of the daemon's request list at the last poll.
    net_len: usize,
    /// Errors that arrived while the panel was closed.
    unread_errors: u32,
    /// Paned position the user last chose for the page area.
    paned_pos: Option<i32>,
    history: Vec<String>,
    history_pos: Option<usize>,
}

struct NetRow {
    row: gtk4::ListBoxRow,
    status: gtk4::Label,
    kind: gtk4::Label,
    size: gtk4::Label,
    time: gtk4::Label,
}

struct Ui {
    btn: gtk4::ToggleButton,
    badge: gtk4::Label,
    paned: gtk4::Paned,
    panel: gtk4::Box,
    stack: gtk4::Stack,
    tab_btns: [gtk4::ToggleButton; 3],
    clear_btn: gtk4::Button,
    refresh_btn: gtk4::Button,
    status: gtk4::Label,
    console_list: gtk4::ListBox,
    console_scroll: gtk4::ScrolledWindow,
    eval_entry: gtk4::Entry,
    net_list: gtk4::ListBox,
    net_scroll: gtk4::ScrolledWindow,
    net_summary: gtk4::Label,
    net_rows: RefCell<HashMap<String, NetRow>>,
    elements_view: gtk4::TextView,
    socket: PathBuf,
    runtime: tokio::runtime::Handle,
    st: RefCell<PollState>,
}

thread_local! {
    /// Live panels, newest last; used by the `win.browser-devtools` action.
    static PANELS: RefCell<Vec<Weak<Ui>>> = const { RefCell::new(Vec::new()) };
}

// ── Daemon access (runs off the GTK thread) ─────────────────────────────────

/// One request/response round trip on the daemon socket, with a timeout.
/// Returns the response `data` on success.
fn daemon_request(socket: &Path, action: &str, mut params: Value, timeout: Duration) -> Result<Value, String> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| format!("daemon not reachable: {e}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if let Some(obj) = params.as_object_mut() {
        obj.insert("id".into(), json!(format!("devtools-{action}")));
        obj.insert("action".into(), json!(action));
    }
    let mut line = serde_json::to_string(&params).map_err(|e| e.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).map_err(|e| format!("{action}: {e}"))?;
    let v: Value = serde_json::from_str(&resp).map_err(|e| format!("{action}: bad response: {e}"))?;
    if v.get("success").and_then(|s| s.as_bool()) == Some(false) {
        let msg = v
            .get("error")
            .map(|e| e.as_str().map(String::from).unwrap_or_else(|| e.to_string()))
            .unwrap_or_else(|| "unknown error".into());
        return Err(msg);
    }
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

struct PollResult {
    console: Option<Value>,
    errors: Option<Value>,
    requests: Option<Value>,
}

fn poll_daemon(socket: &Path, want_requests: bool) -> PollResult {
    let t = Duration::from_secs(2);
    PollResult {
        console: daemon_request(socket, "console", json!({}), t).ok(),
        errors: daemon_request(socket, "errors", json!({}), t).ok(),
        // `requests` also switches the daemon's request tracking on.
        requests: if want_requests {
            daemon_request(socket, "requests", json!({}), t).ok()
        } else {
            None
        },
    }
}

/// Run `work` on the tokio blocking pool and hand its result to `done` on the
/// GTK main thread.
fn run_off_main<T, W, D>(ui: &Rc<Ui>, work: W, done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(&Rc<Ui>, T) + 'static,
{
    let handle = ui.runtime.spawn_blocking(work);
    let weak = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        if let Ok(result) = handle.await {
            if let Some(ui) = weak.upgrade() {
                done(&ui, result);
            }
        }
    });
}

// ── Formatting helpers (pure, unit-tested) ──────────────────────────────────

/// CSS level class and short tag for a console message type.
fn level_of(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "error" | "assert" => ("error", "ERR"),
        "warning" | "warn" => ("warn", "WARN"),
        "info" => ("info", "INFO"),
        "debug" | "trace" => ("debug", "DBG"),
        _ => ("log", "LOG"),
    }
}

/// "file.js:12:5" for an exception (CDP lines/columns are 0-based).
fn error_location(err: &Value) -> String {
    let url = err.get("url").and_then(|u| u.as_str()).unwrap_or("");
    let line = err.get("line").and_then(|l| l.as_i64());
    let col = err.get("column").and_then(|c| c.as_i64());
    let file = if url.starts_with("data:") {
        "(data url)".to_string()
    } else {
        url.rsplit('/').next().unwrap_or(url).to_string()
    };
    match (file.is_empty(), line, col) {
        (true, Some(l), Some(c)) => format!("{}:{}", l + 1, c + 1),
        (false, Some(l), Some(c)) => format!("{file}:{}:{}", l + 1, c + 1),
        (false, Some(l), None) => format!("{file}:{}", l + 1),
        (false, None, _) => file,
        _ => String::new(),
    }
}

fn human_size(bytes: i64) -> String {
    if bytes < 0 {
        return String::new();
    }
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} kB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    }
}

/// (status text, failed?) for a tracked request.
fn request_status(req: &Value) -> (String, bool) {
    if let Some(err) = req.get("errorText").and_then(|e| e.as_str()) {
        if err == "canceled" {
            return ("(canceled)".to_string(), false);
        }
        return ("(failed)".to_string(), true);
    }
    match req.get("status").and_then(|s| s.as_i64()) {
        Some(code) => (code.to_string(), code >= 400),
        None => ("…".to_string(), false),
    }
}

/// Render an `evaluate` result like the DevTools console does.
fn format_eval_result(v: &Value) -> String {
    match v.get("result") {
        Some(Value::String(s)) => format!("{s:?}"),
        Some(Value::Null) | None => "undefined".to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

// ── UI construction ────────────────────────────────────────────────────────

fn label(text: &str, class: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l
}

fn tool_btn(text: &str, tooltip: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(text);
    b.add_css_class("devtools-tool-btn");
    b.set_tooltip_text(Some(tooltip));
    b
}

/// Attach the devtools panel to a freshly created preview pane.
pub fn attach(state: &Rc<RefCell<AppState>>, widgets: &crate::browser::PreviewPaneWidgets) {
    let (socket, runtime) = {
        let s = state.borrow();
        let socket = s.browser_manager.as_ref().map(|bm| bm.daemon_socket_path());
        (socket, s.runtime_handle.clone())
    };
    let (Some(socket), Some(runtime)) = (socket, runtime) else {
        eprintln!("cmux: devtools: no browser session/runtime; panel disabled");
        return;
    };

    // `{ }` button gets a badge for errors that arrive while the panel is closed.
    let badge = gtk4::Label::new(None);
    badge.add_css_class("devtools-badge");
    badge.set_visible(false);
    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    btn_box.append(&gtk4::Label::new(Some("{ }")));
    btn_box.append(&badge);
    widgets.devtools_btn.set_child(Some(&btn_box));
    widgets.devtools_btn.set_tooltip_text(Some("Developer Tools (F12)"));

    // Toolbar: tabs on the left, actions on the right.
    let tab_console = gtk4::ToggleButton::with_label("Console");
    let tab_network = gtk4::ToggleButton::with_label("Network");
    let tab_elements = gtk4::ToggleButton::with_label("Elements");
    tab_network.set_group(Some(&tab_console));
    tab_elements.set_group(Some(&tab_console));
    tab_console.set_active(true);
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    toolbar.add_css_class("devtools-toolbar");
    for t in [&tab_console, &tab_network, &tab_elements] {
        t.add_css_class("devtools-tab");
        toolbar.append(t);
    }
    let status = label("", "devtools-status");
    status.set_hexpand(true);
    status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    status.set_margin_start(8);
    toolbar.append(&status);
    let clear_btn = tool_btn("Clear", "Clear the list");
    let refresh_btn = tool_btn("Refresh", "Take a new DOM snapshot");
    refresh_btn.set_visible(false);
    let hard_reload_btn = tool_btn("\u{21BB} Hard", "Reload bypassing the cache (Ctrl+Shift+R)");
    let chrome_btn = tool_btn("DevTools \u{2197}", "Open the full Chrome DevTools for this page in your browser");
    for b in [&clear_btn, &refresh_btn, &hard_reload_btn, &chrome_btn] {
        toolbar.append(b);
    }

    // Console tab: message list + JS input line.
    let console_list = gtk4::ListBox::new();
    console_list.set_selection_mode(gtk4::SelectionMode::None);
    console_list.add_css_class("devtools-list");
    let console_scroll = gtk4::ScrolledWindow::new();
    console_scroll.set_child(Some(&console_list));
    console_scroll.set_vexpand(true);
    let eval_entry = gtk4::Entry::new();
    eval_entry.add_css_class("devtools-eval");
    eval_entry.set_placeholder_text(Some("\u{203A} Evaluate JavaScript in the page (Enter)"));
    let console_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    console_box.append(&console_scroll);
    console_box.append(&eval_entry);

    // Network tab: header + request list + summary.
    let net_header = net_row_box(["Method", "URL", "Status", "Type", "Size", "Time"]);
    net_header.0.add_css_class("devtools-net-header");
    let net_list = gtk4::ListBox::new();
    net_list.set_selection_mode(gtk4::SelectionMode::None);
    net_list.add_css_class("devtools-list");
    // Header scrolls with the rows so a narrow pane scrolls sideways instead
    // of being forced wider by the columns.
    let net_table = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    net_table.append(&net_header.0);
    net_table.append(&net_list);
    let net_scroll = gtk4::ScrolledWindow::new();
    net_scroll.set_child(Some(&net_table));
    net_scroll.set_vexpand(true);
    let net_summary = label("", "devtools-status");
    net_summary.set_margin_start(8);
    let net_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    net_box.append(&net_scroll);
    net_box.append(&net_summary);

    // Elements tab: accessibility/DOM snapshot text.
    let elements_view = gtk4::TextView::new();
    elements_view.set_editable(false);
    elements_view.set_monospace(true);
    elements_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    elements_view.add_css_class("devtools-elements");
    let elements_scroll = gtk4::ScrolledWindow::new();
    elements_scroll.set_child(Some(&elements_view));
    elements_scroll.set_vexpand(true);

    let stack = gtk4::Stack::new();
    stack.add_named(&console_box, Some(Tab::Console.name()));
    stack.add_named(&net_box, Some(Tab::Network.name()));
    stack.add_named(&elements_scroll, Some(Tab::Elements.name()));
    stack.set_vexpand(true);

    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.add_css_class("devtools-panel");
    panel.set_size_request(-1, 120);
    // Let the toolbar scroll instead of forcing a minimum pane width.
    let toolbar_scroll = gtk4::ScrolledWindow::new();
    toolbar_scroll.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::Never);
    toolbar_scroll.set_child(Some(&toolbar));
    panel.append(&toolbar_scroll);
    panel.append(&stack);

    let ui = Rc::new(Ui {
        btn: widgets.devtools_btn.clone(),
        badge,
        paned: widgets.devtools_paned.clone(),
        panel,
        stack,
        tab_btns: [tab_console.clone(), tab_network.clone(), tab_elements.clone()],
        clear_btn: clear_btn.clone(),
        refresh_btn: refresh_btn.clone(),
        status,
        console_list,
        console_scroll,
        eval_entry: eval_entry.clone(),
        net_list,
        net_scroll,
        net_summary,
        net_rows: RefCell::new(HashMap::new()),
        elements_view,
        socket,
        runtime,
        st: RefCell::new(PollState::default()),
    });
    PANELS.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|w| w.strong_count() > 0);
        p.push(Rc::downgrade(&ui));
    });

    // Signals (closures hold Weak<Ui> so a closed pane frees everything).
    let weak = Rc::downgrade(&ui);
    widgets.devtools_btn.connect_toggled(move |btn| {
        if let Some(ui) = weak.upgrade() {
            set_panel_open(&ui, btn.is_active());
        }
    });
    for (tab, b) in [(Tab::Console, &tab_console), (Tab::Network, &tab_network), (Tab::Elements, &tab_elements)] {
        let weak = Rc::downgrade(&ui);
        b.connect_toggled(move |b| {
            if b.is_active() {
                if let Some(ui) = weak.upgrade() {
                    show_tab(&ui, tab);
                }
            }
        });
    }
    let weak = Rc::downgrade(&ui);
    clear_btn.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            clear_current(&ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    refresh_btn.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            refresh_elements(&ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    hard_reload_btn.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            hard_reload(&ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    chrome_btn.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            open_chrome_devtools(&ui);
        }
    });
    let weak = Rc::downgrade(&ui);
    eval_entry.connect_activate(move |entry| {
        if let Some(ui) = weak.upgrade() {
            let script = entry.text().to_string();
            if !script.trim().is_empty() {
                entry.set_text("");
                evaluate(&ui, script);
            }
        }
    });
    // Up/Down walk the evaluate history.
    {
        let keys = gtk4::EventControllerKey::new();
        let weak = Rc::downgrade(&ui);
        keys.connect_key_pressed(move |_, keyval, _, _| {
            let Some(ui) = weak.upgrade() else { return glib::Propagation::Proceed };
            let up = keyval == gtk4::gdk::Key::Up;
            if !up && keyval != gtk4::gdk::Key::Down {
                return glib::Propagation::Proceed;
            }
            let text = {
                let mut st = ui.st.borrow_mut();
                let n = st.history.len();
                if n == 0 {
                    return glib::Propagation::Stop;
                }
                let pos = match (st.history_pos, up) {
                    (None, true) => Some(n - 1),
                    (None, false) => None,
                    (Some(p), true) => Some(p.saturating_sub(1)),
                    (Some(p), false) if p + 1 < n => Some(p + 1),
                    (Some(_), false) => None,
                };
                st.history_pos = pos;
                pos.map(|p| st.history[p].clone()).unwrap_or_default()
            };
            ui.eval_entry.set_text(&text);
            ui.eval_entry.set_position(-1);
            glib::Propagation::Stop
        });
        eval_entry.add_controller(keys);
    }

    // F12 toggles the panel, Ctrl+Shift+R / Ctrl+F5 hard-reload, while focus is
    // anywhere in this preview. Capture phase: runs before the page key handler.
    {
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let weak = Rc::downgrade(&ui);
        keys.connect_key_pressed(move |_, keyval, _, mods| {
            let Some(ui) = weak.upgrade() else { return glib::Propagation::Proceed };
            use gtk4::gdk::{Key, ModifierType as M};
            let mods = mods & (M::CONTROL_MASK | M::SHIFT_MASK | M::ALT_MASK | M::SUPER_MASK);
            if keyval == Key::F12 && mods.is_empty() {
                ui.btn.set_active(!ui.btn.is_active());
                return glib::Propagation::Stop;
            }
            let ctrl_shift_r = mods == (M::CONTROL_MASK | M::SHIFT_MASK)
                && (keyval == Key::R || keyval == Key::r);
            let ctrl_f5 = mods == M::CONTROL_MASK && keyval == Key::F5;
            if ctrl_shift_r || ctrl_f5 {
                hard_reload(&ui);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        widgets.container.add_controller(keys);
    }

    // Poll loop. It owns the panel state (signal handlers only hold weak
    // refs) and ends when the preview pane is destroyed, freeing it.
    let container = widgets.container.downgrade();
    let mut first = true;
    glib::timeout_add_local(POLL_INTERVAL, move || {
        if container.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        poll(&ui, first);
        first = false;
        glib::ControlFlow::Continue
    });
}

/// (row box, [labels]) with the fixed network column layout.
fn net_row_box(cols: [&str; 6]) -> (gtk4::Box, [gtk4::Label; 6]) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.add_css_class("devtools-net-row");
    // URL (index 1) takes the spare width but keeps a readable minimum.
    let widths = [5, 0, 8, 7, 7, 7];
    let labels: [gtk4::Label; 6] = std::array::from_fn(|i| {
        let l = label(cols[i], "devtools-mono");
        if widths[i] > 0 {
            l.set_width_chars(widths[i]);
            l.set_max_width_chars(widths[i]);
            l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        } else {
            l.set_hexpand(true);
            l.set_width_chars(14);
            l.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        }
        row.append(&l);
        l
    });
    (row, labels)
}

fn set_panel_open(ui: &Rc<Ui>, open: bool) {
    if open {
        if ui.paned.end_child().is_none() {
            ui.paned.set_end_child(Some(&ui.panel));
        }
        let saved = ui.st.borrow().paned_pos;
        let h = ui.paned.height();
        let pos = saved.unwrap_or(if h > 0 { (h - DEFAULT_PANEL_HEIGHT).max(h / 3) } else { 300 });
        ui.paned.set_position(pos);
        {
            let mut st = ui.st.borrow_mut();
            st.unread_errors = 0;
        }
        update_badge(ui);
        if current_tab(ui) == Tab::Elements {
            refresh_elements(ui);
        }
        scroll_to_bottom(&ui.console_scroll);
    } else if ui.paned.end_child().is_some() {
        ui.st.borrow_mut().paned_pos = Some(ui.paned.position());
        ui.paned.set_end_child(None::<&gtk4::Widget>);
    }
}

fn current_tab(ui: &Ui) -> Tab {
    match ui.stack.visible_child_name().as_deref() {
        Some("network") => Tab::Network,
        Some("elements") => Tab::Elements,
        _ => Tab::Console,
    }
}

fn show_tab(ui: &Rc<Ui>, tab: Tab) {
    ui.stack.set_visible_child_name(tab.name());
    ui.clear_btn.set_visible(tab != Tab::Elements);
    ui.refresh_btn.set_visible(tab == Tab::Elements);
    let idx = match tab {
        Tab::Console => 0,
        Tab::Network => 1,
        Tab::Elements => 2,
    };
    if !ui.tab_btns[idx].is_active() {
        ui.tab_btns[idx].set_active(true);
    }
    match tab {
        Tab::Elements => refresh_elements(ui),
        Tab::Console => {
            let _ = ui.eval_entry.grab_focus_without_selecting();
        }
        Tab::Network => {}
    }
}

fn update_badge(ui: &Ui) {
    let n = ui.st.borrow().unread_errors;
    ui.badge.set_visible(n > 0);
    ui.badge.set_text(&if n > 99 { "99+".to_string() } else { n.to_string() });
    if n > 0 {
        ui.btn.add_css_class("devtools-has-errors");
    } else {
        ui.btn.remove_css_class("devtools-has-errors");
    }
}

fn set_status(ui: &Ui, text: &str, is_error: bool) {
    ui.status.set_text(text);
    ui.status.set_tooltip_text(Some(text));
    if is_error {
        ui.status.add_css_class("devtools-status-error");
    } else {
        ui.status.remove_css_class("devtools-status-error");
    }
}

fn at_bottom(scroll: &gtk4::ScrolledWindow) -> bool {
    let adj = scroll.vadjustment();
    adj.value() + adj.page_size() >= adj.upper() - 8.0
}

/// Jump to the bottom once rows appended now have been laid out.
fn scroll_to_bottom(scroll: &gtk4::ScrolledWindow) {
    let scroll = scroll.clone();
    glib::timeout_add_local_once(Duration::from_millis(60), move || {
        let adj = scroll.vadjustment();
        adj.set_value(adj.upper() - adj.page_size());
    });
}

// ── Console ────────────────────────────────────────────────────────────────

/// Append a console row. `kind` is a CSS level suffix (error/warn/info/log/
/// debug/input/result).
fn add_console_row(ui: &Ui, kind: &str, tag: &str, text: &str, location: &str) {
    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row_box.add_css_class("devtools-row");
    row_box.add_css_class(&format!("devtools-row-{kind}"));
    let tag_l = label(tag, "devtools-tag");
    tag_l.set_width_chars(4);
    tag_l.set_yalign(0.0);
    row_box.append(&tag_l);
    let text_l = label(&truncate(text, 20_000), "devtools-mono");
    text_l.set_wrap(true);
    text_l.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    text_l.set_selectable(true);
    text_l.set_hexpand(true);
    text_l.set_focusable(false);
    row_box.append(&text_l);
    if !location.is_empty() {
        let loc = label(location, "devtools-loc");
        loc.set_yalign(0.0);
        loc.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        loc.set_max_width_chars(32);
        row_box.append(&loc);
    }
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_focusable(false);
    row.set_child(Some(&row_box));
    // Follow new output like a terminal unless the user scrolled up.
    if at_bottom(&ui.console_scroll) {
        scroll_to_bottom(&ui.console_scroll);
    }
    ui.console_list.append(&row);

    // Bound memory: drop the oldest rows.
    let mut count = 0usize;
    let mut child = ui.console_list.first_child();
    while let Some(c) = child {
        count += 1;
        child = c.next_sibling();
    }
    while count > MAX_CONSOLE_ROWS {
        if let Some(first) = ui.console_list.row_at_index(0) {
            ui.console_list.remove(&first);
        }
        count -= 1;
    }
}

/// Entries of `list` that are new since `seen`, given the daemon's monotonic
/// `total`. Returns (new entries, new seen value).
fn new_entries(list: &[Value], total: u64, seen: u64) -> (&[Value], u64) {
    // A total below what we saw means a new daemon: start over.
    let seen = if total < seen { 0 } else { seen };
    let fresh = ((total - seen) as usize).min(list.len());
    (&list[list.len() - fresh..], total)
}

fn apply_console(ui: &Rc<Ui>, console: Option<Value>, errors: Option<Value>) {
    let mut new_errors = 0u32;

    if let Some(c) = console {
        let msgs = c.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let total = c.get("total").and_then(|t| t.as_u64()).unwrap_or(msgs.len() as u64);
        let seen = ui.st.borrow().seen_console;
        let (fresh, next) = new_entries(&msgs, total, seen);
        for m in fresh {
            let kind = m.get("type").and_then(|t| t.as_str()).unwrap_or("log");
            let text = m.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let (class, tag) = level_of(kind);
            if class == "error" {
                new_errors += 1;
            }
            add_console_row(ui, class, tag, text, "");
        }
        ui.st.borrow_mut().seen_console = next;
    }
    if let Some(e) = errors {
        let errs = e.get("errors").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let total = e.get("total").and_then(|t| t.as_u64()).unwrap_or(errs.len() as u64);
        let seen = ui.st.borrow().seen_errors;
        let (fresh, next) = new_entries(&errs, total, seen);
        for err in fresh {
            let text = err.get("text").and_then(|t| t.as_str()).unwrap_or("Uncaught error");
            let text = if text.starts_with("Uncaught") { text.to_string() } else { format!("Uncaught {text}") };
            add_console_row(ui, "error", "ERR", &text, &error_location(err));
            new_errors += 1;
        }
        ui.st.borrow_mut().seen_errors = next;
    }

    if new_errors > 0 && !ui.btn.is_active() {
        ui.st.borrow_mut().unread_errors += new_errors;
        update_badge(ui);
    }
}

fn evaluate(ui: &Rc<Ui>, script: String) {
    {
        let mut st = ui.st.borrow_mut();
        if st.history.last() != Some(&script) {
            st.history.push(script.clone());
        }
        st.history_pos = None;
    }
    add_console_row(ui, "input", "\u{203A}", &script, "");
    scroll_to_bottom(&ui.console_scroll);
    let socket = ui.socket.clone();
    run_off_main(
        ui,
        move || daemon_request(&socket, "evaluate", json!({ "script": script }), Duration::from_secs(10)),
        |ui, result| {
            match result {
                Ok(v) => add_console_row(ui, "result", "\u{2190}", &format_eval_result(&v), ""),
                Err(e) => add_console_row(ui, "error", "ERR", &e, ""),
            }
            scroll_to_bottom(&ui.console_scroll);
        },
    );
}

// ── Network ────────────────────────────────────────────────────────────────

fn apply_requests(ui: &Rc<Ui>, data: Value) {
    let reqs = data.get("requests").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let offset = {
        let mut st = ui.st.borrow_mut();
        if reqs.len() < st.net_offset {
            // Daemon list was cleared elsewhere (CLI / new browser).
            st.net_offset = 0;
        }
        st.net_len = reqs.len();
        st.net_offset
    };
    if reqs.len() < offset + ui.net_rows.borrow().len() {
        clear_network_rows(ui);
    }

    let (mut failed, mut total_bytes) = (0usize, 0i64);
    for req in &reqs[offset..] {
        let id = req.get("requestId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (status_text, is_failed) = request_status(req);
        if is_failed {
            failed += 1;
        }
        let size = req.get("encodedDataLength").and_then(|v| v.as_i64());
        total_bytes += size.unwrap_or(0).max(0);
        let kind = req.get("resourceType").and_then(|v| v.as_str()).unwrap_or("");
        let time = req
            .get("durationMs")
            .and_then(|v| v.as_u64())
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_default();
        let size_text = size.map(human_size).unwrap_or_default();

        let mut rows = ui.net_rows.borrow_mut();
        if let Some(r) = rows.get(&id) {
            r.status.set_text(&status_text);
            r.kind.set_text(kind);
            r.size.set_text(&size_text);
            r.time.set_text(&time);
            if is_failed {
                r.row.add_css_class("devtools-net-failed");
            }
            if let Some(err) = req.get("errorText").and_then(|e| e.as_str()) {
                r.status.set_tooltip_text(Some(err));
            }
            continue;
        }
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let url = req.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let shown_url = if url.starts_with("data:") { truncate(url, 80) } else { url.to_string() };
        let (row_box, labels) = net_row_box([method, &shown_url, &status_text, kind, &size_text, &time]);
        labels[1].set_tooltip_text(Some(&truncate(url, 600)));
        if let Some(err) = req.get("errorText").and_then(|e| e.as_str()) {
            labels[2].set_tooltip_text(Some(err));
        }
        let row = gtk4::ListBoxRow::new();
        row.set_activatable(false);
        row.set_focusable(false);
        row.set_child(Some(&row_box));
        if is_failed {
            row.add_css_class("devtools-net-failed");
        }
        if at_bottom(&ui.net_scroll) {
            scroll_to_bottom(&ui.net_scroll);
        }
        ui.net_list.append(&row);
        let [_, _, status, kind_l, size_l, time_l] = labels;
        rows.insert(id, NetRow { row, status, kind: kind_l, size: size_l, time: time_l });
    }
    let n = reqs.len() - offset;
    let mut summary = format!("{n} request{}", if n == 1 { "" } else { "s" });
    if failed > 0 {
        summary.push_str(&format!(" \u{00B7} {failed} failed"));
    }
    if total_bytes > 0 {
        summary.push_str(&format!(" \u{00B7} {} transferred", human_size(total_bytes)));
    }
    ui.net_summary.set_text(&summary);
    if failed > 0 {
        ui.net_summary.add_css_class("devtools-status-error");
    } else {
        ui.net_summary.remove_css_class("devtools-status-error");
    }
}

fn clear_network_rows(ui: &Ui) {
    while let Some(row) = ui.net_list.row_at_index(0) {
        ui.net_list.remove(&row);
    }
    ui.net_rows.borrow_mut().clear();
}

fn clear_current(ui: &Rc<Ui>) {
    match current_tab(ui) {
        Tab::Console => {
            while let Some(row) = ui.console_list.row_at_index(0) {
                ui.console_list.remove(&row);
            }
            ui.st.borrow_mut().unread_errors = 0;
            update_badge(ui);
        }
        Tab::Network => {
            clear_network_rows(ui);
            {
                let mut st = ui.st.borrow_mut();
                st.net_offset = st.net_len;
            }
            ui.net_summary.set_text("0 requests");
            ui.net_summary.remove_css_class("devtools-status-error");
        }
        Tab::Elements => {}
    }
}

// ── Elements / actions ─────────────────────────────────────────────────────

fn refresh_elements(ui: &Rc<Ui>) {
    let socket = ui.socket.clone();
    run_off_main(
        ui,
        move || daemon_request(&socket, "snapshot", json!({}), Duration::from_secs(10)),
        |ui, result| {
            let text = match result {
                Ok(v) => v
                    .get("snapshot")
                    .and_then(|s| s.as_str())
                    .or_else(|| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(e) => format!("Snapshot error: {e}"),
            };
            ui.elements_view.buffer().set_text(&text);
        },
    );
}

fn hard_reload(ui: &Rc<Ui>) {
    set_status(ui, "Hard reloading\u{2026}", false);
    let socket = ui.socket.clone();
    run_off_main(
        ui,
        move || daemon_request(&socket, "reload", json!({ "ignoreCache": true }), Duration::from_secs(15)),
        |ui, result| match result {
            Ok(_) => set_status(ui, "Reloaded (cache bypassed)", false),
            Err(e) => set_status(ui, &format!("Reload failed: {e}"), true),
        },
    );
}

/// Open Chrome's own DevTools frontend for the current page. The daemon's
/// `inspect` action starts a CDP proxy on 127.0.0.1 that redirects to the
/// frontend Chrome serves itself; WSL2 localhost forwarding makes it reachable
/// from the Windows browser.
fn open_chrome_devtools(ui: &Rc<Ui>) {
    set_status(ui, "Starting DevTools proxy\u{2026}", false);
    let socket = ui.socket.clone();
    run_off_main(
        ui,
        move || daemon_request(&socket, "inspect", json!({ "open": false }), Duration::from_secs(5)),
        |ui, result| {
            let url = match result {
                Ok(v) => v.get("url").and_then(|u| u.as_str()).map(String::from),
                Err(e) => {
                    set_status(ui, &format!("DevTools unavailable: {e}"), true);
                    return;
                }
            };
            let Some(url) = url else {
                set_status(ui, "DevTools unavailable: no URL from daemon", true);
                return;
            };
            // Test hook: log instead of launching a browser window.
            if std::env::var_os("CMUX_DEVTOOLS_DRY_RUN").is_some() {
                eprintln!("cmux: devtools frontend url: {url}");
                set_status(ui, &format!("DevTools: {url}"), false);
                return;
            }
            match crate::url_opener::open_url(&url) {
                Ok(_) => set_status(ui, &format!("Opened DevTools: {url}"), false),
                Err(e) => set_status(ui, &format!("Open {url} failed: {e}"), true),
            }
        },
    );
}

fn poll(ui: &Rc<Ui>, first: bool) {
    if ui.st.borrow().in_flight {
        return;
    }
    ui.st.borrow_mut().in_flight = true;
    // Requests only while the panel is open (they carry headers), plus once at
    // start so the daemon tracks the page's initial load.
    let want_requests = first || ui.btn.is_active();
    let socket = ui.socket.clone();
    run_off_main(
        ui,
        move || poll_daemon(&socket, want_requests),
        |ui, r| {
            ui.st.borrow_mut().in_flight = false;
            apply_console(ui, r.console, r.errors);
            if let Some(reqs) = r.requests {
                apply_requests(ui, reqs);
            }
        },
    );
}

/// `win.browser-devtools` action: `""`/`"toggle"` toggles the newest preview's
/// panel; `"console"`, `"network"`, `"elements"` open it on that tab;
/// `"close"`, `"clear"`, `"hard-reload"`, `"chrome"` do what they say.
pub fn activate_action(arg: &str) {
    let ui = PANELS.with(|p| p.borrow().iter().rev().find_map(|w| w.upgrade()));
    let Some(ui) = ui else {
        eprintln!("cmux: devtools: no preview pane for action {arg:?}");
        return;
    };
    match arg {
        "" | "toggle" => ui.btn.set_active(!ui.btn.is_active()),
        "close" => ui.btn.set_active(false),
        "console" | "network" | "elements" => {
            ui.btn.set_active(true);
            let tab = match arg {
                "network" => Tab::Network,
                "elements" => Tab::Elements,
                _ => Tab::Console,
            };
            show_tab(&ui, tab);
        }
        "clear" => clear_current(&ui),
        "hard-reload" => hard_reload(&ui),
        "chrome" => open_chrome_devtools(&ui),
        other => {
            if let Some(script) = other.strip_prefix("eval:") {
                ui.btn.set_active(true);
                show_tab(&ui, Tab::Console);
                evaluate(&ui, script.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_entries_takes_only_unseen_tail() {
        let list: Vec<Value> = (0..5).map(|i| json!(i)).collect();
        let (fresh, seen) = new_entries(&list, 5, 3);
        assert_eq!(fresh, &list[3..]);
        assert_eq!(seen, 5);
        // Trimmed buffer: total grew past the list length.
        let (fresh, _) = new_entries(&list, 1005, 1003);
        assert_eq!(fresh.len(), 2);
        // Daemon restarted: total went backwards.
        let (fresh, seen) = new_entries(&list[..2], 2, 7);
        assert_eq!(fresh.len(), 2);
        assert_eq!(seen, 2);
    }

    #[test]
    fn request_status_flags_failures() {
        assert_eq!(request_status(&json!({"status": 200})), ("200".into(), false));
        assert_eq!(request_status(&json!({"status": 404})), ("404".into(), true));
        assert_eq!(
            request_status(&json!({"errorText": "net::ERR_CONNECTION_REFUSED"})),
            ("(failed)".into(), true)
        );
        assert_eq!(request_status(&json!({"errorText": "canceled"})).1, false);
        assert_eq!(request_status(&json!({})), ("…".into(), false));
    }

    #[test]
    fn error_location_is_one_based() {
        let e = json!({"url": "http://x/app/main.js", "line": 9, "column": 4});
        assert_eq!(error_location(&e), "main.js:10:5");
        assert_eq!(error_location(&json!({"url": null, "line": 0, "column": 3})), "1:4");
        assert_eq!(error_location(&json!({})), "");
    }

    #[test]
    fn formats_values() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 kB");
        assert_eq!(format_eval_result(&json!({"result": "hi"})), "\"hi\"");
        assert_eq!(format_eval_result(&json!({"result": 3})), "3");
        assert_eq!(format_eval_result(&json!({})), "undefined");
        assert_eq!(level_of("warning"), ("warn", "WARN"));
        assert_eq!(level_of("error"), ("error", "ERR"));
    }
}
