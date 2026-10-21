use gtk4::prelude::*;
use gtk4::gio;
use std::cell::RefCell;
use std::rc::Rc;

/// Register all GIO actions on the ApplicationWindow.
/// Actions are named "win.{action-name}" and can be invoked by buttons, menus, and shortcuts.
/// Per D-11, D-12: all menu items mirror existing keyboard shortcut actions.
pub fn register_actions(
    window: &gtk4::ApplicationWindow,
    state: Rc<RefCell<crate::app_state::AppState>>,
    sidebar: &gtk4::Box,
    app: &gtk4::Application,
) {
    // --- File section actions ---

    // win.new-workspace (D-01, D-05)
    let action = gio::SimpleAction::new("new-workspace", None);
    action.connect_activate({
        let state = state.clone();
        let app = app.clone();
        move |_, _| {
            crate::shortcuts::handle_new_workspace(&state, &app);
        }
    });
    window.add_action(&action);

    // win.new-ssh-workspace
    let action = gio::SimpleAction::new("new-ssh-workspace", None);
    action.connect_activate({
        let state = state.clone();
        let app = app.clone();
        move |_, _| {
            crate::shortcuts::handle_new_ssh_workspace(&state, &app);
        }
    });
    window.add_action(&action);

    // win.browser-open (D-07)
    let action = gio::SimpleAction::new("browser-open", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::shortcuts::handle_browser_open(&state);
        }
    });
    window.add_action(&action);

    // win.browser-close: removes the preview panes and stops the browser.
    let action = gio::SimpleAction::new("browser-close", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::shortcuts::handle_browser_close(&state);
        }
    });
    window.add_action(&action);

    // win.close-pane
    let action = gio::SimpleAction::new("close-pane", None);
    action.connect_activate({
        let state = state.clone();
        let app = app.clone();
        move |_, _| {
            crate::shortcuts::handle_close_pane(&state, &app);
        }
    });
    window.add_action(&action);

    // win.close-workspace
    let action = gio::SimpleAction::new("close-workspace", None);
    action.connect_activate({
        let state = state.clone();
        let app = app.clone();
        move |_, _| {
            crate::shortcuts::handle_close_workspace(&state, &app);
        }
    });
    window.add_action(&action);

    // --- Edit section actions ---

    // win.copy (Ctrl+Shift+C) -- invoke Ghostty's copy_to_clipboard binding action
    let action = gio::SimpleAction::new("copy", None);
    action.connect_activate({
        let state = state.clone();
        let window = window.clone();
        move |_, _| {
            perform_terminal_binding(&window, &state, b"copy_to_clipboard");
        }
    });
    window.add_action(&action);

    // win.paste (Ctrl+Shift+V) -- invoke Ghostty's paste_from_clipboard binding action
    let action = gio::SimpleAction::new("paste", None);
    action.connect_activate({
        let state = state.clone();
        let window = window.clone();
        move |_, _| {
            perform_terminal_binding(&window, &state, b"paste_from_clipboard");
        }
    });
    window.add_action(&action);

    // win.find (Ctrl+Shift+F) -- open the terminal find bar on the focused pane
    // (terminal-actions). Ctrl+F stays with terminal apps.
    let action = gio::SimpleAction::new("find", None);
    action.connect_activate(|_, _| {
        crate::find_bar::open_for_current(true);
    });
    window.add_action(&action);

    // win.preferences (D-13) -- open config.toml in $EDITOR
    let action = gio::SimpleAction::new("preferences", None);
    action.connect_activate(move |_, _| {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "xdg-open".to_string());
        let config_path = crate::config::config_path();
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !config_path.exists() {
            let _ = std::fs::write(&config_path, "# cmux configuration\n# See documentation for options\n");
        }
        let _ = std::process::Command::new(&editor)
            .arg(&config_path)
            .spawn();
    });
    window.add_action(&action);

    // --- View section actions ---

    // win.toggle-sidebar
    let action = gio::SimpleAction::new("toggle-sidebar", None);
    action.connect_activate({
        let state = state.clone();
        let sidebar = sidebar.clone();
        move |_, _| {
            let visible = sidebar.is_visible();
            sidebar.set_visible(!visible);
            if let Some(engine) = state.borrow_mut().active_split_engine_mut() {
                engine.focus_active_surface();
            }
        }
    });
    window.add_action(&action);

    // win.split-right
    let action = gio::SimpleAction::new("split-right", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::shortcuts::handle_split(&state, false);
        }
    });
    window.add_action(&action);

    // win.split-down
    let action = gio::SimpleAction::new("split-down", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::shortcuts::handle_split(&state, true);
        }
    });
    window.add_action(&action);

    // pane-zoom: win.toggle-pane-zoom
    let action = gio::SimpleAction::new("toggle-pane-zoom", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::shortcuts::handle_pane_zoom(&state, None);
        }
    });
    window.add_action(&action);

    // win.rename-workspace
    let action = gio::SimpleAction::new("rename-workspace", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            let (active_index, sidebar_list) = {
                let s = state.borrow();
                (s.active_index, s.sidebar_list.clone())
            };
            crate::sidebar::start_inline_rename(&sidebar_list, active_index, state.clone());
        }
    });
    window.add_action(&action);

    // --- Help section actions ---

    // win.keyboard-shortcuts (D-14)
    let action = gio::SimpleAction::new("keyboard-shortcuts", None);
    action.connect_activate({
        let window_weak = window.downgrade();
        move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let sw = build_shortcuts_window();
                sw.set_transient_for(Some(&win));
                sw.present();
            }
        }
    });
    window.add_action(&action);

    // win.about (D-15)
    let action = gio::SimpleAction::new("about", None);
    action.connect_activate({
        let window_weak = window.downgrade();
        move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let about = gtk4::AboutDialog::builder()
                    .program_name("cmux")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("GPU-accelerated terminal multiplexer for Linux")
                    .website("https://github.com/manaflow-ai/cmux")
                    .license_type(gtk4::License::MitX11)
                    .transient_for(&win)
                    .modal(true)
                    .build();
                about.present();
            }
        }
    });
    window.add_action(&action);

    // app.quit
    let quit_action = gio::SimpleAction::new("quit", None);
    quit_action.connect_activate({
        let app = app.clone();
        move |_, _| {
            app.quit();
        }
    });
    app.add_action(&quit_action);

    // --- Browser-specific actions (D-09) ---

    // win.open-external-browser -- opens current browser pane URL in xdg-open
    // Disabled until BrowserManager exposes current_url() (wired in Plan 03)
    let action = gio::SimpleAction::new("open-external-browser", None);
    action.set_enabled(false); // TODO: enable when BrowserManager.current_url() is available
    window.add_action(&action);

    // win.copy-url -- copies current browser pane URL to clipboard
    // Disabled until BrowserManager exposes current_url() (wired in Plan 03)
    let action = gio::SimpleAction::new("copy-url", None);
    action.set_enabled(false); // TODO: enable when BrowserManager.current_url() is available
    window.add_action(&action);

    // tabs: win.rename-tab
    let action = gio::SimpleAction::new("rename-tab", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| crate::tabs::handle_rename_tab(&state)
    });
    window.add_action(&action);

    // tabs: win.new-tab / win.next-tab / win.prev-tab
    let action = gio::SimpleAction::new("new-tab", None);
    action.connect_activate({
        let state = state.clone();
        move |_, _| {
            crate::tabs::handle_new_tab(&state);
        }
    });
    window.add_action(&action);
    for (name, forward) in [("next-tab", true), ("prev-tab", false)] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let state = state.clone();
            move |_, _| crate::tabs::handle_cycle_tab(&state, forward)
        });
        window.add_action(&action);
    }
}

