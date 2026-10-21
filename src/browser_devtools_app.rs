//! Application tab of the browser devtools panel: Local/Session Storage,
//! Cookies, IndexedDB, Cache Storage, Service Workers, Manifest and Clear site
//! data, like the Chrome DevTools panel of the same name.
//!
//! Layout: a section list on the left, one page per section on the right.
//! Nothing polls: a section fetches when it is shown or its Refresh is
//! pressed, through the same daemon socket as the rest of the panel
//! (`browser_devtools::daemon_request` on the tokio blocking pool).
//!
//! The daemon actions behind IndexedDB / Cache / Service Workers / Manifest /
//! site data may be missing from an older agent-browser build; such a section
//! then shows "Not supported by this agent-browser build" instead of an error.

use gtk4::prelude::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::browser_devtools::{daemon_request, label, run_off_main_with, tool_btn, truncate};

const TIMEOUT: Duration = Duration::from_secs(10);
/// Records per IndexedDB / Cache Storage page.
const PAGE: u64 = 50;
/// Width of the per-row action button column (header keeps a spacer this wide).
const ROW_BTN_W: i32 = 24;
/// How long a destructive button stays armed after its first click.
const CONFIRM_WINDOW: Duration = Duration::from_secs(4);
pub(crate) const UNSUPPORTED: &str = "Not supported by this agent-browser build";

/// One page of the Application tab; `name()` is also its stack child name and
/// the `application:<section>` action argument.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Section {
    Local,
    Session,
    Cookies,
    IndexedDb,
    Cache,
    ServiceWorkers,
    Manifest,
    SiteData,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::Local,
        Section::Session,
        Section::Cookies,
        Section::IndexedDb,
        Section::Cache,
        Section::ServiceWorkers,
        Section::Manifest,
        Section::SiteData,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Local => "local",
            Section::Session => "session",
            Section::Cookies => "cookies",
            Section::IndexedDb => "indexeddb",
            Section::Cache => "cache",
            Section::ServiceWorkers => "sw",
            Section::Manifest => "manifest",
            Section::SiteData => "site",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Local => "Local Storage",
            Section::Session => "Session Storage",
            Section::Cookies => "Cookies",
            Section::IndexedDb => "IndexedDB",
            Section::Cache => "Cache Storage",
            Section::ServiceWorkers => "Service Workers",
            Section::Manifest => "Manifest",
            Section::SiteData => "Clear site data",
        }
    }

    fn index(self) -> i32 {
        Section::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32
    }

    /// Section from an action argument (`local`, `cookies`, ...; a few
    /// spelled-out aliases are accepted too).
    pub(crate) fn parse(s: &str) -> Option<Section> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "local" | "localstorage" | "local-storage" => Section::Local,
            "session" | "sessionstorage" | "session-storage" => Section::Session,
            "cookies" | "cookie" => Section::Cookies,
            "indexeddb" | "idb" => Section::IndexedDb,
            "cache" | "caches" | "cache-storage" => Section::Cache,
            "sw" | "service-workers" | "serviceworkers" => Section::ServiceWorkers,
            "manifest" => Section::Manifest,
            "site" | "site-data" | "clear" => Section::SiteData,
            _ => return None,
        })
    }

    /// `storage_*` `type` param for the two Web Storage sections.
    fn storage_type(self) -> &'static str {
        if self == Section::Session {
            "session"
        } else {
            "local"
        }
    }
}

/// Parse a `win.browser-devtools` argument: `None` if it is not an
/// Application-tab argument, `Some(None)` for plain `application` (or an
/// unknown section), `Some(Some(section))` for `application:<section>`.
pub(crate) fn parse_app_arg(arg: &str) -> Option<Option<Section>> {
    let rest = arg.strip_prefix("application")?;
    if rest.is_empty() {
        return Some(None);
    }
    let sec = rest.strip_prefix(':')?;
    let parsed = Section::parse(sec);
    if parsed.is_none() {
        eprintln!("cmux: devtools: unknown application section {sec:?}");
    }
    Some(parsed)
}

// ── Pure helpers (unit-tested) ─────────────────────────────────────────────

/// Did the daemon reject the action itself (not merged into this build)?
pub(crate) fn is_unsupported(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("not yet implemented") || e.contains("unknown action") || e.contains("unknown command")
}

