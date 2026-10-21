use crate::app_state::AppState;
use crate::config::ShortcutAction;
use crate::split_engine::FocusDirection;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Install all cmux keyboard shortcuts on the application window.
///
/// Uses PropagationPhase::Capture (parent -> child) so the window controller fires
/// BEFORE Ghostty's per-GLArea EventControllerKey. Without capture phase, Ghostty
/// eats Ctrl+D, Ctrl+N, etc. (per RESEARCH.md Pattern 4 and Anti-patterns).
///
/// Shortcut bindings are driven by ShortcutMap (config-driven, D-06).
pub fn install_shortcuts(
    window: &gtk4::ApplicationWindow,
    state: Rc<RefCell<AppState>>,
    sidebar: &gtk4::Box,
    app: &gtk4::Application,
    shortcut_map: crate::config::ShortcutMap,
) {
    let key_ctrl = gtk4::EventControllerKey::new();
    // CRITICAL: Capture phase -- fires before GLArea key handlers.
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let sidebar_clone = sidebar.clone();
    let app_clone = app.clone();

    key_ctrl.connect_key_pressed({
        let state = state.clone();
        move |_ctrl, keyval, _keycode, mods| {
            match shortcut_map.lookup(mods, keyval) {
                // -- Workspace shortcuts --
                Some(ShortcutAction::NewWorkspace) => {
                    handle_new_workspace(&state, &app_clone);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::CloseWorkspace) => {
                    handle_close_workspace(&state, &app_clone);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::NextWorkspace | ShortcutAction::NextWorkspaceAlt) => {
                    state.borrow_mut().switch_next();
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::PrevWorkspace | ShortcutAction::PrevWorkspaceAlt) => {
                    state.borrow_mut().switch_prev();
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::RenameWorkspace) => {
                    let (active_index, sidebar_list) = {
                        let s = state.borrow();
                        let idx = s.active_index;
                        let list = s.sidebar_list.clone();
                        (idx, list)
                    };
                    crate::sidebar::start_inline_rename(&sidebar_list, active_index, state.clone());
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ToggleSidebar) => {
                    let visible = sidebar_clone.is_visible();
                    sidebar_clone.set_visible(!visible);
                    if let Some(engine) = state.borrow_mut().active_split_engine_mut() {
                        engine.focus_active_surface();
                    }
                    gtk4::glib::Propagation::Stop
                }
                // -- Pane split shortcuts --
                Some(ShortcutAction::SplitRight) => {
                    handle_split(&state, false);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::SplitDown) => {
                    handle_split(&state, true);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ClosePane) => {
                    handle_close_pane(&state, &app_clone);
                    gtk4::glib::Propagation::Stop
                }
                // -- SSH workspace shortcut --
                Some(ShortcutAction::NewSshWorkspace) => {
                    handle_new_ssh_workspace(&state, &app_clone);
                    gtk4::glib::Propagation::Stop
                }
                // -- Focus direction shortcuts --
                Some(ShortcutAction::FocusLeft) => {
                    handle_focus_direction(&state, FocusDirection::Left);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::FocusRight) => {
                    handle_focus_direction(&state, FocusDirection::Right);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::FocusUp) => {
                    handle_focus_direction(&state, FocusDirection::Up);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::FocusDown) => {
                    handle_focus_direction(&state, FocusDirection::Down);
                    gtk4::glib::Propagation::Stop
                }
                // -- Workspace number shortcuts --
                Some(action @ (
                    ShortcutAction::Workspace1 | ShortcutAction::Workspace2 |
                    ShortcutAction::Workspace3 | ShortcutAction::Workspace4 |
                    ShortcutAction::Workspace5 | ShortcutAction::Workspace6 |
                    ShortcutAction::Workspace7 | ShortcutAction::Workspace8 |
                    ShortcutAction::Workspace9
                )) => {
                    let idx = match action {
                        ShortcutAction::Workspace1 => 0,
                        ShortcutAction::Workspace2 => 1,
                        ShortcutAction::Workspace3 => 2,
                        ShortcutAction::Workspace4 => 3,
                        ShortcutAction::Workspace5 => 4,
                        ShortcutAction::Workspace6 => 5,
                        ShortcutAction::Workspace7 => 6,
                        ShortcutAction::Workspace8 => 7,
                        ShortcutAction::Workspace9 => 8,
                        _ => unreachable!(),
                    };
                    state.borrow_mut().switch_to_index(idx);
                    gtk4::glib::Propagation::Stop
                }
                // -- Browser shortcuts --
                Some(ShortcutAction::BrowserOpen) => {
                    handle_browser_open(&state);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::BrowserClose) => {
                    handle_browser_close(&state);
                    gtk4::glib::Propagation::Stop
                }
                // -- pane-nav --
                Some(action @ (
                    ShortcutAction::FocusPane1 | ShortcutAction::FocusPane2 |
                    ShortcutAction::FocusPane3 | ShortcutAction::FocusPane4 |
                    ShortcutAction::FocusPane5 | ShortcutAction::FocusPane6 |
                    ShortcutAction::FocusPane7 | ShortcutAction::FocusPane8 |
                    ShortcutAction::FocusPane9
                )) => {
                    if let Some(n) = focus_pane_number(action) {
                        handle_focus_pane_number(&state, n);
                    }
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::FocusLastPane) => {
                    handle_focus_last_pane(&state);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ResizeLeft) => {
                    handle_resize_pane(&state, FocusDirection::Left, RESIZE_STEP);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ResizeRight) => {
                    handle_resize_pane(&state, FocusDirection::Right, RESIZE_STEP);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ResizeUp) => {
                    handle_resize_pane(&state, FocusDirection::Up, RESIZE_STEP);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::ResizeDown) => {
                    handle_resize_pane(&state, FocusDirection::Down, RESIZE_STEP);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::EqualizeSplits) => {
                    handle_equalize_splits(&state);
                    gtk4::glib::Propagation::Stop
                }
                // pane-zoom
                Some(ShortcutAction::TogglePaneZoom) => {
                    handle_pane_zoom(&state, None);
                    gtk4::glib::Propagation::Stop
                }
                // -- tabs --
                Some(ShortcutAction::NewTab) => {
                    crate::tabs::handle_new_tab(&state);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::RenameTab) => {
                    crate::tabs::handle_rename_tab(&state);
                    gtk4::glib::Propagation::Stop
                }
                // Tab switching keys reach the terminal when the pane has one tab.
                Some(ShortcutAction::NextTab | ShortcutAction::NextTabAlt)
                    if crate::tabs::active_pane_has_tabs(&state) =>
                {
                    crate::tabs::handle_cycle_tab(&state, true);
                    gtk4::glib::Propagation::Stop
                }
                Some(ShortcutAction::PrevTab | ShortcutAction::PrevTabAlt)
                    if crate::tabs::active_pane_has_tabs(&state) =>
                {
                    crate::tabs::handle_cycle_tab(&state, false);
                    gtk4::glib::Propagation::Stop
                }
                Some(action)
                    if crate::tabs::goto_tab_number(action).is_some()
                        && crate::tabs::active_pane_has_tabs(&state) =>
                {
                    crate::tabs::handle_goto_tab(&state, crate::tabs::goto_tab_number(action).unwrap_or(0));
                    gtk4::glib::Propagation::Stop
                }
                // Everything else passes through to Ghostty.
                _ => gtk4::glib::Propagation::Proceed,
            }
        }
    });

    window.add_controller(key_ctrl);
}

