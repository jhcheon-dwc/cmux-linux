//! cmux CLI — clap-based argument parser and command dispatch.
//!
//! This module is entirely independent of GTK4 and the GUI app.
//! It connects to the cmux-app via Unix socket JSON-RPC.

pub mod discovery;
pub mod format;
pub mod socket_client;
// agent-status: shared, GTK-free status/hook logic (also compiled into cmux-app).
#[allow(dead_code)]
#[path = "../agent_status.rs"]
pub mod agent_status;

pub use socket_client::CliError;

use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "cmux", about = "Control cmux terminal multiplexer")]
pub struct Cli {
    /// Path to the cmux socket (overrides discovery)
    #[arg(long, global = true, env = "CMUX_SOCKET")]
    socket: Option<String>,

    /// Output raw JSON responses
    #[arg(long, global = true)]
    json: bool,

    /// Suppress JSON output for browser commands (browser defaults to JSON)
    #[arg(long, global = true)]
    no_json: bool,

    /// Verbose output (connection info to stderr)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Color mode: always, never, auto
    #[arg(long, global = true, default_value = "auto")]
    color: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Ping the running cmux instance
    Ping,
    /// Show cmux instance identity (version, platform, pid)
    Identify,
    /// List supported socket commands
    Capabilities,
    /// List all workspaces
    ListWorkspaces,
    /// Show the current workspace
    CurrentWorkspace,
    /// Send an arbitrary JSON-RPC method
    Raw {
        /// The method name (e.g. "workspace.list")
        method: String,
        /// JSON params string
        #[arg(long, default_value = "{}")]
        params: String,
    },

    // -- Workspace management --
    /// Create a new workspace
    NewWorkspace,
    /// Select a workspace by ID
    SelectWorkspace {
        /// Workspace UUID
        id: String,
    },
    /// Close a workspace by ID
    CloseWorkspace {
        /// Workspace UUID
        id: String,
    },
    /// Rename a workspace
    RenameWorkspace {
        /// Workspace UUID
        id: String,
        /// New name
        name: String,
    },
    /// Switch to next workspace
    NextWorkspace,
    /// Switch to previous workspace
    PrevWorkspace,
    /// Switch to last active workspace
    LastWorkspace,
    /// Reorder a workspace
    ReorderWorkspace {
        /// Workspace UUID
        id: String,
        /// Target position (0-indexed)
        position: usize,
    },

    // -- Surface commands --
    /// List all surfaces
    ListSurfaces,
    /// Split a surface
    Split {
        /// Split direction: horizontal or vertical
        #[arg(long, default_value = "horizontal")]
        direction: String,
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },
    /// Focus a surface by ID
    FocusSurface {
        /// Surface UUID
        id: String,
    },
    /// Close a surface by ID
    CloseSurface {
        /// Surface UUID
        id: String,
    },
    /// Send text to a surface as typed input ("\n" = Enter, "\t" = Tab, "\\" = backslash)
    #[command(alias = "send")]
    SendText {
        /// Text to send
        text: String,
        /// Target surface ID (default: focused; any workspace)
        #[arg(long, alias = "surface")]
        id: Option<String>,
        /// Paste the text (bracketed paste) instead of typing it
        #[arg(long)]
        paste: bool,
    },
    /// Send a key event to a surface (e.g. enter, ctrl+c, alt+x, ctrl+shift+t, up, f5)
    SendKey {
        /// Key descriptor: [ctrl+][shift+][alt+][super+]<key>
        key: String,
        /// Target surface ID (default: focused; any workspace)
        #[arg(long, alias = "surface")]
        id: Option<String>,
    },
    /// Read text from a surface (default: the visible screen)
    ReadText {
        /// Target surface ID (default: focused; any workspace)
        #[arg(long, alias = "surface")]
        id: Option<String>,
        /// Only the last N lines (includes scrollback)
        #[arg(long, short = 'n')]
        lines: Option<usize>,
        /// Include the whole scrollback
        #[arg(long)]
        scrollback: bool,
    },
    /// Check surface health
    Health {
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },
    /// Refresh a surface
    Refresh {
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },

    // -- Pane commands --
    /// List all panes
    ListPanes,
    /// Focus a pane
    FocusPane {
        /// Pane ID (default: next)
        id: Option<String>,
    },
    /// Switch to last focused pane
    LastPane,

    // -- Window commands --
    /// List all windows
    ListWindows,
    /// Show current window info
    CurrentWindow,

    // -- Debug commands --
    /// Show layout tree
    Layout,
    /// Type text into the focused terminal
    Type {
        /// Text to type
        text: String,
    },

    // -- Notification commands --
    /// List notifications
    ListNotifications,
    /// Clear a notification
    ClearNotification {
        /// Notification ID
        id: String,
    },

    // -- Browser subcommand group (agent primary interface) --
    /// Browser automation (agent primary interface)
    #[command(subcommand)]
    Browser(BrowserCommand),

    // pane-nav
    /// Focus the Nth pane (1-based, reading order) of the current workspace
    FocusPaneNumber {
        /// Pane number (1 = top-left)
        n: u64,
    },
    /// Move the focused pane's nearest divider
    ResizePane {
        /// Direction to move the divider: left, right, up, down
        direction: String,
        /// Step as a fraction of the split's size
        #[arg(long, default_value = "0.05")]
        amount: f64,
    },
    /// Give all panes of the current workspace equal space
    EqualizePanes,
    // notifications
    /// Send a notification (sidebar attention + desktop notification + hook)
    Notify {
        /// Notification title
        #[arg(long, default_value = "")]
        title: String,
        /// Notification body
        #[arg(long, default_value = "")]
        body: String,
        /// Source surface UUID (default: $CMUX_SURFACE_ID, else the focused pane)
        #[arg(long, env = "CMUX_SURFACE_ID")]
        surface: Option<String>,
        /// Source workspace UUID (default: $CMUX_WORKSPACE_ID, else the focused workspace)
        #[arg(long, env = "CMUX_WORKSPACE_ID")]
        workspace: Option<String>,
    },
    // agent-status
    /// Set the agent status shown on a workspace's sidebar row
    SetStatus {
        /// running, needs_input, idle, error, or clear
        #[arg(value_parser = parse_status_arg)]
        state: String,
        /// Optional message shown next to the status badge
        #[arg(long)]
        message: Option<String>,
        /// Target workspace UUID (default: $CMUX_WORKSPACE_ID)
        #[arg(long)]
        workspace: Option<String>,
        /// Target surface UUID (default: $CMUX_SURFACE_ID)
        #[arg(long)]
        surface: Option<String>,
    },
    /// Claude Code hook adapter: reads the hook JSON on stdin and updates the
    /// workspace status. Never fails: always exits 0, silently.
    ClaudeHook {
        /// Hook event name (UserPromptSubmit, PreToolUse, Notification, Stop, ...)
        event: String,
    },
    // socket-cli
    /// Paste text into a surface (bracketed paste); same escapes as send-text
    Paste {
        /// Text to paste
        text: String,
        /// Target surface ID (default: focused; any workspace)
        #[arg(long, alias = "surface")]
        id: Option<String>,
        /// Press Enter after pasting
        #[arg(long)]
        submit: bool,
    },
    /// Print the current selection of a surface
    ReadSelection {
        /// Target surface ID (default: focused; any workspace)
        #[arg(long, alias = "surface")]
        id: Option<String>,
    },
    /// Show the focused workspace, pane and surface
    Current,

