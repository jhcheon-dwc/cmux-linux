//! Human-readable output formatters for cmux CLI responses.
//!
//! Handles color support (D-07), list formatting with active markers (D-08),
//! and mutation success messages (D-09).

use serde_json::Value;
use std::io::IsTerminal;

/// Determine whether to use color output based on the --color flag value.
pub fn use_color(color_flag: &str) -> bool {
    match color_flag {
        "always" => true,
        "never" => false,
        _ => std::io::stdout().is_terminal(),
    }
}

fn green(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[1;32m{}\x1b[0m", s)
    } else {
        s.to_string()
    }
}

fn dim(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[2m{}\x1b[0m", s)
    } else {
        s.to_string()
    }
}

/// First string value among `keys` (handlers and older clients disagree on
/// `id`/`uuid` and `title`/`name`).
fn str_field<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()))
}

/// First bool value among `keys` (`focused`/`active`, `attention`/`has_attention`).
fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_bool())).unwrap_or(false)
}

/// Format a workspace list response with active marker.
pub fn format_workspace_list(result: &Value, color: bool) -> String {
    let workspaces = match result.get("workspaces").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if workspaces.is_empty() {
        return "No workspaces".to_string();
    }
    let mut lines = Vec::new();
    for (i, ws) in workspaces.iter().enumerate() {
        let selected = ws.get("selected").and_then(|v| v.as_bool()).unwrap_or(false);
        let title = ws
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("untitled");
        let pane_count = ws
            .get("pane_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let marker = if selected { "*" } else { " " };
        let mut line = format!(
            "{} {}: {} ({} pane{})",
            marker,
            i + 1,
            title,
            pane_count,
            if pane_count == 1 { "" } else { "s" }
        );
        // agent-status
        line.push_str(&format_status_suffix(ws));
        if selected && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Format a surface list response with focused marker.
pub fn format_surface_list(result: &Value, color: bool) -> String {
    let surfaces = match result.get("surfaces").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if surfaces.is_empty() {
        return "No surfaces".to_string();
    }
    let mut lines = Vec::new();
    for surface in surfaces {
        let focused = bool_field(surface, &["focused", "active"]);
        let id = str_field(surface, &["id", "uuid"]).unwrap_or("unknown");
        let id_short = &id[..id.len().min(8)];
        let title = surface
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let marker = if focused { "*" } else { " " };
        let line = if title.is_empty() {
            format!("{} {}", marker, id_short)
        } else {
            format!("{} {} ({})", marker, id_short, title)
        };
        if focused && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Format a pane list response with focused marker.
pub fn format_pane_list(result: &Value, color: bool) -> String {
    let panes = match result.get("panes").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if panes.is_empty() {
        return "No panes".to_string();
    }
    let mut lines = Vec::new();
    for pane in panes {
        let focused = bool_field(pane, &["focused", "active"]);
        let id = str_field(pane, &["id", "uuid"]).unwrap_or("unknown");
        let id_short = &id[..id.len().min(8)];
        let title = pane.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let marker = if focused { "*" } else { " " };
        let line = if title.is_empty() {
            format!("{} {}", marker, id_short)
        } else {
            format!("{} {} ({})", marker, id_short, title)
        };
        if focused && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Format a window list response.
pub fn format_window_list(result: &Value, color: bool) -> String {
    let windows = match result.get("windows").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if windows.is_empty() {
        return "No windows".to_string();
    }
    let mut lines = Vec::new();
    for (i, win) in windows.iter().enumerate() {
        let focused = win
            .get("focused")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let title = win.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let marker = if focused { "*" } else { " " };
        let line = format!("{} {}: {}", marker, i + 1, title);
        if focused && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Format identify response.
fn format_identify(result: &Value) -> String {
    let version = result
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let platform = result
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let pid = result.get("pid").and_then(|v| v.as_u64());
    match pid {
        Some(p) => format!("cmux {} ({}) pid {}", version, platform, p),
        None => format!("cmux {} ({})", version, platform),
    }
}

/// Format capabilities response.
fn format_capabilities(result: &Value, color: bool) -> String {
    let methods = match result.get("methods").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    let mut lines = vec![format!("{} methods available:", methods.len())];
    for m in methods {
        let name = m.as_str().unwrap_or("?");
        lines.push(format!("  {}", dim(name, color)));
    }
    lines.join("\n")
}

/// Format notification list response.
fn format_notification_list(result: &Value, color: bool) -> String {
    let notifications = match result.get("notifications").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if notifications.is_empty() {
        return "No notifications".to_string();
    }
    let mut lines = Vec::new();
    for n in notifications {
        let id = str_field(n, &["id", "workspace_uuid"]).unwrap_or("unknown");
        let id_short = &id[..id.len().min(8)];
        let attention = bool_field(n, &["attention", "has_attention"]);
        let marker = if attention { "!" } else { " " };
        let line = match n.get("workspace_name").and_then(|v| v.as_str()) {
            Some(name) => format!("{} {} ({})", marker, id_short, name),
            None => format!("{} {}", marker, id_short),
        };
        if attention && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Format a mutation command result with a success message.
pub fn format_mutation(command_name: &str, result: &Value) -> String {
    let id = str_field(result, &["id", "uuid"]).unwrap_or("");
    let title = result
        .get("title")
        .or_else(|| result.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match command_name {
        "workspace.create" => {
            if title.is_empty() {
                format!("Created workspace: {}", id)
            } else {
                format!("Created workspace: {} ({})", title, id)
            }
        }
        "workspace.close" => format!("Closed workspace: {}", id),
        "workspace.rename" => {
            let name = result
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(title);
            format!("Renamed workspace {} to: {}", id, name)
        }
        "surface.split" => format!("Split created: {}", id),
        "surface.close" => format!("Closed surface: {}", id),
        // socket-cli
        "workspace.reorder" => {
            let pos = result.get("position").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Moved workspace {} to position {}", id, pos)
        }
        "workspace.last" | "workspace.select" | "workspace.next" | "workspace.previous" => {
            format!("Switched to workspace: {} ({})", title, id)
        }
        _ => String::new(),
    }
}

/// Format a command response for human-readable output.
///
/// If `json_mode` is true, returns raw JSON (D-06).
/// Otherwise, picks the appropriate formatter based on the method name.
pub fn format_response(method: &str, result: &Value, json_mode: bool, color: bool) -> String {
    if json_mode {
        return serde_json::to_string_pretty(result).unwrap_or_default();
    }

    match method {
        "workspace.list" => format_workspace_list(result, color),
        "workspace.current" => {
            let title = str_field(result, &["title", "name"]).unwrap_or("unknown");
            let id = str_field(result, &["id", "uuid"]).unwrap_or("");
            format!("{} ({})", title, id)
        }
        "surface.list" => format_surface_list(result, color),
        "pane.list" => format_pane_list(result, color),
        "window.list" => format_window_list(result, color),
        "window.current" => {
            str_field(result, &["title", "id"]).unwrap_or("unknown").to_string()
        }
        "system.ping" => {
            result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("pong")
                .to_string()
        }
        "system.identify" => format_identify(result),
        "system.capabilities" => format_capabilities(result, color),
        "notification.list" => format_notification_list(result, color),
        "debug.layout" => serde_json::to_string_pretty(result).unwrap_or_default(),

        // socket-cli: terminal I/O
        "surface.read_text" | "surface.read_selection" => {
            result.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "surface.send_text" | "surface.send_key" | "surface.paste" => "OK".to_string(),
        "surface.current" => format_current(result),

        // Mutation commands: show success message
        "workspace.create" | "workspace.close" | "workspace.rename" | "surface.split"
        | "surface.close" | "workspace.reorder" | "workspace.last" | "workspace.select" | "workspace.next"
        | "workspace.previous" => {
            let msg = format_mutation(method, result);
            if msg.is_empty() {
                format_fallback(result)
            } else {
                msg
            }
        }

        // pane-zoom
        "pane.zoom" => {
            if bool_field(result, &["zoomed"]) { "Zoomed".to_string() } else { "Not zoomed".to_string() }
        }

        // Browser list: human-readable table
        "browser.list" => format_browser_list(result, color),

        // tabs: tab.list and the tab mutations all return the pane's tab list
        "tab.list" | "tab.new" | "tab.select" | "tab.next" | "tab.prev" => format_tab_list(result, color),

        // agent-status
        "workspace.set_status" => {
            let id = result.get("workspace_id").and_then(|v| v.as_str()).unwrap_or("");
            let status = format_status_suffix(result);
            if status.is_empty() {
                format!("Cleared status of workspace {}", id)
            } else {
                format!("Workspace {}:{}", id, status)
            }
        }

        // Default: pretty-print JSON for uncommon commands
        _ => format_fallback(result),
    }
}

// agent-status
/// " [running: message]" for an object with "status"/"status_message", or "" if no status.
fn format_status_suffix(v: &Value) -> String {
    let Some(status) = v.get("status").and_then(|s| s.as_str()) else {
        return String::new();
    };
    match v.get("status_message").and_then(|m| m.as_str()).filter(|m| !m.is_empty()) {
        Some(msg) => format!(" [{}: {}]", status, msg),
        None => format!(" [{}]", status),
    }
}

/// Format surface.current: focused workspace / pane / surface.
fn format_current(result: &Value) -> String {
    let get = |k: &str| result.get(k).and_then(|v| v.as_str()).unwrap_or("");
    format!(
        "workspace: {} ({})\npane:      {}\nsurface:   {}",
        get("workspace_name"),
        get("workspace_id"),
        get("pane_id"),
        get("surface_id"),
    )
}

/// tabs: format a pane's tab list ("* 2: title  uuid8"), focused tab marked.
pub fn format_tab_list(result: &Value, color: bool) -> String {
    let tabs = match result.get("tabs").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    let mut lines = Vec::new();
    for tab in tabs {
        let active = tab.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        let index = tab.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
        let title = tab.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let id = tab.get("surface_id").and_then(|v| v.as_str()).unwrap_or("");
        let id_short = &id[..id.len().min(8)];
        let line = format!("{} {}: {}  {}", if active { "*" } else { " " }, index + 1, title, id_short);
        if active && color {
            lines.push(green(&line, true));
        } else {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        "No tabs".to_string()
    } else {
        lines.join("\n")
    }
}

/// Format a browser surface list response.
pub fn format_browser_list(result: &Value, _color: bool) -> String {
    let surfaces = match result.get("surfaces").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return format_fallback(result),
    };
    if surfaces.is_empty() {
        return "No browser surfaces".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("{:<12} {:<38} {:<50} {}", "REF", "UUID", "URL", "STATUS"));
    for s in surfaces {
        let ref_str = s.get("ref").and_then(|v| v.as_str()).unwrap_or("-");
        let uuid = s.get("uuid").and_then(|v| v.as_str()).unwrap_or("-");
        let url = s.get("url").and_then(|v| v.as_str()).unwrap_or("-");
        let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
        lines.push(format!("{:<12} {:<38} {:<50} {}", ref_str, uuid, url, status));
    }
    lines.join("\n")
}

/// Fallback: pretty-print JSON.
fn format_fallback(result: &Value) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn human(method: &str, result: Value) -> String {
        format_response(method, &result, false, false)
    }

    #[test]
    fn workspace_current_accepts_both_key_styles() {
        assert_eq!(human("workspace.current", json!({"id": "u1", "title": "dev"})), "dev (u1)");
        assert_eq!(human("workspace.current", json!({"uuid": "u1", "name": "dev"})), "dev (u1)");
    }

    #[test]
    fn rename_and_create_show_id_and_name() {
        assert_eq!(
            human("workspace.rename", json!({"id": "w1", "name": "작업"})),
            "Renamed workspace w1 to: 작업"
        );
        assert_eq!(human("workspace.create", json!({"uuid": "w2"})), "Created workspace: w2");
        assert_eq!(human("surface.split", json!({"uuid": "s9"})), "Split created: s9");
        assert_eq!(
            format_mutation("workspace.reorder", &json!({"id": "w1", "position": 2})),
            "Moved workspace w1 to position 2"
        );
    }

    #[test]
    fn read_text_prints_raw_text() {
        assert_eq!(human("surface.read_text", json!({"text": "$ echo 한글\n한글"})), "$ echo 한글\n한글");
        assert_eq!(human("surface.read_selection", json!({"text": "", "has_selection": false})), "");
        assert_eq!(human("surface.send_key", json!({"key": "enter"})), "OK");
    }

    #[test]
    fn current_lists_all_ids() {
        let out = human("surface.current", json!({
            "workspace_id": "w", "workspace_name": "main", "pane_id": "p", "surface_id": "p",
        }));
        assert_eq!(out, "workspace: main (w)\npane:      p\nsurface:   p");
    }

    #[test]
    fn surface_list_uses_uuid_and_active() {
        let out = human("surface.list", json!({"surfaces": [
            {"uuid": "0123456789abcdef", "active": true},
            {"uuid": "fedcba9876543210", "active": false},
        ]}));
        assert_eq!(out, "* 01234567\n  fedcba98");
    }

    #[test]
    fn json_mode_is_raw() {
        let out = format_response("surface.current", &json!({"a": 1}), true, false);
        assert_eq!(out, "{\n  \"a\": 1\n}");
    }
}
