use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Build the sidebar widget: outer Box(V) > [ScrolledWindow(ListBox), Button(+)].
/// Returns (sidebar_box, scrolled_window, list_box).
///
/// Per Pitfall 5 from RESEARCH.md: the '+' button is OUTSIDE the ScrolledWindow
/// so it doesn't scroll away.
///
/// Per UI-SPEC:
/// - Width: 200px (set_size_request(200, -1))
/// - Styling: src/style/components.css (cmux Signal design system, DESIGN.md)
/// - Row height: 36px min-height (CSS)
/// - Row padding: 8px top/bottom, 16px left/right
/// - Active row: #5b8dd9 background, #ffffff text, font-weight 600
/// - Inactive row: transparent bg, #cccccc text, font-weight 400
/// - Hover (inactive): #2e2e2e
pub fn build_sidebar() -> (gtk4::Box, gtk4::ScrolledWindow, gtk4::ListBox) {
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("workspace-list");

    let scrolled = gtk4::ScrolledWindow::new();
    // Fixed width (DESIGN.md: sidebar 200px); rows never widen it (names and
    // status messages ellipsize, progress bars have no minimum width).
    scrolled.set_size_request(200, -1);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scrolled.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&list_box));
    scrolled.set_vexpand(true);

    // Sidebar container: Box(V) > [ScrolledWindow(ListBox), Button(+)]
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");
    sidebar_box.append(&scrolled);

    // '+' button at the bottom (D-01)
    let add_btn = gtk4::Button::with_label("+");
    add_btn.add_css_class("sidebar-add-btn");
    add_btn.set_tooltip_text(Some("New Workspace (Ctrl+N)"));
    add_btn.set_action_name(Some("win.new-workspace"));
    sidebar_box.append(&add_btn);

    (sidebar_box, scrolled, list_box)
}

/// Wire sidebar click-to-switch. Called from main.rs after AppState is constructed.
/// Per WS-03: clicking a row calls AppState.switch_to_index.
pub fn wire_sidebar_clicks(
    list_box: &gtk4::ListBox,
    state: Rc<RefCell<crate::app_state::AppState>>,
) {
    list_box.connect_row_activated({
        let state = state.clone();
        move |_list, row| {
            let index = {
                let s = state.borrow();
                workspace_index_for_row(&s, row).unwrap_or(row.index() as usize)
            };
            state.borrow_mut().switch_to_index(index);
            // SPLIT-07: call ghostty_surface_set_focus on the newly active pane.
            // Workspace switches are focus changes — must call set_focus after switch.
            let surface = {
                let mut s = state.borrow_mut();
                s.active_split_engine_mut()
                    .and_then(|engine| engine.root.find_active_pane_id())
                    .and_then(|pane_id| {
                        if let Ok(reg) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
                            reg.iter()
                                .find(|(_, &pid)| pid == pane_id)
                                .map(|(&ptr, _)| ptr as crate::ghostty::ffi::ghostty_surface_t)
                        } else {
                            None
                        }
                    })
            };
            if let Some(surface) = surface {
                unsafe {
                    crate::ghostty::ffi::ghostty_surface_set_focus(surface, true);
                }
            }
        }
    });
}

/// Widgets of one workspace's sidebar row, stored on the Workspace so updates never
/// have to walk the widget tree.
///
/// Layout: ListBoxRow > GtkBox(H, 4) > [
///   GtkBox(V, 0) > [name_label, subtitle_label, GtkBox(H) > [status_badge, status_message], progress],
///   attention_dot, close_button ]
///
/// - subtitle_label: SSH connection state (hidden for local workspaces)
/// - status_badge/status_message: agent status (hidden when no status is set)
/// - progress: OSC 9;4 progress of a pane in this workspace (hidden when none)
/// - close_button: hidden by default, shown on row hover via CSS (D-02)
#[derive(Clone, Debug)]
pub struct SidebarRow {
    pub row: gtk4::ListBoxRow,
    pub name_label: gtk4::Label,
    pub subtitle_label: gtk4::Label,
    pub status_box: gtk4::Box,
    pub status_lamp: gtk4::Box,
    pub status_badge: gtk4::Label,
    pub status_message: gtk4::Label,
    pub progress: gtk4::ProgressBar,
    pub attention_dot: gtk4::Box,
    pub close_button: gtk4::Button,
}

