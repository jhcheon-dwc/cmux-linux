use crate::ghostty::ffi;
use crate::split_engine::SplitEngine;
use crate::workspace::{ConnectionState, Workspace};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub type AppStateRef = Rc<RefCell<AppState>>;

pub struct AppState {
    pub split_engines: Vec<SplitEngine>,
    pub gtk_app: gtk4::Application,
    /// All open workspaces. Never empty after initialization — create_workspace is called in new().
    pub workspaces: Vec<Workspace>,
    /// Index into workspaces of the currently visible workspace.
    pub active_index: usize,
    /// GtkStack holding one page per workspace (the workspace's root GTK widget).
    pub stack: gtk4::Stack,
    /// GtkListBox in the sidebar showing workspace names.
    pub sidebar_list: gtk4::ListBox,
    /// Ghostty app handle — used by create_surface() for new panes.
    pub ghostty_app: ffi::ghostty_app_t,
    /// Next workspace ID (monotonically increasing).
    next_id: u64,
    /// Next display number for default names ("Workspace N").
    next_display_number: usize,
    /// Notified after any workspace/pane mutation to trigger a debounced session save.
    pub save_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Sender for session snapshots to the debounce task.
    /// Each mutation snapshots SessionData on the main thread and sends it here.
    pub session_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::session::SessionData>>,
    /// Sender for SSH events (cloned into SSH lifecycle tokio tasks).
    pub ssh_event_tx: Option<crate::ssh::SshEventTx>,
    /// Tokio runtime handle for spawning SSH lifecycle tasks.
    pub runtime_handle: Option<tokio::runtime::Handle>,
    /// Handles to SSH lifecycle tasks, keyed by workspace id. Used for cleanup on close.
    pub ssh_task_handles: std::collections::HashMap<u64, tokio::task::JoinHandle<()>>,
    /// Maps pane_id -> IoWriteContext for remote panes (needed to set stream_id after proxy.open).
    pub remote_pane_contexts: std::collections::HashMap<u64, std::sync::Arc<crate::ssh::bridge::IoWriteContext>>,
    /// Maps workspace_id -> SshBridge for remote workspaces.
    pub workspace_bridges: std::collections::HashMap<u64, std::sync::Arc<crate::ssh::bridge::SshBridge>>,
    /// Browser preview daemon manager (Phase 8).
    pub browser_manager: Option<crate::browser::BrowserManager>,
    /// Next browser surface short-ref counter (monotonically increasing, per D-06).
    pub browser_surface_counter: u32,
    /// Maps short-ref ID -> surface UUID (lost on restart, per D-06).
    pub browser_surface_refs: std::collections::HashMap<u32, String>,
}

impl AppState {
    /// Create a new AppState. Does NOT create the first workspace — caller must call
    /// create_workspace() after constructing the GTK widget tree (Plan 04 wires this).
    pub fn new(
        stack: gtk4::Stack,
        sidebar_list: gtk4::ListBox,
        ghostty_app: ffi::ghostty_app_t,
        gtk_app: gtk4::Application,
    ) -> AppStateRef {
        let state = AppState {
            workspaces: Vec::new(),
            split_engines: Vec::new(),
            active_index: 0,
            stack,
            sidebar_list,
            ghostty_app,
            gtk_app,
            next_id: 1,
            next_display_number: 1,
            save_notify: None, // Set to Some(...) after tokio runtime is available in main.rs
            session_tx: None,
            ssh_event_tx: None,
            runtime_handle: None,
            ssh_task_handles: std::collections::HashMap::new(),
            remote_pane_contexts: std::collections::HashMap::new(),
            workspace_bridges: std::collections::HashMap::new(),
            browser_manager: None,
            browser_surface_counter: 0,
            browser_surface_refs: std::collections::HashMap::new(),
        };
        Rc::new(RefCell::new(state))
    }