/// Run a Ghostty binding action on the terminal that has keyboard focus.
///
/// Prefer the focused GLArea (resolved through GL_TO_SURFACE) over the split
/// tree: after a session restore, panes in different workspaces can share a
/// pane_id, so find_surface_for_pane may return a stale or unrealized surface.
fn perform_terminal_binding(
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<crate::app_state::AppState>>,
    action: &[u8],
) {
    let focused = gtk4::prelude::GtkWindowExt::focus(window).and_then(|w| {
        let map = crate::ghostty::callbacks::GL_TO_SURFACE.lock().ok()?;
        map.get(&(w.as_ptr() as usize)).copied()
    });
    let surface = match focused {
        Some(s) => s as crate::ghostty::ffi::ghostty_surface_t,
        None => {
            let s = state.borrow();
            let Some(surface) = s.active_split_engine().and_then(|engine| {
                let pane_id = engine.root.find_active_pane_id()?;
                engine.root.find_surface_for_pane(pane_id)
            }) else {
                return;
            };
            surface
        }
    };
    if surface.is_null() {
        return;
    }
    unsafe {
        crate::ghostty::ffi::ghostty_surface_binding_action(
            surface,
            action.as_ptr() as *const _,
            action.len(),
        );
    }
}

thread_local! {
    /// The effective shortcut map, stored by register_accels so the shortcuts
    /// help window shows `[shortcuts]` overrides from config.toml.
    static SHORTCUTS: RefCell<Option<crate::config::ShortcutMap>> = const { RefCell::new(None) };
}

