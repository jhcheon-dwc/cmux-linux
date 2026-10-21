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
    /// Weak self-reference, handed to sidebar row callbacks (agent-status).
    self_weak: std::rc::Weak<RefCell<AppState>>,
    // socket-cli
    /// UUID of the previously active workspace (backs workspace.last).
    pub previous_workspace: Option<uuid::Uuid>,
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
        Rc::new_cyclic(|self_weak| RefCell::new(AppState {
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
            self_weak: self_weak.clone(),
            previous_workspace: None, // socket-cli
        }))
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
        self.attach_sidebar_row(&mut workspace);

        // Create surface and split engine
        let pane_id = id * 1000;
        eprintln!(
            "cmux: create_workspace calling create_surface for workspace_id={}, pane_id={}",
            id, pane_id
        );
        // pane-nav: start in the focused terminal's working directory.
        let pane_uuid = uuid::Uuid::new_v4();
        let config = crate::split_engine::local_pane_config(
            self.active_split_engine().and_then(|e| e.active_terminal_source()),
            crate::ghostty::ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_TAB,
        );
        let ids = crate::ghostty::surface::PaneIds { workspace: workspace.uuid, surface: pane_uuid };
        let (gl_area, surface_cell) =
            crate::ghostty::surface::create_surface(&self.gtk_app, self.ghostty_app, Some(config), pane_id, crate::ghostty::surface::SurfaceIoMode::Exec, ids);
        let engine = SplitEngine::new(
            self.gtk_app.clone(),
            self.ghostty_app,
            gl_area.clone(),
            surface_cell,
            pane_id,
            ids,
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

        // Build split tree from session data (D-05)
        let engine = crate::split_engine::SplitEngine::from_data(
            self.gtk_app.clone(),
            self.ghostty_app,
            &ws.layout,
            ws.active_pane_uuid.as_deref(),
            workspace.uuid,
        )?;

        // Sidebar row only once the tree is valid, so a failed restore leaves no orphan row.
        self.attach_sidebar_row(&mut workspace);

        // Add to stack
        let page_name = format!("workspace-{}", id);
        self.stack.add_named(&engine.root_widget(), Some(&page_name));
        workspace.stack_page_name = page_name;

        self.workspaces.push(workspace);
        self.split_engines.push(engine);

        Some(id)
    }

    /// Build the sidebar row for `workspace` (shared by local, restored and SSH
    /// workspaces), append it to the sidebar list and wire its close button and
    /// context menu.
    fn attach_sidebar_row(&self, workspace: &mut Workspace) {
        let row = crate::sidebar::SidebarRow::new(workspace);
        self.sidebar_list.append(&row.row);
        crate::sidebar::wire_row(&row, self.self_weak.clone(), &self.gtk_app);
        workspace.sidebar_row = Some(row);
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

        let mut workspace = Workspace::new_remote(id, display_number, target);
        self.attach_sidebar_row(&mut workspace);
        let ids = crate::ghostty::surface::PaneIds::new(workspace.uuid);

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
            ids,
        );
        let engine = SplitEngine::new(
            self.gtk_app.clone(),
            self.ghostty_app,
            gl_area,
            surface_cell,
            pane_id,
            ids,
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
            if let Some(row) = &self.workspaces[idx].sidebar_row {
                row.set_connection_state(&state);
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

        // Resolve this workspace's Ghostty surfaces before tearing anything down.
        // Leaf `surface` pointers are often null (only filled after a session
        // restore), so look surfaces up through GL_TO_SURFACE by GLArea; otherwise
        // they were never freed and their shells kept running.
        //
        // Order matters, as in SplitEngine::close_active: detach each surface from
        // its GLArea and drop the GLArea from the registries first, so the events
        // GTK delivers while the widgets are removed (unrealize, focus-leave) are
        // no-ops, and only free the surfaces after the widgets are gone.
        let mut surfaces_to_free: Vec<crate::ghostty::ffi::ghostty_surface_t> = Vec::new();
        if let Some(engine) = self.split_engines.get(index) {
            let mut areas = Vec::new();
            engine.root.collect_gl_areas(&mut areas);
            let mut leaf_surfaces = Vec::new();
            engine.root.collect_surfaces(&mut leaf_surfaces);
            for area in &areas {
                let ptr = area.as_ptr() as usize;
                let surface = crate::ghostty::callbacks::GL_TO_SURFACE
                    .lock()
                    .ok()
                    .and_then(|mut m| m.remove(&ptr));
                if let Ok(mut gl_areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                    gl_areas.retain(|p| p.0 as usize != ptr);
                }
                crate::ghostty::surface::detach_surface(area);
                if let Some(s) = surface {
                    surfaces_to_free.push(s as crate::ghostty::ffi::ghostty_surface_t);
                }
            }
            for s in leaf_surfaces {
                if !s.is_null() && !surfaces_to_free.contains(&s) {
                    surfaces_to_free.push(s);
                }
            }
        }
        self.split_engines.remove(index);

        let workspace = self.workspaces.remove(index);

        // Remove sidebar row.
        if let Some(row) = &workspace.sidebar_row {
            self.sidebar_list.remove(&row.row);
        }

        // Remove GtkStack page.
        if let Some(child) = self.stack.child_by_name(&workspace.stack_page_name) {
            self.stack.remove(&child);
        }

        // Widgets are gone and detached; now free the surfaces (ends their shells).
        for surface in surfaces_to_free {
            unsafe {
                crate::ghostty::ffi::ghostty_surface_free(surface);
            }
            if let Ok(mut reg) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
                reg.remove(&(surface as usize));
            }
            // agent-status
            if let Ok(mut map) = crate::ghostty::callbacks::SURFACE_WORKSPACE.lock() {
                map.remove(&(surface as usize));
            }
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
        // socket-cli: remember the previously active workspace for workspace.last.
        if index != self.active_index {
            self.previous_workspace = self.workspaces.get(self.active_index).map(|ws| ws.uuid);
        }
        // Phase 4: clear attention when user switches to a workspace (D-05).
        self.clear_workspace_attention(index);
        self.switch_to_index_keep_unread(index);
    }

    /// agent-status: switch like `switch_to_index` without marking the workspace read
    /// (used when a sidebar row's context menu is opened, which is not a visit).
    pub fn switch_to_index_keep_unread(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        self.active_index = index;
        let page_name = self.workspaces[index].stack_page_name.clone();
        self.stack.set_visible_child_name(&page_name);
        // Update CSS classes: active row gets "active-workspace" for styling.
        for (i, ws) in self.workspaces.iter().enumerate() {
            if let Some(row) = &ws.sidebar_row {
                row.set_active(i == index);
            }
        }
        if let Some(row) = &self.workspaces[index].sidebar_row {
            self.sidebar_list.select_row(Some(&row.row));
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
        if let Some(id) = self.workspaces.get(self.active_index).map(|ws| ws.id) {
            self.rename_workspace(id, new_name);
        }
    }

    /// Rename the workspace with internal id `workspace_id`. Only the name label
    /// changes; status badge, progress and attention indicators are preserved.
    pub fn rename_workspace(&mut self, workspace_id: u64, new_name: String) {
        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
            ws.rename(new_name);
            if let Some(row) = &ws.sidebar_row {
                row.set_name(&ws.name);
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
        if let Some(ws) = self.workspaces.get(index) {
            if let Some(row) = &ws.sidebar_row {
                row.set_attention(ws.has_attention);
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
                                tabs: Vec::new(), // tabs
                                active_tab: 0,
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
                    tab_titles: {
                        // tabs: only names of tabs that still exist.
                        let live: std::collections::HashSet<uuid::Uuid> = self
                            .split_engines
                            .iter()
                            .flat_map(|e| e.all_surfaces().into_iter().map(|s| s.uuid))
                            .collect();
                        crate::tabs::custom_titles_for_session(|u| live.contains(u))
                    },
                };
                let _ = tx.send(session);
            }
            notify.notify_one();
        }
    }

    // agent-status
    /// Mark the workspace at `index` read (clears attention) or unread (shows the dot).
    pub fn set_workspace_unread(&mut self, index: usize, unread: bool) {
        if !unread {
            self.clear_workspace_attention(index);
            return;
        }
        if let Some(ws) = self.workspaces.get_mut(index) {
            ws.has_attention = true;
        }
        self.update_sidebar_attention(index);
    }

    /// Index of the workspace with UUID `workspace_id` or containing surface `surface_id`
    /// (workspace takes precedence when both are given).
    pub fn find_workspace_for_status(
        &self,
        workspace_id: Option<&str>,
        surface_id: Option<&str>,
    ) -> Option<usize> {
        if let Some(wid) = workspace_id.filter(|s| !s.is_empty()) {
            return self.workspaces.iter().position(|ws| ws.uuid.to_string() == wid);
        }
        let sid = surface_id.filter(|s| !s.is_empty())?;
        self.split_engines.iter().position(|engine| {
            // tabs: every tab's surface, not only each pane's visible one.
            engine.all_surfaces().iter().any(|sf| sf.uuid.to_string() == sid)
        })
    }

    /// Set or clear the agent status of the workspace at `index` and refresh its row.
    pub fn set_workspace_status(
        &mut self,
        index: usize,
        update: crate::agent_status::StatusUpdate,
        message: Option<&str>,
    ) {
        use crate::agent_status::{normalize_message, AgentStatus, StatusUpdate};
        let Some(ws) = self.workspaces.get_mut(index) else { return };
        ws.agent_status = match update {
            StatusUpdate::Set(state) => Some(AgentStatus { state, message: normalize_message(message) }),
            StatusUpdate::Clear => None,
        };
        if let Some(row) = &ws.sidebar_row {
            row.set_agent_status(ws.agent_status.as_ref());
        }
    }

    /// Apply queued OSC 9;4 progress reports (from callbacks::PROGRESS_QUEUE) to the
    /// rows of the workspaces owning the reporting surfaces, and drop reports that
    /// have not been refreshed for a while (a program may exit without removing it).
    pub fn apply_progress_reports(&mut self, events: Vec<crate::ghostty::callbacks::ProgressEvent>) {
        use crate::agent_status::ProgressState;
        const STALE: std::time::Duration = std::time::Duration::from_secs(15);
        let now = std::time::Instant::now();
        for ev in events {
            let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.uuid == ev.workspace) else {
                continue;
            };
            // Each pane keeps its own report, so one pane's Remove does not hide another's bar.
            ws.progress_reports.retain(|r| r.0 != ev.surface);
            if ev.state != ProgressState::Remove {
                ws.progress_reports.push((ev.surface, ev.state, ev.progress, now));
            }
        }
        for ws in self.workspaces.iter_mut() {
            ws.progress_reports.retain(|r| now.duration_since(r.3) < STALE);
            // Show the most recently updated report (pushed last). Without one, an
            // agent that is running gets an indeterminate bar: Claude Code only
            // sends OSC 9;4 for some long operations, not while it is responding.
            let shown = crate::agent_status::displayed_progress(
                ws.progress_reports.last().map(|r| (r.1, r.2)),
                ws.agent_status.as_ref().map(|s| s.state),
            );
            if shown != ws.progress {
                ws.progress = shown;
                if let Some(row) = &ws.sidebar_row {
                    row.set_progress(shown);
                }
            } else if matches!(shown, Some((ProgressState::Indeterminate, _))) {
                // Animate indeterminate bars on each timer tick.
                if let Some(row) = &ws.sidebar_row {
                    row.progress.pulse();
                }
            }
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
        SplitNode::Leaf { pane_id, gl_area, surface, tabs, .. } => {
            pred(gl_area.as_ptr() as usize, *surface as usize)
                .then_some(*pane_id)
                // tabs: background tabs ring and notify too.
                .or_else(|| {
                    tabs.others
                        .iter()
                        .find(|t| pred(t.gl_area.as_ptr() as usize, t.surface as usize))
                        .map(|t| t.pane_id)
                })
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