/// Create a new workspace with an initial GLArea pane and add it to AppState + GtkStack.
pub fn handle_new_workspace(state: &Rc<RefCell<AppState>>, _app: &gtk4::Application) {
    // The sidebar row's close button + context menu are wired by AppState.
    state.borrow_mut().create_workspace();
}

/// Show close-workspace confirmation dialog. If confirmed, closes the active workspace.
pub fn handle_close_workspace(state: &Rc<RefCell<AppState>>, app: &gtk4::Application) {
    // Cannot close the last workspace.
    let (active_index, workspace_count) = {
        let s = state.borrow();
        (s.active_index, s.workspaces.len())
    };
    if workspace_count <= 1 {
        return; // No-op: cannot close the last workspace
    }

    let dialog = gtk4::AlertDialog::builder()
        .message("Close Workspace?")
        .detail("All panes in this workspace will be closed. This cannot be undone.")
        .modal(true)
        .build();
    dialog.set_buttons(&["Keep Workspace", "Close Workspace"]);
    dialog.set_default_button(0);
    dialog.set_cancel_button(0);

    let window = app.windows().into_iter().next();

    dialog.choose(window.as_ref(), None::<&gtk4::gio::Cancellable>, {
        let state = state.clone();
        move |result| {
            // Button index 1 = "Close Workspace" (destructive)
            if let Ok(1) = result {
                state.borrow_mut().close_workspace(active_index);
            }
        }
    });
}

/// Split the active pane. `vertical=false` -> split right (Ctrl+D), `vertical=true` -> split down.
pub fn handle_split(state: &Rc<RefCell<AppState>>, vertical: bool) {
    let mut s = state.borrow_mut();
    if let Some(engine) = s.active_split_engine_mut() {
        let _new_pane_id = if vertical {
            engine.split_down()
        } else {
            engine.split_right()
        };
        // The new GLArea is already added to the widget tree inside SplitEngine.
        // CSS active-pane class is updated inside SplitEngine.
    }
}