    /// Create a new workspace. Allocates an ID, creates a sidebar row, and adds a placeholder
    /// page to the GtkStack. The actual GLArea/split root is added by the caller (Plan 04).
    /// Returns the new workspace id.
    pub fn create_workspace(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let display_number = self.next_display_number;
        self.next_display_number += 1;

        let mut workspace = Workspace::new(id, display_number);
        let name = workspace.name.clone();

        // Phase 9: Use shared row builder for consistent layout including close button
        let hbox = crate::sidebar::rebuild_sidebar_row_content(&name);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        unsafe {
            row.set_data("workspace-id", id);
        }
        self.sidebar_list.append(&row);

        // Create surface and split engine
        let pane_id = id * 1000;
        eprintln!(
            "cmux: create_workspace calling create_surface for workspace_id={}, pane_id={}",
            id, pane_id
        );
        let (gl_area, surface_cell) =
            crate::ghostty::surface::create_surface(&self.gtk_app, self.ghostty_app, None, pane_id, crate::ghostty::surface::SurfaceIoMode::Exec);
        let engine = SplitEngine::new(
            self.gtk_app.clone(),
            self.ghostty_app,
            gl_area.clone(),
            surface_cell,
            pane_id,
        );

        // Add to stack
        let page_name = format!("workspace-{}", id);
        self.stack
            .add_named(&engine.root_widget(), Some(&page_name));
        workspace.stack_page_name = page_name;

        self.workspaces.push(workspace);
        self.split_engines.push(engine);

        let new_index = self.workspaces.len() - 1;
        self.switch_to_index(new_index);

        self.trigger_session_save();
        id
    }

    /// Restore a workspace from a session snapshot (SESS-02).
    /// Creates sidebar row, uses SplitEngine::from_data() for full tree restore.
    /// Returns the workspace id, or None if tree is invalid (D-14 depth limit).
    pub fn restore_workspace(&mut self, ws: &crate::session::WorkspaceSession) -> Option<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let display_number = self.next_display_number;
        self.next_display_number += 1;

        let mut workspace = Workspace::new(id, display_number);
        workspace.name = ws.name.clone();

        // Phase 9: Use shared row builder for consistent layout including close button
        let hbox = crate::sidebar::rebuild_sidebar_row_content(&workspace.name);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        unsafe { row.set_data("workspace-id", id); }
        self.sidebar_list.append(&row);

        // Build split tree from session data (D-05)
        let engine = crate::split_engine::SplitEngine::from_data(
            self.gtk_app.clone(),
            self.ghostty_app,
            &ws.layout,
            ws.active_pane_uuid.as_deref(),
        )?;

        // Add to stack
        let page_name = format!("workspace-{}", id);
        self.stack.add_named(&engine.root_widget(), Some(&page_name));
        workspace.stack_page_name = page_name;

        self.workspaces.push(workspace);
        self.split_engines.push(engine);