impl SidebarRow {
    /// Build the row for `workspace` and apply its current state.
    pub fn new(workspace: &crate::workspace::Workspace) -> Self {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.set_hexpand(true);

        let name_label = gtk4::Label::new(Some(&workspace.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        vbox.append(&name_label);

        let subtitle_label = gtk4::Label::new(None);
        subtitle_label.set_halign(gtk4::Align::Start);
        subtitle_label.add_css_class("connection-state");
        subtitle_label.set_visible(false);
        vbox.append(&subtitle_label);

        // Status line: lamp + state word in its signal color + agent message (DESIGN.md).
        let status_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        status_box.add_css_class("status-line");
        status_box.set_visible(false);
        let status_lamp = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        status_lamp.add_css_class("status-lamp");
        status_lamp.set_valign(gtk4::Align::Center);
        status_box.append(&status_lamp);
        let status_badge = gtk4::Label::new(None);
        status_badge.add_css_class("agent-status");
        status_box.append(&status_badge);
        let status_message = gtk4::Label::new(None);
        status_message.set_halign(gtk4::Align::Start);
        status_message.set_hexpand(true);
        status_message.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        status_message.add_css_class("agent-status-message");
        status_box.append(&status_message);
        vbox.append(&status_box);

        let progress = gtk4::ProgressBar::new();
        progress.add_css_class("workspace-progress");
        progress.set_visible(false);
        vbox.append(&progress);

        hbox.append(&vbox);

        let attention_dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        attention_dot.add_css_class("attention-dot");
        attention_dot.set_valign(gtk4::Align::Center);
        attention_dot.set_visible(false);
        hbox.append(&attention_dot);

        let close_button = gtk4::Button::with_label("\u{00D7}"); // Unicode multiplication sign
        close_button.add_css_class("sidebar-close-btn");
        close_button.set_tooltip_text(Some("Close Workspace"));
        close_button.set_valign(gtk4::Align::Center);
        hbox.append(&close_button);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        unsafe {
            row.set_data("workspace-id", workspace.id);
        }

        let sidebar_row = SidebarRow {
            row,
            name_label,
            subtitle_label,
            status_box,
            status_lamp,
            status_badge,
            status_message,
            progress,
            attention_dot,
            close_button,
        };
        sidebar_row.sync(workspace);
        sidebar_row
    }

    /// Apply all of the workspace's displayed state to the row.
    pub fn sync(&self, workspace: &crate::workspace::Workspace) {
        self.set_name(&workspace.name);
        self.set_connection_state(&workspace.connection_state);
        self.set_attention(workspace.has_attention);
        self.set_agent_status(workspace.agent_status.as_ref());
        self.set_progress(workspace.progress);
    }

    pub fn set_name(&self, name: &str) {
        self.name_label.set_text(name);
        self.name_label.set_tooltip_text(Some(name));
    }

    pub fn set_active(&self, active: bool) {
        if active {
            self.row.add_css_class("active-workspace");
            self.name_label.add_css_class("active-workspace-label");
        } else {
            self.row.remove_css_class("active-workspace");
            self.name_label.remove_css_class("active-workspace-label");
        }
    }

    pub fn set_attention(&self, has_attention: bool) {
        self.attention_dot.set_visible(has_attention);
    }

    pub fn set_connection_state(&self, state: &crate::workspace::ConnectionState) {
        use crate::workspace::ConnectionState;
        self.subtitle_label.set_visible(state.is_remote());
        self.subtitle_label.set_text(state.display_text());
        for s in [ConnectionState::Connected, ConnectionState::Disconnected, ConnectionState::Reconnecting(0)] {
            self.subtitle_label.remove_css_class(s.css_class());
        }
        if state.is_remote() {
            self.subtitle_label.add_css_class(state.css_class());
        }
    }

    pub fn set_agent_status(&self, status: Option<&crate::agent_status::AgentStatus>) {
        for s in crate::agent_status::AgentState::ALL {
            self.status_badge.remove_css_class(s.css_class());
            self.status_lamp.remove_css_class(s.css_class());
        }
        match status {
            Some(st) => {
                self.status_badge.set_text(st.state.label());
                self.status_badge.add_css_class(st.state.css_class());
                self.status_lamp.add_css_class(st.state.css_class());
                let msg = st.message.as_deref().unwrap_or("");
                self.status_message.set_text(msg);
                self.status_message.set_tooltip_text(if msg.is_empty() { None } else { Some(msg) });
                self.status_message.set_visible(!msg.is_empty());
                self.status_box.set_visible(true);
            }
            None => {
                self.status_box.set_visible(false);
                self.status_badge.set_text("");
                self.status_message.set_text("");
            }
        }
        // Keep the reserved (transparent) progress space in step with the status line.
        if self.progress.opacity() == 0.0 {
            self.progress.set_visible(self.status_box.is_visible());
        }
    }

    pub fn set_progress(&self, progress: Option<(crate::agent_status::ProgressState, i8)>) {
        use crate::agent_status::ProgressState;
        self.progress.remove_css_class("progress-error");
        self.progress.remove_css_class("progress-paused");
        let Some((state, value)) = progress else {
            self.hide_progress();
            return;
        };
        match state {
            ProgressState::Remove => {
                self.hide_progress();
                return;
            }
            ProgressState::Error => self.progress.add_css_class("progress-error"),
            ProgressState::Pause => self.progress.add_css_class("progress-paused"),
            ProgressState::Set | ProgressState::Indeterminate => {}
        }
        if state == ProgressState::Indeterminate || value < 0 {
            self.progress.pulse();
        } else {
            self.progress.set_fraction(value.clamp(0, 100) as f64 / 100.0);
        }
        self.progress.set_opacity(1.0);
        self.progress.set_visible(true);
    }

    /// Hide the progress bar. A row with a status line keeps the bar's space
    /// (transparent) so the list doesn't jump when a bar comes and goes.
    fn hide_progress(&self) {
        self.progress.set_opacity(0.0);
        self.progress.set_visible(self.status_box.is_visible());
    }
}

/// Index of the workspace whose sidebar row is `row`.
fn workspace_index_for_row(state: &crate::app_state::AppState, row: &gtk4::ListBoxRow) -> Option<usize> {
    state.workspaces.iter().position(|ws| {
        ws.sidebar_row.as_ref().map(|r| &r.row == row).unwrap_or(false)
    })
}

/// Start inline rename for the active workspace row.
/// Per UI-SPEC: swaps the name label for a GtkEntry; Enter/focus-out commits, Escape cancels.
/// Only the name label is swapped, so the status badge, progress and attention dot stay.
/// Per D-03: rename triggered by Ctrl+Shift+R (keyboard only).
pub fn start_inline_rename(
    _list_box: &gtk4::ListBox,
    active_index: usize,
    state: Rc<RefCell<crate::app_state::AppState>>,
) {
    let (sidebar_row, workspace_id, current_name) = {
        let s = state.borrow();
        let Some(ws) = s.workspaces.get(active_index) else { return };
        let Some(row) = ws.sidebar_row.clone() else { return };
        (row, ws.id, ws.name.clone())
    };
    // Already renaming: the label is hidden while the entry is shown.
    if !sidebar_row.name_label.is_visible() {
        return;
    }
    let Some(parent) = sidebar_row.name_label.parent().and_downcast::<gtk4::Box>() else {
        return;
    };

    let entry = gtk4::Entry::new();
    entry.set_text(&current_name);
    entry.set_placeholder_text(Some("Workspace name"));
    entry.add_css_class("rename-entry");
    parent.insert_child_after(&entry, Some(&sidebar_row.name_label));
    sidebar_row.name_label.set_visible(false);
    entry.grab_focus();

    // Remove the entry and show the label again. Returns false if already finished
    // (Enter and the focus-out that follows both try to finish).
    let finish = {
        let label = sidebar_row.name_label.clone();
        move |e: &gtk4::Entry| -> bool {
            let Some(p) = e.parent().and_downcast::<gtk4::Box>() else { return false };
            p.remove(e);
            label.set_visible(true);
            true
        }
    };
    let commit = {
        let state = state.clone();
        let finish = finish.clone();
        move |e: &gtk4::Entry| {
            let trimmed = e.text().trim().to_string();
            if finish(e) && !trimmed.is_empty() {
                // Deferred: focus-out can fire synchronously while the caller holds a
                // mutable borrow of state (e.g. switch_to_index → grab_focus).
                let state = state.clone();
                gtk4::glib::idle_add_local_once(move || {
                    state.borrow_mut().rename_workspace(workspace_id, trimmed);
                });
            }
        }
    };

    // Enter key: commit rename.
    entry.connect_activate({
        let commit = commit.clone();
        move |e| commit(e)
    });

    // Focus-out: commit rename (same as Enter).
    entry.connect_notify_local(Some("has-focus"), move |e, _| {
        if !e.has_focus() {
            commit(e);
        }
    });

    // Escape key: cancel rename and restore the label.
    let key_ctrl = gtk4::EventControllerKey::new();
    key_ctrl.connect_key_pressed({
        let entry = entry.clone();
        move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                finish(&entry);
                gtk4::glib::Propagation::Stop
            } else {
                gtk4::glib::Propagation::Proceed
            }
        }
    });
    entry.add_controller(key_ctrl);
}