/// Register keyboard accelerators for GIO actions so menus show shortcut hints.
/// Per Pitfall 3 from RESEARCH.md: GTK4 shows accels in menus ONLY if registered via set_accels_for_action.
/// Accelerators of configurable actions come from the effective ShortcutMap.
pub fn register_accels(app: &gtk4::Application, shortcuts: &crate::config::ShortcutMap) {
    use crate::config::ShortcutAction as A;
    let configurable: &[(&str, A)] = &[
        ("win.new-workspace", A::NewWorkspace),
        ("win.close-workspace", A::CloseWorkspace),
        ("win.new-ssh-workspace", A::NewSshWorkspace),
        ("win.browser-open", A::BrowserOpen),
        ("win.browser-close", A::BrowserClose),
        ("win.close-pane", A::ClosePane),
        ("win.toggle-sidebar", A::ToggleSidebar),
        ("win.split-right", A::SplitRight),
        ("win.split-down", A::SplitDown),
        ("win.rename-workspace", A::RenameWorkspace),
        ("win.toggle-pane-zoom", A::TogglePaneZoom), // pane-zoom
    ];
    for (action, shortcut_action) in configurable {
        if let Some(accel) = shortcuts.accel(*shortcut_action) {
            app.set_accels_for_action(action, &[accel]);
        }
    }
    // Not configurable through [shortcuts].
    app.set_accels_for_action("win.copy", &["<Ctrl><Shift>c"]);
    app.set_accels_for_action("win.paste", &["<Ctrl><Shift>v"]);
    app.set_accels_for_action("win.find", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    // tabs: next/previous tab have two bindings each.
    let pairs: &[(&str, &[A])] = &[
        ("win.new-tab", &[A::NewTab]),
        ("win.rename-tab", &[A::RenameTab]),
        ("win.next-tab", &[A::NextTab, A::NextTabAlt]),
        ("win.prev-tab", &[A::PrevTab, A::PrevTabAlt]),
    ];
    for (action, actions) in pairs {
        let accels: Vec<&str> = actions.iter().filter_map(|a| shortcuts.accel(*a)).collect();
        app.set_accels_for_action(action, &accels);
    }
    SHORTCUTS.with(|s| *s.borrow_mut() = Some(shortcuts.clone()));
}

/// Build the hamburger menu model (D-11, D-12).
/// Returns a gio::Menu that can be set on a MenuButton.
/// Per D-12: sections use File/Edit/View/Help labels.
pub fn build_hamburger_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // File section (D-12)
    let file_section = gio::Menu::new();
    file_section.append(Some("New Workspace"), Some("win.new-workspace"));
    file_section.append(Some("New SSH Workspace"), Some("win.new-ssh-workspace"));
    file_section.append(Some("New Browser"), Some("win.browser-open"));
    file_section.append(Some("New Tab"), Some("win.new-tab")); // tabs
    file_section.append(Some("Rename Tab…"), Some("win.rename-tab")); // tabs
    file_section.append(Some("Close Pane"), Some("win.close-pane"));
    file_section.append(Some("Close Workspace"), Some("win.close-workspace"));
    file_section.append(Some("Quit"), Some("app.quit"));
    menu.append_section(Some("File"), &file_section);

    // Edit section (D-12)
    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy"), Some("win.copy"));
    edit_section.append(Some("Paste"), Some("win.paste"));
    edit_section.append(Some("Find"), Some("win.find"));
    edit_section.append(Some("Preferences"), Some("win.preferences"));
    menu.append_section(Some("Edit"), &edit_section);

    // View section (D-12)
    let view_section = gio::Menu::new();
    view_section.append(Some("Toggle Sidebar"), Some("win.toggle-sidebar"));
    view_section.append(Some("Split Right"), Some("win.split-right"));
    view_section.append(Some("Split Down"), Some("win.split-down"));
    view_section.append(Some("Toggle Pane Zoom"), Some("win.toggle-pane-zoom")); // pane-zoom
    menu.append_section(Some("View"), &view_section);

    // Help section (D-12)
    let help_section = gio::Menu::new();
    help_section.append(Some("Keyboard Shortcuts"), Some("win.keyboard-shortcuts"));
    help_section.append(Some("About cmux"), Some("win.about"));
    menu.append_section(Some("Help"), &help_section);

    menu
}