/// Bytes with binary units, for usage/quota (which can reach terabytes).
fn format_bytes(bytes: f64) -> String {
    if !bytes.is_finite() || bytes < 0.0 {
        return String::new();
    }
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", bytes as u64)
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Seconds since the Unix epoch as "YYYY-MM-DD HH:MM:SS" (UTC).
fn format_epoch(secs: f64) -> String {
    let secs = secs.floor() as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (H. Hinnant), valid for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", tod / 3600, tod % 3600 / 60, tod % 60)
}

/// Expires column: "Session" for session cookies, else the UTC date.
fn cookie_expires(c: &Value) -> String {
    let session = c.get("session").and_then(|v| v.as_bool()).unwrap_or(false);
    match c.get("expires").and_then(|v| v.as_f64()) {
        Some(e) if !session && e > 0.0 => format_epoch(e),
        _ => "Session".to_string(),
    }
}

/// Size column: CDP's `size`, else name + value length like Chrome counts it.
fn cookie_size(c: &Value) -> u64 {
    c.get("size").and_then(|v| v.as_u64()).unwrap_or_else(|| {
        let len = |k: &str| c.get(k).and_then(|v| v.as_str()).map_or(0, str::len) as u64;
        len("name") + len("value")
    })
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// `cookies_set` params that rewrite cookie `c` with `value`, keeping its
/// domain/path/flags. A host-only cookie (domain without a leading dot) is
/// addressed by URL, since passing `domain` would turn it into a domain cookie.
fn cookie_set_params(c: &Value, value: &str) -> Value {
    let domain = str_field(c, "domain");
    let path = match str_field(c, "path") {
        "" => "/",
        p => p,
    };
    let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut p = json!({ "name": str_field(c, "name"), "value": value, "path": path });
    if domain.starts_with('.') {
        p["domain"] = json!(domain);
    } else {
        let scheme = if secure { "https" } else { "http" };
        p["url"] = json!(format!("{scheme}://{domain}{path}"));
    }
    let session = c.get("session").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(e) = c.get("expires").and_then(|v| v.as_f64()) {
        if !session && e > 0.0 {
            p["expires"] = json!(e);
        }
    }
    if c.get("httpOnly").and_then(|v| v.as_bool()) == Some(true) {
        p["httpOnly"] = json!(true);
    }
    if secure {
        p["secure"] = json!(true);
    }
    if let Some(ss) = c.get("sameSite").and_then(|v| v.as_str()) {
        p["sameSite"] = json!(ss);
    }
    // A partitioned (CHIPS) cookie is only addressed together with its key.
    if let Some(pk) = c.get("partitionKey").filter(|k| !k.is_null()) {
        p["partitionKey"] = pk.clone();
    }
    p
}

/// `cookies_delete` params for cookie `c`.
fn cookie_delete_params(c: &Value) -> Value {
    let mut p = json!({ "name": str_field(c, "name"), "domain": str_field(c, "domain"), "path": str_field(c, "path") });
    if let Some(pk) = c.get("partitionKey").filter(|k| !k.is_null()) {
        p["partitionKey"] = pk.clone();
    }
    p
}

/// Fallback delete for daemons without `cookies_delete`: re-set the cookie
/// already expired, which makes Chrome drop it.
fn cookie_expire_params(c: &Value) -> Value {
    let mut p = cookie_set_params(c, "");
    p["expires"] = json!(1);
    p
}

/// Fallback for daemons without `storage_remove`.
fn storage_remove_script(storage_type: &str, key: &str) -> String {
    let st = if storage_type == "session" { "sessionStorage" } else { "localStorage" };
    format!("{st}.removeItem({})", serde_json::to_string(key).unwrap_or_default())
}

/// One-line preview of an IndexedDB key/value or storage value: strings as-is,
/// other JSON serialized, control characters flattened, at most `max` chars.
fn preview(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    };
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    truncate(&flat, max)
}

/// "1–50", "51–73 of 73", or "No records".
fn page_label(skip: u64, count: usize, total: Option<u64>) -> String {
    if count == 0 {
        return if skip == 0 { "No records".to_string() } else { format!("No records after {skip}") };
    }
    let range = format!("{}\u{2013}{}", skip + 1, skip + count as u64);
    match total {
        Some(t) => format!("{range} of {t}"),
        None => range,
    }
}

fn count_label(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// `clear_site_data` `types` param from the checkboxes: a CSV of the ticked
/// CDP storage types (never "all", which would also clear types not offered);
/// `None` when nothing is ticked.
fn clear_types(checks: &[(&str, bool)]) -> Option<String> {
    let on: Vec<&str> = checks.iter().filter(|(_, c)| *c).map(|(t, _)| *t).collect();
    (!on.is_empty()).then(|| on.join(","))
}

/// Usage bar fraction in 0..=1.
fn usage_fraction(usage: f64, quota: f64) -> f64 {
    if quota > 0.0 {
        (usage / quota).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

// ── Widgets ────────────────────────────────────────────────────────────────

/// Header bar + body of one section, with the "not supported" fallback page.
struct Frame {
    root: gtk4::Box,
    status: gtk4::Label,
    body: gtk4::Stack,
}

impl Frame {
    fn new(sec: Section, content: &impl IsA<gtk4::Widget>, buttons: &[&gtk4::Button]) -> Frame {
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        header.add_css_class("devtools-app-header");
        let title = label(sec.title(), "devtools-app-title");
        header.append(&title);
        let status = label("", "devtools-status");
        status.set_hexpand(true);
        status.set_margin_start(8);
        status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.append(&status);
        for b in buttons {
            header.append(*b);
        }
        let unsupported = gtk4::Label::new(Some(UNSUPPORTED));
        unsupported.add_css_class("devtools-app-unsupported");
        unsupported.set_wrap(true);
        let body = gtk4::Stack::new();
        body.add_named(content, Some("body"));
        body.add_named(&unsupported, Some("unsupported"));
        body.set_vexpand(true);
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&body);
        Frame { root, status, body }
    }

    fn status(&self, text: &str, is_error: bool) {
        self.status.set_text(text);
        self.status.set_tooltip_text(Some(text));
        if is_error {
            self.status.add_css_class("devtools-status-error");
        } else {
            self.status.remove_css_class("devtools-status-error");
        }
    }

    /// Show the data page; returns `Some(data)` on success. On failure shows
    /// the unsupported page or an error status and returns `None`.
    fn take(&self, r: Result<Value, String>, what: &str) -> Option<Value> {
        match r {
            Ok(v) => {
                self.body.set_visible_child_name("body");
                Some(v)
            }
            Err(e) if is_unsupported(&e) => {
                self.body.set_visible_child_name("unsupported");
                self.status("", false);
                None
            }
            Err(e) => {
                self.body.set_visible_child_name("body");
                self.status(&format!("{what} failed: {e}"), true);
                None
            }
        }
    }
}

/// A list with a column header that scrolls along with the rows.
struct Table {
    scroll: gtk4::ScrolledWindow,
    list: gtk4::ListBox,
}

/// A row of fixed-width monospace cells; width 0 takes the spare width.
fn cells(texts: &[&str], widths: &[i32]) -> (gtk4::Box, Vec<gtk4::Label>) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.add_css_class("devtools-net-row");
    let labels = texts
        .iter()
        .zip(widths)
        .map(|(t, &w)| {
            let l = label(t, "devtools-mono");
            l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            if w > 0 {
                l.set_width_chars(w);
                l.set_max_width_chars(w);
            } else {
                l.set_hexpand(true);
                l.set_width_chars(14);
            }
            row.append(&l);
            l
        })
        .collect();
    (row, labels)
}

fn table(headers: &[&str], widths: &[i32], row_btns: i32, empty: &str) -> Table {
    let (header, _) = cells(headers, widths);
    header.add_css_class("devtools-net-header");
    if row_btns > 0 {
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_size_request(ROW_BTN_W * row_btns, -1);
        header.append(&spacer);
    }
    let list = plain_list(empty);
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    body.append(&header);
    body.append(&list);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&body));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    Table { scroll, list }
}

/// A non-selectable row list showing `empty` when it has no rows.
fn plain_list(empty: &str) -> gtk4::ListBox {
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("devtools-list");
    let placeholder = label(empty, "devtools-app-unsupported");
    placeholder.set_margin_top(12);
    placeholder.set_xalign(0.5);
    list.set_placeholder(Some(&placeholder));
    list
}

fn add_row(list: &gtk4::ListBox, child: &impl IsA<gtk4::Widget>) {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_focusable(false);
    row.set_child(Some(child));
    list.append(&row);
}

fn row_btn(text: &str, tooltip: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(text);
    b.add_css_class("devtools-app-row-btn");
    b.set_tooltip_text(Some(tooltip));
    b.set_size_request(ROW_BTN_W, -1);
    b.set_valign(gtk4::Align::Center);
    b
}

/// A selectable tree/list (IndexedDB databases, caches) on the left of a page.
fn side_list() -> (gtk4::ScrolledWindow, gtk4::ListBox) {
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.add_css_class("devtools-app-tree");
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_size_request(150, -1);
    (scroll, list)
}

fn split(left: &impl IsA<gtk4::Widget>, right: &impl IsA<gtk4::Widget>) -> gtk4::Paned {
    let p = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    p.set_start_child(Some(left));
    p.set_end_child(Some(right));
    p.set_resize_start_child(false);
    p.set_shrink_start_child(false);
    p.set_shrink_end_child(false);
    p
}

/// Two-click guard for destructive buttons. The first click captures what
/// would be affected (`target`; `None` = nothing to do) and arms the button
/// (amber `armed` label) for a few seconds; a second click runs `action` on
/// that captured target, so a selection change in between cannot redirect it.
fn confirm_on_click<V, S, F>(btn: &gtk4::Button, armed: &'static str, target: S, action: F)
where
    V: 'static,
    S: Fn() -> Option<V> + 'static,
    F: Fn(V) + 'static,
{
    let idle = btn.label().map(|l| l.to_string()).unwrap_or_default();
    let pending: Rc<RefCell<Option<V>>> = Rc::new(RefCell::new(None));
    // Bumped on every arm/fire so a stale disarm timer does nothing.
    let token = Rc::new(Cell::new(0u64));
    btn.connect_clicked(move |b| {
        token.set(token.get() + 1);
        let fired = pending.borrow_mut().take();
        if let Some(v) = fired {
            b.set_label(&idle);
            b.remove_css_class("devtools-app-armed");
            action(v);
            return;
        }
        let Some(v) = target() else { return };
        *pending.borrow_mut() = Some(v);
        b.set_label(armed);
        b.add_css_class("devtools-app-armed");
        let t = token.get();
        let (token, pending, b, idle) = (token.clone(), pending.clone(), b.downgrade(), idle.clone());
        glib::timeout_add_local_once(CONFIRM_WINDOW, move || {
            if token.get() == t {
                pending.borrow_mut().take();
                if let Some(b) = b.upgrade() {
                    b.set_label(&idle);
                    b.remove_css_class("devtools-app-armed");
                }
            }
        });
    });
}

/// `confirm_on_click` wired to the panel through a weak ref.
fn confirm<V: 'static>(
    app: &Rc<AppPanel>,
    btn: &gtk4::Button,
    armed: &'static str,
    target: fn(&AppPanel) -> Option<V>,
    action: fn(&Rc<AppPanel>, V),
) {
    let (w1, w2) = (Rc::downgrade(app), Rc::downgrade(app));
    confirm_on_click(
        btn,
        armed,
        move || w1.upgrade().and_then(|a| target(&a)),
        move |v| {
            if let Some(a) = w2.upgrade() {
                action(&a, v);
            }
        },
    );
}