        Some(id)
    }

    /// Build a sidebar row for a workspace. Used by create_workspace and create_remote_workspace.
    fn build_sidebar_row(&self, workspace: &Workspace) -> gtk4::ListBoxRow {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let label = gtk4::Label::new(Some(&workspace.name));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        vbox.append(&label);

        // SSH connection state subtitle (only for remote workspaces)
        if workspace.connection_state.is_remote() {
            let status = gtk4::Label::new(Some(workspace.connection_state.display_text()));
            status.set_halign(gtk4::Align::Start);
            status.add_css_class("connection-state");
            status.add_css_class(workspace.connection_state.css_class());
            vbox.append(&status);
        }
        vbox.set_hexpand(true);
        hbox.append(&vbox);

        // Attention dot — hidden by default, shown when has_attention
        let dot = gtk4::Label::new(None);
        dot.add_css_class("attention-dot");
        dot.set_visible(false);
        hbox.append(&dot);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        unsafe {
            row.set_data("workspace-id", workspace.id);
        }
        row
    }

    /// Create a remote SSH workspace. Returns workspace id.
    /// The bridge is used to create an IoWriteContext for the initial pane's manual I/O mode surface.
    pub fn create_remote_workspace(
        &mut self,
        target: String,
        bridge: &std::sync::Arc<crate::ssh::bridge::SshBridge>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let display_number = self.next_display_number;
        self.next_display_number += 1;

        let workspace = Workspace::new_remote(id, display_number, target);
        let row = self.build_sidebar_row(&workspace);
        self.sidebar_list.append(&row);

        // Create remote surface with manual I/O mode
        let pane_id = id * 1000;
        let io_ctx = std::sync::Arc::new(crate::ssh::bridge::IoWriteContext {
            pane_id,
            write_tx: bridge.clone_write_tx(),
            stream_id: std::sync::Mutex::new(None),
            eof_received: std::sync::atomic::AtomicBool::new(false),
            ssh_tx: self.ssh_event_tx.clone().expect("ssh_event_tx must be set before creating remote workspaces"),
        });
        let (gl_area, surface_cell) = crate::ghostty::surface::create_surface(
            &self.gtk_app,
            self.ghostty_app,
            None,
            pane_id,
            crate::ghostty::surface::SurfaceIoMode::Manual { io_write_ctx: io_ctx.clone() },
        );
        let engine = SplitEngine::new(
            self.gtk_app.clone(),
            self.ghostty_app,
            gl_area,
            surface_cell,
            pane_id,
        );

        let page_name = workspace.stack_page_name.clone();
        self.stack
            .add_named(&engine.root_widget(), Some(&page_name));

        self.workspaces.push(workspace);
        self.split_engines.push(engine);

        // Register pane in bridge.streams so run_proxy_routing can find it
        // and open a remote stream after SSH handshake completes.
        bridge.register_pane_placeholder(pane_id);

        // Store IoWriteContext for stream_id wiring when StreamOpened arrives
        self.remote_pane_contexts.insert(pane_id, io_ctx);

        let new_index = self.workspaces.len() - 1;
        self.switch_to_index(new_index);
        self.trigger_session_save();
        id
    }

    /// Update the connection state of a workspace and refresh its sidebar row.
    pub fn update_connection_state(&mut self, workspace_id: u64, state: ConnectionState) {
        if let Some(idx) = self.workspaces.iter().position(|ws| ws.id == workspace_id) {
            self.workspaces[idx].connection_state = state.clone();
            // Update sidebar subtitle
            if let Some(row) = self.sidebar_list.row_at_index(idx as i32) {
                if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                    if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                        // Last child in vbox is the status label (if it has connection-state class)
                        if let Some(status) = vbox.last_child().and_downcast::<gtk4::Label>() {
                            if status.has_css_class("connection-state") {
                                status.set_text(state.display_text());
                                status.remove_css_class("connected");
                                status.remove_css_class("disconnected");
                                status.remove_css_class("reconnecting");
                                status.add_css_class(state.css_class());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Close the workspace at `index`. Removes the sidebar row and GtkStack page.
    /// Returns false if there is only one workspace (cannot close the last one).
    /// The caller (Plan 04) is responsible for calling ghostty_surface_free on all panes first.
    pub fn close_workspace(&mut self, index: usize) -> bool {
        if self.workspaces.len() <= 1 {
            return false; // Cannot close the last workspace
        }

        // Abort SSH lifecycle task if this is a remote workspace.
        if let Some(ws) = self.workspaces.get(index) {
            if let Some(handle) = self.ssh_task_handles.remove(&ws.id) {
                handle.abort();
            }
        }

        // Before removing from workspaces, free all Ghostty surfaces in the split engine.
        if let Some(engine) = self.split_engines.get(index) {
            let mut surfaces = Vec::new();
            engine.root.collect_surfaces(&mut surfaces);
            for surface in surfaces {
                if !surface.is_null() {
                    unsafe {
                        crate::ghostty::ffi::ghostty_surface_free(surface);
                    }
                    if let Ok(mut reg) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
                        reg.remove(&(surface as usize));
                    }
                }
            }
        }
        self.split_engines.remove(index);

        let workspace = self.workspaces.remove(index);

        // Remove sidebar row.
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            self.sidebar_list.remove(&row);
        }

        // Remove GtkStack page.
        if let Some(child) = self.stack.child_by_name(&workspace.stack_page_name) {
            self.stack.remove(&child);
        }

        // Adjust active_index: if we removed before or at active, clamp.
        if self.active_index >= self.workspaces.len() {
            self.active_index = self.workspaces.len() - 1;
        } else if index <= self.active_index && self.active_index > 0 {
            self.active_index -= 1;
        }

        self.switch_to_index(self.active_index);
        self.trigger_session_save();
        true
    }

    /// Switch to the workspace at `index` (0-based). Updates GtkStack visible child and
    /// sidebar selection. Does nothing if index is out of bounds.
    pub fn switch_to_index(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        // Phase 4: clear attention when user switches to a workspace (D-05).
        self.clear_workspace_attention(index);
        self.active_index = index;
        let page_name = self.workspaces[index].stack_page_name.clone();
        self.stack.set_visible_child_name(&page_name);
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            self.sidebar_list.select_row(Some(&row));
            // Update CSS classes: active row gets "active-workspace" for styling.
            // All rows: remove first, then add to active.
            let count = self.workspaces.len() as i32;
            for i in 0..count {
                if let Some(r) = self.sidebar_list.row_at_index(i) {
                    r.remove_css_class("active-workspace");
                    // Phase 4: navigate nested layout: row > hbox > vbox > label
                    if let Some(hbox) = r.child().and_downcast::<gtk4::Box>() {
                        if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                            if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                                label.set_css_classes(&[]);
                            }
                        }
                    }
                }
            }
            row.add_css_class("active-workspace");
            // Phase 4: navigate nested layout: row > hbox > vbox > label
            if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                    if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                        label.add_css_class("active-workspace-label");
                    }
                }
            }
        }
        // Grab GTK keyboard focus on the active pane so key events reach Ghostty.
        if let Some(engine) = self.split_engines.get(index) {
            engine.grab_active_focus();
        }
    }

    /// Switch to next workspace (wrap-around). Per D-10: Ctrl+].
    pub fn switch_next(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let next = (self.active_index + 1) % self.workspaces.len();
        self.switch_to_index(next);
    }

    /// Switch to previous workspace (wrap-around). Per D-10: Ctrl+[.
    pub fn switch_prev(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        let prev = if self.active_index == 0 {
            self.workspaces.len() - 1
        } else {
            self.active_index - 1
        };
        self.switch_to_index(prev);
    }

    pub fn active_split_engine(&self) -> Option<&SplitEngine> {
        self.split_engines.get(self.active_index)
    }

    pub fn active_split_engine_mut(&mut self) -> Option<&mut SplitEngine> {
        self.split_engines.get_mut(self.active_index)
    }

    /// Rename the active workspace. Per D-03/D-10: Ctrl+Shift+R (UI wired in Plan 04/05).
    pub fn rename_active(&mut self, new_name: String) {
        if let Some(ws) = self.workspaces.get_mut(self.active_index) {
            ws.rename(new_name.clone());
            // Update the sidebar label (Phase 4 nested layout: row > hbox > vbox > label).
            if let Some(row) = self.sidebar_list.row_at_index(self.active_index as i32) {
                if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                    if let Some(vbox) = hbox.first_child().and_downcast::<gtk4::Box>() {
                        if let Some(label) = vbox.first_child().and_downcast::<gtk4::Label>() {
                            label.set_text(&new_name);
                        }
                    }
                }
            }
            self.trigger_session_save();
        }
    }

    /// Returns the active workspace, if any.
    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.active_index)
    }

    /// Resolve a Ghostty surface pointer to (workspace index, pane_id).
    /// pane_id alone is only unique per workspace, so this goes through the GLArea:
    /// GL_TO_SURFACE (GLArea → surface) and each engine's leaves (GLArea → pane).
    /// Falls back to the leaves' surface pointers (set once the GLArea realizes).
    pub fn resolve_surface(&self, surface: usize) -> Option<(usize, u64)> {
        if surface == 0 {
            return None;
        }
        let gl_area = crate::ghostty::callbacks::GL_TO_SURFACE
            .lock()
            .ok()
            .and_then(|m| m.iter().find(|(_, &s)| s == surface).map(|(&gl, _)| gl));
        self.split_engines.iter().enumerate().find_map(|(idx, engine)| {
            find_leaf(&engine.root, &|gl, s| Some(gl) == gl_area || s == surface)
                .map(|pane_id| (idx, pane_id))
        })
    }

    /// Deliver a notification from pane `pane_id` of workspace `ws_idx` — bell, OSC 9/777
    /// or `cmux notify`: sidebar attention, desktop toast and the `[notifications] command`
    /// hook, subject to `notification_policy`. `pane_id: None` means the source pane is
    /// unknown: attention goes to the active pane and focus suppression does not apply.
    /// Returns true if the notification was delivered (not suppressed).
    pub fn notify(
        &mut self,
        ws_idx: usize,
        pane_id: Option<u64>,
        kind: NotificationKind,
        title: &str,
        body: &str,
    ) -> bool {
        if ws_idx >= self.workspaces.len() || ws_idx >= self.split_engines.len() {
            return false;
        }
        let cfg = crate::config::notifications_config();
        let active_pane = self.split_engines[ws_idx].active_pane_id;
        let window_active = self.gtk_app.active_window()
            .map(|w| w.is_active())
            .unwrap_or(false);
        let source_focused = ws_idx == self.active_index && pane_id == Some(active_pane);
        let pane_id = pane_id.unwrap_or(active_pane);
        let policy = notification_policy(kind, window_active, source_focused, cfg);

        if policy.attention {
            let engine = &mut self.split_engines[ws_idx];
            if !engine.root.set_attention(pane_id, true) {
                // Not a terminal leaf (e.g. a browser preview): mark the active pane instead.
                let active = engine.active_pane_id;
                engine.root.set_attention(active, true);
            }
            self.workspaces[ws_idx].has_attention = self.split_engines[ws_idx].root.any_attention();
            self.update_sidebar_attention(ws_idx);
        }
        if !policy.deliver {
            if kind != NotificationKind::Bell {
                eprintln!("cmux: {} notification suppressed (source pane is focused)", kind.as_str());
            }
            return false;
        }
        // Bells can arrive in bursts: at most one bell toast/hook per workspace per 5s.
        if kind == NotificationKind::Bell {
            let ws = &mut self.workspaces[ws_idx];
            if ws.last_notification.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(5)) {
                return false;
            }
            ws.last_notification = Some(std::time::Instant::now());
        }

        let ws = &self.workspaces[ws_idx];
        let ws_uuid = ws.uuid.to_string();
        let (title, body) = match kind {
            NotificationKind::Bell => ("Terminal Bell".to_string(), format!("{} - Terminal bell", ws.name)),
            _ if title.is_empty() => ("cmux".to_string(), body.to_string()),
            _ => (title.to_string(), body.to_string()),
        };
        if policy.toast {
            send_desktop_notification(&title, &body, Some(&ws_uuid));
        }
        if let Some(command) = cfg.command.as_deref().filter(|c| !c.trim().is_empty()) {
            let surface_uuid = self.split_engines[ws_idx].root.find_uuid_for_pane(pane_id)
                .unwrap_or_default();
            let env = notification_env(kind, &title, &body, &ws_uuid, &ws.name, &surface_uuid);
            run_notification_hook(command, env);
        }
        true
    }

    /// Clear all attention in the workspace at `index`.
    pub fn clear_workspace_attention(&mut self, index: usize) {
        if let Some(engine) = self.split_engines.get_mut(index) {
            engine.root.clear_all_attention();
        }
        if let Some(ws) = self.workspaces.get_mut(index) {
            ws.has_attention = false;
        }
        self.update_sidebar_attention(index);
    }

    /// Update the sidebar dot visibility for workspace at `index`.
    fn update_sidebar_attention(&self, index: usize) {
        if let Some(row) = self.sidebar_list.row_at_index(index as i32) {
            let has_attention = self.workspaces.get(index)
                .map(|ws| ws.has_attention)
                .unwrap_or(false);
            // Row layout: GtkBox(H) > [GtkBox(V) > [GtkLabel(name)], GtkLabel(dot)]
            if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                if let Some(dot) = hbox.last_child() {
                    dot.set_visible(has_attention);
                }
            }
        }
    }

    /// Shut down the agent-browser daemon if running (called on app exit).
    pub fn shutdown_browser(&mut self) {
        if let Some(ref mut bm) = self.browser_manager {
            eprintln!("cmux: shutting down browser daemon");
            bm.shutdown();
            self.browser_manager = None;
        }
    }

    /// Trigger a debounced session save. Call after any workspace/pane mutation.
    /// Snapshots SessionData on the main thread (safe for Rc) and sends to the
    /// tokio debounce task which handles the file I/O.
    pub fn trigger_session_save(&self) {
        if let Some(ref notify) = self.save_notify {
            // Snapshot session data on main thread where Rc<RefCell<AppState>> is safe.
            if let Some(ref tx) = self.session_tx {
                let session = crate::session::SessionData {
                    version: 2, // D-01: bump to version 2 for full tree serialization
                    active_index: self.active_index,
                    workspaces: self.workspaces.iter().enumerate().map(|(i, ws)| {
                        // D-02: save full split tree for ALL workspaces
                        let layout = if i < self.split_engines.len() {
                            self.split_engines[i].root.to_data()
                        } else {
                            // Fallback: shouldn't happen, but be safe
                            crate::split_engine::SplitNodeData::Leaf {
                                pane_id: 0,
                                surface_uuid: uuid::Uuid::nil(),
                                shell: String::new(),
                                cwd: String::new(),
                            }
                        };
                        // D-04: save active_pane_uuid per workspace
                        let active_pane_uuid = if i < self.split_engines.len() {
                            self.split_engines[i].active_pane_uuid()
                        } else {
                            None
                        };
                        crate::session::WorkspaceSession {
                            uuid: ws.uuid.to_string(),
                            name: ws.name.clone(),
                            active_pane_uuid,
                            layout,
                        }
                    }).collect(),
                };
                let _ = tx.send(session);
            }
            notify.notify_one();
        }
    }
}