/// Close the active pane (Ctrl+Shift+X).
pub fn handle_close_pane(state: &Rc<RefCell<AppState>>, app: &gtk4::Application) {
    let (close_workspace, active_index) = {
        let mut s = state.borrow_mut();
        if let Some(engine) = s.active_split_engine_mut() {
            match engine.close_active() {
                None => (true, s.active_index), // last pane -> close workspace
                Some(_) => (false, 0),
            }
        } else {
            (false, 0)
        }
    };
    if close_workspace {
        handle_close_workspace(state, app);
    }
}

/// Open the SSH connect dialog (Ctrl+Shift+S).
pub fn handle_new_ssh_workspace(state: &Rc<RefCell<AppState>>, app: &gtk4::Application) {
    crate::ssh_dialog::show_ssh_dialog(app, state.clone());
}

/// Move focus to adjacent pane in `direction`.
pub fn handle_focus_direction(state: &Rc<RefCell<AppState>>, direction: FocusDirection) {
    let mut s = state.borrow_mut();
    if let Some(engine) = s.active_split_engine_mut() {
        engine.focus_next_in_direction(direction);
    }
}

// ── pane-nav ────────────────────────────────────────────────────────────────

/// Divider step for keyboard resize, as a fraction of the split's size.
pub const RESIZE_STEP: f64 = 0.05;

/// Pane number (1-9) for a FocusPaneN action.
pub fn focus_pane_number(action: ShortcutAction) -> Option<usize> {
    Some(match action {
        ShortcutAction::FocusPane1 => 1,
        ShortcutAction::FocusPane2 => 2,
        ShortcutAction::FocusPane3 => 3,
        ShortcutAction::FocusPane4 => 4,
        ShortcutAction::FocusPane5 => 5,
        ShortcutAction::FocusPane6 => 6,
        ShortcutAction::FocusPane7 => 7,
        ShortcutAction::FocusPane8 => 8,
        ShortcutAction::FocusPane9 => 9,
        _ => return None,
    })
}

/// Focus the Nth terminal pane of the active workspace (Ctrl+Alt+1..9).
pub fn handle_focus_pane_number(state: &Rc<RefCell<AppState>>, n: usize) -> Option<u64> {
    state.borrow_mut().active_split_engine_mut()?.focus_pane_number(n)
}

/// Jump back to the previously focused pane (Ctrl+Alt+`).
pub fn handle_focus_last_pane(state: &Rc<RefCell<AppState>>) -> Option<u64> {
    state.borrow_mut().active_split_engine_mut()?.focus_last_pane()
}

/// Move the active pane's nearest divider (Ctrl+Alt+Shift+arrows).
pub fn handle_resize_pane(state: &Rc<RefCell<AppState>>, direction: FocusDirection, fraction: f64) -> bool {
    let resized = state
        .borrow_mut()
        .active_split_engine_mut()
        .map(|engine| engine.resize_active(direction, fraction))
        .unwrap_or(false);
    if resized {
        save_session_after_layout(state);
    }
    resized
}

/// Give all panes of the active workspace equal space (Ctrl+Alt+=).
pub fn handle_equalize_splits(state: &Rc<RefCell<AppState>>) {
    if let Some(engine) = state.borrow_mut().active_split_engine_mut() {
        engine.equalize();
    }
    save_session_after_layout(state);
}

/// pane-zoom: zoom (`Some(true)`), unzoom (`Some(false)`) or toggle (`None`)
/// the active pane of the active workspace (Ctrl+Shift+Enter). Returns whether
/// the workspace is zoomed afterwards, or None without a workspace.
pub fn handle_pane_zoom(state: &Rc<RefCell<AppState>>, on: Option<bool>) -> Option<bool> {
    // try_borrow_mut: also runs from the preview's nav-bar button.
    let mut s = state.try_borrow_mut().ok()?;
    Some(s.active_split_engine_mut()?.set_zoom(on))
}

/// Save the session once GTK has re-allocated the panes: split ratios are
/// read from live Paned sizes, and nested Paneds only learn their new size
/// on the next layout pass.
fn save_session_after_layout(state: &Rc<RefCell<AppState>>) {
    let state = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
        if let Ok(s) = state.try_borrow() {
            s.trigger_session_save();
        }
    });
}