    // tabs
    /// Open a new terminal tab in a pane (default: the focused pane)
    NewTab {
        /// A surface in the target pane (default: focused)
        #[arg(long)]
        surface: Option<String>,
    },
    /// Rename a terminal tab (an empty name restores the terminal's own title)
    RenameTab {
        /// New tab name ("" clears the custom name)
        title: String,
        /// Tab surface ID (default: the focused tab)
        #[arg(long, alias = "id")]
        surface: Option<String>,
    },
    /// Close a terminal tab (closes the pane on its last tab)
    CloseTab {
        /// Tab surface ID (default: the focused tab)
        id: Option<String>,
    },
    /// Select a tab by its 1-based number in the focused pane, or by surface ID
    SelectTab {
        /// Tab number (1 = leftmost) or surface UUID
        target: String,
    },
    /// Switch to the next tab of the focused pane
    NextTab,
    /// Switch to the previous tab of the focused pane
    PrevTab,
    /// List the tabs of a pane (default: the focused pane)
    ListTabs {
        /// A surface in the pane to list
        #[arg(long)]
        pane: Option<String>,
    },
}

/// Browser subcommands for `cmux browser <action>` / `cmux browser <surface> <action>`.
#[derive(Subcommand)]
pub enum BrowserCommand {
    /// Open a URL in the browser pane
    Open {
        /// URL to open
        url: String,
        /// Target workspace ID
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List browser surfaces
    List,
    /// Close browser surface(s)
    Close {
        /// Surface reference (surface:N or UUID); closes all if omitted
        #[arg(long)]
        surface: Option<String>,
    },
    /// Take a browser snapshot (accessibility tree / DOM text)
    Snapshot {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Include interactive element annotations
        #[arg(long)]
        interactive: bool,
        /// Compact output
        #[arg(long)]
        compact: bool,
        /// Maximum depth
        #[arg(long)]
        max_depth: Option<u32>,
    },
    /// Click an element
    Click {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Target element (e1 or CSS selector)
        target: String,
        /// Take snapshot after action
        #[arg(long)]
        snapshot_after: bool,
    },
    /// Fill an input field (clears first, then types)
    Fill {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Target element (CSS selector)
        target: String,
        /// Value to fill
        text: String,
        /// Take snapshot after action
        #[arg(long)]
        snapshot_after: bool,
    },
    /// Type text into an element
    #[command(name = "type")]
    BrowserType {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
        /// Text to type
        text: String,
    },
    /// Press a key (e.g. "Enter", "Tab", "Escape")
    Press {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Key name
        key: String,
    },
    /// Hover over an element
    Hover {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Scroll the page
    Scroll {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Direction: up, down, left, right
        direction: String,
        /// Amount in pixels
        #[arg(long, default_value = "300")]
        amount: i32,
    },
    /// Select an option from a dropdown
    #[command(name = "select")]
    Select {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the select element
        selector: String,
        /// Value to select
        value: String,
    },
    /// Evaluate JavaScript in the browser
    Eval {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// JavaScript expression to evaluate
        expression: String,
    },
    /// Wait for a condition
    Wait {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector to wait for
        #[arg(long)]
        selector: Option<String>,
        /// Text to wait for
        #[arg(long)]
        text: Option<String>,
        /// URL substring to wait for
        #[arg(long)]
        url_contains: Option<String>,
        /// Load state to wait for
        #[arg(long)]
        load_state: Option<String>,
        /// JavaScript function to wait for
        #[arg(long)]
        function: Option<String>,
        /// Timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout_ms: u64,
    },
    /// Navigate to a URL
    Goto {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// URL to navigate to
        url: String,
    },
    /// Go back in browser history
    Back {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Go forward in browser history
    Forward {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Reload the current page
    Reload {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get the current page URL
    #[command(name = "get-url")]
    GetUrl {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get the current page title
    #[command(name = "get-title")]
    GetTitle {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get text content of an element
    #[command(name = "get-text")]
    GetText {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Get HTML content of an element
    #[command(name = "get-html")]
    GetHtml {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Take a browser screenshot (base64 PNG)
    Screenshot {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Enable browser streaming
    #[command(name = "stream-enable")]
    StreamEnable,
    /// Disable browser streaming
    #[command(name = "stream-disable")]
    StreamDisable,
}

/// Run the CLI with the parsed arguments.
pub fn run(cli: Cli) -> Result<(), CliError> {
    // agent-status: hooks must never block or fail the agent, so this runs before any
    // fallible socket setup and swallows every error.
    if let Commands::ClaudeHook { ref event } = cli.command {
        run_claude_hook(event, cli.socket.as_deref());
        return Ok(());
    }

    // Resolve socket path: --socket flag > discovery > error
    let socket_path = if let Some(ref path) = cli.socket {
        path.clone()
    } else {
        discovery::discover_socket().ok_or_else(|| {
            CliError::ConnectionError(
                "no cmux socket found (is cmux-app running?)".into(),
            )
        })?
    };

    // Use longer timeout for browser wait commands
    let timeout = match &cli.command {
        Commands::Browser(BrowserCommand::Wait { timeout_ms, .. }) => {
            Duration::from_millis(timeout_ms + 5000)
        }
        _ => Duration::from_secs(5),
    };

    let mut client =
        socket_client::SocketClient::connect(&socket_path, timeout)?;

    if cli.verbose {
        eprintln!("Connected to {}", socket_path);
    }

    let use_color = format::use_color(&cli.color);

    // Handle Raw command separately (dynamic method name)
    let (method_name, result) = if let Commands::Raw { ref method, ref params } = cli.command {
        let params_val: serde_json::Value = serde_json::from_str(params).map_err(|e| {
            CliError::ProtocolError(format!("invalid JSON params: {}", e))
        })?;
        let result = client.call(method, params_val)?;
        (method.clone(), result)
    } else {
        let (method, params) = command_to_rpc(&cli.command);
        let result = client.call(method, params)?;
        (method.to_string(), result)
    };

    // Browser commands default to JSON; everything else defaults to human-readable
    let json_mode = match &cli.command {
        Commands::Browser(_) => !cli.no_json,
        _ => cli.json,
    };

    // Output formatted result
    let output = format::format_response(&method_name, &result, json_mode, use_color);
    if !output.is_empty() {
        println!("{}", output);
    }

    Ok(())
}

/// Map a BrowserCommand variant to its JSON-RPC method and params.
fn browser_command_to_rpc(cmd: &BrowserCommand) -> (&'static str, serde_json::Value) {
    use serde_json::json;
    match cmd {
        BrowserCommand::Open { url, workspace } => {
            let url = if !url.contains("://") {
                format!("https://{}", url)
            } else {
                url.clone()
            };
            ("browser.open", json!({"url": url, "workspace": workspace}))
        }
        BrowserCommand::List => ("browser.list", json!({})),
        BrowserCommand::Close { surface } => {
            ("browser.close", json!({"surface_ref": surface}))
        }
        BrowserCommand::Snapshot { surface, interactive, compact, max_depth } => {
            ("browser.snapshot", json!({
                "surface_ref": surface,
                "interactive": interactive,
                "compact": compact,
                "max_depth": max_depth
            }))
        }
        BrowserCommand::Click { surface, target, snapshot_after } => {
            ("browser.click", json!({
                "surface_ref": surface,
                "target": target,
                "snapshot_after": snapshot_after
            }))
        }
        BrowserCommand::Fill { surface, target, text, snapshot_after } => {
            ("browser.fill", json!({
                "surface_ref": surface,
                "target": target,
                "text": text,
                "snapshot_after": snapshot_after
            }))
        }
        BrowserCommand::BrowserType { surface, selector, text } => {
            ("browser.type", json!({
                "surface_ref": surface,
                "selector": selector,
                "text": text
            }))
        }
        BrowserCommand::Press { surface, key } => {
            ("browser.press", json!({"surface_ref": surface, "key": key}))
        }
        BrowserCommand::Hover { surface, selector } => {
            ("browser.hover", json!({"surface_ref": surface, "selector": selector}))
        }
        BrowserCommand::Scroll { surface, direction, amount } => {
            ("browser.scroll", json!({
                "surface_ref": surface,
                "direction": direction,
                "amount": amount
            }))
        }
        BrowserCommand::Select { surface, selector, value } => {
            ("browser.select", json!({
                "surface_ref": surface,
                "selector": selector,
                "value": value
            }))
        }
        BrowserCommand::Eval { surface, expression } => {
            ("browser.eval", json!({"surface_ref": surface, "script": expression}))
        }
        BrowserCommand::Wait { surface, selector, text, url_contains, load_state, function, timeout_ms } => {
            ("browser.wait", json!({
                "surface_ref": surface,
                "selector": selector,
                "text": text,
                "url_contains": url_contains,
                "load_state": load_state,
                "function": function,
                "timeout_ms": timeout_ms
            }))
        }
        BrowserCommand::Goto { surface, url } => {
            ("browser.goto", json!({"surface_ref": surface, "url": url}))
        }
        BrowserCommand::Back { surface } => {
            ("browser.back", json!({"surface_ref": surface}))
        }
        BrowserCommand::Forward { surface } => {
            ("browser.forward", json!({"surface_ref": surface}))
        }
        BrowserCommand::Reload { surface } => {
            ("browser.reload", json!({"surface_ref": surface}))
        }
        BrowserCommand::GetUrl { surface } => {
            ("browser.url", json!({"surface_ref": surface}))
        }
        BrowserCommand::GetTitle { surface } => {
            ("browser.title", json!({"surface_ref": surface}))
        }
        BrowserCommand::GetText { surface, selector } => {
            ("browser.gettext", json!({"surface_ref": surface, "selector": selector}))
        }
        BrowserCommand::GetHtml { surface, selector } => {
            ("browser.gethtml", json!({"surface_ref": surface, "selector": selector}))
        }
        BrowserCommand::Screenshot { surface } => {
            ("browser.screenshot", json!({"surface_ref": surface}))
        }
        BrowserCommand::StreamEnable => ("browser.stream.enable", json!({})),
        BrowserCommand::StreamDisable => ("browser.stream.disable", json!({})),
    }
}

/// Convert a CLI command to a JSON-RPC method and params.
/// Raw is handled separately in run() — panics if called with Raw.
fn command_to_rpc(cmd: &Commands) -> (&'static str, serde_json::Value) {
    use serde_json::{json, Value};
    match cmd {
        Commands::Ping => ("system.ping", json!({})),
        Commands::Identify => ("system.identify", json!({})),
        Commands::Capabilities => ("system.capabilities", json!({})),
        Commands::ListWorkspaces => ("workspace.list", json!({})),
        Commands::CurrentWorkspace => ("workspace.current", json!({})),

        Commands::Raw { .. } => unreachable!("Raw handled separately"),

        Commands::NewWorkspace => ("workspace.create", json!({})),
        Commands::SelectWorkspace { id } => ("workspace.select", json!({"id": id})),
        Commands::CloseWorkspace { id } => ("workspace.close", json!({"id": id})),
        Commands::RenameWorkspace { id, name } => {
            ("workspace.rename", json!({"id": id, "name": name}))
        }
        Commands::NextWorkspace => ("workspace.next", json!({})),
        Commands::PrevWorkspace => ("workspace.previous", json!({})),
        Commands::LastWorkspace => ("workspace.last", json!({})),
        Commands::ReorderWorkspace { id, position } => {
            ("workspace.reorder", json!({"id": id, "position": position}))
        }

        Commands::ListSurfaces => ("surface.list", json!({})),
        Commands::Split { direction, id } => {
            let mut p = serde_json::Map::new();
            p.insert("direction".into(), json!(direction));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.split", Value::Object(p))
        }
        Commands::FocusSurface { id } => ("surface.focus", json!({"id": id})),
        Commands::CloseSurface { id } => ("surface.close", json!({"id": id})),
        Commands::SendText { text, id, paste } => {
            let mut p = serde_json::Map::new();
            p.insert("text".into(), json!(unescape_text(text)));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            let method = if *paste { "surface.paste" } else { "surface.send_text" };
            (method, Value::Object(p))
        }
        Commands::SendKey { key, id } => {
            let mut p = serde_json::Map::new();
            p.insert("key".into(), json!(key));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.send_key", Value::Object(p))
        }
        Commands::ReadText { id, lines, scrollback } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            if let Some(n) = lines {
                p.insert("lines".into(), json!(n));
            }
            if *scrollback {
                p.insert("scrollback".into(), json!(true));
            }
            ("surface.read_text", Value::Object(p))
        }
        Commands::Health { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.health", Value::Object(p))
        }
        Commands::Refresh { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.refresh", Value::Object(p))
        }

        Commands::ListPanes => ("pane.list", json!({})),
        Commands::FocusPane { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("pane.focus", Value::Object(p))
        }
        Commands::LastPane => ("pane.last", json!({})),

        Commands::ListWindows => ("window.list", json!({})),
        Commands::CurrentWindow => ("window.current", json!({})),

        Commands::Layout => ("debug.layout", json!({})),
        Commands::Type { text } => ("debug.type", json!({"text": text})),

        Commands::ListNotifications => ("notification.list", json!({})),
        Commands::ClearNotification { id } => {
            ("notification.clear", json!({"id": id}))
        }

        Commands::Browser(cmd) => browser_command_to_rpc(cmd),

        // pane-nav
        Commands::FocusPaneNumber { n } => ("pane.focus_number", json!({"n": n})),
        Commands::ResizePane { direction, amount } => {
            ("pane.resize", json!({"direction": direction, "amount": amount}))
        }
        Commands::EqualizePanes => ("pane.equalize", json!({})),
        // notifications
        Commands::Notify { title, body, surface, workspace } => {
            let mut p = serde_json::Map::new();
            p.insert("title".into(), json!(title));
            p.insert("body".into(), json!(body));
            if let Some(ref s) = surface {
                p.insert("surface".into(), json!(s));
            }
            if let Some(ref w) = workspace {
                p.insert("workspace".into(), json!(w));
            }
            ("notification.create", Value::Object(p))
        }
        // tabs
        Commands::NewTab { surface } => {
            let mut p = serde_json::Map::new();
            if let Some(ref s) = surface {
                p.insert("surface_id".into(), json!(s));
            }
            ("tab.new", Value::Object(p))
        }
        Commands::RenameTab { title, surface } => {
            let mut p = serde_json::Map::new();
            p.insert("title".into(), json!(title));
            if let Some(ref s) = surface {
                p.insert("surface_id".into(), json!(s));
            }
            ("tab.rename", Value::Object(p))
        }
        Commands::CloseTab { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("surface_id".into(), json!(id));
            }
            ("tab.close", Value::Object(p))
        }
        Commands::SelectTab { target } => match target.parse::<u64>() {
            Ok(n) if n >= 1 => ("tab.select", json!({"index": n - 1})),
            _ => ("tab.select", json!({"surface_id": target})),
        },
        Commands::NextTab => ("tab.next", json!({})),
        Commands::PrevTab => ("tab.prev", json!({})),
        Commands::ListTabs { pane } => {
            let mut p = serde_json::Map::new();
            if let Some(ref pane) = pane {
                p.insert("pane".into(), json!(pane));
            }
            ("tab.list", Value::Object(p))
        }
        // socket-cli
        Commands::Paste { text, id, submit } => {
            let mut p = serde_json::Map::new();
            p.insert("text".into(), json!(unescape_text(text)));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            if *submit {
                p.insert("submit".into(), json!(true));
            }
            ("surface.paste", Value::Object(p))
        }
        Commands::ReadSelection { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.read_selection", Value::Object(p))
        }
        Commands::Current => ("surface.current", json!({})),
        // agent-status
        Commands::SetStatus { state, message, workspace, surface } => {
            ("workspace.set_status", set_status_params(
                state,
                message.as_deref(),
                workspace.as_deref(),
                surface.as_deref(),
                env_id(agent_status::ENV_WORKSPACE_ID).as_deref(),
                env_id(agent_status::ENV_SURFACE_ID).as_deref(),
            ))
        }
        Commands::ClaudeHook { .. } => unreachable!("ClaudeHook handled separately"),
    }
}

// agent-status
/// clap value parser for `set-status <state>`: accepts what the server accepts
/// (agent_status::StatusUpdate::parse) and normalizes it to the wire name.
fn parse_status_arg(s: &str) -> Result<String, String> {
    agent_status::StatusUpdate::parse(s)
        .map(|u| u.as_str().to_string())
        .ok_or_else(|| "expected running, needs_input, idle, error or clear".to_string())
}

/// Non-empty value of env var `key`.
fn env_id(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Params for `workspace.set_status`. Explicit --workspace/--surface win; only when
/// neither is given do the pane's CMUX_WORKSPACE_ID / CMUX_SURFACE_ID apply.
fn set_status_params(
    state: &str,
    message: Option<&str>,
    workspace: Option<&str>,
    surface: Option<&str>,
    env_workspace: Option<&str>,
    env_surface: Option<&str>,
) -> serde_json::Value {
    let (workspace, surface) = if workspace.is_some() || surface.is_some() {
        (workspace, surface)
    } else {
        (env_workspace, env_surface)
    };
    let mut p = serde_json::Map::new();
    let state = agent_status::StatusUpdate::parse(state).map(|u| u.as_str()).unwrap_or(state);
    p.insert("state".into(), serde_json::json!(state));
    if let Some(m) = message {
        p.insert("message".into(), serde_json::json!(m));
    }
    if let Some(w) = workspace {
        p.insert("workspace_id".into(), serde_json::json!(w));
    }
    if let Some(s) = surface {
        p.insert("surface_id".into(), serde_json::json!(s));
    }
    serde_json::Value::Object(p)
}

/// Read stdin (the hook's JSON payload) with a size cap and a deadline, so a hook run
/// with an open-but-silent stdin can never hang the agent.
fn read_hook_stdin(deadline: Duration) -> String {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::stdin().take(4 << 20).read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(deadline).unwrap_or_default()
}

/// `cmux claude-hook <event>`: map a Claude Code hook event to a status update for the
/// pane's workspace. Silent and infallible; does nothing outside a cmux pane.
fn run_claude_hook(event: &str, socket_override: Option<&str>) {
    let input = read_hook_stdin(Duration::from_secs(2));
    let payload: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let Some((update, message)) = agent_status::claude_hook_update(event, &payload) else {
        return;
    };
    let workspace = env_id(agent_status::ENV_WORKSPACE_ID);
    let surface = env_id(agent_status::ENV_SURFACE_ID);
    if workspace.is_none() && surface.is_none() {
        return; // Not running inside a cmux pane.
    }
    // The pane's own CMUX_SOCKET_PATH names the instance that owns CMUX_WORKSPACE_ID,
    // so it wins over --socket / CMUX_SOCKET (clap fills `socket_override` from that
    // env var, which may point at another instance) and over discovery.
    let socket = match env_id(agent_status::ENV_SOCKET_PATH)
        .or_else(|| socket_override.filter(|s| !s.is_empty()).map(String::from))
        .or_else(discovery::discover_socket)
    {
        Some(s) => s,
        None => return,
    };
    let Ok(mut client) = socket_client::SocketClient::connect(&socket, Duration::from_millis(1500)) else {
        return;
    };
    let params = set_status_params(
        update.as_str(),
        message.as_deref(),
        None,
        None,
        workspace.as_deref(),
        surface.as_deref(),
    );
    let _ = client.call("workspace.set_status", params);
}

/// Expand C-style escapes in text arguments: \n (Enter), \r, \t, \e (Escape),
/// \\ (backslash). Unknown escapes are kept verbatim.
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('e') => out.push('\x1b'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_status_params_prefers_explicit_target() {
        let p = set_status_params("needs-input", Some("확인"), None, Some("sf"), Some("env-ws"), Some("env-sf"));
        assert_eq!(p, json!({"state": "needs_input", "message": "확인", "surface_id": "sf"}));
        let p = set_status_params("idle", None, None, None, Some("env-ws"), Some("env-sf"));
        assert_eq!(p, json!({"state": "idle", "workspace_id": "env-ws", "surface_id": "env-sf"}));
    }

    #[test]
    fn set_status_cli_parses() {
        let cli = Cli::try_parse_from(["cmux", "set-status", "running", "--message", "빌드 중", "--workspace", "w"]).unwrap();
        match cli.command {
            Commands::SetStatus { state, message, workspace, surface } => {
                assert_eq!(state, "running");
                assert_eq!(message.as_deref(), Some("빌드 중"));
                assert_eq!(workspace.as_deref(), Some("w"));
                assert!(surface.is_none());
            }
            _ => panic!("expected SetStatus"),
        }
        assert!(Cli::try_parse_from(["cmux", "set-status", "bogus"]).is_err());
        match Cli::try_parse_from(["cmux", "set-status", "Needs-Input"]).unwrap().command {
            Commands::SetStatus { state, .. } => assert_eq!(state, "needs_input"),
            _ => panic!("expected SetStatus"),
        }
        assert!(Cli::try_parse_from(["cmux", "claude-hook", "Stop"]).is_ok());
    }

    fn rpc(args: &[&str]) -> (&'static str, serde_json::Value) {
        let mut argv = vec!["cmux"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv).expect("parse");
        command_to_rpc(&cli.command)
    }

    #[test]
    fn send_text_is_typed_input_with_escapes() {
        assert_eq!(
            rpc(&["send-text", "echo 한글\\n"]),
            ("surface.send_text", json!({"text": "echo 한글\n"}))
        );
        // `send` alias + --surface alias + --paste switches to surface.paste.
        assert_eq!(
            rpc(&["send", "--surface", "abcd", "--paste", "x\\ty"]),
            ("surface.paste", json!({"text": "x\ty", "id": "abcd"}))
        );
    }

    #[test]
    fn paste_submit() {
        assert_eq!(
            rpc(&["paste", "--surface", "u1", "--submit", "ls"]),
            ("surface.paste", json!({"text": "ls", "id": "u1", "submit": true}))
        );
        assert_eq!(rpc(&["paste", "ls"]), ("surface.paste", json!({"text": "ls"})));
    }

    #[test]
    fn send_key_and_read() {
        assert_eq!(
            rpc(&["send-key", "ctrl+c", "--id", "u2"]),
            ("surface.send_key", json!({"key": "ctrl+c", "id": "u2"}))
        );
        assert_eq!(rpc(&["read-text"]), ("surface.read_text", json!({})));
        assert_eq!(
            rpc(&["read-text", "--lines", "5", "--scrollback"]),
            ("surface.read_text", json!({"lines": 5, "scrollback": true}))
        );
        assert_eq!(
            rpc(&["read-selection", "--surface", "u3"]),
            ("surface.read_selection", json!({"id": "u3"}))
        );
    }

    #[test]
    fn workspace_and_misc_commands() {
        assert_eq!(rpc(&["current"]), ("surface.current", json!({})));
        assert_eq!(rpc(&["last-workspace"]), ("workspace.last", json!({})));
        assert_eq!(
            rpc(&["rename-workspace", "w1", "새 이름"]),
            ("workspace.rename", json!({"id": "w1", "name": "새 이름"}))
        );
        assert_eq!(
            rpc(&["reorder-workspace", "w1", "2"]),
            ("workspace.reorder", json!({"id": "w1", "position": 2}))
        );
        assert_eq!(
            rpc(&["split", "--direction", "vertical", "--id", "s1"]),
            ("surface.split", json!({"direction": "vertical", "id": "s1"}))
        );
    }

    #[test]
    fn unescape() {
        assert_eq!(unescape_text("a\\nb"), "a\nb");
        assert_eq!(unescape_text("\\e[A"), "\x1b[A");
        assert_eq!(unescape_text("c:\\\\x"), "c:\\x");
        assert_eq!(unescape_text("keep \\q"), "keep \\q");
        assert_eq!(unescape_text("trailing\\"), "trailing\\");
        assert_eq!(unescape_text("한글"), "한글");
    }
}
