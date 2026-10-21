use gtk4::gdk::{Key, ModifierType};
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level config loaded from ~/.config/cmux/config.toml.
/// Phase 5: shortcuts section only (D-07).
#[derive(serde::Deserialize, Default, Debug)]
pub struct Config {
    #[serde(default)]
    pub shortcuts: ShortcutConfig,
    #[serde(default)]
    pub ui: UiConfig,
    // notifications
    #[serde(default)]
    pub notifications: NotificationsConfig,
}

/// Per-action shortcut overrides. Each value is a GTK accelerator string (e.g. "<Ctrl>n").
/// None means "use default".
#[derive(serde::Deserialize, Default, Debug)]
pub struct ShortcutConfig {
    pub new_workspace: Option<String>,
    pub close_workspace: Option<String>,
    pub next_workspace: Option<String>,
    pub prev_workspace: Option<String>,
    pub rename_workspace: Option<String>,
    pub toggle_sidebar: Option<String>,
    pub split_right: Option<String>,
    pub split_down: Option<String>,
    pub close_pane: Option<String>,
    pub new_ssh_workspace: Option<String>,
    pub focus_left: Option<String>,
    pub focus_right: Option<String>,
    pub focus_up: Option<String>,
    pub focus_down: Option<String>,
    pub workspace_1: Option<String>,
    pub workspace_2: Option<String>,
    pub workspace_3: Option<String>,
    pub workspace_4: Option<String>,
    pub workspace_5: Option<String>,
    pub workspace_6: Option<String>,
    pub workspace_7: Option<String>,
    pub workspace_8: Option<String>,
    pub workspace_9: Option<String>,
    pub browser_open: Option<String>,
    pub browser_close: Option<String>,
    // pane-nav
    pub focus_pane_1: Option<String>,
    pub focus_pane_2: Option<String>,
    pub focus_pane_3: Option<String>,
    pub focus_pane_4: Option<String>,
    pub focus_pane_5: Option<String>,
    pub focus_pane_6: Option<String>,
    pub focus_pane_7: Option<String>,
    pub focus_pane_8: Option<String>,
    pub focus_pane_9: Option<String>,
    pub focus_last_pane: Option<String>,
    pub resize_left: Option<String>,
    pub resize_right: Option<String>,
    pub resize_up: Option<String>,
    pub resize_down: Option<String>,
    pub equalize_splits: Option<String>,
    // tabs
    pub new_tab: Option<String>,
    pub next_tab: Option<String>,
    pub prev_tab: Option<String>,
    pub next_tab_alt: Option<String>,
    pub prev_tab_alt: Option<String>,
    pub goto_tab_1: Option<String>,
    pub goto_tab_2: Option<String>,
    pub goto_tab_3: Option<String>,
    pub goto_tab_4: Option<String>,
    pub goto_tab_5: Option<String>,
    pub goto_tab_6: Option<String>,
    pub goto_tab_7: Option<String>,
    pub goto_tab_8: Option<String>,
    pub goto_tab_9: Option<String>,
}

/// UI configuration section -- [ui] in config.toml (D-16).
#[derive(serde::Deserialize, Default, Debug)]
pub struct UiConfig {
    #[serde(default)]
    pub header_bar: HeaderBarConfig,
    /// tabs: show the tab bar on panes with a single tab too.
    #[serde(default)]
    pub always_show_tabs: bool,
}

/// Header bar configuration -- [ui.header_bar] in config.toml (D-16).
/// Requires app restart to take effect.
#[derive(serde::Deserialize, Debug)]
pub struct HeaderBarConfig {
    /// "gtk" (default, full header bar), "custom" (user-specified buttons), "none" (no header bar)
    #[serde(default = "default_header_style")]
    pub style: String,
    /// Button names for left side (only used when style="custom")
    pub buttons_left: Option<Vec<String>>,
    /// Button names for right side (only used when style="custom")
    pub buttons_right: Option<Vec<String>>,
}

fn default_header_style() -> String {
    "gtk".to_string()
}