/// Open a browser preview pane (Ctrl+Shift+B).
/// Launches the agent-browser daemon, navigates to about:blank, creates a preview pane,
/// and starts the CDP screencast stream so frames render in the Picture widget.
pub fn handle_browser_open(state: &Rc<RefCell<AppState>>) {
    // Step 1: Ensure daemon is running, launch browser, enable streaming, navigate
    {
        let mut s = state.borrow_mut();
        if s.browser_manager.is_none() {
            s.browser_manager = Some(crate::browser::BrowserManager::new());
        }
        let bm = s.browser_manager.as_mut().unwrap();
        if let Err(e) = bm.ensure_daemon() {
            eprintln!("cmux: browser.open failed to start daemon: {e}");
            return;
        }
        // Enable WebSocket stream server, launch Chrome, navigate, start screencast
        let _ = bm.send_command("stream_enable", serde_json::json!({}));
        let window = s.gtk_app.active_window().or_else(|| s.gtk_app.windows().into_iter().next());
        let bm = s.browser_manager.as_mut().unwrap();
        let _ = bm.send_command("launch", crate::browser::launch_params(window.as_ref()));
        if let Err(e) = bm.send_command("navigate", serde_json::json!({"url": "about:blank"})) {
            eprintln!("cmux: browser.open navigate failed: {e}");
            return;
        }
        let _ = bm.send_command("screencast_start", serde_json::json!({"format": "jpeg", "quality": 90}));
    } // drop borrow

    // Step 2: Create preview pane
    let pane_result = {
        let mut s = state.borrow_mut();
        if let Some(engine) = s.active_split_engine_mut() {
            engine.split_active_with_preview()
        } else {
            None
        }
    }; // drop borrow

    let Some(widgets) = pane_result else {
        return;
    };
    // pane-zoom: the preview opens filling the workspace; its nav-bar button
    // (or Ctrl+Shift+Enter) puts the terminal back beside it.
    if let Some(engine) = state.borrow_mut().active_split_engine_mut() {
        engine.zoom_pane(widgets.pane_id);
    }
    let picture = widgets.picture.clone();
    let url_entry = widgets.url_entry.clone();
    let picture_ref = picture.clone();

    // Step 3: Start WebSocket stream to pipe frames to Picture widget
    {
        let mut s = state.borrow_mut();
        let runtime = s.runtime_handle.clone();
        let bm = s.browser_manager.as_mut().unwrap();
        if let Some(ref rt) = runtime {
            if let Err(e) = bm.start_stream(rt, picture) {
                eprintln!("cmux: browser start_stream failed: {e}");
            }
        }
    } // drop borrow

    // Step 3b: Wire nav button signals (D-06, D-07)
    {
        // Back button
        let state_for_back = state.clone();
        widgets.back_btn.connect_clicked(move |_| {
            let s = state_for_back.borrow();
            if let Some(ref bm) = s.browser_manager {
                let _ = bm.send_command("back", serde_json::json!({}));
            }
        });

        // Forward button
        let state_for_fwd = state.clone();
        widgets.forward_btn.connect_clicked(move |_| {
            let s = state_for_fwd.borrow();
            if let Some(ref bm) = s.browser_manager {
                let _ = bm.send_command("forward", serde_json::json!({}));
            }
        });

        // Reload button
        let state_for_reload = state.clone();
        widgets.reload_btn.connect_clicked(move |_| {
            let s = state_for_reload.borrow();
            if let Some(ref bm) = s.browser_manager {
                let _ = bm.send_command("reload", serde_json::json!({}));
            }
        });

        // Go button: reads URL entry, auto-prepends https://, navigates
        let state_for_go = state.clone();
        let url_entry_for_go = url_entry.clone();
        let picture_for_go = picture_ref.clone();
        widgets.go_btn.connect_clicked(move |_| {
            let raw_url = url_entry_for_go.text().to_string();
            if raw_url.is_empty() {
                return;
            }
            let url = if raw_url.contains("://") { raw_url } else { format!("https://{raw_url}") };
            url_entry_for_go.set_text(&url);
            let s = state_for_go.borrow();
            if let Some(ref bm) = s.browser_manager {
                if let Some(params) = crate::browser::viewport_params(&picture_for_go) {
                    let _ = bm.send_command("viewport", params);
                }
                let _ = bm.send_command("navigate", serde_json::json!({"url": url}));
            }
        });
    }

    // Close button: same as Ctrl+Shift+Q.
    {
        let state_for_close = state.clone();
        widgets.close_btn.connect_clicked(move |_| {
            let state = state_for_close.clone();
            // Not from inside the click handler: closing unparents this button.
            glib::idle_add_local_once(move || handle_browser_close(&state));
        });
    }

    // A click anywhere in the preview makes it the active pane, so Ctrl+Shift+W
    // closes it and focus restoration no longer pulls focus back to the terminal.
    // Capture phase: runs before the URL bar and page take the click.
    {
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let state_for_click = state.clone();
        let pane_id = widgets.pane_id;
        click.connect_pressed(move |_, _, _, _| {
            if let Ok(mut s) = state_for_click.try_borrow_mut() {
                if let Some(engine) = s.active_split_engine_mut() {
                    engine.activate_preview(pane_id);
                }
            }
        });
        widgets.container.add_controller(click);

        // Take focus explicitly on press: the entry otherwise only focuses once
        // its inner text widget accepts the click.
        let entry_click = gtk4::GestureClick::new();
        entry_click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let entry_for_click = url_entry.clone();
        entry_click.connect_pressed(move |_, _, _, _| {
            // GtkEntry hands focus to its inner text widget; grabbing again
            // while focused would select all and lose the click position.
            let focused = entry_for_click
                .root()
                .and_then(|r| r.focus())
                .is_some_and(|f| f.is_ancestor(&entry_for_click));
            if !focused {
                entry_for_click.grab_focus();
            }
        });
        url_entry.add_controller(entry_click);
    }

    // Step 3.5: Create async motion forwarder channel (D-08)
    let motion_tx = {
        let s = state.borrow();
        let runtime = s.runtime_handle.clone();
        let bm = s.browser_manager.as_ref();
        match (runtime, bm) {
            (Some(rt), Some(bm)) => {
                Some(crate::browser::spawn_motion_forwarder(&rt, bm.daemon_socket_path()))
            }
            _ => None,
        }
    };

    // Step 4: Keep the page viewport at the pane's size in device pixels so
    // frames map 1:1 onto the screen (no rescaling blur). The sensor resizes
    // with the Picture on every layout change (window resize, divider drag,
    // sidebar toggle); updates are debounced so a drag doesn't flood the daemon.
    {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let state_for_viewport = state.clone();
        let picture_for_viewport = picture_ref.clone();
        let schedule_viewport = Rc::new(move || {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let state = state_for_viewport.clone();
            let picture = picture_for_viewport.clone();
            let pending_for_fire = pending.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                pending_for_fire.borrow_mut().take();
                let Some(params) = crate::browser::viewport_params(&picture) else {
                    return;
                };
                if let Ok(s) = state.try_borrow() {
                    if let Some(ref bm) = s.browser_manager {
                        let _ = bm.send_command("viewport", params);
                    }
                }
            });
            *pending.borrow_mut() = Some(id);
        });
        let on_resize = schedule_viewport.clone();
        widgets.viewport_sensor.connect_resize(move |_, _, _| on_resize());
        let on_scale = schedule_viewport.clone();
        picture_ref.connect_scale_factor_notify(move |_| on_scale());
        schedule_viewport();
    }

    // Attach input controllers to the Picture for browser interaction.
    // Coordinates go through browser::widget_to_page_point: the frame is
    // letterboxed inside the Picture and input is in page CSS pixels.
    {
        let page_css_size = state
            .borrow()
            .browser_manager
            .as_ref()
            .map(|bm| bm.page_css_size.clone())
            .unwrap_or_default();
        let to_page = {
            let picture = picture_ref.clone();
            move |x: f64, y: f64| preview_point_to_page(&picture, &page_css_size, x, y)
        };

        // The Picture itself takes keyboard focus on click (the container would
        // also receive keys bubbling up from the URL entry).
        picture_ref.set_focusable(true);

        // Press and release are sent separately so double clicks (n_press) and
        // drag selection work. The release comes from a GestureDrag: GestureClick
        // stops without "released" once the pointer moves past the drag threshold.
        let pressed: Rc<std::cell::Cell<Option<i32>>> = Rc::default();
        let click_ctrl = gtk4::GestureClick::new();
        {
            let state = state.clone();
            let picture = picture_ref.clone();
            let to_page = to_page.clone();
            let pressed = pressed.clone();
            click_ctrl.connect_pressed(move |_gesture, n_press, x, y| {
                // D-09: focus the page so keyboard events flow to Chrome.
                picture.grab_focus();
                let Some((px, py, inside)) = to_page(x, y) else { return };
                if !inside {
                    return; // letterbox bar
                }
                pressed.set(Some(n_press));
                browser_send(&state, "input_mouse", serde_json::json!({
                    "type": "mousePressed", "x": px, "y": py,
                    "button": "left", "clickCount": n_press
                }));
            });
        }
        picture_ref.add_controller(click_ctrl);

        let release_ctrl = gtk4::GestureDrag::new();
        {
            let state = state.clone();
            let to_page = to_page.clone();
            release_ctrl.connect_drag_end(move |gesture, dx, dy| {
                let Some(n_press) = pressed.take() else { return };
                let (sx, sy) = gesture.start_point().unwrap_or((0.0, 0.0));
                let Some((px, py, _)) = to_page(sx + dx, sy + dy) else { return };
                browser_send(&state, "input_mouse", serde_json::json!({
                    "type": "mouseReleased", "x": px, "y": py,
                    "button": "left", "clickCount": n_press
                }));
            });
        }
        picture_ref.add_controller(release_ctrl);

        // Attach mouse motion controller for hover effects (async channel, D-08).
        // The last pointer position also anchors wheel scrolling.
        let pointer: Rc<std::cell::Cell<Option<(f64, f64)>>> = Rc::default();
        let motion_ctrl = gtk4::EventControllerMotion::new();
        {
            let pointer = pointer.clone();
            let to_page = to_page.clone();
            motion_ctrl.connect_motion(move |_ctrl, x, y| {
                pointer.set(Some((x, y)));
                if let (Some(mtx), Some((px, py, _))) = (motion_tx.as_ref(), to_page(x, y)) {
                    let _ = mtx.send((px as i64, py as i64));
                }
            });
        }
        {
            let pointer = pointer.clone();
            motion_ctrl.connect_leave(move |_| pointer.set(None));
        }
        picture_ref.add_controller(motion_ctrl);

        // Attach scroll controller for scroll wheel forwarding (both axes)
        let scroll_ctrl = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES,
        );
        {
            let state = state.clone();
            let picture = picture_ref.clone();
            scroll_ctrl.connect_scroll(move |ctrl, dx, dy| {
                // Scroll under the pointer, else at the center of the page.
                let (x, y) = pointer.get().unwrap_or((
                    picture.width() as f64 / 2.0,
                    picture.height() as f64 / 2.0,
                ));
                let Some((px, py, _)) = to_page(x, y) else {
                    return gtk4::glib::Propagation::Proceed;
                };
                // Wheel notches arrive as +-1 steps; touchpads already in pixels.
                let step = match ctrl.unit() {
                    gtk4::gdk::ScrollUnit::Wheel => 120.0,
                    _ => 1.0,
                };
                browser_send(&state, "input_mouse", serde_json::json!({
                    "type": "mouseWheel", "x": px, "y": py,
                    "deltaX": dx * step, "deltaY": dy * step
                }));
                gtk4::glib::Propagation::Stop
            });
        }
        picture_ref.add_controller(scroll_ctrl);

        // Keyboard: text goes through an input method context so IME input
        // (e.g. Korean) arrives as committed strings; keys the IM does not
        // consume (Enter, Backspace, arrows, Tab, Ctrl chords) come to
        // key-pressed. Both use the daemon's "press" action, which sends the
        // virtual key codes Chrome needs for editing keys.
        let im_context = gtk4::IMMulticontext::new();
        im_context.set_client_widget(Some(&picture_ref));
        {
            let state = state.clone();
            im_context.connect_commit(move |_, text| {
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c == ' ' || c.is_ascii_graphic() => {
                        browser_send(&state, "press", serde_json::json!({"key": text}));
                    }
                    _ => browser_send(&state, "inserttext", serde_json::json!({"text": text})),
                }
            });
        }
        let focus_ctrl = gtk4::EventControllerFocus::new();
        {
            let im = im_context.clone();
            focus_ctrl.connect_enter(move |_| im.focus_in());
            let im = im_context.clone();
            focus_ctrl.connect_leave(move |_| im.focus_out());
        }
        picture_ref.add_controller(focus_ctrl);

        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.set_im_context(Some(&im_context));
        // Bubble phase so cmux capture-phase shortcuts (Ctrl+Shift+B etc) take priority
        key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        {
            let state = state.clone();
            key_ctrl.connect_key_pressed(move |_ctrl, keyval, _keycode, mods| {
                let Some(chord) = cdp_key_chord(keyval, mods) else {
                    return gtk4::glib::Propagation::Proceed;
                };
                browser_send(&state, "press", serde_json::json!({"key": chord}));
                gtk4::glib::Propagation::Stop
            });
        }
        picture_ref.add_controller(key_ctrl);
    }

    // Keep the URL bar in sync with the page (link clicks, back/forward, CLI).
    {
        let s = state.borrow();
        if let (Some(rt), Some(bm)) = (s.runtime_handle.as_ref(), s.browser_manager.as_ref()) {
            crate::browser::spawn_url_watcher(rt, bm.daemon_socket_path(), &url_entry);
        }
    }

    // Step 5: Connect URL entry — Enter navigates the browser
    let state_for_entry = state.clone();
    let picture_for_nav = picture_ref.clone();
    url_entry.connect_activate(move |entry| {
        let raw_url = entry.text().to_string();
        if raw_url.is_empty() {
            return;
        }
        // Auto-prepend https:// if no scheme is present
        let url = if raw_url.contains("://") {
            raw_url
        } else {
            format!("https://{raw_url}")
        };
        entry.set_text(&url);
        let s = state_for_entry.borrow();
        if let Some(ref bm) = s.browser_manager {
            // Resize viewport to match current pane size before navigating
            if let Some(params) = crate::browser::viewport_params(&picture_for_nav) {
                let _ = bm.send_command("viewport", params);
            }
            let params = serde_json::json!({"url": url});
            let _ = bm.send_command("navigate", params);
        }
    });

    // Step 6: DevTools toggle (D-10)
    let state_for_devtools = state.clone();
    let picture_for_devtools = picture_ref.clone();
    widgets.devtools_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            // Fetch DOM snapshot from daemon
            let snapshot_text = {
                let s = state_for_devtools.borrow();
                if let Some(ref bm) = s.browser_manager {
                    match bm.send_command("snapshot", serde_json::json!({})) {
                        Ok(result) => {
                            if let Some(text) = result.get("data").and_then(|d| d.as_str()) {
                                text.to_string()
                            } else if let Some(text) = result.get("result").and_then(|d| d.as_str()) {
                                text.to_string()
                            } else {
                                serde_json::to_string_pretty(&result).unwrap_or_default()
                            }
                        }
                        Err(e) => format!("Snapshot error: {e}"),
                    }
                } else {
                    "No browser session active".to_string()
                }
            };

            // Create scrollable text overlay on the Picture's parent Overlay
            if let Some(overlay) = picture_for_devtools.parent().and_then(|p| p.downcast::<gtk4::Overlay>().ok()) {
                let label = gtk4::Label::new(Some(&snapshot_text));
                label.set_selectable(true);
                label.set_wrap(true);
                label.set_xalign(0.0);
                label.set_yalign(0.0);
                label.add_css_class("devtools-snapshot");
                let scrolled = gtk4::ScrolledWindow::new();
                scrolled.set_child(Some(&label));
                scrolled.set_hexpand(true);
                scrolled.set_vexpand(true);
                scrolled.add_css_class("devtools-overlay");
                overlay.add_overlay(&scrolled);
            }
        } else {
            // Remove the DevTools overlay
            if let Some(overlay) = picture_for_devtools.parent().and_then(|p| p.downcast::<gtk4::Overlay>().ok()) {
                if let Some(child) = overlay.first_child() {
                    let mut current = Some(child);
                    while let Some(widget) = current {
                        let next = widget.next_sibling();
                        if widget.has_css_class("devtools-overlay") {
                            overlay.remove_overlay(&widget);
                        }
                        current = next;
                    }
                }
            }
        }
    });
}

