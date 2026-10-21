use gtk4::ffi;
use gtk4::prelude::{GLAreaExt, WidgetExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

// A wrapper around a raw pointer to a GtkGLArea to mark it as Send+Sync.
// This is safe because we only ever access the pointer on the GLib main thread,
// inside glib::idle_add_once closures.
#[derive(Copy, Clone)]
pub(crate) struct GtkGLAreaPtr(pub(crate) *mut ffi::GtkGLArea);
unsafe impl Send for GtkGLAreaPtr {}
unsafe impl Sync for GtkGLAreaPtr {}

/// Coalesces burst wakeup calls into a single GLib idle dispatch.
/// GLib's idle_add does not deduplicate — this flag prevents queueing N
/// idle tasks when Ghostty fires N wakeups in a single frame burst.
pub static WAKEUP_PENDING: AtomicBool = AtomicBool::new(false);

/// The GhosttyApp handle — stored as usize to be Send across the idle closure.
/// Safety: ghostty_app_t is opaque void* and only called from the GLib main thread.
pub static APP_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The GhostttySurface handle — stored so read_clipboard_cb can complete paste requests.
/// Safety: ghostty_surface_t is opaque void* and only accessed from the GLib main thread.
pub static SURFACE_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Registry of all live GLArea instances. wakeup_cb iterates this to queue_render all panes.
/// Stores raw pointers because gtk4::GLArea is not Send/Sync. The pointers are only
/// dereferenced on the main thread inside glib::idle_add_once closures.
pub static GL_AREA_REGISTRY: Mutex<Vec<GtkGLAreaPtr>> = Mutex::new(Vec::new());

/// Maps surface_ptr (as usize) → pane_id for close_surface_cb routing.
pub static SURFACE_REGISTRY: LazyLock<Mutex<HashMap<usize, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// GLArea pointer (as usize) of the terminal the user last clicked, or 0. Set by the
/// click handler in surface.rs; the main-loop timer in main.rs makes it the active pane.
pub static CLICK_FOCUS_GL_AREA: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
/// Bells and OSC 9/777 notifications waiting for the main-loop timer in main.rs, which
/// resolves each source surface to its workspace/pane and hands it to AppState::notify.
/// action_cb only queues: it runs inside ghostty_app_tick, where AppState may be borrowed.
pub static PENDING_NOTIFICATIONS: Mutex<Vec<PendingNotification>> = Mutex::new(Vec::new());

pub struct PendingNotification {
    /// ghostty_surface_t (as usize) that raised the notification.
    pub surface: usize,
    pub kind: crate::app_state::NotificationKind,
    pub title: String,
    pub body: String,
}

fn queue_notification(target: crate::ghostty::ffi::ghostty_target_s, n: PendingNotification) {
    if target.tag != crate::ghostty::ffi::ghostty_target_tag_e_GHOSTTY_TARGET_SURFACE {
        return;
    }
    let surface = unsafe { target.target.surface } as usize;
    if let Ok(mut q) = PENDING_NOTIFICATIONS.lock() {
        // Coalesce bell storms: one pending bell per surface per timer tick.
        let bell = crate::app_state::NotificationKind::Bell;
        if n.kind == bell && q.iter().any(|p| p.kind == bell && p.surface == surface) {
            return;
        }
        q.push(PendingNotification { surface, ..n });
    }
}

/// Called by Ghostty from its renderer thread. Must not call any ghostty_* API inline.
/// Instead, schedules ghostty_app_tick() on the GLib main loop (per D-04, GHOST-07).
/// Wakeup count for diagnostic logging (only logs occasionally to avoid spam)
static WAKEUP_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub unsafe extern "C" fn wakeup_cb(_userdata: *mut std::ffi::c_void) {
    // Swap: if already pending, another idle task is queued — skip.
    if WAKEUP_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    glib::idle_add_once(|| {
        WAKEUP_PENDING.store(false, Ordering::SeqCst);

        // Log wakeup occasionally to verify it's firing
        let count = WAKEUP_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        if count % 60 == 1 {
            eprintln!("cmux: wakeup_cb #{}", count);
        }

        let app_ptr = APP_PTR.load(Ordering::SeqCst);
        if app_ptr != 0 {
            unsafe {
                let app = app_ptr as crate::ghostty::ffi::ghostty_app_t;
                crate::ghostty::ffi::ghostty_app_tick(app);
            }
        }
        // queue_render on ALL registered GLAreas
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            for area_ptr in areas.iter() {
                let area: glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { glib::translate::from_glib_borrow(area_ptr.0) };
                if area.is_realized() {
                    area.queue_render();
                }
            }
        }
    });
}