/// A table section with Refresh and Clear all (Web Storage, Cookies).
struct TableView {
    frame: Frame,
    table: Table,
    refresh: gtk4::Button,
    clear: gtk4::Button,
}

/// A paged record table with Prev/Next under it.
struct Pager {
    root: gtk4::Box,
    label: gtk4::Label,
    prev: gtk4::Button,
    next: gtk4::Button,
}

fn pager() -> Pager {
    let prev = tool_btn("\u{25C0} Prev", "Previous page");
    let next = tool_btn("Next \u{25B6}", "Next page");
    let label = label("", "devtools-status");
    label.set_hexpand(true);
    let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    root.add_css_class("devtools-app-pager");
    root.append(&label);
    root.append(&prev);
    root.append(&next);
    prev.set_sensitive(false);
    next.set_sensitive(false);
    Pager { root, label, prev, next }
}

struct IdbView {
    frame: Frame,
    tree: gtk4::ListBox,
    table: Table,
    pager: Pager,
    refresh: gtk4::Button,
    clear_store: gtk4::Button,
    delete_db: gtk4::Button,
}

struct CacheView {
    frame: Frame,
    tree: gtk4::ListBox,
    table: Table,
    pager: Pager,
    refresh: gtk4::Button,
    delete_cache: gtk4::Button,
}

struct SwView {
    frame: Frame,
    list: gtk4::ListBox,
    refresh: gtk4::Button,
}

struct ManifestView {
    frame: Frame,
    info: gtk4::Label,
    errors: gtk4::Label,
    json: gtk4::TextView,
    refresh: gtk4::Button,
}

/// Storage types offered by Clear site data, as CDP `Storage.clearDataForOrigin` names.
const CLEAR_TYPES: [(&str, &str); 6] = [
    ("cookies", "Cookies"),
    ("local_storage", "Local and session storage"),
    ("indexeddb", "IndexedDB"),
    ("cache_storage", "Cache storage"),
    ("service_workers", "Service workers"),
    ("file_systems", "File systems"),
];

struct SiteView {
    frame: Frame,
    origin: gtk4::Label,
    bar: gtk4::LevelBar,
    usage: gtk4::Label,
    breakdown: gtk4::Label,
    checks: Vec<(&'static str, gtk4::CheckButton)>,
    clear: gtk4::Button,
    refresh: gtk4::Button,
}

#[derive(Clone, PartialEq, Debug)]
enum IdbItem {
    Db(String),
    Store(String, String),
}

/// Selection/paging state (never borrowed across a GTK call that can re-enter).
#[derive(Default)]
struct AppSt {
    idb_rows: Vec<IdbItem>,
    idb_sel: Option<IdbItem>,
    idb_skip: u64,
    /// (cacheId, cacheName) per row of the cache list.
    cache_rows: Vec<(String, String)>,
    cache_sel: Option<String>,
    cache_skip: u64,
}

pub(crate) struct AppPanel {
    root: gtk4::Paned,
    sections: gtk4::ListBox,
    stack: gtk4::Stack,
    local: TableView,
    session: TableView,
    cookies: TableView,
    idb: IdbView,
    cache: CacheView,
    sw: SwView,
    manifest: ManifestView,
    site: SiteView,
    /// Set while a section row is selected programmatically, so the selection
    /// handler switches pages without fetching (the caller fetches once).
    quiet_select: Cell<bool>,
    /// Latest request number per (section, action); older replies are dropped.
    gens: RefCell<HashMap<(Section, &'static str), u64>>,
    socket: PathBuf,
    runtime: tokio::runtime::Handle,
    st: RefCell<AppSt>,
}

fn storage_view(sec: Section) -> TableView {
    let table = table(&["Key", "Value"], &[24, 0], 1, "No items");
    let refresh = tool_btn("Refresh", "Reload the items");
    let clear = tool_btn("Clear all", "Remove every item for this origin (asks to confirm)");
    let frame = Frame::new(sec, &table.scroll, &[&refresh, &clear]);
    TableView { frame, table, refresh, clear }
}

fn cookie_view() -> TableView {
    let table = table(
        &["Name", "Value", "Domain", "Path", "Expires", "Size", "HttpOnly", "Secure", "SameSite"],
        &COOKIE_WIDTHS,
        1,
        "No cookies for this page",
    );
    let refresh = tool_btn("Refresh", "Reload the cookies");
    let clear = tool_btn("Clear all", "Delete every cookie in the browser (asks to confirm)");
    let frame = Frame::new(Section::Cookies, &table.scroll, &[&refresh, &clear]);
    TableView { frame, table, refresh, clear }
}

const COOKIE_WIDTHS: [i32; 9] = [16, 0, 16, 8, 19, 5, 8, 6, 8];
const IDB_WIDTHS: [i32; 4] = [4, 18, 12, 0];
const CACHE_WIDTHS: [i32; 5] = [0, 6, 6, 8, 19];

fn idb_view() -> IdbView {
    let (tree_scroll, tree) = side_list();
    let table = table(&["#", "Key", "Primary key", "Value"], &IDB_WIDTHS, 0, "Select an object store");
    let pager = pager();
    let right = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    right.append(&table.scroll);
    right.append(&pager.root);
    let refresh = tool_btn("Refresh", "Reload databases and records");
    let clear_store = tool_btn("Clear store", "Delete every record in the selected object store");
    let delete_db = tool_btn("Delete DB", "Delete the selected database (asks to confirm)");
    clear_store.set_sensitive(false);
    delete_db.set_sensitive(false);
    let frame = Frame::new(Section::IndexedDb, &split(&tree_scroll, &right), &[&refresh, &clear_store, &delete_db]);
    IdbView { frame, tree, table, pager, refresh, clear_store, delete_db }
}

fn cache_view() -> CacheView {
    let (tree_scroll, tree) = side_list();
    let table = table(&["Request URL", "Method", "Status", "Type", "Time cached"], &CACHE_WIDTHS, 1, "Select a cache");
    let pager = pager();
    let right = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    right.append(&table.scroll);
    right.append(&pager.root);
    let refresh = tool_btn("Refresh", "Reload caches and entries");
    let delete_cache = tool_btn("Delete cache", "Delete the selected cache (asks to confirm)");
    delete_cache.set_sensitive(false);
    let frame = Frame::new(Section::Cache, &split(&tree_scroll, &right), &[&refresh, &delete_cache]);
    CacheView { frame, tree, table, pager, refresh, delete_cache }
}

fn sw_view() -> SwView {
    let list = plain_list("No service workers registered for this origin");
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let refresh = tool_btn("Refresh", "Reload the registrations");
    let frame = Frame::new(Section::ServiceWorkers, &scroll, &[&refresh]);
    SwView { frame, list, refresh }
}

fn manifest_view() -> ManifestView {
    let info = label("", "devtools-mono");
    info.set_selectable(true);
    info.set_wrap(true);
    info.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    let errors = label("", "devtools-mono");
    errors.add_css_class("devtools-app-error");
    errors.set_wrap(true);
    errors.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    errors.set_selectable(true);
    errors.set_visible(false);
    let json = gtk4::TextView::new();
    json.set_editable(false);
    json.set_monospace(true);
    json.set_wrap_mode(gtk4::WrapMode::WordChar);
    json.add_css_class("devtools-elements");
    json.set_vexpand(true);
    let inner = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    inner.add_css_class("devtools-app-pad");
    inner.append(&info);
    inner.append(&errors);
    inner.append(&json);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&inner));
    scroll.set_vexpand(true);
    let refresh = tool_btn("Refresh", "Reload the web app manifest");
    let frame = Frame::new(Section::Manifest, &scroll, &[&refresh]);
    ManifestView { frame, info, errors, json, refresh }
}