/// Map GDK keyval to (CDP key name, CDP code name).
/// Returns empty strings for unmapped keys.
fn gdk_keyval_to_cdp(keyval: gtk4::gdk::Key) -> (String, String) {
    use gtk4::gdk::Key;
    match keyval {
        Key::Return | Key::KP_Enter => ("Enter".into(), "Enter".into()),
        Key::Tab => ("Tab".into(), "Tab".into()),
        Key::Escape => ("Escape".into(), "Escape".into()),
        Key::BackSpace => ("Backspace".into(), "Backspace".into()),
        Key::Delete => ("Delete".into(), "Delete".into()),
        Key::Home => ("Home".into(), "Home".into()),
        Key::End => ("End".into(), "End".into()),
        Key::Page_Up => ("PageUp".into(), "PageUp".into()),
        Key::Page_Down => ("PageDown".into(), "PageDown".into()),
        Key::Left => ("ArrowLeft".into(), "ArrowLeft".into()),
        Key::Right => ("ArrowRight".into(), "ArrowRight".into()),
        Key::Up => ("ArrowUp".into(), "ArrowUp".into()),
        Key::Down => ("ArrowDown".into(), "ArrowDown".into()),
        Key::space => (" ".into(), "Space".into()),
        Key::F1 => ("F1".into(), "F1".into()),
        Key::F2 => ("F2".into(), "F2".into()),
        Key::F3 => ("F3".into(), "F3".into()),
        Key::F4 => ("F4".into(), "F4".into()),
        Key::F5 => ("F5".into(), "F5".into()),
        Key::F6 => ("F6".into(), "F6".into()),
        Key::F7 => ("F7".into(), "F7".into()),
        Key::F8 => ("F8".into(), "F8".into()),
        Key::F9 => ("F9".into(), "F9".into()),
        Key::F10 => ("F10".into(), "F10".into()),
        Key::F11 => ("F11".into(), "F11".into()),
        Key::F12 => ("F12".into(), "F12".into()),
        other => {
            // For printable characters, use the unicode value
            if let Some(ch) = other.to_unicode() {
                let s = ch.to_string();
                let code = if ch.is_ascii_alphabetic() {
                    format!("Key{}", ch.to_ascii_uppercase())
                } else if ch.is_ascii_digit() {
                    format!("Digit{}", ch)
                } else {
                    s.clone()
                };
                (s, code)
            } else {
                (String::new(), String::new())
            }
        }
    }
}