/// Maps GLArea raw pointer (as usize) → surface pointer (as usize).
/// Used by notify::position handler to restore Ghostty focus after divider drag.
pub static GL_TO_SURFACE: LazyLock<Mutex<HashMap<usize, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Called by Ghostty when a surface wants to close (e.g. shell exits).
/// Runs on the GLib main thread (called during ghostty_app_tick).
/// Per D-09: no GUI dialog — exit the process.
/// The bool argument indicates whether the process was still active when closed.
pub unsafe extern "C" fn close_surface_cb(_userdata: *mut std::ffi::c_void, _process_alive: bool) {
    // Do NOT call process::exit — Phase 2 handles per-pane close gracefully.
    // Identify which pane closed via SURFACE_REGISTRY (populated at surface creation).
    // Full AppState.close_pane() dispatch is wired in Plan 04.
    // For now, log the event so the executor can verify routing works.
    eprintln!("cmux: close_surface_cb fired — per-pane close will be handled by AppState");
}

/// Action callback — Ghostty fires actions (e.g. new tab, font size changes).
/// Handles the `.render` action to trigger a GtkGLArea redraw on the main thread.
/// This is required because must_draw_from_app_thread=true in embedded.zig means
/// the renderer thread sends redraw_surface → App.redrawSurface → action_cb(.render).
/// Returns true if handled, false otherwise.
pub unsafe extern "C" fn action_cb(
    _app: crate::ghostty::ffi::ghostty_app_t,
    _target: crate::ghostty::ffi::ghostty_target_s,
    action: crate::ghostty::ffi::ghostty_action_s,
) -> bool {
    use crate::ghostty::ffi;

    // Phase 4: Bell — queue it; the main.rs timer sets pane/workspace attention.
    if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_RING_BELL {
        queue_notification(_target, PendingNotification {
            surface: 0,
            kind: crate::app_state::NotificationKind::Bell,
            title: String::new(),
            body: String::new(),
        });
        return true;
    }

    // OSC 9 / OSC 777 desktop notifications (e.g. Claude Code's "ghostty"
    // notification channel). Queued like bells so the source pane gets attention
    // and the toast can be suppressed when that pane is already focused.
    if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_DESKTOP_NOTIFICATION {
        let n = unsafe { action.action.desktop_notification };
        let to_string = |p: *const std::os::raw::c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        queue_notification(_target, PendingNotification {
            surface: 0,
            kind: crate::app_state::NotificationKind::Osc,
            title: to_string(n.title),
            body: to_string(n.body),
        });
        return true;
    }

    if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_RENDER {
        // Trigger a render on the GLArea — will call ghostty_surface_draw on main thread.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            for area_ptr in areas.iter() {
                let area: glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { glib::translate::from_glib_borrow(area_ptr.0) };
                if area.is_realized() {
                    area.queue_render();
                }
            }
        }
        return true;
    }

    // terminal-actions: links, titles and search.
    {
        let target_surface = if _target.tag == ffi::ghostty_target_tag_e_GHOSTTY_TARGET_SURFACE {
            (unsafe { _target.target.surface }) as usize
        } else {
            0
        };
        let c_str = |p: *const std::os::raw::c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };

        // Links (ctrl+click). Always report handled: returning false would make
        // Ghostty fall back to its own opener, bypassing our scheme checks.
        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_OPEN_URL {
            let u = unsafe { action.action.open_url };
            if !u.url.is_null() && u.len > 0 {
                let bytes = unsafe { std::slice::from_raw_parts(u.url as *const u8, u.len as usize) };
                let url = String::from_utf8_lossy(bytes);
                match crate::url_opener::open_url(&url) {
                    Ok(argv) => eprintln!("cmux: open_url via {}: {url}", argv[0]),
                    Err(e) => eprintln!("cmux: open_url rejected ({e}): {url:?}"),
                }
            }
            return true;
        }

        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_SET_TITLE {
            if target_surface != 0 {
                let title = c_str(unsafe { action.action.set_title }.title);
                crate::header_bar::on_set_title(target_surface, &title);
            }
            return true;
        }

        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_START_SEARCH {
            if target_surface == 0 {
                return false;
            }
            let needle = c_str(unsafe { action.action.start_search }.needle);
            crate::find_bar::on_start_search(target_surface, &needle);
            return true;
        }

        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_END_SEARCH {
            if target_surface != 0 {
                crate::find_bar::on_end_search(target_surface);
            }
            return true;
        }

        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_SEARCH_TOTAL {
            let total = unsafe { action.action.search_total }.total;
            crate::find_bar::on_search_total(
                target_surface,
                crate::find_bar::count_from_ssize(total as isize),
            );
            return true;
        }

        if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_SEARCH_SELECTED {
            let selected = unsafe { action.action.search_selected }.selected;
            crate::find_bar::on_search_selected(
                target_surface,
                crate::find_bar::count_from_ssize(selected as isize),
            );
            return true;
        }
    }

    // agent-status: OSC 9;4 progress reports. Queue for the main-loop timer in main.rs,
    // which shows them on the sidebar row of the workspace owning the surface.
    if action.tag == ffi::ghostty_action_tag_e_GHOSTTY_ACTION_PROGRESS_REPORT {
        if _target.tag == ffi::ghostty_target_tag_e_GHOSTTY_TARGET_SURFACE {
            let surface_ptr = unsafe { _target.target.surface } as usize;
            let report = unsafe { action.action.progress_report };
            let workspace = SURFACE_WORKSPACE.lock().ok().and_then(|m| m.get(&surface_ptr).copied());
            if let (Some(workspace), Some(state)) = (workspace, progress_state_from_ffi(report.state)) {
                if let Ok(mut q) = PROGRESS_QUEUE.lock() {
                    // Bound the queue in case the timer stalls; the drain keeps only
                    // the latest report per workspace anyway.
                    if q.len() >= 1024 {
                        q.drain(..512);
                    }
                    q.push(ProgressEvent { workspace, surface: surface_ptr, state, progress: report.progress });
                }
            }
        }
        return true;
    }
    // Phase 1 ignores all other actions — return false (unhandled)
    false
}

