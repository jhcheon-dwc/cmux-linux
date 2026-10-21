// src/socket/handlers.rs — GTK main thread command dispatch

use crate::socket::commands::SocketCommand;
use gtk4::prelude::*;
use serde_json::{json, Value};

/// Build a success response with the given result payload.
fn ok(req_id: Value, result: Value) -> Value {
    json!({"id": req_id, "ok": true, "result": result})
}

/// Build an error response.
fn err(req_id: Value, code: &str, message: &str) -> Value {
    json!({"id": req_id, "ok": false, "error": {"code": code, "message": message}})
}

/// Resolve a surface_ref string ("surface:N" or UUID) to a UUID string.
/// Returns Ok(uuid_string) or Err((error_message, available_refs)).
fn resolve_surface_ref(
    surface_ref: &str,
    refs: &std::collections::HashMap<u32, String>,
) -> Result<String, (String, Vec<String>)> {
    if let Some(n_str) = surface_ref.strip_prefix("surface:") {
        if let Ok(n) = n_str.parse::<u32>() {
            if let Some(uuid) = refs.get(&n) {
                return Ok(uuid.clone());
            }
            let available: Vec<String> = refs.keys()
                .map(|k| format!("surface:{}", k))
                .collect();
            return Err((format!("surface:{} not found", n), available));
        }
    }
    // Treat as UUID directly
    Ok(surface_ref.to_string())
}

// ── socket-cli: surface resolution + terminal I/O helpers ───────────────────

// Exported by libghostty (cmux fork) but not declared in ghostty.h: committed
// typed-text input (no bracketed paste, newlines normalized to CR).
unsafe extern "C" {
    fn ghostty_surface_text_input(
        surface: crate::ghostty::ffi::ghostty_surface_t,
        text: *const std::os::raw::c_char,
        len: usize,
    );
}

/// Find (workspace index, pane_id) for a pane/surface UUID across ALL workspaces.
/// pane_id is only unique within one workspace, so callers must keep the pair.
/// Accepts a full UUID or a unique prefix (the CLI lists show 8-char short ids).
fn locate_pane(s: &crate::app_state::AppState, target: &str) -> Option<(usize, u64)> {
    let mut prefix_hit: Option<(usize, u64)> = None;
    let mut prefix_hits = 0;
    for (ws_idx, engine) in s.split_engines.iter().enumerate() {
        for (uuid, pane_id, _) in engine.all_panes() {
            let u = uuid.to_string();
            if u == target {
                return Some((ws_idx, pane_id));
            }
            if target.len() >= 4 && u.starts_with(target) {
                prefix_hit = Some((ws_idx, pane_id));
                prefix_hits += 1;
            }
        }
    }
    if prefix_hits == 1 { prefix_hit } else { None }
}

/// Resolve the target pane: an explicit UUID (any workspace) or the focused pane
/// of the active workspace.
fn resolve_pane(s: &crate::app_state::AppState, id: Option<&str>) -> Result<(usize, u64), String> {
    match id {
        Some(target) => locate_pane(s, target).ok_or_else(|| format!("surface {target} not found")),
        None => {
            let engine = s.split_engines.get(s.active_index).ok_or("no active workspace")?;
            let pid = engine.root.find_active_pane_id().unwrap_or(engine.active_pane_id);
            Ok((s.active_index, pid))
        }
    }
}

/// Resolve `id` (or the focused pane) all the way to a live surface.
fn resolve_surface(
    s: &crate::app_state::AppState,
    id: Option<&str>,
) -> Result<crate::ghostty::ffi::ghostty_surface_t, String> {
    let (ws_idx, pid) = resolve_pane(s, id)?;
    s.split_engines.get(ws_idx)
        .and_then(|engine| engine.find_surface(pid))
        .ok_or_else(|| "surface not ready (terminal not realized yet)".to_string())
}

/// Send one key press + release to a surface.
fn send_key_to_surface(surface: crate::ghostty::ffi::ghostty_surface_t, spec: &crate::socket::keys::KeySpec) {
    use crate::ghostty::ffi;
    let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
    input.keycode = spec.keycode;
    input.mods = spec.mods;
    input.consumed_mods = 0;
    input.text = spec.text_ptr();
    input.unshifted_codepoint = spec.unshifted_codepoint;
    input.composing = false;
    input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_PRESS;
    unsafe { ffi::ghostty_surface_key(surface, input) };
    input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_RELEASE;
    input.text = std::ptr::null();
    unsafe { ffi::ghostty_surface_key(surface, input) };
}

/// Read the whole region of `tag` (viewport or full screen incl. scrollback)
/// from a surface as UTF-8 (lossy on invalid bytes).
fn read_surface_text(
    surface: crate::ghostty::ffi::ghostty_surface_t,
    tag: crate::ghostty::ffi::ghostty_point_tag_e,
) -> Option<String> {
    use crate::ghostty::ffi;
    let point = |coord| ffi::ghostty_point_s { tag, coord, x: 0, y: 0 };
    let sel = ffi::ghostty_selection_s {
        top_left: point(ffi::ghostty_point_coord_e_GHOSTTY_POINT_COORD_TOP_LEFT),
        bottom_right: point(ffi::ghostty_point_coord_e_GHOSTTY_POINT_COORD_BOTTOM_RIGHT),
        rectangle: false,
    };
    let mut text = unsafe { std::mem::zeroed::<ffi::ghostty_text_s>() };
    if !unsafe { ffi::ghostty_surface_read_text(surface, sel, &mut text) } {
        return None;
    }
    Some(take_ghostty_text(surface, &mut text))
}