impl Default for HeaderBarConfig {
    fn default() -> Self {
        Self {
            style: default_header_style(),
            buttons_left: None,
            buttons_right: None,
        }
    }
}

/// All bindable shortcut actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    NewWorkspace,
    CloseWorkspace,
    NextWorkspace,
    PrevWorkspace,
    RenameWorkspace,
    ToggleSidebar,
    SplitRight,
    SplitDown,
    ClosePane,
    NewSshWorkspace,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    Workspace1,
    Workspace2,
    Workspace3,
    Workspace4,
    Workspace5,
    Workspace6,
    Workspace7,
    Workspace8,
    Workspace9,
    BrowserOpen,
    BrowserClose,
    // pane-nav
    FocusPane1,
    FocusPane2,
    FocusPane3,
    FocusPane4,
    FocusPane5,
    FocusPane6,
    FocusPane7,
    FocusPane8,
    FocusPane9,
    FocusLastPane,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    EqualizeSplits,
    // tabs
    NewTab,
    NextTab,
    PrevTab,
    /// Second binding for NextTab (Ctrl+PageDown).
    NextTabAlt,
    /// Second binding for PrevTab (Ctrl+PageUp).
    PrevTabAlt,
    GotoTab1,
    GotoTab2,
    GotoTab3,
    GotoTab4,
    GotoTab5,
    GotoTab6,
    GotoTab7,
    GotoTab8,
    GotoTab9,
}

/// HashMap-based shortcut lookup table built from config + defaults.
#[derive(Clone)]
pub struct ShortcutMap {
    map: HashMap<(ModifierType, Key), ShortcutAction>,
    /// Effective accelerator string per action, in declaration order. Menus and
    /// the shortcuts window are generated from this so overrides show up there.
    accels: Vec<(ShortcutAction, String)>,
}

/// Known shortcut action names for unknown-key detection.
const KNOWN_SHORTCUTS: &[&str] = &[
    "new_workspace", "close_workspace", "next_workspace", "prev_workspace",
    "rename_workspace", "toggle_sidebar", "split_right", "split_down",
    "close_pane", "new_ssh_workspace", "focus_left", "focus_right", "focus_up", "focus_down",
    "workspace_1", "workspace_2", "workspace_3", "workspace_4",
    "workspace_5", "workspace_6", "workspace_7", "workspace_8", "workspace_9",
    "browser_open", "browser_close",
    // pane-nav
    "focus_pane_1", "focus_pane_2", "focus_pane_3", "focus_pane_4", "focus_pane_5",
    "focus_pane_6", "focus_pane_7", "focus_pane_8", "focus_pane_9",
    "focus_last_pane", "resize_left", "resize_right", "resize_up", "resize_down",
    "equalize_splits",
    // tabs
    "new_tab", "next_tab", "prev_tab", "next_tab_alt", "prev_tab_alt",
    "goto_tab_1", "goto_tab_2", "goto_tab_3", "goto_tab_4", "goto_tab_5",
    "goto_tab_6", "goto_tab_7", "goto_tab_8", "goto_tab_9",
];

/// Modifier mask for lookup: ignore Caps Lock, Num Lock, etc.
const MOD_MASK: ModifierType = ModifierType::from_bits_truncate(
    ModifierType::CONTROL_MASK.bits()
        | ModifierType::SHIFT_MASK.bits()
        | ModifierType::ALT_MASK.bits(),
);

/// Returns the config file path.
/// Respects $XDG_CONFIG_HOME/cmux/config.toml; falls back to ~/.config/cmux/config.toml (CFG-04).
pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/.config")
    });
    PathBuf::from(base).join("cmux").join("config.toml")
}

/// Load config from disk. Always returns a usable Config (D-10).
/// Missing file is silent; read/parse errors warn to stderr and fall back to defaults.
pub fn load_config() -> Config {
    let path = config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Config::default();
        }
        Err(e) => {
            eprintln!("cmux: config read error at {}: {e}", path.display());
            return Config::default();
        }
    };

    warn_unknown_shortcuts(&content);

    match toml::from_str::<Config>(&content) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("cmux: config parse error at {}: {e}", path.display());
            Config::default()
        }
    }
}