// agent-status
/// Maps surface_ptr (as usize) → UUID of the workspace owning it. Populated at surface
/// creation; used to route progress reports to a sidebar row.
pub static SURFACE_WORKSPACE: LazyLock<Mutex<HashMap<usize, uuid::Uuid>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// An OSC 9;4 progress report waiting to be applied on the main thread.
#[derive(Debug, Clone, Copy)]
pub struct ProgressEvent {
    pub workspace: uuid::Uuid,
    /// Reporting surface (ptr as usize): each pane keeps its own report.
    pub surface: usize,
    pub state: crate::agent_status::ProgressState,
    /// 0-100, or -1 when no percentage was reported.
    pub progress: i8,
}

/// Progress reports queued by action_cb, drained by the 100ms timer in main.rs.
/// A queue (not a single slot like BELL_PENDING) since reports arrive in rapid bursts.
pub static PROGRESS_QUEUE: Mutex<Vec<ProgressEvent>> = Mutex::new(Vec::new());

fn progress_state_from_ffi(
    state: crate::ghostty::ffi::ghostty_action_progress_report_state_e,
) -> Option<crate::agent_status::ProgressState> {
    use crate::agent_status::ProgressState;
    use crate::ghostty::ffi;
    #[allow(non_upper_case_globals)]
    match state {
        ffi::ghostty_action_progress_report_state_e_GHOSTTY_PROGRESS_STATE_REMOVE => Some(ProgressState::Remove),
        ffi::ghostty_action_progress_report_state_e_GHOSTTY_PROGRESS_STATE_SET => Some(ProgressState::Set),
        ffi::ghostty_action_progress_report_state_e_GHOSTTY_PROGRESS_STATE_ERROR => Some(ProgressState::Error),
        ffi::ghostty_action_progress_report_state_e_GHOSTTY_PROGRESS_STATE_INDETERMINATE => Some(ProgressState::Indeterminate),
        ffi::ghostty_action_progress_report_state_e_GHOSTTY_PROGRESS_STATE_PAUSE => Some(ProgressState::Pause),
        _ => None,
    }
}