fn site_view() -> SiteView {
    let origin = label("", "devtools-mono");
    origin.set_selectable(true);
    let bar = gtk4::LevelBar::for_interval(0.0, 1.0);
    bar.add_css_class("devtools-app-usage");
    // Plain fill: the default low/high/full offsets would recolor it.
    for name in ["low", "high", "full"] {
        bar.remove_offset_value(Some(name));
    }
    let usage = label("", "devtools-status");
    let breakdown = label("", "devtools-mono");
    breakdown.add_css_class("devtools-app-dim");
    breakdown.set_wrap(true);
    let inner = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    inner.add_css_class("devtools-app-pad");
    inner.append(&origin);
    inner.append(&bar);
    inner.append(&usage);
    inner.append(&breakdown);
    let checks: Vec<(&'static str, gtk4::CheckButton)> = CLEAR_TYPES
        .iter()
        .map(|(ty, text)| {
            let c = gtk4::CheckButton::with_label(text);
            c.add_css_class("devtools-app-check");
            c.set_active(true);
            inner.append(&c);
            (*ty, c)
        })
        .collect();
    let clear = gtk4::Button::with_label("Clear site data");
    clear.add_css_class("devtools-app-danger");
    clear.set_halign(gtk4::Align::Start);
    clear.set_tooltip_text(Some("Delete the checked data for this origin"));
    inner.append(&clear);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&inner));
    scroll.set_vexpand(true);
    let refresh = tool_btn("Refresh", "Reload storage usage");
    let frame = Frame::new(Section::SiteData, &scroll, &[&refresh]);
    SiteView { frame, origin, bar, usage, breakdown, checks, clear, refresh }
}

impl AppPanel {
    pub(crate) fn new(socket: PathBuf, runtime: tokio::runtime::Handle) -> Rc<AppPanel> {
        let sections = gtk4::ListBox::new();
        sections.set_selection_mode(gtk4::SelectionMode::Single);
        sections.add_css_class("devtools-app-sections");
        for sec in Section::ALL {
            let row = gtk4::ListBoxRow::new();
            row.set_child(Some(&label(sec.title(), "devtools-app-section")));
            sections.append(&row);
        }
        sections.set_header_func(|row, _| {
            let group = match row.index() {
                0 => Some("STORAGE"),
                i if i == Section::ServiceWorkers.index() => Some("APPLICATION"),
                _ => None,
            };
            if row.header().is_none() {
                if let Some(g) = group {
                    row.set_header(Some(&label(g, "devtools-app-group")));
                }
            }
        });
        let side = gtk4::ScrolledWindow::new();
        side.set_child(Some(&sections));
        side.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        side.add_css_class("devtools-app-sidebar");
        side.set_size_request(140, -1);

        let app = Rc::new(AppPanel {
            root: gtk4::Paned::new(gtk4::Orientation::Horizontal),
            sections,
            stack: gtk4::Stack::new(),
            local: storage_view(Section::Local),
            session: storage_view(Section::Session),
            cookies: cookie_view(),
            idb: idb_view(),
            cache: cache_view(),
            sw: sw_view(),
            manifest: manifest_view(),
            site: site_view(),
            quiet_select: Cell::new(false),
            gens: RefCell::new(HashMap::new()),
            socket,
            runtime,
            st: RefCell::new(AppSt::default()),
        });
        for sec in Section::ALL {
            app.stack.add_named(&app.frame(sec).root, Some(sec.name()));
        }
        app.stack.set_hexpand(true);
        app.root.set_start_child(Some(&side));
        app.root.set_end_child(Some(&app.stack));
        app.root.set_resize_start_child(false);
        app.root.set_shrink_start_child(false);
        app.root.set_shrink_end_child(false);
        app.root.set_vexpand(true);
        if let Some(first) = app.sections.row_at_index(0) {
            app.sections.select_row(Some(&first));
        }
        app.connect_signals();
        app
    }

    pub(crate) fn widget(&self) -> &gtk4::Paned {
        &self.root
    }

    fn frame(&self, sec: Section) -> &Frame {
        match sec {
            Section::Local => &self.local.frame,
            Section::Session => &self.session.frame,
            Section::Cookies => &self.cookies.frame,
            Section::IndexedDb => &self.idb.frame,
            Section::Cache => &self.cache.frame,
            Section::ServiceWorkers => &self.sw.frame,
            Section::Manifest => &self.manifest.frame,
            Section::SiteData => &self.site.frame,
        }
    }

    fn storage(&self, sec: Section) -> &TableView {
        if sec == Section::Session {
            &self.session
        } else {
            &self.local
        }
    }

    fn current(&self) -> Section {
        let name = self.stack.visible_child_name();
        Section::ALL
            .into_iter()
            .find(|s| Some(s.name()) == name.as_deref())
            .unwrap_or(Section::Local)
    }

    /// Switch to `sec` without fetching; the caller refreshes afterwards.
    pub(crate) fn select(&self, sec: Section) {
        self.stack.set_visible_child_name(sec.name());
        if let Some(row) = self.sections.row_at_index(sec.index()) {
            self.quiet_select.set(true);
            self.sections.select_row(Some(&row));
            self.quiet_select.set(false);
        }
    }

    /// Fetch the visible section (tab shown, panel opened, or Refresh).
    pub(crate) fn refresh_current(self: &Rc<Self>) {
        refresh(self, self.current());
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.sections.connect_row_selected(move |_, row| {
            let (Some(row), Some(app)) = (row, weak.upgrade()) else { return };
            let Some(sec) = Section::ALL.get(row.index() as usize).copied() else { return };
            app.stack.set_visible_child_name(sec.name());
            if !app.quiet_select.get() {
                refresh(&app, sec);
            }
        });

        let on = |btn: &gtk4::Button, f: fn(&Rc<AppPanel>)| {
            let weak = Rc::downgrade(self);
            btn.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    f(&app);
                }
            });
        };

        // Web Storage
        on(&self.local.refresh, |a| refresh(a, Section::Local));
        on(&self.session.refresh, |a| refresh(a, Section::Session));
        confirm(self, &self.local.clear, "Clear all items?", |_| Some(()), |a, ()| clear_storage(a, Section::Local));
        confirm(self, &self.session.clear, "Clear all items?", |_| Some(()), |a, ()| clear_storage(a, Section::Session));

        // Cookies
        on(&self.cookies.refresh, |a| refresh(a, Section::Cookies));
        confirm(self, &self.cookies.clear, "Clear ALL browser cookies?", |_| Some(()), |app, ()| {
            let socket = app.socket.clone();
            act(app, Section::Cookies, move || daemon_request(&socket, "cookies_clear", json!({}), TIMEOUT), |a| {
                refresh(a, Section::Cookies)
            });
        });

        // IndexedDB
        on(&self.idb.refresh, |a| refresh(a, Section::IndexedDb));
        on(&self.idb.pager.prev, |a| page_idb(a, false));
        on(&self.idb.pager.next, |a| page_idb(a, true));
        confirm(
            self,
            &self.idb.clear_store,
            "Clear this store?",
            |a| match a.st.borrow().idb_sel.clone() {
                Some(IdbItem::Store(db, store)) => Some((db, store)),
                _ => None,
            },
            clear_idb_store,
        );
        confirm(
            self,
            &self.idb.delete_db,
            "Delete this database?",
            |a| match a.st.borrow().idb_sel.clone() {
                Some(IdbItem::Db(db)) | Some(IdbItem::Store(db, _)) => Some(db),
                None => None,
            },
            delete_idb,
        );
        let weak = Rc::downgrade(self);
        self.idb.tree.connect_row_selected(move |_, row| {
            let (Some(row), Some(app)) = (row, weak.upgrade()) else { return };
            let item = app.st.borrow().idb_rows.get(row.index() as usize).cloned();
            if let Some(item) = item {
                select_idb(&app, item);
            }
        });

        // Cache Storage
        on(&self.cache.refresh, |a| refresh(a, Section::Cache));
        on(&self.cache.pager.prev, |a| page_cache(a, false));
        on(&self.cache.pager.next, |a| page_cache(a, true));
        confirm(self, &self.cache.delete_cache, "Delete this cache?", |a| a.st.borrow().cache_sel.clone(), |a, id| {
            delete_cache(a, id, None)
        });
        let weak = Rc::downgrade(self);
        self.cache.tree.connect_row_selected(move |_, row| {
            let (Some(row), Some(app)) = (row, weak.upgrade()) else { return };
            let id = app.st.borrow().cache_rows.get(row.index() as usize).map(|(id, _)| id.clone());
            if let Some(id) = id {
                select_cache(&app, id);
            }
        });

        // Service workers / manifest / site data
        on(&self.sw.refresh, |a| refresh(a, Section::ServiceWorkers));
        on(&self.manifest.refresh, |a| refresh(a, Section::Manifest));
        on(&self.site.refresh, |a| refresh(a, Section::SiteData));
        confirm(self, &self.site.clear, "Clear site data?", |a| clear_types(&a.site_checks()), clear_site);
        for (_, c) in &self.site.checks {
            let weak = Rc::downgrade(self);
            c.connect_toggled(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.site.clear.set_sensitive(clear_types(&app.site_checks()).is_some());
                }
            });
        }
    }

    fn site_checks(&self) -> Vec<(&'static str, bool)> {
        self.site.checks.iter().map(|(t, c)| (*t, c.is_active())).collect()
    }
}