/// Warn about unknown keys in the [shortcuts] table (D-03).
fn warn_unknown_shortcuts(content: &str) {
    for key in unknown_shortcut_keys(content) {
        eprintln!("cmux: unknown shortcut action '{}' in config, ignoring", key);
    }
}

/// Keys of the [shortcuts] table that are not known actions.
fn unknown_shortcut_keys(content: &str) -> Vec<String> {
    let table: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return Vec::new(), // Parse errors are reported by load_config
    };
    table
        .get("shortcuts")
        .and_then(|v| v.as_table())
        .map(|shortcuts| {
            shortcuts
                .keys()
                .filter(|key| !KNOWN_SHORTCUTS.contains(&key.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

impl ShortcutMap {
    /// Build lookup table from config, falling back to defaults for unset/invalid entries.
    pub fn from_config(config: &ShortcutConfig) -> Self {
        let entries: &[(ShortcutAction, &Option<String>, &str)] = &[
            (ShortcutAction::NewWorkspace,    &config.new_workspace,    "<Ctrl>n"),
            (ShortcutAction::CloseWorkspace,  &config.close_workspace,  "<Ctrl><Shift>w"),
            (ShortcutAction::NextWorkspace,   &config.next_workspace,   "<Ctrl>bracketright"),
            (ShortcutAction::PrevWorkspace,   &config.prev_workspace,   "<Ctrl>bracketleft"),
            (ShortcutAction::RenameWorkspace, &config.rename_workspace, "<Ctrl><Shift>r"),
            (ShortcutAction::ToggleSidebar,   &config.toggle_sidebar,   "<Ctrl>b"),
            (ShortcutAction::SplitRight,      &config.split_right,      "<Ctrl>d"),
            (ShortcutAction::SplitDown,       &config.split_down,       "<Ctrl><Shift>d"),
            (ShortcutAction::ClosePane,       &config.close_pane,       "<Ctrl><Shift>x"),
            (ShortcutAction::NewSshWorkspace, &config.new_ssh_workspace, "<Ctrl><Shift>s"),
            (ShortcutAction::FocusLeft,       &config.focus_left,       "<Ctrl><Shift>Left"),
            (ShortcutAction::FocusRight,      &config.focus_right,      "<Ctrl><Shift>Right"),
            (ShortcutAction::FocusUp,         &config.focus_up,         "<Ctrl><Shift>Up"),
            (ShortcutAction::FocusDown,       &config.focus_down,       "<Ctrl><Shift>Down"),
            (ShortcutAction::Workspace1,      &config.workspace_1,      "<Ctrl>1"),
            (ShortcutAction::Workspace2,      &config.workspace_2,      "<Ctrl>2"),
            (ShortcutAction::Workspace3,      &config.workspace_3,      "<Ctrl>3"),
            (ShortcutAction::Workspace4,      &config.workspace_4,      "<Ctrl>4"),
            (ShortcutAction::Workspace5,      &config.workspace_5,      "<Ctrl>5"),
            (ShortcutAction::Workspace6,      &config.workspace_6,      "<Ctrl>6"),
            (ShortcutAction::Workspace7,      &config.workspace_7,      "<Ctrl>7"),
            (ShortcutAction::Workspace8,      &config.workspace_8,      "<Ctrl>8"),
            (ShortcutAction::Workspace9,      &config.workspace_9,      "<Ctrl>9"),
            (ShortcutAction::BrowserOpen,     &config.browser_open,     "<Ctrl><Shift>b"),
            (ShortcutAction::BrowserClose,    &config.browser_close,    "<Ctrl><Shift>q"),
            // pane-nav
            (ShortcutAction::FocusPane1,      &config.focus_pane_1,     "<Ctrl><Alt>1"),
            (ShortcutAction::FocusPane2,      &config.focus_pane_2,     "<Ctrl><Alt>2"),
            (ShortcutAction::FocusPane3,      &config.focus_pane_3,     "<Ctrl><Alt>3"),
            (ShortcutAction::FocusPane4,      &config.focus_pane_4,     "<Ctrl><Alt>4"),
            (ShortcutAction::FocusPane5,      &config.focus_pane_5,     "<Ctrl><Alt>5"),
            (ShortcutAction::FocusPane6,      &config.focus_pane_6,     "<Ctrl><Alt>6"),
            (ShortcutAction::FocusPane7,      &config.focus_pane_7,     "<Ctrl><Alt>7"),
            (ShortcutAction::FocusPane8,      &config.focus_pane_8,     "<Ctrl><Alt>8"),
            (ShortcutAction::FocusPane9,      &config.focus_pane_9,     "<Ctrl><Alt>9"),
            (ShortcutAction::FocusLastPane,   &config.focus_last_pane,  "<Ctrl><Alt>grave"),
            (ShortcutAction::ResizeLeft,      &config.resize_left,      "<Ctrl><Alt><Shift>Left"),
            (ShortcutAction::ResizeRight,     &config.resize_right,     "<Ctrl><Alt><Shift>Right"),
            (ShortcutAction::ResizeUp,        &config.resize_up,        "<Ctrl><Alt><Shift>Up"),
            (ShortcutAction::ResizeDown,      &config.resize_down,      "<Ctrl><Alt><Shift>Down"),
            (ShortcutAction::EqualizeSplits,  &config.equalize_splits,  "<Ctrl><Alt>equal"),
            // tabs
            (ShortcutAction::NewTab,          &config.new_tab,          "<Ctrl><Shift>t"),
            (ShortcutAction::NextTab,         &config.next_tab,         "<Ctrl>Tab"),
            (ShortcutAction::PrevTab,         &config.prev_tab,         "<Ctrl><Shift>Tab"),
            (ShortcutAction::NextTabAlt,      &config.next_tab_alt,     "<Ctrl>Page_Down"),
            (ShortcutAction::PrevTabAlt,      &config.prev_tab_alt,     "<Ctrl>Page_Up"),
            (ShortcutAction::GotoTab1,        &config.goto_tab_1,       "<Alt>1"),
            (ShortcutAction::GotoTab2,        &config.goto_tab_2,       "<Alt>2"),
            (ShortcutAction::GotoTab3,        &config.goto_tab_3,       "<Alt>3"),
            (ShortcutAction::GotoTab4,        &config.goto_tab_4,       "<Alt>4"),
            (ShortcutAction::GotoTab5,        &config.goto_tab_5,       "<Alt>5"),
            (ShortcutAction::GotoTab6,        &config.goto_tab_6,       "<Alt>6"),
            (ShortcutAction::GotoTab7,        &config.goto_tab_7,       "<Alt>7"),
            (ShortcutAction::GotoTab8,        &config.goto_tab_8,       "<Alt>8"),
            (ShortcutAction::GotoTab9,        &config.goto_tab_9,       "<Alt>9"),
        ];

        let mut map = HashMap::new();
        let mut accels = Vec::with_capacity(entries.len());

        for (action, config_val, default_accel) in entries {
            let accel_str = config_val.as_deref().unwrap_or(*default_accel);
            let action_name = format!("{:?}", action);

            if let Some((key, mods)) = gtk4::accelerator_parse(accel_str) {
                map.insert((mods & MOD_MASK, key), *action);
                accels.push((*action, accel_str.to_string()));
            } else {
                // D-11: invalid accelerator — warn and use default
                eprintln!(
                    "cmux: invalid shortcut '{}' for {}, using default '{}'",
                    accel_str, action_name, default_accel
                );
                if let Some((key, mods)) = gtk4::accelerator_parse(*default_accel) {
                    map.insert((mods & MOD_MASK, key), *action);
                }
                accels.push((*action, default_accel.to_string()));
            }
        }

        ShortcutMap { map, accels }
    }

    /// The effective accelerator string (GTK syntax) bound to `action`.
    pub fn accel(&self, action: ShortcutAction) -> Option<&str> {
        self.accels
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, s)| s.as_str())
    }

    /// Look up a shortcut action for the given modifier+key combination.
    /// Masks modifiers to ignore Caps Lock, Num Lock, etc.
    /// Normalizes keyval to lowercase because GTK4 key events give uppercase
    /// when Shift is held (e.g. Key::R), but accelerator_parse stores lowercase
    /// with the Shift modifier flag (e.g. Key::r + SHIFT_MASK).
    pub fn lookup(&self, mods: ModifierType, key: Key) -> Option<ShortcutAction> {
        let masked = mods & MOD_MASK;
        // tabs: Shift+Tab arrives as ISO_Left_Tab; accelerators say Tab.
        let key = if key == Key::ISO_Left_Tab { Key::Tab } else { key };
        let lower_key = key.to_lower();
        self.map.get(&(masked, lower_key)).copied()
    }
}

// notifications
/// Notification configuration -- [notifications] in config.toml.
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(default)]
pub struct NotificationsConfig {
    /// Show desktop notifications (toasts). Sidebar attention dots are unaffected.
    pub enabled: bool,
    /// Skip the toast (and sidebar dot) for OSC 9/777 and `cmux notify` notifications
    /// whose source pane is the focused pane while the cmux window is active.
    pub suppress_when_focused: bool,
    /// Show a toast for terminal bells while the cmux window is unfocused.
    pub bell: bool,
    /// Shell command run via `sh -c` (detached) for every delivered notification, with
    /// CMUX_NOTIFICATION_{TITLE,BODY,KIND}, CMUX_WORKSPACE_{ID,NAME}, CMUX_SURFACE_ID set.
    pub command: Option<String>,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true, suppress_when_focused: true, bell: true, command: None }
    }
}