/// Key chord for the daemon's "press" action ("Enter", "Shift+Tab", "Control+a"),
/// or None for keys that are not forwarded (lone modifiers, unknown keys).
fn cdp_key_chord(keyval: gtk4::gdk::Key, mods: gtk4::gdk::ModifierType) -> Option<String> {
    use gtk4::gdk::ModifierType;
    let (key, _code) = gdk_keyval_to_cdp(keyval);
    if key.is_empty() {
        return None;
    }
    let ctrl = mods.contains(ModifierType::CONTROL_MASK);
    let alt = mods.contains(ModifierType::ALT_MASK);
    // For characters Shift is already in the keyval ("A", "!").
    let shift = mods.contains(ModifierType::SHIFT_MASK) && (key.chars().count() > 1 || ctrl || alt);
    let mut chord = String::new();
    for (on, name) in [(ctrl, "Control+"), (alt, "Alt+"), (shift, "Shift+")] {
        if on {
            chord.push_str(name);
        }
    }
    chord.push_str(&key);
    Some(chord)
}

/// Map a point in the preview Picture to page CSS pixels: (x, y, inside page).
/// The page size comes from the screencast metadata, falling back to the
/// frame size until the first frame has arrived.
fn preview_point_to_page(
    picture: &gtk4::Picture,
    page_css_size: &std::sync::Mutex<Option<(f64, f64)>>,
    x: f64,
    y: f64,
) -> Option<(f64, f64, bool)> {
    let paintable = picture.paintable()?;
    let image = (paintable.intrinsic_width() as f64, paintable.intrinsic_height() as f64);
    let page = page_css_size.lock().ok().and_then(|s| *s).unwrap_or(image);
    let widget = (picture.width() as f64, picture.height() as f64);
    let (px, py) =
        crate::browser::widget_to_page_point(picture.content_fit(), widget, image, page, (x, y))?;
    let inside = px >= 0.0 && py >= 0.0 && px < page.0 && py < page.1;
    Some((px, py, inside))
}

