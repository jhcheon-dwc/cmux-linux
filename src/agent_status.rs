//! Agent status: per-workspace "Running / Needs input / Idle / Error" state reported by
//! coding agents (Claude Code, Codex, ...) via `cmux set-status` / `cmux claude-hook`.
//!
//! This module is GTK-free so it can be shared by the GUI (`cmux-app`) and the CLI
//! (`cmux`, which includes it via `#[path]` from src/cli/mod.rs).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Env var exported into every pane: UUID of the workspace that owns the pane.
pub const ENV_WORKSPACE_ID: &str = "CMUX_WORKSPACE_ID";
/// Env var exported into every pane: UUID of the pane's surface.
pub const ENV_SURFACE_ID: &str = "CMUX_SURFACE_ID";
/// Env var exported into every pane: path of the cmux control socket.
pub const ENV_SOCKET_PATH: &str = "CMUX_SOCKET_PATH";

/// Agent state shown as a badge on the workspace's sidebar row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    NeedsInput,
    Idle,
    Error,
}

impl AgentState {
    /// Wire name used by the socket protocol (`workspace.set_status` / `workspace.list`).
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Running => "running",
            AgentState::NeedsInput => "needs_input",
            AgentState::Idle => "idle",
            AgentState::Error => "error",
        }
    }

    /// Human-readable badge text.
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Running => "Running",
            AgentState::NeedsInput => "Needs input",
            AgentState::Idle => "Idle",
            AgentState::Error => "Error",
        }
    }

    /// CSS class applied to the sidebar badge (in addition to `agent-status`).
    pub fn css_class(self) -> &'static str {
        match self {
            AgentState::Running => "status-running",
            AgentState::NeedsInput => "status-needs-input",
            AgentState::Idle => "status-idle",
            AgentState::Error => "status-error",
        }
    }

    pub const ALL: [AgentState; 4] = [
        AgentState::Running,
        AgentState::NeedsInput,
        AgentState::Idle,
        AgentState::Error,
    ];
}

/// A requested status change: set a state, or clear the badge entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusUpdate {
    Set(AgentState),
    Clear,
}

impl StatusUpdate {
    /// Parse the `state` param of `workspace.set_status`. Accepts `needs-input` too.
    pub fn parse(s: &str) -> Option<StatusUpdate> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "running" => Some(StatusUpdate::Set(AgentState::Running)),
            "needs_input" => Some(StatusUpdate::Set(AgentState::NeedsInput)),
            "idle" => Some(StatusUpdate::Set(AgentState::Idle)),
            "error" => Some(StatusUpdate::Set(AgentState::Error)),
            "clear" | "none" => Some(StatusUpdate::Clear),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatusUpdate::Set(s) => s.as_str(),
            StatusUpdate::Clear => "clear",
        }
    }
}

/// Current agent status of a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub state: AgentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Longest status message kept (in chars); longer messages are truncated with an ellipsis.
pub const MAX_MESSAGE_CHARS: usize = 200;

/// Normalize a status message: collapse whitespace/newlines, drop empty, truncate on a
/// char boundary (messages are UTF-8 and may be Korean).
pub fn normalize_message(msg: Option<&str>) -> Option<String> {
    let collapsed = msg?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() > MAX_MESSAGE_CHARS {
        let mut s: String = collapsed.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        s.push('…');
        Some(s)
    } else {
        Some(collapsed)
    }
}

/// OSC 9;4 progress report for a pane (from Ghostty's PROGRESS_REPORT action).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Remove,
    Set,
    Error,
    Indeterminate,
    Pause,
}

impl ProgressState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProgressState::Remove => "remove",
            ProgressState::Set => "set",
            ProgressState::Error => "error",
            ProgressState::Indeterminate => "indeterminate",
            ProgressState::Pause => "pause",
        }
    }
}

/// Build the env vars exported into a pane's shell.
pub fn pane_env_vars(workspace_id: &str, surface_id: &str, socket_path: &str) -> Vec<(String, String)> {
    vec![
        (ENV_WORKSPACE_ID.to_string(), workspace_id.to_string()),
        (ENV_SURFACE_ID.to_string(), surface_id.to_string()),
        (ENV_SOCKET_PATH.to_string(), socket_path.to_string()),
    ]
}