/// Source of a notification; exported to hooks as CMUX_NOTIFICATION_KIND.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind {
    Bell,
    /// OSC 9 / OSC 777 desktop notification from a terminal program.
    Osc,
    /// `cmux notify` / `notification.create` over the socket.
    Cli,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Bell => "bell",
            NotificationKind::Osc => "osc",
            NotificationKind::Cli => "cli",
        }
    }
}

/// What to do with one notification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NotificationPolicy {
    /// Set the pane/workspace attention dot.
    pub attention: bool,
    /// Treat it as delivered: run the hook, and show the toast if `toast`.
    pub deliver: bool,
    /// Show the desktop toast.
    pub toast: bool,
}

/// Decide attention/delivery/toast for a notification.
/// Bells keep the original behavior: always mark attention, deliver only while the cmux
/// window is unfocused. OSC/CLI notifications are dropped entirely when the source pane
/// is the focused pane of the active window (if `suppress_when_focused`).
pub(crate) fn notification_policy(
    kind: NotificationKind,
    window_active: bool,
    source_focused: bool,
    cfg: &crate::config::NotificationsConfig,
) -> NotificationPolicy {
    match kind {
        NotificationKind::Bell => NotificationPolicy {
            attention: true,
            deliver: !window_active,
            toast: !window_active && cfg.enabled && cfg.bell,
        },
        NotificationKind::Osc | NotificationKind::Cli => {
            let suppressed = cfg.suppress_when_focused && window_active && source_focused;
            NotificationPolicy {
                attention: !suppressed,
                deliver: !suppressed,
                toast: !suppressed && cfg.enabled,
            }
        }
    }
}