/// Send a command to the browser daemon if the browser is running. Skips instead
/// of panicking when a re-entrant callback finds AppState already borrowed.
fn browser_send(state: &Rc<RefCell<AppState>>, action: &str, params: serde_json::Value) {
    if let Ok(s) = state.try_borrow() {
        if let Some(ref bm) = s.browser_manager {
            if let Err(e) = bm.send_command(action, params) {
                eprintln!("cmux: browser {action} failed: {e}");
            }
        }
    }
}

/// Close the browser preview and shut down the daemon (Ctrl+Shift+Q).
pub fn handle_browser_close(state: &Rc<RefCell<AppState>>) {
    let mut s = state.borrow_mut();
    // All previews show the one shared browser, so they all go with it.
    for engine in s.split_engines.iter_mut() {
        for id in engine.preview_pane_ids() {
            engine.close_preview(id);
        }
    }
    if let Some(engine) = s.active_split_engine() {
        engine.focus_active_surface();
    }
    if let Some(ref mut bm) = s.browser_manager {
        bm.shutdown();
        s.browser_manager = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gdk::{Key, ModifierType};

    #[test]
    fn key_chords_for_browser_press() {
        let none = ModifierType::empty();
        assert_eq!(cdp_key_chord(Key::Return, none).as_deref(), Some("Enter"));
        assert_eq!(cdp_key_chord(Key::BackSpace, none).as_deref(), Some("Backspace"));
        assert_eq!(cdp_key_chord(Key::Tab, ModifierType::SHIFT_MASK).as_deref(), Some("Shift+Tab"));
        assert_eq!(
            cdp_key_chord(Key::Left, ModifierType::SHIFT_MASK).as_deref(),
            Some("Shift+ArrowLeft")
        );
        assert_eq!(cdp_key_chord(Key::a, ModifierType::CONTROL_MASK).as_deref(), Some("Control+a"));
        // Shift is already part of a typed character.
        assert_eq!(cdp_key_chord(Key::A, ModifierType::SHIFT_MASK).as_deref(), Some("A"));
        // Lone modifiers are not forwarded.
        assert_eq!(cdp_key_chord(Key::Shift_L, ModifierType::SHIFT_MASK), None);
    }
}