/// Build sidebar workspace row context menu (D-03).
pub fn build_sidebar_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Rename"), Some("win.rename-workspace"));
    menu.append(Some("Close"), Some("win.close-workspace"));
    menu.append(Some("Split Right"), Some("win.split-right"));
    menu.append(Some("Split Down"), Some("win.split-down"));
    menu
}

/// Build terminal pane context menu (D-08).
pub fn build_terminal_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy"), Some("win.copy"));
    edit_section.append(Some("Paste"), Some("win.paste"));
    menu.append_section(None, &edit_section);

    let pane_section = gio::Menu::new();
    pane_section.append(Some("Split Right"), Some("win.split-right"));
    pane_section.append(Some("Split Down"), Some("win.split-down"));
    pane_section.append(Some("Toggle Pane Zoom"), Some("win.toggle-pane-zoom")); // pane-zoom
    pane_section.append(Some("New Tab"), Some("win.new-tab")); // tabs
    pane_section.append(Some("Rename Tab…"), Some("win.rename-tab")); // tabs
    pane_section.append(Some("Close Pane"), Some("win.close-pane"));
    menu.append_section(None, &pane_section);

    let browser_section = gio::Menu::new();
    browser_section.append(Some("Open Browser Here"), Some("win.browser-open"));
    menu.append_section(None, &browser_section);

    menu
}

/// Build browser preview pane context menu (D-09).
/// Includes Open in External Browser and Copy URL actions.
pub fn build_browser_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Open in External Browser"), Some("win.open-external-browser"));
    menu.append(Some("Copy URL"), Some("win.copy-url"));
    menu.append(Some("Toggle Full Pane"), Some("win.toggle-pane-zoom")); // pane-zoom
    menu.append(Some("Close Browser"), Some("win.browser-close"));
    menu
}