/// Environment passed to the `[notifications] command` hook.
pub(crate) fn notification_env(
    kind: NotificationKind,
    title: &str,
    body: &str,
    workspace_id: &str,
    workspace_name: &str,
    surface_id: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("CMUX_NOTIFICATION_TITLE", title.to_string()),
        ("CMUX_NOTIFICATION_BODY", body.to_string()),
        ("CMUX_NOTIFICATION_KIND", kind.as_str().to_string()),
        ("CMUX_WORKSPACE_ID", workspace_id.to_string()),
        ("CMUX_WORKSPACE_NAME", workspace_name.to_string()),
        ("CMUX_SURFACE_ID", surface_id.to_string()),
    ]
}

/// Run the user's notification hook via `sh -c` in its own process group, without
/// blocking the main loop. A short-lived thread reaps it so it does not linger as a zombie.
fn run_notification_hook(command: &str, env: Vec<(&'static str, String)>) {
    use std::os::unix::process::CommandExt;
    let spawned = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("cmux: failed to run notification command: {e}"),
    }
}

/// Show a toast for an OSC 9/777 notification whose source surface could not be
/// resolved to a pane (e.g. closed before the timer ran), as cmux did before it
/// tracked notification sources. No attention or hook: there is no workspace.
pub(crate) fn notify_without_source(title: &str, body: &str) {
    if crate::config::notifications_config().enabled {
        let title = if title.is_empty() { "cmux" } else { title };
        send_desktop_notification(title, body, None);
    }
}