// ── Daemon plumbing ────────────────────────────────────────────────────────

fn run<T, W, D>(app: &Rc<AppPanel>, work: W, done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(&Rc<AppPanel>, T) + 'static,
{
    run_off_main_with(&app.runtime, app, work, done);
}

/// Fetch `action` and hand the result to `done` on the GTK thread.
fn fetch<D>(app: &Rc<AppPanel>, sec: Section, action: &'static str, params: Value, done: D)
where
    D: FnOnce(&Rc<AppPanel>, Value) + 'static,
{
    app.frame(sec).status("Loading\u{2026}", false);
    let gen = {
        let mut gens = app.gens.borrow_mut();
        let g = gens.entry((sec, action)).or_insert(0);
        *g += 1;
        *g
    };
    let socket = app.socket.clone();
    run(
        app,
        move || daemon_request(&socket, action, params, TIMEOUT),
        move |app, r| {
            // A newer request of the same kind is in flight: its reply wins.
            if app.gens.borrow().get(&(sec, action)) != Some(&gen) {
                return;
            }
            if let Some(v) = app.frame(sec).take(r, action) {
                app.frame(sec).status("", false);
                done(app, v);
            }
        },
    );
}

/// Run a mutating request; on success call `then` (usually a refresh), else
/// show the error in the section's status line.
fn act<W, N>(app: &Rc<AppPanel>, sec: Section, work: W, then: N)
where
    W: FnOnce() -> Result<Value, String> + Send + 'static,
    N: FnOnce(&Rc<AppPanel>) + 'static,
{
    run(app, work, move |app, r| match r {
        Ok(_) => then(app),
        Err(e) => {
            let msg = if is_unsupported(&e) { UNSUPPORTED.to_string() } else { e };
            app.frame(sec).status(&msg, true);
        }
    });
}

fn refresh(app: &Rc<AppPanel>, sec: Section) {
    match sec {
        Section::Local | Section::Session => refresh_storage(app, sec),
        Section::Cookies => refresh_cookies(app),
        Section::IndexedDb => refresh_idb(app),
        Section::Cache => refresh_cache(app),
        Section::ServiceWorkers => refresh_sw(app),
        Section::Manifest => refresh_manifest(app),
        Section::SiteData => refresh_site(app, ""),
    }
}

// ── Local / Session Storage ────────────────────────────────────────────────

fn refresh_storage(app: &Rc<AppPanel>, sec: Section) {
    fetch(app, sec, "storage_get", json!({ "type": sec.storage_type() }), move |app, v| {
        let view = app.storage(sec);
        let map = v.get("data").and_then(|d| d.as_object()).cloned().unwrap_or_default();
        view.table.list.remove_all();
        for (key, val) in &map {
            let full = preview(val, usize::MAX);
            let (row, labels) = cells(&[key, &preview(val, 300)], &[24, 0]);
            labels[0].set_tooltip_text(Some(&truncate(key, 2000)));
            labels[1].set_tooltip_text(Some(&truncate(&full, 2000)));
            labels[1].set_selectable(true);
            labels[1].set_focusable(false);
            let del = row_btn("\u{2715}", "Delete this key");
            row.append(&del);
            let weak = Rc::downgrade(app);
            let key = key.clone();
            del.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    remove_storage_key(&app, sec, key.clone());
                }
            });
            add_row(&view.table.list, &row);
        }
        view.frame.status(&count_label(map.len(), "item"), false);
    });
}

fn remove_storage_key(app: &Rc<AppPanel>, sec: Section, key: String) {
    let socket = app.socket.clone();
    let ty = sec.storage_type();
    act(
        app,
        sec,
        move || match daemon_request(&socket, "storage_remove", json!({ "type": ty, "key": key }), TIMEOUT) {
            Err(e) if is_unsupported(&e) => {
                daemon_request(&socket, "evaluate", json!({ "script": storage_remove_script(ty, &key) }), TIMEOUT)
            }
            r => r,
        },
        move |a| refresh(a, sec),
    );
}

fn clear_storage(app: &Rc<AppPanel>, sec: Section) {
    let socket = app.socket.clone();
    let ty = sec.storage_type();
    act(app, sec, move || daemon_request(&socket, "storage_clear", json!({ "type": ty }), TIMEOUT), move |a| {
        refresh(a, sec)
    });
}

// ── Cookies ────────────────────────────────────────────────────────────────

fn check(b: bool) -> &'static str {
    if b {
        "\u{2713}"
    } else {
        ""
    }
}

fn refresh_cookies(app: &Rc<AppPanel>) {
    fetch(app, Section::Cookies, "cookies_get", json!({}), |app, v| {
        let mut cookies = v.get("cookies").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        cookies.sort_by(|a, b| {
            (str_field(a, "domain"), str_field(a, "name")).cmp(&(str_field(b, "domain"), str_field(b, "name")))
        });
        let view = &app.cookies;
        view.table.list.remove_all();
        for c in &cookies {
            let flag = |k: &str| c.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
            let size = cookie_size(c).to_string();
            let expires = cookie_expires(c);
            let texts = [
                str_field(c, "name"),
                "",
                str_field(c, "domain"),
                str_field(c, "path"),
                expires.as_str(),
                size.as_str(),
                check(flag("httpOnly")),
                check(flag("secure")),
                str_field(c, "sameSite"),
            ];
            let (row, labels) = cells(&texts, &COOKIE_WIDTHS);
            labels[0].set_tooltip_text(Some(str_field(c, "name")));
            labels[2].set_tooltip_text(Some(str_field(c, "domain")));
            // Value column: an inline editor in place of the label.
            let entry = gtk4::Entry::new();
            entry.add_css_class("devtools-app-cell");
            entry.set_has_frame(false);
            entry.set_text(str_field(c, "value"));
            entry.set_hexpand(true);
            entry.set_width_chars(14);
            entry.set_tooltip_text(Some("Edit and press Enter to save"));
            row.insert_child_after(&entry, Some(&labels[1]));
            row.remove(&labels[1]);
            let weak = Rc::downgrade(app);
            let cookie = c.clone();
            entry.connect_activate(move |e| {
                if let Some(app) = weak.upgrade() {
                    let params = cookie_set_params(&cookie, &e.text());
                    let socket = app.socket.clone();
                    act(&app, Section::Cookies, move || daemon_request(&socket, "cookies_set", params, TIMEOUT), |a| {
                        refresh(a, Section::Cookies)
                    });
                }
            });
            let del = row_btn("\u{2715}", "Delete this cookie");
            row.append(&del);
            let weak = Rc::downgrade(app);
            let cookie = c.clone();
            del.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    delete_cookie(&app, &cookie);
                }
            });
            add_row(&view.table.list, &row);
        }
        view.frame.status(&count_label(cookies.len(), "cookie"), false);
    });
}