/// Build GtkShortcutsWindow (D-14) with all shortcuts grouped by category.
/// Uses GTK 4.14 add_section/add_group API.
fn build_shortcuts_window() -> gtk4::ShortcutsWindow {
    use crate::config::ShortcutAction as A;
    let window = gtk4::ShortcutsWindow::builder().build();

    // Configurable shortcuts are shown with their effective accelerators.
    let map = SHORTCUTS
        .with(|s| s.borrow().clone())
        .unwrap_or_else(|| crate::config::ShortcutMap::from_config(&Default::default()));
    let accel = |action: A| map.accel(action).unwrap_or_default().to_string();
    let accel_range = |actions: &[A]| {
        let accels: Vec<String> = actions.iter().map(|a| accel(*a)).collect();
        numbered_accel_range(&accels)
    };

    // Workspaces section
    let ws_section = gtk4::ShortcutsSection::builder()
        .section_name("workspaces")
        .title("Workspaces")
        .build();

    let ws_group = gtk4::ShortcutsGroup::builder()
        .title("Workspaces")
        .build();
    ws_group.add_shortcut(&shortcut(&accel(A::NewWorkspace), "New Workspace"));
    ws_group.add_shortcut(&shortcut(&accel(A::CloseWorkspace), "Close Workspace"));
    ws_group.add_shortcut(&shortcut(&format!("{} {}", accel(A::NextWorkspace), accel(A::NextWorkspaceAlt)), "Next Workspace"));
    ws_group.add_shortcut(&shortcut(&format!("{} {}", accel(A::PrevWorkspace), accel(A::PrevWorkspaceAlt)), "Previous Workspace"));
    ws_group.add_shortcut(&shortcut(&accel(A::RenameWorkspace), "Rename Workspace"));
    ws_group.add_shortcut(&shortcut(
        &accel_range(&[
            A::Workspace1, A::Workspace2, A::Workspace3, A::Workspace4, A::Workspace5,
            A::Workspace6, A::Workspace7, A::Workspace8, A::Workspace9,
        ]),
        "Switch to Workspace 1-9",
    ));
    ws_section.add_group(&ws_group);
    window.add_section(&ws_section);

    // Panes section
    let pane_section = gtk4::ShortcutsSection::builder()
        .section_name("panes")
        .title("Panes")
        .build();

    let pane_group = gtk4::ShortcutsGroup::builder()
        .title("Panes")
        .build();
    pane_group.add_shortcut(&shortcut(&accel(A::SplitRight), "Split Right"));
    pane_group.add_shortcut(&shortcut(&accel(A::SplitDown), "Split Down"));
    pane_group.add_shortcut(&shortcut(&accel(A::ClosePane), "Close Pane"));
    pane_group.add_shortcut(&shortcut(&accel(A::FocusLeft), "Focus Left"));
    pane_group.add_shortcut(&shortcut(&accel(A::FocusRight), "Focus Right"));
    pane_group.add_shortcut(&shortcut(&accel(A::FocusUp), "Focus Up"));
    pane_group.add_shortcut(&shortcut(&accel(A::FocusDown), "Focus Down"));
    // pane-nav
    pane_group.add_shortcut(&shortcut(
        &accel_range(&[
            A::FocusPane1, A::FocusPane2, A::FocusPane3, A::FocusPane4, A::FocusPane5,
            A::FocusPane6, A::FocusPane7, A::FocusPane8, A::FocusPane9,
        ]),
        "Focus Pane 1-9",
    ));
    pane_group.add_shortcut(&shortcut(&accel(A::FocusLastPane), "Focus Previous Pane"));
    pane_group.add_shortcut(&shortcut(&accel(A::ResizeLeft), "Move Divider Left"));
    pane_group.add_shortcut(&shortcut(&accel(A::ResizeRight), "Move Divider Right"));
    pane_group.add_shortcut(&shortcut(&accel(A::ResizeUp), "Move Divider Up"));
    pane_group.add_shortcut(&shortcut(&accel(A::ResizeDown), "Move Divider Down"));
    pane_group.add_shortcut(&shortcut(&accel(A::EqualizeSplits), "Equalize Splits"));
    pane_group.add_shortcut(&shortcut(&accel(A::TogglePaneZoom), "Toggle Pane Zoom")); // pane-zoom
    pane_section.add_group(&pane_group);
    // tabs
    let tab_group = gtk4::ShortcutsGroup::builder()
        .title("Tabs")
        .build();
    tab_group.add_shortcut(&shortcut(&accel(A::NewTab), "New Tab"));
    tab_group.add_shortcut(&shortcut(&accel(A::RenameTab), "Rename Tab"));
    tab_group.add_shortcut(&shortcut(&accel(A::ClosePane), "Close Tab (closes the pane on its last tab)"));
    tab_group.add_shortcut(&shortcut(&format!("{} {}", accel(A::NextTab), accel(A::NextTabAlt)), "Next Tab"));
    tab_group.add_shortcut(&shortcut(&format!("{} {}", accel(A::PrevTab), accel(A::PrevTabAlt)), "Previous Tab"));
    tab_group.add_shortcut(&shortcut(
        &accel_range(&[
            A::GotoTab1, A::GotoTab2, A::GotoTab3, A::GotoTab4, A::GotoTab5,
            A::GotoTab6, A::GotoTab7, A::GotoTab8, A::GotoTab9,
        ]),
        "Go to Tab 1-9",
    ));
    pane_section.add_group(&tab_group);
    window.add_section(&pane_section);

    // Edit section
    let edit_section = gtk4::ShortcutsSection::builder()
        .section_name("edit")
        .title("Edit")
        .build();

    let edit_group = gtk4::ShortcutsGroup::builder()
        .title("Edit")
        .build();
    edit_group.add_shortcut(&shortcut("<Ctrl><Shift>c", "Copy"));
    edit_group.add_shortcut(&shortcut("<Ctrl><Shift>v", "Paste"));
    edit_group.add_shortcut(&shortcut("<Ctrl><Shift>f", "Find"));
    edit_section.add_group(&edit_group);
    window.add_section(&edit_section);

    // View section
    let view_section = gtk4::ShortcutsSection::builder()
        .section_name("view")
        .title("View")
        .build();

    let view_group = gtk4::ShortcutsGroup::builder()
        .title("View")
        .build();
    view_group.add_shortcut(&shortcut(&accel(A::ToggleSidebar), "Toggle Sidebar"));
    view_group.add_shortcut(&shortcut(&accel(A::BrowserOpen), "Open Browser"));
    view_group.add_shortcut(&shortcut(&accel(A::BrowserClose), "Close Browser"));
    view_group.add_shortcut(&shortcut(&accel(A::NewSshWorkspace), "New SSH Workspace"));
    view_section.add_group(&view_group);
    window.add_section(&view_section);

    // General section
    let general_section = gtk4::ShortcutsSection::builder()
        .section_name("general")
        .title("General")
        .build();

    let general_group = gtk4::ShortcutsGroup::builder()
        .title("General")
        .build();
    general_group.add_shortcut(&shortcut("<Ctrl>q", "Quit"));
    general_section.add_group(&general_group);
    window.add_section(&general_section);

    window
}