/// pane_id of the first terminal leaf for which `pred(gl_area_ptr, surface_ptr)` holds.
fn find_leaf(
    node: &crate::split_engine::SplitNode,
    pred: &dyn Fn(usize, usize) -> bool,
) -> Option<u64> {
    use crate::split_engine::SplitNode;
    match node {
        SplitNode::Leaf { pane_id, gl_area, surface, .. } => {
            pred(gl_area.as_ptr() as usize, *surface as usize).then_some(*pane_id)
        }
        SplitNode::Preview { .. } => None,
        SplitNode::Split { start, end, .. } => {
            find_leaf(start, pred).or_else(|| find_leaf(end, pred))
        }
    }
}

/// Arguments for `notify-send`. The workspace UUID goes in a `cmux-workspace` string
/// hint (ignored by notification servers that do not know it) so the WSL toast shim
/// can make the toast clickable.
pub(crate) fn notify_send_args(summary: &str, body: &str, workspace_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--app-name=cmux".to_string(),
        "--icon=utilities-terminal".to_string(),
        "--expire-time=5000".to_string(),
    ];
    if let Some(id) = workspace_id.filter(|id| !id.is_empty()) {
        args.push(format!("--hint=string:cmux-workspace:{id}"));
    }
    args.push("--".to_string());
    args.push(summary.to_string());
    args.push(body.to_string());
    args
}