/// Notification config, set once at startup from the loaded Config.
static NOTIFICATIONS_CONFIG: std::sync::OnceLock<NotificationsConfig> = std::sync::OnceLock::new();

pub fn set_notifications_config(config: NotificationsConfig) {
    let _ = NOTIFICATIONS_CONFIG.set(config);
}

/// Current notification config (defaults if never set). Read at startup only:
/// changes to [notifications] need a restart, like the rest of config.toml.
pub fn notifications_config() -> &'static NotificationsConfig {
    NOTIFICATIONS_CONFIG.get_or_init(NotificationsConfig::default)
}

/// Serializes tests that change process-wide environment variables
/// (XDG_CONFIG_HOME, XDG_RUNTIME_DIR). Rust runs tests on parallel threads, so
/// without this they race and read each other's directories.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path_xdg() {
        let _env = crate::config::test_env_lock();
        // Temporarily set XDG_CONFIG_HOME and verify config_path() uses it.
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/test-xdg-config");
        let path = config_path();
        assert_eq!(path, PathBuf::from("/tmp/test-xdg-config/cmux/config.toml"));
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn test_load_config_missing_file() {
        let _env = crate::config::test_env_lock();
        // Point to a nonexistent dir so load_config returns defaults silently.
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/cmux-test-nonexistent-dir-xyz");
        let config = load_config();
        assert!(config.shortcuts.new_workspace.is_none());
        assert!(config.shortcuts.close_workspace.is_none());
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn test_load_config_empty_file() {
        let _env = crate::config::test_env_lock();
        let dir = std::env::temp_dir().join(format!("cmux-cfg-empty-{}", std::process::id()));
        let cfg_dir = dir.join("cmux");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        let cfg_file = cfg_dir.join("config.toml");
        std::fs::write(&cfg_file, "").unwrap();

        std::env::set_var("XDG_CONFIG_HOME", dir.to_str().unwrap());
        let config = load_config();
        assert!(config.shortcuts.new_workspace.is_none());
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_config_valid_shortcuts() {
        let _env = crate::config::test_env_lock();
        let dir = std::env::temp_dir().join(format!("cmux-cfg-valid-{}", std::process::id()));
        let cfg_dir = dir.join("cmux");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        let cfg_file = cfg_dir.join("config.toml");
        std::fs::write(&cfg_file, "[shortcuts]\nnew_workspace = \"<Ctrl>t\"\n").unwrap();

        std::env::set_var("XDG_CONFIG_HOME", dir.to_str().unwrap());
        let config = load_config();
        assert_eq!(config.shortcuts.new_workspace, Some("<Ctrl>t".to_string()));
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_config_invalid_toml() {
        let _env = crate::config::test_env_lock();
        let dir = std::env::temp_dir().join(format!("cmux-cfg-invalid-{}", std::process::id()));
        let cfg_dir = dir.join("cmux");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        let cfg_file = cfg_dir.join("config.toml");
        std::fs::write(&cfg_file, "[shortcuts\n").unwrap();

        std::env::set_var("XDG_CONFIG_HOME", dir.to_str().unwrap());
        let config = load_config();
        // Falls back to defaults on parse error
        assert!(config.shortcuts.new_workspace.is_none());
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ui_config_default() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.ui.header_bar.style, "gtk");
        assert!(config.ui.header_bar.buttons_left.is_none());
    }

    #[test]
    fn test_ui_config_custom_style() {
        let config: Config = toml::from_str(r#"
[ui.header_bar]
style = "none"
buttons_left = ["new_workspace"]
buttons_right = ["split_right", "toggle_sidebar"]
"#).unwrap();
        assert_eq!(config.ui.header_bar.style, "none");
        assert_eq!(config.ui.header_bar.buttons_left.as_ref().unwrap().len(), 1);
        assert_eq!(config.ui.header_bar.buttons_right.as_ref().unwrap().len(), 2);
    }

    // Tests that require GTK4 initialization (accelerator_parse).
    // These will only work in environments with a display (or virtual display).

    /// Run `f` on the one thread that initialized GTK. GTK may only be used from
    /// the thread that initialized it, and libtest runs every test on its own
    /// thread, so without this only the first GTK test to run would get GTK.
    /// Skips (returns) when GTK cannot be initialized.
    fn on_gtk_thread(name: &str, f: impl FnOnce() + Send + 'static) {
        use std::sync::{mpsc, Mutex, OnceLock};
        type Job = Box<dyn FnOnce() + Send>;
        type Done = mpsc::Sender<Option<std::thread::Result<()>>>;
        static GTK_THREAD: OnceLock<Mutex<mpsc::Sender<(Job, Done)>>> = OnceLock::new();
        let jobs = GTK_THREAD.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<(Job, Done)>();
            std::thread::spawn(move || {
                let gtk_ok = gtk4::init().is_ok();
                for (job, done) in rx {
                    let result = gtk_ok.then(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)));
                    let _ = done.send(result);
                }
            });
            Mutex::new(tx)
        });
        let (done_tx, done_rx) = mpsc::channel();
        jobs.lock().unwrap().send((Box::new(f), done_tx)).unwrap();
        match done_rx.recv().unwrap() {
            None => eprintln!("Skipping {name}: GTK4 init failed (headless)"),
            Some(Err(panic)) => std::panic::resume_unwind(panic),
            Some(Ok(())) => {}
        }
    }

    #[test]
    fn test_shortcut_map_defaults() {
        on_gtk_thread("test_shortcut_map_defaults", || {
            let smap = ShortcutMap::from_config(&ShortcutConfig::default());
            // Ctrl+N should map to NewWorkspace
            let result = smap.lookup(ModifierType::CONTROL_MASK, Key::n);
            assert_eq!(result, Some(ShortcutAction::NewWorkspace));
        });
    }

    #[test]
    fn test_shortcut_map_custom() {
        on_gtk_thread("test_shortcut_map_custom", || {
            let config = ShortcutConfig {
                new_workspace: Some("<Ctrl>t".to_string()),
                ..Default::default()
            };
            let smap = ShortcutMap::from_config(&config);
            // Ctrl+T should now map to NewWorkspace
            assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::t), Some(ShortcutAction::NewWorkspace));
            // Ctrl+N should no longer map to NewWorkspace
            assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::n), None);
        });
    }

    // pane-nav
    #[test]
    fn test_shortcut_map_pane_nav_defaults() {
        on_gtk_thread("test_shortcut_map_pane_nav_defaults", || {
            let smap = ShortcutMap::from_config(&ShortcutConfig::default());
            let ca = ModifierType::CONTROL_MASK | ModifierType::ALT_MASK;
            assert_eq!(smap.lookup(ca, Key::_1), Some(ShortcutAction::FocusPane1));
            assert_eq!(smap.lookup(ca, Key::_9), Some(ShortcutAction::FocusPane9));
            assert_eq!(smap.lookup(ca, Key::grave), Some(ShortcutAction::FocusLastPane));
            assert_eq!(smap.lookup(ca, Key::equal), Some(ShortcutAction::EqualizeSplits));
            let cas = ca | ModifierType::SHIFT_MASK;
            assert_eq!(smap.lookup(cas, Key::Left), Some(ShortcutAction::ResizeLeft));
            assert_eq!(smap.lookup(cas, Key::Down), Some(ShortcutAction::ResizeDown));
            // Existing bindings are untouched: Ctrl+1 is still workspace 1 and
            // Ctrl+Shift+Left is still focus-left.
            assert_eq!(smap.lookup(ModifierType::CONTROL_MASK, Key::_1), Some(ShortcutAction::Workspace1));
            assert_eq!(
                smap.lookup(ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK, Key::Left),
                Some(ShortcutAction::FocusLeft)
            );
            assert_eq!(smap.accel(ShortcutAction::EqualizeSplits), Some("<Ctrl><Alt>equal"));
            // Every action has an effective accelerator for menus/help.
            assert_eq!(smap.accels.len(), KNOWN_SHORTCUTS.len());
        });
    }

    #[test]
    fn test_shortcut_map_pane_nav_overrides() {
        on_gtk_thread("test_shortcut_map_pane_nav_overrides", || {
            let config: Config = toml::from_str(r#"
    [shortcuts]
    focus_last_pane = "<Ctrl><Alt>Tab"
    equalize_splits = "<Ctrl><Alt>0"
    resize_left = "not a valid accel <<<"
    split_right = "<Ctrl>t"
    "#).unwrap();
            let smap = ShortcutMap::from_config(&config.shortcuts);
            let ca = ModifierType::CONTROL_MASK | ModifierType::ALT_MASK;
            assert_eq!(smap.lookup(ca, Key::Tab), Some(ShortcutAction::FocusLastPane));
            assert_eq!(smap.lookup(ca, Key::grave), None);
            assert_eq!(smap.lookup(ca, Key::_0), Some(ShortcutAction::EqualizeSplits));
            assert_eq!(smap.lookup(ca, Key::equal), None);
            // Invalid override falls back to the default.
            assert_eq!(
                smap.lookup(ca | ModifierType::SHIFT_MASK, Key::Left),
                Some(ShortcutAction::ResizeLeft)
            );
            assert_eq!(smap.accel(ShortcutAction::ResizeLeft), Some("<Ctrl><Alt><Shift>Left"));
            // Accelerators exposed to menus follow the override.
            assert_eq!(smap.accel(ShortcutAction::SplitRight), Some("<Ctrl>t"));
            assert_eq!(smap.accel(ShortcutAction::FocusLastPane), Some("<Ctrl><Alt>Tab"));
        });
    }

    #[test]
    fn test_pane_nav_config_keys_are_accepted() {
        let content = r#"
[shortcuts]
focus_pane_1 = "<Ctrl><Alt>F1"
focus_pane_9 = "<Ctrl><Alt>F9"
focus_last_pane = "<Ctrl><Alt>Tab"
resize_left = "<Ctrl><Alt><Shift>h"
resize_right = "<Ctrl><Alt><Shift>l"
resize_up = "<Ctrl><Alt><Shift>k"
resize_down = "<Ctrl><Alt><Shift>j"
equalize_splits = "<Ctrl><Alt>0"
focus_pane_typo = "<Ctrl><Alt>F2"
"#;
        // Only the misspelled key draws the unknown-action warning.
        assert_eq!(unknown_shortcut_keys(content), vec!["focus_pane_typo".to_string()]);
        let cfg: Config = toml::from_str(content).unwrap();
        assert_eq!(cfg.shortcuts.focus_pane_9.as_deref(), Some("<Ctrl><Alt>F9"));
        assert_eq!(cfg.shortcuts.resize_down.as_deref(), Some("<Ctrl><Alt><Shift>j"));
        assert_eq!(cfg.shortcuts.equalize_splits.as_deref(), Some("<Ctrl><Alt>0"));
    }

    // tabs
    #[test]
    fn test_shortcut_map_tab_defaults() {
        on_gtk_thread("test_shortcut_map_tab_defaults", || {
            let smap = ShortcutMap::from_config(&ShortcutConfig::default());
            let c = ModifierType::CONTROL_MASK;
            let cs = c | ModifierType::SHIFT_MASK;
            assert_eq!(smap.lookup(cs, Key::T), Some(ShortcutAction::NewTab));
            assert_eq!(smap.lookup(c, Key::Tab), Some(ShortcutAction::NextTab));
            // Shift+Tab arrives as ISO_Left_Tab.
            assert_eq!(smap.lookup(cs, Key::ISO_Left_Tab), Some(ShortcutAction::PrevTab));
            assert_eq!(smap.lookup(c, Key::Page_Down), Some(ShortcutAction::NextTabAlt));
            assert_eq!(smap.lookup(c, Key::Page_Up), Some(ShortcutAction::PrevTabAlt));
            let alt = ModifierType::ALT_MASK;
            assert_eq!(smap.lookup(alt, Key::_1), Some(ShortcutAction::GotoTab1));
            assert_eq!(smap.lookup(alt, Key::_9), Some(ShortcutAction::GotoTab9));
            // Existing defaults are untouched; close tab is close pane.
            assert_eq!(smap.lookup(cs, Key::X), Some(ShortcutAction::ClosePane));
            assert_eq!(smap.lookup(c, Key::_1), Some(ShortcutAction::Workspace1));
            assert_eq!(smap.lookup(c | alt, Key::_1), Some(ShortcutAction::FocusPane1));
        });
    }

    #[test]
    fn test_tab_config_keys() {
        let content = r#"
[ui]
always_show_tabs = true
[shortcuts]
new_tab = "<Ctrl><Alt>t"
next_tab_alt = "<Alt>Right"
goto_tab_9 = "<Alt>0"
"#;
        assert!(unknown_shortcut_keys(content).is_empty());
        let cfg: Config = toml::from_str(content).unwrap();
        assert!(cfg.ui.always_show_tabs);
        assert_eq!(cfg.shortcuts.new_tab.as_deref(), Some("<Ctrl><Alt>t"));
        assert_eq!(cfg.shortcuts.goto_tab_9.as_deref(), Some("<Alt>0"));
        let cfg: Config = toml::from_str("").unwrap();
        assert!(!cfg.ui.always_show_tabs);
    }

    // notifications
    #[test]
    fn test_notifications_config() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.notifications.enabled);
        assert!(config.notifications.suppress_when_focused);
        assert!(config.notifications.bell);
        assert!(config.notifications.command.is_none());

        let config: Config = toml::from_str(r#"
[notifications]
suppress_when_focused = false
command = "echo hi"
"#).unwrap();
        assert!(config.notifications.enabled);
        assert!(!config.notifications.suppress_when_focused);
        assert_eq!(config.notifications.command.as_deref(), Some("echo hi"));
    }
}