/// Show nine numbered accelerators as one GtkShortcutsShortcut range
/// ("<Ctrl>1...<Ctrl>9") when they share modifiers and run 1..9 in order;
/// otherwise list them space-separated.
fn numbered_accel_range(accels: &[String]) -> String {
    let parsed: Vec<_> = accels.iter().map(|a| gtk4::accelerator_parse(a)).collect();
    let digits = [
        gtk4::gdk::Key::_1, gtk4::gdk::Key::_2, gtk4::gdk::Key::_3,
        gtk4::gdk::Key::_4, gtk4::gdk::Key::_5, gtk4::gdk::Key::_6,
        gtk4::gdk::Key::_7, gtk4::gdk::Key::_8, gtk4::gdk::Key::_9,
    ];
    let uniform = parsed.len() == digits.len()
        && parsed.iter().zip(digits).all(|(p, digit)| {
            matches!(p, Some((key, mods)) if *key == digit && Some(*mods) == parsed[0].map(|(_, m)| m))
        });
    match (uniform, accels.first(), accels.last()) {
        (true, Some(first), Some(last)) => format!("{first}...{last}"),
        _ => accels.join(" "),
    }
}

/// Helper to create a ShortcutsShortcut widget.
fn shortcut(accel: &str, title: &str) -> gtk4::ShortcutsShortcut {
    gtk4::ShortcutsShortcut::builder()
        .accelerator(accel)
        .title(title)
        .build()
}