/// Send a desktop notification via the `notify-send` subprocess
/// (org.freedesktop.Notifications, or whatever `notify-send` is on PATH).
///
/// We use a subprocess instead of notify-rust (zbus D-Bus client) because GNOME Shell
/// destroys notifications when the D-Bus sender name vanishes. With notify-rust in a
/// spawned thread, the zbus connection drops when the thread exits, causing GNOME Shell's
/// FdoNotificationDaemonSource._onNameVanished() to destroy the notification immediately.
/// `notify-send` avoids this because it's a separate process whose D-Bus lifetime is
/// independent of cmux.
pub(crate) fn send_desktop_notification(summary: &str, body: &str, workspace_id: Option<&str>) {
    let args = notify_send_args(summary, body, workspace_id);
    std::thread::spawn(move || {
        let result = std::process::Command::new("notify-send").args(&args).status();
        match result {
            Ok(status) if !status.success() => {
                eprintln!("cmux: notify-send exited with {status}");
            }
            Err(e) => {
                eprintln!("cmux: failed to run notify-send: {e}");
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod notification_tests {
    use super::*;
    use crate::config::NotificationsConfig;

    #[test]
    fn notification_env_has_all_keys() {
        let env = notification_env(NotificationKind::Osc, "제목", "본문", "ws-uuid", "Workspace 1", "sf-uuid");
        let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("CMUX_NOTIFICATION_TITLE"), Some("제목"));
        assert_eq!(get("CMUX_NOTIFICATION_BODY"), Some("본문"));
        assert_eq!(get("CMUX_NOTIFICATION_KIND"), Some("osc"));
        assert_eq!(get("CMUX_WORKSPACE_ID"), Some("ws-uuid"));
        assert_eq!(get("CMUX_WORKSPACE_NAME"), Some("Workspace 1"));
        assert_eq!(get("CMUX_SURFACE_ID"), Some("sf-uuid"));
        assert_eq!(NotificationKind::Bell.as_str(), "bell");
        assert_eq!(NotificationKind::Cli.as_str(), "cli");
    }

    #[test]
    fn osc_suppressed_only_when_focused_pane_in_active_window() {
        let cfg = NotificationsConfig::default();
        for kind in [NotificationKind::Osc, NotificationKind::Cli] {
            let p = notification_policy(kind, true, true, &cfg);
            assert_eq!(p, NotificationPolicy { attention: false, deliver: false, toast: false });
            // Other pane, or window in the background: delivered.
            let all = NotificationPolicy { attention: true, deliver: true, toast: true };
            assert_eq!(notification_policy(kind, true, false, &cfg), all);
            assert_eq!(notification_policy(kind, false, true, &cfg), all);
            assert_eq!(notification_policy(kind, false, false, &cfg), all);
        }
    }

    #[test]
    fn suppression_can_be_disabled() {
        let cfg = NotificationsConfig { suppress_when_focused: false, ..Default::default() };
        let p = notification_policy(NotificationKind::Osc, true, true, &cfg);
        assert_eq!(p, NotificationPolicy { attention: true, deliver: true, toast: true });
    }

    #[test]
    fn disabled_skips_toast_but_still_delivers() {
        let cfg = NotificationsConfig { enabled: false, ..Default::default() };
        let p = notification_policy(NotificationKind::Osc, false, false, &cfg);
        assert_eq!(p, NotificationPolicy { attention: true, deliver: true, toast: false });
        let p = notification_policy(NotificationKind::Bell, false, false, &cfg);
        assert!(p.attention && p.deliver && !p.toast);
    }

    #[test]
    fn bell_keeps_unfocused_window_behavior() {
        let cfg = NotificationsConfig::default();
        // Window active: attention only, even for a background pane.
        let p = notification_policy(NotificationKind::Bell, true, false, &cfg);
        assert_eq!(p, NotificationPolicy { attention: true, deliver: false, toast: false });
        let p = notification_policy(NotificationKind::Bell, false, true, &cfg);
        assert_eq!(p, NotificationPolicy { attention: true, deliver: true, toast: true });
        let cfg = NotificationsConfig { bell: false, ..Default::default() };
        assert!(!notification_policy(NotificationKind::Bell, false, true, &cfg).toast);
    }

    #[test]
    fn notify_send_args_with_workspace_hint() {
        let args = notify_send_args("-제목", "본문", Some("1234-abcd"));
        assert_eq!(args, vec![
            "--app-name=cmux", "--icon=utilities-terminal", "--expire-time=5000",
            "--hint=string:cmux-workspace:1234-abcd", "--", "-제목", "본문",
        ]);
        let args = notify_send_args("t", "", None);
        assert!(!args.iter().any(|a| a.starts_with("--hint")));
        assert_eq!(&args[args.len() - 3..], ["--", "t", ""]);
    }
}