fn delete_cookie(app: &Rc<AppPanel>, cookie: &Value) {
    let (params, fallback) = (cookie_delete_params(cookie), cookie_expire_params(cookie));
    let socket = app.socket.clone();
    act(
        app,
        Section::Cookies,
        move || match daemon_request(&socket, "cookies_delete", params, TIMEOUT) {
            Err(e) if is_unsupported(&e) => daemon_request(&socket, "cookies_set", fallback, TIMEOUT),
            r => r,
        },
        |a| refresh(a, Section::Cookies),
    );
}

// ── IndexedDB ──────────────────────────────────────────────────────────────

fn update_idb_buttons(app: &AppPanel) {
    let sel = app.st.borrow().idb_sel.clone();
    app.idb.delete_db.set_sensitive(sel.is_some());
    app.idb.clear_store.set_sensitive(matches!(sel, Some(IdbItem::Store(..))));
}

fn reset_pager(p: &Pager, text: &str) {
    p.label.set_text(text);
    p.prev.set_sensitive(false);
    p.next.set_sensitive(false);
}

fn refresh_idb(app: &Rc<AppPanel>) {
    fetch(app, Section::IndexedDb, "indexeddb_databases", json!({}), |app, v| {
        let dbs = v.get("databases").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        let mut items = Vec::new();
        let mut widgets = Vec::new();
        for db in &dbs {
            let name = str_field(db, "name").to_string();
            let version = db.get("version").map(|v| v.to_string()).unwrap_or_default();
            let l = label(&format!("{name}  v{version}"), "devtools-app-db");
            l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            l.set_tooltip_text(Some(&name));
            items.push(IdbItem::Db(name.clone()));
            widgets.push(l);
            for store in db.get("objectStores").and_then(|s| s.as_array()).into_iter().flatten() {
                let sname = str_field(store, "name").to_string();
                let l = label(&sname, "devtools-app-store");
                l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                let indexes: Vec<String> = store
                    .get("indexes")
                    .and_then(|i| i.as_array())
                    .into_iter()
                    .flatten()
                    .map(|i| str_field(i, "name").to_string())
                    .collect();
                let key_path = store.get("keyPath").map(|k| preview(k, 80)).unwrap_or_default();
                let auto = store.get("autoIncrement").and_then(|a| a.as_bool()).unwrap_or(false);
                let mut tip = format!("{sname}\nkeyPath: {key_path}\nautoIncrement: {auto}");
                if !indexes.is_empty() {
                    tip.push_str(&format!("\nindexes: {}", indexes.join(", ")));
                }
                l.set_tooltip_text(Some(&tip));
                items.push(IdbItem::Store(name.clone(), sname));
                widgets.push(l);
            }
        }
        let prev = {
            let mut st = app.st.borrow_mut();
            st.idb_rows = items.clone();
            st.idb_sel.clone()
        };
        app.idb.tree.remove_all();
        for w in &widgets {
            app.idb.tree.append(w);
        }
        let origin = str_field(&v, "origin");
        let summary = count_label(dbs.len(), "database");
        app.idb.frame.status(&if origin.is_empty() { summary } else { format!("{origin} \u{00B7} {summary}") }, false);
        // Keep the selection (and page) across refreshes when it still exists.
        match prev.and_then(|p| items.iter().position(|i| *i == p)) {
            Some(idx) => {
                if let Some(row) = app.idb.tree.row_at_index(idx as i32) {
                    app.idb.tree.select_row(Some(&row));
                }
                // Re-selecting the same item loads nothing by itself; reload here.
                let (is_store, skip) = {
                    let st = app.st.borrow();
                    (matches!(st.idb_sel, Some(IdbItem::Store(..))), st.idb_skip)
                };
                if is_store {
                    load_idb_records(app, skip);
                }
            }
            None => {
                {
                    let mut st = app.st.borrow_mut();
                    st.idb_sel = None;
                    st.idb_skip = 0;
                }
                app.idb.table.list.remove_all();
                reset_pager(&app.idb.pager, "");
                update_idb_buttons(app);
                // Nothing selected yet: open the first object store so records
                // show without a click (its row-selected handler loads them).
                if let Some(idx) = items.iter().position(|i| matches!(i, IdbItem::Store(..))) {
                    if let Some(row) = app.idb.tree.row_at_index(idx as i32) {
                        app.idb.tree.select_row(Some(&row));
                    }
                }
            }
        }
    });
}

fn select_idb(app: &Rc<AppPanel>, item: IdbItem) {
    let changed = {
        let mut st = app.st.borrow_mut();
        let changed = st.idb_sel.as_ref() != Some(&item);
        if changed {
            st.idb_skip = 0;
        }
        st.idb_sel = Some(item.clone());
        changed
    };
    update_idb_buttons(app);
    match item {
        IdbItem::Store(..) => {
            if changed {
                load_idb_records(app, 0);
            }
        }
        IdbItem::Db(_) => {
            app.idb.table.list.remove_all();
            reset_pager(&app.idb.pager, "Select an object store");
        }
    }
}

/// Load the page at `skip` of the selected store; `idb_skip` follows only once
/// that page has arrived.
fn load_idb_records(app: &Rc<AppPanel>, skip: u64) {
    let sel = app.st.borrow().idb_sel.clone();
    let Some(IdbItem::Store(db, store)) = sel else { return };
    let params = json!({ "database": db, "store": store, "skip": skip, "limit": PAGE });
    fetch(app, Section::IndexedDb, "indexeddb_data", params, move |app, v| {
        // Ignore a late reply for a store that is no longer selected.
        let still = matches!(&app.st.borrow().idb_sel, Some(IdbItem::Store(d, s)) if *d == db && *s == store);
        if !still {
            return;
        }
        app.st.borrow_mut().idb_skip = skip;
        let entries = v.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let has_more = v.get("hasMore").and_then(|h| h.as_bool()).unwrap_or(false);
        app.idb.table.list.remove_all();
        for (i, e) in entries.iter().enumerate() {
            let n = (skip + i as u64 + 1).to_string();
            let key = e.get("key").map(|k| preview(k, 200)).unwrap_or_default();
            let pkey = e.get("primaryKey").map(|k| preview(k, 200)).unwrap_or_default();
            let val = e.get("value").map(|k| preview(k, 400)).unwrap_or_default();
            let (row, labels) = cells(&[&n, &key, &pkey, &val], &IDB_WIDTHS);
            labels[1].set_tooltip_text(Some(&key));
            labels[2].set_tooltip_text(Some(&pkey));
            if let Some(full) = e.get("value") {
                labels[3].set_tooltip_text(Some(&preview(full, 2000)));
            }
            add_row(&app.idb.table.list, &row);
        }
        app.idb.pager.label.set_text(&page_label(skip, entries.len(), None));
        app.idb.pager.prev.set_sensitive(skip > 0);
        app.idb.pager.next.set_sensitive(has_more);
        app.idb.frame.status(&format!("{db} \u{203A} {store}"), false);
    });
}

fn page_idb(app: &Rc<AppPanel>, forward: bool) {
    let skip = app.st.borrow().idb_skip;
    load_idb_records(app, if forward { skip + PAGE } else { skip.saturating_sub(PAGE) });
}

fn clear_idb_store(app: &Rc<AppPanel>, (db, store): (String, String)) {
    let socket = app.socket.clone();
    act(
        app,
        Section::IndexedDb,
        move || daemon_request(&socket, "indexeddb_clear", json!({ "database": db, "store": store }), TIMEOUT),
        |a| load_idb_records(a, 0),
    );
}

fn delete_idb(app: &Rc<AppPanel>, db: String) {
    let socket = app.socket.clone();
    act(
        app,
        Section::IndexedDb,
        move || daemon_request(&socket, "indexeddb_delete", json!({ "database": db }), TIMEOUT),
        |a| {
            a.st.borrow_mut().idb_sel = None;
            refresh(a, Section::IndexedDb);
        },
    );
}