/// Copy a ghostty_text_s into a String and free it.
fn take_ghostty_text(
    surface: crate::ghostty::ffi::ghostty_surface_t,
    text: &mut crate::ghostty::ffi::ghostty_text_s,
) -> String {
    let out = if text.text.is_null() || text.text_len == 0 {
        String::new()
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(text.text as *const u8, text.text_len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    unsafe { crate::ghostty::ffi::ghostty_surface_free_text(surface, text) };
    out
}

/// Keep the last `n` lines of `text`, ignoring trailing blank lines (the unused
/// rows below the prompt).
fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(0);
    let start = end.saturating_sub(n);
    lines[start..end].join("\n")
}

/// After a socket command touched a pane in the background workspace `ws_idx`:
/// drop the Ghostty focus the operation gave that workspace's pane, optionally make
/// `prev` its active pane again, and bring the visible page and GTK/Ghostty focus
/// back to the active workspace (split/close re-show their own stack page and
/// grab focus).
fn restore_visible_workspace(s: &mut crate::app_state::AppState, ws_idx: usize, prev: Option<u64>) {
    use crate::ghostty::ffi;
    if let Some(engine) = s.split_engines.get_mut(ws_idx) {
        if let Some(surf) = engine.find_surface(engine.active_pane_id) {
            unsafe { ffi::ghostty_surface_set_focus(surf, false) };
        }
        if let Some(p) = prev.filter(|&p| engine.root.find_uuid_for_pane(p).is_some()) {
            engine.active_pane_id = p;
            engine.root.update_focus_css(p);
        }
    }
    let active = s.active_index;
    if let Some(page) = s.workspaces.get(active).map(|ws| ws.stack_page_name.clone()) {
        s.stack.set_visible_child_name(&page);
    }
    if let Some(engine) = s.split_engines.get(active) {
        if let Some(surf) = engine.find_surface(engine.active_pane_id) {
            unsafe { ffi::ghostty_surface_set_focus(surf, true) };
        }
        engine.grab_active_focus();
    }
}

/// `{"id", "title"}` of the active workspace (result of the workspace switch commands).
fn active_workspace_json(s: &crate::app_state::AppState) -> Value {
    match s.active_workspace() {
        Some(ws) => json!({"id": ws.uuid.to_string(), "title": ws.name}),
        None => json!({}),
    }
}

/// Dispatch a SocketCommand on the GTK main thread.
/// SOCK-05: Only focus-intent commands (workspace.select, workspace.next/previous/last,
/// pane.focus, pane.last, surface.focus) may call grab_active_focus() or focus_active_surface().
#[allow(unused_variables)]
pub fn handle_socket_command(
    cmd: SocketCommand,
    state: &crate::app_state::AppStateRef,
) {
    match cmd {
        // -- system.* --
        SocketCommand::Ping { req_id, resp_tx } => {
            let _ = resp_tx.send(ok(req_id, json!({"pong": true})));
        }

        SocketCommand::Identify { req_id, resp_tx } => {
            let socket_path = crate::socket::socket_path().to_string_lossy().to_string();
            let _ = resp_tx.send(ok(req_id, json!({
                "version": env!("CARGO_PKG_VERSION"),
                "platform": "linux",
                "socket_path": socket_path,
            })));
        }

        SocketCommand::Capabilities { req_id, resp_tx } => {
            let methods: Vec<&str> = vec![
                "system.ping", "system.identify", "system.capabilities",
                "workspace.list", "workspace.current", "workspace.create",
                "workspace.select", "workspace.close", "workspace.rename",
                "workspace.next", "workspace.previous", "workspace.last", "workspace.reorder",
                "surface.list", "surface.split", "surface.focus", "surface.close",
                "surface.send_text", "surface.send_key", "surface.read_text",
                "surface.health", "surface.refresh",
                "pane.list", "pane.focus", "pane.last",
                "window.list", "window.current",
                "notification.list", "notification.clear",
                // notifications
                "notification.create",
                // Browser lifecycle + streaming
                "browser.open", "browser.close", "browser.list",
                "browser.stream.enable", "browser.stream.disable",
                "browser.snapshot", "browser.screenshot",
                // P0: navigation
                "browser.navigate", "browser.goto",
                "browser.back", "browser.forward", "browser.reload",
                // P0: interaction
                "browser.click", "browser.dblclick", "browser.type", "browser.fill",
                "browser.press", "browser.keydown", "browser.keyup",
                "browser.hover", "browser.focus",
                "browser.check", "browser.uncheck", "browser.select",
                "browser.scroll", "browser.scroll_into_view",
                "browser.drag", "browser.upload", "browser.download", "browser.pdf",
                // P0: evaluation + waiting
                "browser.eval", "browser.wait",
                // P0: getters
                "browser.get.url", "browser.get.title", "browser.get.text",
                "browser.get.html", "browser.get.value", "browser.get.attr",
                "browser.get.count", "browser.get.box", "browser.get.styles",
                // P0: state checks
                "browser.is.visible", "browser.is.enabled", "browser.is.checked",
                // P1: locators
                "browser.find.role", "browser.find.text", "browser.find.label",
                "browser.find.placeholder", "browser.find.alt", "browser.find.title",
                "browser.find.testid", "browser.find.nth", "browser.find.first",
                "browser.find.last",
                // P1: frames, dialogs, console, errors
                "browser.frame.select", "browser.frame.main",
                "browser.dialog.accept", "browser.dialog.dismiss",
                "browser.console.list", "browser.errors.list",
                "browser.highlight",
                "browser.state.save", "browser.state.load",
                // Debug
                "debug.layout", "debug.type",
                // pane-nav
                "pane.focus_number", "pane.resize", "pane.equalize",
                // terminal-actions
                "surface.find", "surface.find_next", "surface.find_close", "surface.find_status",
                // agent-status
                "workspace.set_status",
                // socket-cli
                "surface.paste", "surface.read_selection", "surface.current",
            ];
            let _ = resp_tx.send(ok(req_id, json!({"methods": methods})));
        }

        // -- workspace.* --
        SocketCommand::WorkspaceList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let list: Vec<Value> = s.workspaces.iter().enumerate().map(|(i, ws)| {
                json!({
                    "index": i,
                    "id": ws.uuid.to_string(),
                    "title": ws.name,
                    "selected": i == s.active_index,
                    // agent-status
                    "status": ws.agent_status.as_ref().map(|st| st.state.as_str()),
                    "status_message": ws.agent_status.as_ref().and_then(|st| st.message.clone()),
                    "unread": ws.has_attention,
                    "progress": ws.progress.map(|(st, v)| json!({"state": st.as_str(), "value": v})),
                    "pane_count": s.split_engines.get(i).map(|e| e.all_panes().len()).unwrap_or(0),
                })
            }).collect();
            let _ = resp_tx.send(ok(req_id, json!({"workspaces": list})));
        }

        SocketCommand::WorkspaceCurrent { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            match s.active_workspace() {
                Some(ws) => {
                    // `id`/`title` match workspace.list; `uuid`/`name` kept for compatibility.
                    let _ = resp_tx.send(ok(req_id, json!({
                        "id": ws.uuid.to_string(),
                        "title": ws.name,
                        "uuid": ws.uuid.to_string(),
                        "name": ws.name,
                        "index": s.active_index,
                    })));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "no_workspace", "no active workspace"));
                }
            }
        }

        SocketCommand::WorkspaceCreate { req_id, remote_target, resp_tx } => {
            if let Some(target) = remote_target {
                // SSH workspace creation per D-13, D-15
                // Create per-workspace bridge for SSH I/O routing
                let (write_tx, write_rx) = tokio::sync::mpsc::unbounded_channel();
                let (output_tx, _output_rx) = tokio::sync::mpsc::unbounded_channel();
                let bridge = std::sync::Arc::new(crate::ssh::bridge::SshBridge::new(write_tx, write_rx, output_tx));
                let id = state.borrow_mut().create_remote_workspace(target.clone(), &bridge);
                // Store bridge on AppState for later access
                state.borrow_mut().workspace_bridges.insert(id, bridge.clone());
                let uuid_str = {
                    let s = state.borrow();
                    s.workspaces.iter()
                        .find(|ws| ws.id == id)
                        .map(|ws| ws.uuid.to_string())
                        .unwrap_or_default()
                };
                // Spawn SSH lifecycle task using the runtime_handle stored on AppState
                let ssh_tx = state.borrow().ssh_event_tx.clone();
                let rt_handle = state.borrow().runtime_handle.clone();
                if let (Some(tx), Some(rt)) = (ssh_tx, rt_handle) {
                    let handle = rt.spawn(crate::ssh::tunnel::run_ssh_lifecycle(id, target, tx, bridge));
                    state.borrow_mut().ssh_task_handles.insert(id, handle);
                }
                let _ = resp_tx.send(ok(req_id, json!({"id": uuid_str, "uuid": uuid_str, "remote": true})));
            } else {
                // Local workspace (existing behavior)
                let id = state.borrow_mut().create_workspace();
                let s = state.borrow();
                let uuid_str = s.workspaces.iter()
                    .find(|ws| ws.id == id)
                    .map(|ws| ws.uuid.to_string())
                    .unwrap_or_default();
                let _ = resp_tx.send(ok(req_id, json!({"id": uuid_str, "uuid": uuid_str})));
            }
        }

        SocketCommand::WorkspaceSelect { req_id, id, resp_tx } => {
            // SOCK-05: workspace.select IS a focus-intent command.
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    state.borrow_mut().switch_to_index(i);
                    let _ = resp_tx.send(ok(req_id, active_workspace_json(&state.borrow())));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceClose { req_id, id, resp_tx } => {
            // SOCK-05: No focus side effects (close_workspace adjusts index internally).
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    let closed = state.borrow_mut().close_workspace(i);
                    if closed {
                        let _ = resp_tx.send(ok(req_id, json!({"id": id})));
                    } else {
                        let _ = resp_tx.send(err(req_id, "last_workspace", "cannot close the last workspace"));
                    }
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceRename { req_id, id, name, resp_tx } => {
            // SOCK-05: No focus side effects. Rename in place (no workspace switch,
            // so attention state and focus history are untouched).
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    // Rename in place: no switch, so no focus or read-state side effects.
                    let mut s = state.borrow_mut();
                    let ws_id = s.workspaces[i].id;
                    s.rename_workspace(ws_id, name);
                    let label = s.workspaces[i].name.clone();
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({"id": id, "name": label, "title": label})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceNext { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            state.borrow_mut().switch_next();
            let _ = resp_tx.send(ok(req_id, active_workspace_json(&state.borrow())));
        }

        SocketCommand::WorkspacePrev { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            state.borrow_mut().switch_prev();
            let _ = resp_tx.send(ok(req_id, active_workspace_json(&state.borrow())));
        }

        SocketCommand::WorkspaceLast { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            // "Last" = most recently active workspace (recorded by switch_to_index), so
            // repeated calls toggle between two workspaces.
            let target = {
                let s = state.borrow();
                s.previous_workspace.and_then(|uuid| {
                    s.workspaces.iter().position(|ws| ws.uuid == uuid)
                        .filter(|&i| i != s.active_index)
                })
            };
            match target {
                Some(i) => {
                    state.borrow_mut().switch_to_index(i);
                    let _ = resp_tx.send(ok(req_id, active_workspace_json(&state.borrow())));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "no_history", "no previously active workspace"));
                }
            }
        }

        SocketCommand::WorkspaceReorder { req_id, id, position, resp_tx } => {
            // SOCK-05: No focus side effects.
            let mut s = state.borrow_mut();
            let idx = s.workspaces.iter().position(|ws| ws.uuid.to_string() == id);
            match idx {
                Some(from) => {
                    let to = position.min(s.workspaces.len().saturating_sub(1));
                    let ws = s.workspaces.remove(from);
                    let engine = s.split_engines.remove(from);
                    // Keep the sidebar row order in sync with the workspace order.
                    if let Some(row) = ws.sidebar_row.as_ref().map(|r| r.row.clone()) {
                        s.sidebar_list.remove(&row);
                        s.sidebar_list.insert(&row, to as i32);
                    }
                    s.workspaces.insert(to, ws);
                    s.split_engines.insert(to, engine);
                    // Move the sidebar row too. Sidebar handlers look rows up via
                    // row.index(), so moving the row keeps clicks/close buttons correct.
                    if from != to {
                        if let Some(row) = s.sidebar_list.row_at_index(from as i32) {
                            s.sidebar_list.remove(&row);
                            s.sidebar_list.insert(&row, to as i32);
                        }
                    }
                    // Adjust active_index after reorder.
                    if from == s.active_index {
                        s.active_index = to;
                    } else if from < s.active_index && to >= s.active_index {
                        s.active_index -= 1;
                    } else if from > s.active_index && to <= s.active_index {
                        s.active_index += 1;
                    }
                    // Removing the row from the ListBox drops its selection; restore it.
                    if let Some(row) = s.workspaces.get(s.active_index).and_then(|w| w.sidebar_row.as_ref()) {
                        s.sidebar_list.select_row(Some(&row.row));
                    }
                    s.trigger_session_save();
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({"id": id, "position": to})));
                }
                None => {
                    drop(s);
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        // -- window.* --
        SocketCommand::WindowList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let workspace_count = state.borrow().workspaces.len();
            let _ = resp_tx.send(ok(req_id, json!({
                "windows": [{"id": "main", "workspaces": workspace_count}]
            })));
        }

        SocketCommand::WindowCurrent { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let _ = resp_tx.send(ok(req_id, json!({"id": "main"})));
        }

        // -- debug.* --
        SocketCommand::DebugLayout { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            match s.split_engines.get(s.active_index) {
                Some(engine) => {
                    let data = engine.root.to_data();
                    let json_tree = serde_json::to_value(&data).unwrap_or(Value::Null);
                    let _ = resp_tx.send(ok(req_id, json!({"layout": json_tree})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "no_workspace", "no active workspace"));
                }
            }
        }

        SocketCommand::DebugType { req_id, text, resp_tx } => {
            // SOCK-05: No focus side effects (sends text to active surface without changing focus).
            let surface = resolve_surface(&state.borrow(), None);
            match surface {
                Ok(surf) => {
                    unsafe {
                        crate::ghostty::ffi::ghostty_surface_text(
                            surf,
                            text.as_ptr() as *const _,
                            text.len(),
                        );
                    }
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        // ── surface.* ────────────────────────────────────────────────────
        SocketCommand::SurfaceList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let mut panes: Vec<Value> = Vec::new();
            for (ws_idx, (ws, engine)) in s.workspaces.iter().zip(s.split_engines.iter()).enumerate() {
                for (pane_uuid, pane_id, active) in engine.all_panes() {
                    // terminal-actions: last OSC title of this pane, if any.
                    let title = engine
                        .gl_area_for_pane(pane_id)
                        .and_then(|a| crate::header_bar::surface_for_widget(a.upcast_ref()))
                        .and_then(crate::header_bar::title_for_surface);
                    panes.push(json!({
                        "uuid": pane_uuid.to_string(),
                        "workspace_uuid": ws.uuid.to_string(),
                        "active": active && ws_idx == s.active_index,
                        "title": title,
                    }));
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({"surfaces": panes})));
        }

        SocketCommand::SurfaceSplit { req_id, id, direction, resp_tx } => {
            // Split the pane named by `id` (any workspace), or the focused pane of the
            // active workspace. SplitEngine::split_active splits by orientation and
            // returns the new pane_id.
            // macOS direction names are accepted too; the new pane always goes
            // after (right of / below) the target.
            let orientation = match direction.as_str() {
                "vertical" | "down" | "up" => gtk4::Orientation::Vertical,
                "horizontal" | "right" | "left" => gtk4::Orientation::Horizontal,
                other => {
                    let msg = format!("invalid direction '{other}' (horizontal, vertical, right, down)");
                    let _ = resp_tx.send(err(req_id, "invalid_params", &msg));
                    return;
                }
            };
            let result = {
                let mut s = state.borrow_mut();
                match resolve_pane(&s, id.as_deref()) {
                    Err(e) => Err(("not_found", e)),
                    Ok((ws_idx, pid)) => {
                        let background = ws_idx != s.active_index;
                        let engine = &mut s.split_engines[ws_idx];
                        // split_active inherits config from the active pane's surface; make
                        // sure the tree holds live pointers (they may still be null).
                        engine.sync_surfaces_from_registry();
                        if engine.active_pane_id != pid {
                            if background {
                                engine.active_pane_id = pid;
                                engine.root.update_focus_css(pid);
                            } else {
                                engine.focus_pane(pid);
                            }
                        }
                        let r = engine.split_active(orientation)
                            .and_then(|new_pane_id| engine.root.find_uuid_for_pane(new_pane_id))
                            .ok_or(("split_failed", "could not split pane".to_string()));
                        if background {
                            // The new pane becomes that workspace's active pane, but the
                            // user stays where they are.
                            restore_visible_workspace(&mut s, ws_idx, None);
                        }
                        r
                    }
                }
            };
            match result {
                Ok(uuid_str) => {
                    let _ = resp_tx.send(ok(req_id, json!({"id": uuid_str, "uuid": uuid_str})));
                }
                Err((code, msg)) => {
                    let _ = resp_tx.send(err(req_id, code, &msg));
                }
            }
        }

        SocketCommand::SurfaceFocus { req_id, id, resp_tx } => {
            // SOCK-05: surface.focus IS a focus-intent command — allowed to change focus,
            // including switching to the workspace that owns the surface.
            let target = locate_pane(&state.borrow(), &id);
            match target {
                Some((ws_idx, pid)) => {
                    if ws_idx != state.borrow().active_index {
                        state.borrow_mut().switch_to_index(ws_idx);
                    }
                    let mut s = state.borrow_mut();
                    s.split_engines[ws_idx].focus_pane(pid);
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => { let _ = resp_tx.send(err(req_id, "not_found", "surface not found")); }
            }
        }

        SocketCommand::SurfaceClose { req_id, id, resp_tx } => {
            // Close pane by uuid (any workspace). Set it as active, then close_active().
            let target = locate_pane(&state.borrow(), &id);
            match target {
                Some((ws_idx, pid)) => {
                    // SOCK-05: not focus-intent. close_active() moves focus to the
                    // closed pane's sibling, so put the user's focus back afterwards.
                    let result = {
                        let mut s = state.borrow_mut();
                        let background = ws_idx != s.active_index;
                        let engine = &mut s.split_engines[ws_idx];
                        let orig = engine.active_pane_id;
                        let prev = Some(orig).filter(|&p| p != pid);
                        engine.sync_surfaces_from_registry();
                        engine.active_pane_id = pid;
                        engine.root.update_focus_css(pid);
                        let r = engine.close_active();
                        if r.is_none() {
                            engine.active_pane_id = orig;
                            engine.root.update_focus_css(orig);
                        } else if background {
                            restore_visible_workspace(&mut s, ws_idx, prev);
                        } else if let Some(p) = prev {
                            engine.focus_pane(p);
                        }
                        r
                    };
                    match result {
                        Some(_) => { let _ = resp_tx.send(ok(req_id, json!({"id": id}))); }
                        None => {
                            let msg = "cannot close pane (last pane, or its terminal is not realized yet)";
                            let _ = resp_tx.send(err(req_id, "close_failed", msg));
                        }
                    }
                }
                None => { let _ = resp_tx.send(err(req_id, "not_found", "surface not found")); }
            }
        }

        SocketCommand::SurfaceSendText { req_id, id, text, resp_tx } => {
            // SOCK-05: send_text is NOT a focus-intent command — NO focus change.
            // Typed input (not a paste): no bracketed paste, "\n" becomes Enter.
            let surface = resolve_surface(&state.borrow(), id.as_deref());
            match surface {
                Ok(surf) => {
                    unsafe {
                        ghostty_surface_text_input(surf, text.as_ptr() as *const _, text.len());
                    }
                    let _ = resp_tx.send(ok(req_id, json!({"bytes": text.len()})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceSendKey { req_id, id, key, resp_tx } => {
            // SOCK-05: send_key is NOT a focus-intent command — NO focus change.
            // Descriptor: "enter", "ctrl+c", "alt+x", "ctrl+shift+t", "f5", "a", ...
            let spec = match crate::socket::keys::parse_key_descriptor(&key) {
                Ok(spec) => spec,
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "invalid_params", &e));
                    return;
                }
            };
            let surface = resolve_surface(&state.borrow(), id.as_deref());
            match surface {
                Ok(surf) => {
                    send_key_to_surface(surf, &spec);
                    let _ = resp_tx.send(ok(req_id, json!({"key": key})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceReadText { req_id, id, lines, scrollback, resp_tx } => {
            // SOCK-05: No focus side effects.
            // Default: visible viewport. `scrollback`: the whole screen (scrollback +
            // active area). `lines`: the last N lines — from the active area when it
            // holds enough, else from the whole screen (avoids copying a large
            // scrollback on the main thread for the common "tail" case).
            let surface = resolve_surface(&state.borrow(), id.as_deref());
            match surface {
                Ok(surf) => {
                    use crate::ghostty::ffi;
                    let text = match (lines, scrollback) {
                        (Some(n), false) => read_surface_text(surf, ffi::ghostty_point_tag_e_GHOSTTY_POINT_ACTIVE)
                            .map(|t| last_lines(&t, n))
                            .filter(|t| t.lines().count() >= n)
                            .or_else(|| {
                                read_surface_text(surf, ffi::ghostty_point_tag_e_GHOSTTY_POINT_SCREEN)
                                    .map(|t| last_lines(&t, n))
                            }),
                        (Some(n), true) => read_surface_text(surf, ffi::ghostty_point_tag_e_GHOSTTY_POINT_SCREEN)
                            .map(|t| last_lines(&t, n)),
                        (None, true) => read_surface_text(surf, ffi::ghostty_point_tag_e_GHOSTTY_POINT_SCREEN),
                        (None, false) => read_surface_text(surf, ffi::ghostty_point_tag_e_GHOSTTY_POINT_VIEWPORT),
                    };
                    match text {
                        Some(text) => { let _ = resp_tx.send(ok(req_id, json!({"text": text}))); }
                        None => { let _ = resp_tx.send(err(req_id, "read_failed", "could not read surface text")); }
                    }
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceHealth { req_id, id, resp_tx } => {
            // SOCK-05: health is NOT focus-intent — NO focus change.
            let s = state.borrow();
            match resolve_pane(&s, id.as_deref()) {
                Ok((ws_idx, pid)) => {
                    let engine = &s.split_engines[ws_idx];
                    let alive = engine.find_surface(pid).is_some();
                    let attn = engine.root.pane_has_attention(pid);
                    let _ = resp_tx.send(ok(req_id, json!({"alive": alive, "has_attention": attn})));
                }
                // An explicit id that no longer exists: the surface is gone.
                Err(_) if id.is_some() => {
                    let _ = resp_tx.send(ok(req_id, json!({"alive": false, "has_attention": false})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceRefresh { req_id, id, resp_tx } => {
            // SOCK-05: refresh is NOT focus-intent — NO focus change.
            // Queue a render on the target surface's GLArea.
            let gl_area = {
                let s = state.borrow();
                resolve_pane(&s, id.as_deref())
                    .map(|(ws_idx, pid)| s.split_engines[ws_idx].gl_area_for_pane(pid))
            };
            match gl_area {
                Ok(area) => {
                    if let Some(area) = area {
                        area.queue_render();
                    }
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        // ── pane.* ───────────────────────────────────────────────────────────
        SocketCommand::PaneList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Alias for surface.list.
            let s = state.borrow();
            let mut panes: Vec<Value> = Vec::new();
            for (ws_idx, (ws, engine)) in s.workspaces.iter().zip(s.split_engines.iter()).enumerate() {
                for (pane_uuid, _pane_id, active) in engine.all_panes() {
                    panes.push(json!({
                        "uuid": pane_uuid.to_string(),
                        "workspace_uuid": ws.uuid.to_string(),
                        "active": active && ws_idx == s.active_index,
                    }));
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({"panes": panes})));
        }

        SocketCommand::PaneFocus { req_id, id, resp_tx } => {
            // SOCK-05: pane.focus IS focus-intent — allowed to change focus.
            // With an id: focus that pane (switching workspace if needed).
            // Without: cycle to the next pane in the active workspace.
            let target = {
                let s = state.borrow();
                match id.as_deref() {
                    Some(uuid_str) => locate_pane(&s, uuid_str),
                    None => s.split_engines.get(s.active_index).and_then(|engine| {
                        let panes = engine.all_panes();
                        let cur = panes.iter().position(|(_, pid, _)| *pid == engine.active_pane_id)?;
                        let (_, next, _) = panes.get((cur + 1) % panes.len())?;
                        Some((s.active_index, *next))
                    }),
                }
            };
            match target {
                Some((ws_idx, pid)) => {
                    if ws_idx != state.borrow().active_index {
                        state.borrow_mut().switch_to_index(ws_idx);
                    }
                    let mut s = state.borrow_mut();
                    s.split_engines[ws_idx].focus_pane(pid);
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => { let _ = resp_tx.send(err(req_id, "not_found", "pane not found")); }
            }
        }

        SocketCommand::PaneLast { req_id, resp_tx } => {
            // SOCK-05: pane.last IS focus-intent — allowed to change focus.
            // Jumps to the previously focused pane of the active workspace.
            // With no history it re-grabs focus on the current pane, as before.
            let pid = crate::shortcuts::handle_focus_last_pane(state);
            let s = state.borrow();
            let engine = s.active_split_engine();
            if pid.is_none() {
                if let Some(engine) = engine {
                    engine.grab_active_focus();
                }
            }
            let uuid = engine.and_then(|e| e.active_pane_uuid());
            let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid})));
        }

        // -- notification.* (Phase 4) --
        SocketCommand::NotificationList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Read-only attention state query.
            let s = state.borrow();
            let notifications: Vec<Value> = s.workspaces.iter().map(|ws| {
                json!({
                    "workspace_uuid": ws.uuid.to_string(),
                    "workspace_name": ws.name,
                    "has_attention": ws.has_attention,
                })
            }).collect();
            let _ = resp_tx.send(ok(req_id, json!({"notifications": notifications})));
        }

        SocketCommand::NotificationClear { req_id, id, resp_tx } => {
            // SOCK-05: No focus side effects. Clears attention without switching workspace.
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    state.borrow_mut().clear_workspace_attention(i);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        // -- browser.* (Phase 8: D-04 lifecycle + streaming) --
        // SOCK-05: None of these commands steal focus.
        SocketCommand::BrowserOpen { req_id, url, workspace, resp_tx } => {
            let mut s = state.borrow_mut();
            // Lazy-init BrowserManager per D-05
            if s.browser_manager.is_none() {
                s.browser_manager = Some(crate::browser::BrowserManager::new());
            }
            let bm = s.browser_manager.as_mut().unwrap();
            // Ensure daemon is running (auto-start per D-05)
            if let Err(e) = bm.ensure_daemon() {
                let _ = resp_tx.send(err(req_id, "daemon_error", &e));
                return;
            }
            // Build params for agent-browser, including workspace if provided
            let mut open_params = serde_json::json!({"url": url});
            if let Some(ref ws) = workspace {
                open_params["workspace"] = serde_json::json!(ws);
            }
            match bm.send_command("navigate", open_params) {
                Ok(result) => {
                    // Allocate surface ref (D-06)
                    s.browser_surface_counter += 1;
                    let ref_id = s.browser_surface_counter;
                    let uuid = result.get("id")
                        .or_else(|| result.get("surface_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    s.browser_surface_refs.insert(ref_id, uuid.clone());
                    // Augment response with surface_ref
                    let mut response = result.clone();
                    if let Some(obj) = response.as_object_mut() {
                        obj.insert("surface_ref".to_string(), serde_json::json!(format!("surface:{}", ref_id)));
                        obj.insert("uuid".to_string(), serde_json::json!(uuid));
                    }
                    // Create preview pane and auto-enable streaming
                    let picture = {
                        let engine = s.active_split_engine_mut();
                        if let Some(eng) = engine {
                            find_preview_picture(&eng.root)
                                .or_else(|| eng.split_active_with_preview().map(|w| w.picture))
                        } else {
                            None
                        }
                    };
                    // Enable streaming so the preview pane shows the page
                    let runtime = s.runtime_handle.clone();
                    let bm = s.browser_manager.as_mut().unwrap();
                    let _ = bm.send_command("stream_enable", serde_json::json!({}));
                    if let Some(pic) = picture {
                        if let Some(ref rt) = runtime {
                            let _ = bm.start_stream(rt, pic);
                        }
                    }
                    let _ = resp_tx.send(ok(req_id, response));
                }
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "browser_error", &e));
                }
            }
        }

        SocketCommand::BrowserStreamEnable { req_id, resp_tx } => {
            let mut s = state.borrow_mut();
            if s.browser_manager.is_none() {
                s.browser_manager = Some(crate::browser::BrowserManager::new());
            }
            let bm = s.browser_manager.as_mut().unwrap();
            if let Err(e) = bm.ensure_daemon() {
                let _ = resp_tx.send(err(req_id, "daemon_error", &e));
                return;
            }
            match bm.send_command("stream_enable", serde_json::json!({})) {
                Ok(result) => {
                    // Find the Picture widget from the Preview pane in the active workspace.
                    // If no preview pane exists yet, create one first.
                    let picture = {
                        let engine = s.active_split_engine_mut();
                        if let Some(eng) = engine {
                            // Try to find existing Preview node's Picture
                            find_preview_picture(&eng.root)
                                .or_else(|| {
                                    // No preview pane yet -- create one
                                    eng.split_active_with_preview().map(|w| w.picture)
                                })
                        } else {
                            None
                        }
                    };

                    // Wire the WebSocket stream to the Picture widget (Gap 1 fix)
                    if let Some(pic) = picture {
                        let runtime = s.runtime_handle.clone();
                        let bm = s.browser_manager.as_mut().unwrap();
                        if let Some(ref rt) = runtime {
                            match bm.start_stream(rt, pic) {
                                Ok(()) => {
                                    // stream wired to preview pane
                                }
                                Err(e) => {
                                    eprintln!("cmux: stream enable failed: {}", e);
                                }
                            }
                        } else {
                            // no runtime handle
                        }
                    } else {
                        // no preview pane available
                    }

                    let _ = resp_tx.send(ok(req_id, result));
                }
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "stream_error", &e));
                }
            }
        }

        SocketCommand::BrowserStreamDisable { req_id, resp_tx } => {
            let mut s = state.borrow_mut();
            if let Some(ref mut bm) = s.browser_manager {
                match bm.send_command("stream_disable", serde_json::json!({})) {
                    Ok(result) => {
                        let _ = resp_tx.send(ok(req_id, result));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(err(req_id, "stream_error", &e));
                    }
                }
            } else {
                let _ = resp_tx.send(err(req_id, "not_running", "No browser session active"));
            }
        }

        SocketCommand::BrowserList { req_id, resp_tx } => {
            let s = state.borrow();
            let surfaces: Vec<serde_json::Value> = s.browser_surface_refs.iter()
                .map(|(ref_id, uuid)| {
                    serde_json::json!({
                        "ref": format!("surface:{}", ref_id),
                        "uuid": uuid,
                        "status": "registered",
                    })
                })
                .collect();
            let _ = resp_tx.send(ok(req_id, serde_json::json!({"surfaces": surfaces})));
        }

        // -- browser.* generic proxy (P0/P1 parity) --
        SocketCommand::BrowserAction { req_id, action, mut params, surface_ref, resp_tx } => {
            let s = state.borrow();
            if let Some(ref bm) = s.browser_manager {
                // Resolve surface ref if provided
                if let Some(ref sref) = surface_ref {
                    match resolve_surface_ref(sref, &s.browser_surface_refs) {
                        Ok(uuid) => {
                            if let Some(obj) = params.as_object_mut() {
                                obj.remove("surface_ref");
                                obj.insert("surface_id".to_string(), serde_json::json!(uuid));
                            }
                        }
                        Err((msg, available)) => {
                            let _ = resp_tx.send(json!({
                                "id": req_id,
                                "ok": false,
                                "error": {"code": "surface_not_found", "message": msg},
                                "available": available,
                            }));
                            return;
                        }
                    }
                }
                // Translate cmux CLI action names to agent-browser action names
                let daemon_action = match action.as_str() {
                    "open" => "launch",
                    "goto" => "navigate",
                    "eval" => "evaluate",
                    "gethtml" => "innerhtml",
                    "stream.enable" => "stream_enable",
                    "stream.disable" => "stream_disable",
                    _ => &action,
                };
                match bm.send_command(daemon_action, params) {
                    Ok(result) => {
                        let _ = resp_tx.send(ok(req_id, result));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(err(req_id, "browser_error", &e));
                    }
                }
            } else {
                let _ = resp_tx.send(err(req_id, "not_running", "No browser session active"));
            }
        }

        // pane-nav
        SocketCommand::PaneFocusNumber { req_id, n, resp_tx } => {
            // SOCK-05: focus-intent command (keyboard equivalent: Ctrl+Alt+N).
            match crate::shortcuts::handle_focus_pane_number(state, n as usize) {
                Some(pid) => {
                    let uuid = state.borrow().active_split_engine()
                        .and_then(|e| e.root.find_uuid_for_pane(pid));
                    let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid, "n": n})));
                }
                None => { let _ = resp_tx.send(err(req_id, "not_found", &format!("no pane {n}"))); }
            }
        }

        SocketCommand::PaneResize { req_id, direction, amount, resp_tx } => {
            // SOCK-05: layout-only; does not move focus.
            let dir = match direction.as_str() {
                "left" => Some(crate::split_engine::FocusDirection::Left),
                "right" => Some(crate::split_engine::FocusDirection::Right),
                "up" => Some(crate::split_engine::FocusDirection::Up),
                "down" => Some(crate::split_engine::FocusDirection::Down),
                _ => None,
            };
            match dir {
                None => {
                    let _ = resp_tx.send(err(req_id, "invalid_params", "direction must be left, right, up or down"));
                }
                Some(_) if !(amount > 0.0 && amount <= 1.0) => {
                    let _ = resp_tx.send(err(req_id, "invalid_params", "amount must be in (0, 1]"));
                }
                Some(d) => {
                    if crate::shortcuts::handle_resize_pane(state, d, amount) {
                        let _ = resp_tx.send(ok(req_id, json!({})));
                    } else {
                        let _ = resp_tx.send(err(req_id, "no_split", "no divider in that direction"));
                    }
                }
            }
        }

        SocketCommand::PaneEqualize { req_id, resp_tx } => {
            // SOCK-05: layout-only; does not move focus.
            crate::shortcuts::handle_equalize_splits(state);
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        // socket-cli
        SocketCommand::SurfacePaste { req_id, id, text, submit, resp_tx } => {
            // SOCK-05: NOT focus-intent. Paste semantics (bracketed paste when the
            // program enabled it); `submit` presses Enter afterwards.
            let surface = resolve_surface(&state.borrow(), id.as_deref());
            match surface {
                Ok(surf) => {
                    unsafe {
                        crate::ghostty::ffi::ghostty_surface_text(surf, text.as_ptr() as *const _, text.len());
                    }
                    if submit {
                        if let Ok(enter) = crate::socket::keys::parse_key_descriptor("enter") {
                            send_key_to_surface(surf, &enter);
                        }
                    }
                    let _ = resp_tx.send(ok(req_id, json!({"bytes": text.len(), "submitted": submit})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceReadSelection { req_id, id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let surface = resolve_surface(&state.borrow(), id.as_deref());
            match surface {
                Ok(surf) => {
                    let mut text = unsafe { std::mem::zeroed::<crate::ghostty::ffi::ghostty_text_s>() };
                    let has = unsafe { crate::ghostty::ffi::ghostty_surface_read_selection(surf, &mut text) };
                    let text = if has { take_ghostty_text(surf, &mut text) } else { String::new() };
                    let _ = resp_tx.send(ok(req_id, json!({"text": text, "has_selection": has})));
                }
                Err(e) => { let _ = resp_tx.send(err(req_id, "not_found", &e)); }
            }
        }

        SocketCommand::SurfaceCurrent { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Pane and surface share one UUID in this
            // port (one terminal per pane); both keys are returned for parity.
            let s = state.borrow();
            match (s.active_workspace(), resolve_pane(&s, None)) {
                (Some(ws), Ok((ws_idx, pid))) => {
                    let engine = &s.split_engines[ws_idx];
                    let uuid = engine.root.find_uuid_for_pane(pid).unwrap_or_default();
                    let _ = resp_tx.send(ok(req_id, json!({
                        "workspace_id": ws.uuid.to_string(),
                        "workspace_name": ws.name,
                        "workspace_index": s.active_index,
                        "pane_id": uuid,
                        "surface_id": uuid,
                        "alive": engine.find_surface(pid).is_some(),
                    })));
                }
                _ => { let _ = resp_tx.send(err(req_id, "no_workspace", "no active workspace")); }
            }
        }

        // -- Tier-2 stubs (D-10) --
        SocketCommand::NotImplemented { req_id, method, resp_tx } => {
            let _ = resp_tx.send(err(req_id, "not_implemented", &format!("{method} is not implemented")));
        }

        // notifications
        SocketCommand::NotificationCreate { req_id, title, body, surface, workspace, resp_tx } => {
            // SOCK-05: No focus side effects. Goes through the same path as OSC 9/777
            // notifications (attention + focus suppression + toast + hook).
            // Main thread: notify() mutates AppState and the sidebar (GTK widgets).
            // Source: the surface (if it is in the given workspace, when one is given),
            // else the workspace, else the active workspace. Without a surface the source
            // pane is unknown, so focus suppression does not apply.
            let target = {
                let s = state.borrow();
                let n = s.split_engines.len().min(s.workspaces.len());
                let ws_idx = workspace.as_ref()
                    .map(|w| s.workspaces.iter().position(|ws| ws.uuid.to_string() == *w));
                let by_surface = surface.as_ref().and_then(|sf| {
                    (0..n)
                        .filter(|&i| ws_idx.is_none_or(|w| w == Some(i)))
                        .find_map(|i| s.split_engines[i].root.find_pane_id_by_uuid(sf).map(|p| (i, Some(p))))
                });
                match (by_surface, ws_idx) {
                    (Some(t), _) => Ok(t),
                    (None, Some(Some(i))) => Ok((i, None)),
                    (None, Some(None)) => Err("workspace not found"),
                    (None, None) if surface.is_some() => Err("surface not found"),
                    (None, None) => Ok((s.active_index, None)),
                }
            };
            match target {
                Ok((ws_idx, pane_id)) => {
                    let delivered = state.borrow_mut().notify(
                        ws_idx, pane_id, crate::app_state::NotificationKind::Cli, &title, &body,
                    );
                    let ws_uuid = state.borrow().workspaces.get(ws_idx)
                        .map(|ws| ws.uuid.to_string())
                        .unwrap_or_default();
                    let _ = resp_tx.send(ok(req_id, json!({
                        "delivered": delivered,
                        "workspace_uuid": ws_uuid,
                    })));
                }
                Err(msg) => {
                    let _ = resp_tx.send(err(req_id, "not_found", msg));
                }
            }
        }
        // ── terminal-actions: find bar ───────────────────────────────────
        // Main thread: drives GTK widgets and Ghostty binding actions. Results
        // (SEARCH_TOTAL/SELECTED) arrive asynchronously; poll find_status.
        // Not focus-intent: the bar opens without taking keyboard focus.
        SocketCommand::SurfaceFind { req_id, query, resp_tx } => {
            let Some(query) = query else {
                let _ = resp_tx.send(err(req_id, "invalid_params", "query (string) is required"));
                return;
            };
            if crate::find_bar::open_for_current(false) {
                crate::find_bar::set_query(&query);
                let _ = resp_tx.send(ok(req_id, crate::find_bar::status()));
            } else {
                let _ = resp_tx.send(err(req_id, "not_found", "no active terminal surface"));
            }
        }

        SocketCommand::SurfaceFindNext { req_id, previous, resp_tx } => {
            crate::find_bar::navigate(previous);
            let _ = resp_tx.send(ok(req_id, crate::find_bar::status()));
        }

        SocketCommand::SurfaceFindClose { req_id, resp_tx } => {
            // Not focus-intent: never move keyboard focus (SOCK-05).
            crate::find_bar::close(false);
            let _ = resp_tx.send(ok(req_id, crate::find_bar::status()));
        }

        SocketCommand::SurfaceFindStatus { req_id, resp_tx } => {
            let mut status = crate::find_bar::status();
            status["window_title"] = json!(crate::header_bar::current_window_title());
            let _ = resp_tx.send(ok(req_id, status));
        }
        // agent-status
        SocketCommand::WorkspaceSetStatus { req_id, workspace_id, surface_id, state: status, message, resp_tx } => {
            // SOCK-05: No focus side effects; only the sidebar row is updated.
            // Runs on the GTK main loop because workspace state (Rc<RefCell<AppState>>)
            // and the row widgets live there; the work is a lookup plus a few label
            // updates, so hook callers get a fast reply.
            let Some(update) = crate::agent_status::StatusUpdate::parse(&status) else {
                let _ = resp_tx.send(err(
                    req_id,
                    "invalid_params",
                    "state must be one of running, needs_input, idle, error, clear",
                ));
                return;
            };
            if workspace_id.as_deref().unwrap_or("").is_empty() && surface_id.as_deref().unwrap_or("").is_empty() {
                let _ = resp_tx.send(err(req_id, "invalid_params", "workspace_id or surface_id is required"));
                return;
            }
            let mut s = state.borrow_mut();
            match s.find_workspace_for_status(workspace_id.as_deref(), surface_id.as_deref()) {
                Some(i) => {
                    s.set_workspace_status(i, update, message.as_deref());
                    let ws = &s.workspaces[i];
                    let result = json!({
                        "workspace_id": ws.uuid.to_string(),
                        "status": ws.agent_status.as_ref().map(|st| st.state.as_str()),
                        "status_message": ws.agent_status.as_ref().and_then(|st| st.message.clone()),
                    });
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, result));
                }
                None => {
                    drop(s);
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }
    }
}

/// Walk the split tree to find the first Preview node's Picture widget.
fn find_preview_picture(node: &crate::split_engine::SplitNode) -> Option<gtk4::Picture> {
    match node {
        crate::split_engine::SplitNode::Preview { picture, .. } => Some(picture.clone()),
        crate::split_engine::SplitNode::Split { start, end, .. } => {
            find_preview_picture(start).or_else(|| find_preview_picture(end))
        }
        crate::split_engine::SplitNode::Leaf { .. } => None,
    }
}