/// Tools that block on the user when Claude Code invokes them (macOS cmux flags these
/// as "Needs input").
const BLOCKING_TOOLS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

fn str_field<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

/// Map a Claude Code hook event (+ its stdin JSON payload) to a status update and an
/// optional message. Returns None when the event should not change the status.
pub fn claude_hook_update(event: &str, payload: &Value) -> Option<(StatusUpdate, Option<String>)> {
    let tool = str_field(payload, "tool_name");
    match event {
        "UserPromptSubmit" => Some((StatusUpdate::Set(AgentState::Running), None)),
        "PreToolUse" => match tool {
            Some(t) if BLOCKING_TOOLS.contains(&t) => {
                Some((StatusUpdate::Set(AgentState::NeedsInput), Some(t.to_string())))
            }
            _ => Some((StatusUpdate::Set(AgentState::Running), tool.map(String::from))),
        },
        // A tool finished or was denied (e.g. after the user answered a permission
        // prompt): the agent carries on, back to running.
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
            Some((StatusUpdate::Set(AgentState::Running), None))
        }
        // MCP elicitation dialog waiting for the user.
        "Elicitation" => Some((
            StatusUpdate::Set(AgentState::NeedsInput),
            str_field(payload, "message").map(String::from),
        )),
        "PermissionRequest" => Some((
            StatusUpdate::Set(AgentState::NeedsInput),
            Some(match tool {
                Some(t) => format!("Permission: {t}"),
                None => "Permission requested".to_string(),
            }),
        )),
        "Notification" => {
            // Claude Code sends Notification for permission prompts, idle prompts
            // ("waiting for your input") and elicitation dialogs. auth_success is
            // informational only.
            if str_field(payload, "notification_type") == Some("auth_success") {
                return None;
            }
            Some((
                StatusUpdate::Set(AgentState::NeedsInput),
                str_field(payload, "message").map(String::from),
            ))
        }
        // Turn finished: idle (this also clears a pending needs_input). SubagentStop is
        // not: the main agent usually carries on after a subagent returns.
        "Stop" => Some((StatusUpdate::Set(AgentState::Idle), None)),
        "StopFailure" | "Error" => Some((
            StatusUpdate::Set(AgentState::Error),
            str_field(payload, "error")
                .or_else(|| str_field(payload, "error_details"))
                .or_else(|| str_field(payload, "message"))
                .map(String::from),
        )),
        // A new/cleared/resumed session waits for a prompt. Mid-turn auto-compaction
        // (source "compact") also fires SessionStart and must not reset Running.
        "SessionStart" => match str_field(payload, "source") {
            Some("compact") => None,
            _ => Some((StatusUpdate::Set(AgentState::Idle), None)),
        },
        "SessionEnd" => Some((StatusUpdate::Clear, None)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state_of(event: &str, payload: Value) -> Option<StatusUpdate> {
        claude_hook_update(event, &payload).map(|(u, _)| u)
    }

    #[test]
    fn hook_running_events() {
        assert_eq!(state_of("UserPromptSubmit", json!({"prompt": "hi"})), Some(StatusUpdate::Set(AgentState::Running)));
        assert_eq!(state_of("PreToolUse", json!({"tool_name": "Bash"})), Some(StatusUpdate::Set(AgentState::Running)));
        assert_eq!(state_of("PostToolUse", json!({})), Some(StatusUpdate::Set(AgentState::Running)));
        assert_eq!(state_of("PermissionDenied", json!({})), Some(StatusUpdate::Set(AgentState::Running)));
    }

    #[test]
    fn hook_needs_input_events() {
        let (u, msg) = claude_hook_update(
            "Notification",
            &json!({"message": "Claude needs your permission to use Bash", "notification_type": "permission_prompt"}),
        ).unwrap();
        assert_eq!(u, StatusUpdate::Set(AgentState::NeedsInput));
        assert_eq!(msg.as_deref(), Some("Claude needs your permission to use Bash"));
        assert_eq!(state_of("Notification", json!({})), Some(StatusUpdate::Set(AgentState::NeedsInput)));
        assert_eq!(state_of("Notification", json!({"notification_type": "auth_success"})), None);
        let (u, msg) = claude_hook_update("PermissionRequest", &json!({"tool_name": "Edit"})).unwrap();
        assert_eq!(u, StatusUpdate::Set(AgentState::NeedsInput));
        assert_eq!(msg.as_deref(), Some("Permission: Edit"));
        // Blocking tools are flagged as needing input before they run.
        assert_eq!(state_of("PreToolUse", json!({"tool_name": "AskUserQuestion"})), Some(StatusUpdate::Set(AgentState::NeedsInput)));
        assert_eq!(state_of("PreToolUse", json!({"tool_name": "ExitPlanMode"})), Some(StatusUpdate::Set(AgentState::NeedsInput)));
    }

    #[test]
    fn hook_idle_error_and_unknown_events() {
        assert_eq!(state_of("Stop", json!({})), Some(StatusUpdate::Set(AgentState::Idle)));
        assert_eq!(state_of("SubagentStop", json!({})), None);
        assert_eq!(state_of("SessionStart", json!({"source": "startup"})), Some(StatusUpdate::Set(AgentState::Idle)));
        assert_eq!(state_of("SessionStart", json!({"source": "compact"})), None);
        let (u, msg) = claude_hook_update("StopFailure", &json!({"error": "rate limited"})).unwrap();
        assert_eq!(u, StatusUpdate::Set(AgentState::Error));
        assert_eq!(msg.as_deref(), Some("rate limited"));
        assert_eq!(state_of("SessionEnd", json!({})), Some(StatusUpdate::Clear));
        assert_eq!(state_of("PreCompact", json!({})), None);
        assert_eq!(state_of("", Value::Null), None);
    }

    #[test]
    fn status_update_parse_roundtrip() {
        for s in AgentState::ALL {
            assert_eq!(StatusUpdate::parse(s.as_str()), Some(StatusUpdate::Set(s)));
        }
        assert_eq!(StatusUpdate::parse("needs-input"), Some(StatusUpdate::Set(AgentState::NeedsInput)));
        assert_eq!(StatusUpdate::parse("CLEAR"), Some(StatusUpdate::Clear));
        assert_eq!(StatusUpdate::parse("bogus"), None);
    }

    #[test]
    fn status_serde() {
        let st = AgentStatus { state: AgentState::NeedsInput, message: Some("확인 필요".into()) };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v, json!({"state": "needs_input", "message": "확인 필요"}));
        let back: AgentStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back, st);
        let no_msg = AgentStatus { state: AgentState::Idle, message: None };
        assert_eq!(serde_json::to_value(&no_msg).unwrap(), json!({"state": "idle"}));
        let parsed: AgentStatus = serde_json::from_str(r#"{"state":"running"}"#).unwrap();
        assert_eq!(parsed.state, AgentState::Running);
        assert!(parsed.message.is_none());
    }

    #[test]
    fn message_normalization_handles_korean() {
        assert_eq!(normalize_message(Some("  ")), None);
        assert_eq!(normalize_message(None), None);
        assert_eq!(normalize_message(Some("a\n b")).as_deref(), Some("a b"));
        let long: String = "가".repeat(MAX_MESSAGE_CHARS + 10);
        let n = normalize_message(Some(&long)).unwrap();
        assert_eq!(n.chars().count(), MAX_MESSAGE_CHARS);
        assert!(n.ends_with('…'));
    }

    #[test]
    fn env_var_list() {
        let vars = pane_env_vars("ws-uuid", "sf-uuid", "/run/user/1000/cmux/cmux.sock");
        assert_eq!(vars, vec![
            ("CMUX_WORKSPACE_ID".to_string(), "ws-uuid".to_string()),
            ("CMUX_SURFACE_ID".to_string(), "sf-uuid".to_string()),
            ("CMUX_SOCKET_PATH".to_string(), "/run/user/1000/cmux/cmux.sock".to_string()),
        ]);
    }
}