// ── Cache Storage ──────────────────────────────────────────────────────────

fn refresh_cache(app: &Rc<AppPanel>) {
    fetch(app, Section::Cache, "cache_list", json!({}), |app, v| {
        let caches: Vec<(String, String)> = v
            .get("caches")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .map(|c| (str_field(c, "cacheId").to_string(), str_field(c, "cacheName").to_string()))
            .collect();
        let prev = {
            let mut st = app.st.borrow_mut();
            st.cache_rows = caches.clone();
            st.cache_sel.clone()
        };
        app.cache.tree.remove_all();
        for (_, name) in &caches {
            let l = label(name, "devtools-app-store");
            l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            l.set_tooltip_text(Some(name));
            app.cache.tree.append(&l);
        }
        let origin = str_field(&v, "origin");
        let summary = count_label(caches.len(), "cache");
        app.cache.frame.status(&if origin.is_empty() { summary } else { format!("{origin} \u{00B7} {summary}") }, false);
        match prev.and_then(|p| caches.iter().position(|(id, _)| *id == p)) {
            Some(idx) => {
                if let Some(row) = app.cache.tree.row_at_index(idx as i32) {
                    app.cache.tree.select_row(Some(&row));
                }
                let skip = app.st.borrow().cache_skip;
                load_cache_entries(app, skip);
            }
            None => {
                {
                    let mut st = app.st.borrow_mut();
                    st.cache_sel = None;
                    st.cache_skip = 0;
                }
                app.cache.table.list.remove_all();
                reset_pager(&app.cache.pager, "");
                app.cache.delete_cache.set_sensitive(false);
                // Same as IndexedDB: show the first cache's entries right away.
                if let Some(row) = app.cache.tree.row_at_index(0) {
                    app.cache.tree.select_row(Some(&row));
                }
            }
        }
    });
}

fn select_cache(app: &Rc<AppPanel>, id: String) {
    let changed = {
        let mut st = app.st.borrow_mut();
        let changed = st.cache_sel.as_ref() != Some(&id);
        if changed {
            st.cache_skip = 0;
        }
        st.cache_sel = Some(id);
        changed
    };
    app.cache.delete_cache.set_sensitive(true);
    if changed {
        load_cache_entries(app, 0);
    }
}

/// Load the page at `skip` of the selected cache (`cache_skip` follows on arrival).
fn load_cache_entries(app: &Rc<AppPanel>, skip: u64) {
    let sel = app.st.borrow().cache_sel.clone();
    let Some(id) = sel else { return };
    let params = json!({ "cacheId": id, "skip": skip, "limit": PAGE });
    fetch(app, Section::Cache, "cache_entries", params, move |app, v| {
        if app.st.borrow().cache_sel.as_deref() != Some(id.as_str()) {
            return;
        }
        app.st.borrow_mut().cache_skip = skip;
        let entries = v.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let total = v.get("total").and_then(|t| t.as_u64());
        app.cache.table.list.remove_all();
        for e in &entries {
            let url = str_field(e, "requestURL").to_string();
            let status = e.get("responseStatus").map(|s| preview(s, 6)).unwrap_or_default();
            let time = e
                .get("responseTime")
                .and_then(|t| t.as_f64())
                .filter(|t| *t > 0.0)
                .map(format_epoch)
                .unwrap_or_default();
            let (row, labels) = cells(
                &[&url, str_field(e, "requestMethod"), &status, str_field(e, "responseType"), &time],
                &CACHE_WIDTHS,
            );
            labels[0].set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            labels[0].set_tooltip_text(Some(&truncate(&url, 600)));
            let del = row_btn("\u{2715}", "Delete this entry");
            row.append(&del);
            let weak = Rc::downgrade(app);
            let cache_id = id.clone();
            del.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    delete_cache(&app, cache_id.clone(), Some(url.clone()));
                }
            });
            add_row(&app.cache.table.list, &row);
        }
        let name = app
            .st
            .borrow()
            .cache_rows
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        app.cache.pager.label.set_text(&page_label(skip, entries.len(), total));
        app.cache.pager.prev.set_sensitive(skip > 0);
        app.cache.pager.next.set_sensitive(total.is_some_and(|t| skip + (entries.len() as u64) < t));
        app.cache.frame.status(&name, false);
    });
}

fn page_cache(app: &Rc<AppPanel>, forward: bool) {
    let skip = app.st.borrow().cache_skip;
    load_cache_entries(app, if forward { skip + PAGE } else { skip.saturating_sub(PAGE) });
}

/// Delete one entry (`request_url`) of cache `id` or, with `None`, the whole cache.
fn delete_cache(app: &Rc<AppPanel>, id: String, request_url: Option<String>) {
    let whole = request_url.is_none();
    let mut params = json!({ "cacheId": id.clone() });
    if let Some(u) = request_url {
        params["requestURL"] = json!(u);
    }
    let socket = app.socket.clone();
    act(app, Section::Cache, move || daemon_request(&socket, "cache_delete", params, TIMEOUT), move |a| {
        if whole {
            {
                let mut st = a.st.borrow_mut();
                if st.cache_sel.as_deref() == Some(id.as_str()) {
                    st.cache_sel = None;
                }
            }
            refresh(a, Section::Cache);
        } else {
            let skip = a.st.borrow().cache_skip;
            load_cache_entries(a, skip);
        }
    });
}

// ── Service workers ────────────────────────────────────────────────────────

fn refresh_sw(app: &Rc<AppPanel>) {
    fetch(app, Section::ServiceWorkers, "service_workers", json!({}), |app, v| {
        let regs = v.get("registrations").and_then(|r| r.as_array()).cloned().unwrap_or_default();
        app.sw.list.remove_all();
        for r in &regs {
            let scope = str_field(r, "scopeURL").to_string();
            let text = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            text.set_hexpand(true);
            let scope_l = label(&scope, "devtools-mono");
            scope_l.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            scope_l.set_tooltip_text(Some(&scope));
            text.append(&scope_l);
            let mut meta = vec![str_field(r, "scriptURL").to_string()];
            for k in ["status", "runningStatus"] {
                let s = str_field(r, k);
                if !s.is_empty() {
                    meta.push(s.to_string());
                }
            }
            let meta_l = label(&meta.join(" \u{00B7} "), "devtools-loc");
            meta_l.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            text.append(&meta_l);
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            row.add_css_class("devtools-row");
            row.append(&text);
            for (title, action, tip) in [
                ("Update", "service_worker_update", "Check for an updated service worker script"),
                ("Unregister", "service_worker_unregister", "Unregister this service worker"),
            ] {
                let b = tool_btn(title, tip);
                b.set_valign(gtk4::Align::Center);
                row.append(&b);
                let weak = Rc::downgrade(app);
                let scope = scope.clone();
                b.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        let socket = app.socket.clone();
                        let scope = scope.clone();
                        act(
                            &app,
                            Section::ServiceWorkers,
                            move || daemon_request(&socket, action, json!({ "scopeURL": scope }), TIMEOUT),
                            |a| refresh(a, Section::ServiceWorkers),
                        );
                    }
                });
            }
            add_row(&app.sw.list, &row);
        }
        app.sw.frame.status(&count_label(regs.len(), "registration"), false);
    });
}

// ── Manifest ───────────────────────────────────────────────────────────────

fn refresh_manifest(app: &Rc<AppPanel>) {
    fetch(app, Section::Manifest, "manifest", json!({}), |app, v| {
        let url = str_field(&v, "url");
        let errors: Vec<String> = v
            .get("errors")
            .and_then(|e| e.as_array())
            .into_iter()
            .flatten()
            .map(|e| preview(e, 2000))
            .collect();
        let data = v.get("data").filter(|d| !d.is_null());
        app.manifest.info.set_text(&if url.is_empty() {
            "No manifest linked from this page".to_string()
        } else {
            format!("Manifest: {url}")
        });
        app.manifest.errors.set_visible(!errors.is_empty());
        app.manifest.errors.set_text(
            &errors.iter().map(|e| format!("\u{2716} {e}")).collect::<Vec<_>>().join("\n"),
        );
        let text = data.map(|d| serde_json::to_string_pretty(d).unwrap_or_default()).unwrap_or_default();
        app.manifest.json.set_visible(data.is_some());
        app.manifest.json.buffer().set_text(&text);
        if !errors.is_empty() {
            app.manifest.frame.status(&count_label(errors.len(), "error"), true);
        }
    });
}