/// Wire the close button and right-click context menu of a freshly built row.
/// Called by AppState when the row is created; `state` is weak because AppState owns the row.
pub fn wire_row(
    sidebar_row: &SidebarRow,
    state: std::rc::Weak<RefCell<crate::app_state::AppState>>,
    app: &gtk4::Application,
) {
    sidebar_row.close_button.connect_clicked({
        let state = state.clone();
        let app = app.clone();
        let row = sidebar_row.row.clone();
        move |_| {
            let Some(state) = state.upgrade() else { return };
            let index = {
                let s = state.borrow();
                if s.workspaces.len() <= 1 {
                    return; // Cannot close last workspace
                }
                match workspace_index_for_row(&s, &row) {
                    Some(i) => i,
                    None => return,
                }
            };
            // Switch to this workspace first (so close_workspace operates on the right one)
            state.borrow_mut().switch_to_index(index);
            crate::shortcuts::handle_close_workspace(&state, &app);
        }
    });
    attach_sidebar_context_menu(&sidebar_row.row, state);
}

/// Attach right-click context menu to a sidebar row (D-03), including the
/// "Mark as Unread / Mark as Read" toggle.
fn attach_sidebar_context_menu(
    row: &gtk4::ListBoxRow,
    state: std::rc::Weak<RefCell<crate::app_state::AppState>>,
) {
    let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);
    popover.set_parent(row);
    popover.set_has_arrow(false);

    // Row-scoped action for the read/unread toggle ("sidebar-row.toggle-unread").
    let group = gtk4::gio::SimpleActionGroup::new();
    let toggle = gtk4::gio::SimpleAction::new("toggle-unread", None);
    toggle.connect_activate({
        let state = state.clone();
        let row = row.clone();
        move |_, _| {
            let Some(state) = state.upgrade() else { return };
            let mut s = state.borrow_mut();
            if let Some(i) = workspace_index_for_row(&s, &row) {
                let unread = s.workspaces[i].has_attention;
                s.set_workspace_unread(i, !unread);
            }
        }
    });
    group.add_action(&toggle);
    row.insert_action_group("sidebar-row", Some(&group));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // Right-click
    gesture.connect_released({
        let popover = popover.clone();
        let row = row.clone();
        move |_, _, x, y| {
            let Some(state) = state.upgrade() else { return };
            let unread = {
                let mut s = state.borrow_mut();
                let Some(index) = workspace_index_for_row(&s, &row) else { return };
                // Switch to this workspace first so context menu actions apply to it.
                // Opening the menu is not a visit: keep the unread/attention state so
                // that "Mark as Read" is offered for unread workspaces.
                let unread = s.workspaces[index].has_attention;
                s.switch_to_index_keep_unread(index);
                unread
            };
            let menu = crate::menus::build_sidebar_context_menu();
            let read_section = gtk4::gio::Menu::new();
            read_section.append(
                Some(if unread { "Mark as Read" } else { "Mark as Unread" }),
                Some("sidebar-row.toggle-unread"),
            );
            menu.append_section(None, &read_section);
            popover.set_menu_model(Some(&menu));
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                x as i32, y as i32, 1, 1,
            )));
            popover.popup();
        }
    });
    row.add_controller(gesture);
}