// ── Clear site data ────────────────────────────────────────────────────────

/// Load usage/quota; `note` (if any) replaces the status once it lands.
fn refresh_site(app: &Rc<AppPanel>, note: &'static str) {
    fetch(app, Section::SiteData, "storage_usage", json!({}), move |app, v| {
        let usage = v.get("usage").and_then(|u| u.as_f64()).unwrap_or(0.0);
        let quota = v.get("quota").and_then(|q| q.as_f64()).unwrap_or(0.0);
        let frac = usage_fraction(usage, quota);
        app.site.origin.set_text(str_field(&v, "origin"));
        app.site.bar.set_value(frac);
        app.site.usage.set_text(&format!(
            "{} used of {} storage quota ({:.2}%)",
            format_bytes(usage),
            format_bytes(quota),
            frac * 100.0
        ));
        let parts: Vec<String> = v
            .get("breakdown")
            .and_then(|b| b.as_array())
            .into_iter()
            .flatten()
            .filter_map(|b| {
                let u = b.get("usage").and_then(|u| u.as_f64()).unwrap_or(0.0);
                (u > 0.0).then(|| format!("{}  {}", str_field(b, "storageType"), format_bytes(u)))
            })
            .collect();
        app.site.breakdown.set_text(&parts.join("\n"));
        app.site.breakdown.set_visible(!parts.is_empty());
        if !note.is_empty() {
            app.site.frame.status(note, false);
        }
    });
}

fn clear_site(app: &Rc<AppPanel>, types: String) {
    let socket = app.socket.clone();
    app.site.frame.status("Clearing\u{2026}", false);
    act(
        app,
        Section::SiteData,
        move || daemon_request(&socket, "clear_site_data", json!({ "types": types }), TIMEOUT),
        |a| refresh_site(a, "Site data cleared"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_section_args() {
        assert_eq!(parse_app_arg("application"), Some(None));
        assert_eq!(parse_app_arg("application:cookies"), Some(Some(Section::Cookies)));
        assert_eq!(parse_app_arg("application:sw"), Some(Some(Section::ServiceWorkers)));
        assert_eq!(parse_app_arg("application:IDB"), Some(Some(Section::IndexedDb)));
        assert_eq!(parse_app_arg("application:bogus"), Some(None));
        assert_eq!(parse_app_arg("applications"), None);
        assert_eq!(parse_app_arg("console"), None);
        for s in Section::ALL {
            assert_eq!(Section::parse(s.name()), Some(s));
            assert_eq!(Section::ALL[s.index() as usize], s);
        }
    }

    #[test]
    fn detects_unsupported_actions() {
        assert!(is_unsupported("Not yet implemented: indexeddb_databases"));
        assert!(is_unsupported("Unknown action: cache_list"));
        assert!(!is_unsupported("Browser not launched"));
    }

    #[test]
    fn formats_bytes_and_dates() {
        assert_eq!(format_bytes(0.0), "0 B");
        assert_eq!(format_bytes(1023.0), "1023 B");
        assert_eq!(format_bytes(1536.0), "1.5 kB");
        assert_eq!(format_bytes(3.0 * 1024.0 * 1024.0 * 1024.0), "3.0 GB");
        assert_eq!(format_bytes(-1.0), "");
        assert_eq!(format_epoch(0.0), "1970-01-01 00:00:00");
        assert_eq!(format_epoch(1_700_000_000.5), "2023-11-14 22:13:20");
        assert_eq!(format_epoch(951_782_400.0), "2000-02-29 00:00:00");
        assert_eq!(usage_fraction(50.0, 200.0), 0.25);
        assert_eq!(usage_fraction(5.0, 0.0), 0.0);
    }

    #[test]
    fn formats_cookie_columns() {
        let session = json!({"name": "a", "value": "bcd", "expires": -1, "session": true});
        assert_eq!(cookie_expires(&session), "Session");
        assert_eq!(cookie_size(&session), 4);
        let persistent = json!({"expires": 1_700_000_000.0, "session": false, "size": 9});
        assert_eq!(cookie_expires(&persistent), "2023-11-14 22:13:20");
        assert_eq!(cookie_size(&persistent), 9);
    }

    #[test]
    fn builds_cookie_params() {
        let host_only = json!({
            "name": "sid", "value": "old", "domain": "127.0.0.1", "path": "/app",
            "expires": -1, "session": true, "httpOnly": true, "secure": false, "sameSite": "Lax"
        });
        let p = cookie_set_params(&host_only, "new");
        assert_eq!(p["name"], "sid");
        assert_eq!(p["value"], "new");
        assert_eq!(p["url"], "http://127.0.0.1/app");
        assert_eq!(p["path"], "/app");
        assert!(p.get("domain").is_none());
        assert!(p.get("expires").is_none());
        assert_eq!(p["httpOnly"], true);
        assert!(p.get("secure").is_none());
        assert_eq!(p["sameSite"], "Lax");

        let domain = json!({
            "name": "t", "value": "1", "domain": ".example.com", "path": "/",
            "expires": 1_800_000_000.0, "session": false, "secure": true
        });
        let p = cookie_set_params(&domain, "2");
        assert_eq!(p["domain"], ".example.com");
        assert!(p.get("url").is_none());
        assert_eq!(p["expires"], 1_800_000_000.0);
        assert_eq!(p["secure"], true);

        assert_eq!(
            cookie_delete_params(&host_only),
            json!({"name": "sid", "domain": "127.0.0.1", "path": "/app"})
        );
        let mut partitioned = host_only.clone();
        partitioned["partitionKey"] = json!({"topLevelSite": "http://127.0.0.1", "hasCrossSiteAncestor": false});
        assert_eq!(cookie_set_params(&partitioned, "v")["partitionKey"], partitioned["partitionKey"]);
        assert_eq!(cookie_delete_params(&partitioned)["partitionKey"], partitioned["partitionKey"]);
        let gone = cookie_expire_params(&host_only);
        assert_eq!(gone["expires"], 1);
        assert_eq!(gone["value"], "");
        assert_eq!(
            storage_remove_script("session", "a\"b"),
            "sessionStorage.removeItem(\"a\\\"b\")"
        );
        assert_eq!(storage_remove_script("local", "k"), "localStorage.removeItem(\"k\")");
    }

    #[test]
    fn previews_and_pages() {
        assert_eq!(preview(&json!("line1\nline2"), 100), "line1 line2");
        assert_eq!(preview(&json!({"a": 1}), 100), "{\"a\":1}");
        assert_eq!(preview(&json!(42), 100), "42");
        assert_eq!(preview(&Value::Null, 100), "null");
        // Char-based: never splits a multi-byte (Korean) character.
        assert_eq!(preview(&json!("한글값입니다"), 3), "한글값…");
        assert_eq!(page_label(0, 0, None), "No records");
        assert_eq!(page_label(0, 50, None), "1\u{2013}50");
        assert_eq!(page_label(50, 23, Some(73)), "51\u{2013}73 of 73");
        assert_eq!(count_label(1, "item"), "1 item");
        assert_eq!(count_label(2, "cookie"), "2 cookies");
    }

    #[test]
    fn builds_clear_types() {
        assert_eq!(clear_types(&[("cookies", false), ("indexeddb", false)]), None);
        assert_eq!(clear_types(&[("cookies", true), ("indexeddb", true)]).as_deref(), Some("cookies,indexeddb"));
        assert_eq!(
            clear_types(&[("cookies", true), ("indexeddb", false), ("cache_storage", true)]).as_deref(),
            Some("cookies,cache_storage")
        );
    }
}
