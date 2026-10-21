use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use serde_json::Value;
use base64::Engine as _;
use futures_util::StreamExt;
use gtk4::prelude::*;
use uuid::Uuid;

/// Session name for the agent-browser daemon (one daemon per cmux instance).
const SESSION_NAME: &str = "cmux";

/// Preview pane state tracked by BrowserManager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewState {
    Empty,
    Loading,
    Connected,
    Streaming,
    Error(String),
}

pub struct BrowserManager {
    daemon_process: Option<Child>,
    session_name: String,
    stream_task: Option<tokio::task::JoinHandle<()>>,
    pub frame_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    pub preview_state: PreviewState,
    /// Page viewport in CSS pixels, from the latest screencast frame metadata.
    /// Input coordinates are in CSS pixels, which differ from the frame's
    /// pixel size whenever the device scale factor is not 1.
    pub page_css_size: std::sync::Arc<std::sync::Mutex<Option<(f64, f64)>>>,
}

impl BrowserManager {
    pub fn new() -> Self {
        BrowserManager {
            daemon_process: None,
            session_name: SESSION_NAME.to_string(),
            stream_task: None,
            frame_tx: None,
            preview_state: PreviewState::Empty,
            page_css_size: Default::default(),
        }
    }

    /// Mirrors agent-browser/cli/src/connection.rs socket dir discovery.
    fn agent_browser_socket_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("AGENT_BROWSER_SOCKET_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("agent-browser");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".agent-browser");
        }
        std::env::temp_dir().join("agent-browser")
    }

    pub fn daemon_socket_path(&self) -> PathBuf {
        Self::agent_browser_socket_dir().join(format!("{}.sock", self.session_name))
    }

    pub fn stream_port_path(&self) -> PathBuf {
        Self::agent_browser_socket_dir().join(format!("{}.stream", self.session_name))
    }

    fn daemon_ready(&self) -> bool {
        std::os::unix::net::UnixStream::connect(self.daemon_socket_path()).is_ok()
    }

    /// Auto-start the agent-browser daemon (D-05).
    pub fn ensure_daemon(&mut self) -> Result<(), String> {
        if self.daemon_ready() {
            return Ok(());
        }

        // Find agent-browser binary: check PATH, then alongside cmux binary.
        let binary_path = which_agent_browser().ok_or_else(|| {
            "agent-browser not found in PATH. Install it or place it alongside the cmux binary."
                .to_string()
        })?;

        // Create socket dir if needed.
        let socket_dir = Self::agent_browser_socket_dir();
        std::fs::create_dir_all(&socket_dir)
            .map_err(|e| format!("Failed to create socket dir {}: {}", socket_dir.display(), e))?;

        let child = Command::new(&binary_path)
            .env("AGENT_BROWSER_DAEMON", "1")
            .env("AGENT_BROWSER_SESSION", &self.session_name)
            .env("AGENT_BROWSER_STREAM_PORT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn agent-browser: {}", e))?;

        self.daemon_process = Some(child);

        // Poll daemon_ready() with 200ms intervals, up to 50 retries (10s).
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if self.daemon_ready() {
                self.preview_state = PreviewState::Connected;
                return Ok(());
            }
        }

        Err("agent-browser daemon failed to start within 10 seconds".to_string())
    }

    /// Send a newline-delimited JSON command to the daemon socket.
    pub fn send_command(&self, action: &str, params: Value) -> Result<Value, String> {
        let socket_path = self.daemon_socket_path();
        let mut stream = std::os::unix::net::UnixStream::connect(&socket_path)
            .map_err(|e| format!("Failed to connect to daemon socket: {}", e))?;

        let req_id = format!("cmux-{}", rand_u64());
        let mut request = if let Value::Object(map) = params {
            Value::Object(map)
        } else {
            Value::Object(serde_json::Map::new())
        };
        request
            .as_object_mut()
            .unwrap()
            .insert("id".to_string(), Value::String(req_id));
        request
            .as_object_mut()
            .unwrap()
            .insert("action".to_string(), Value::String(action.to_string()));

        let mut json = serde_json::to_string(&request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;
        json.push('\n');
        stream
            .write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write to daemon socket: {}", e))?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        serde_json::from_str(&line)
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    /// Read the stream port from the port file.
    pub fn read_stream_port(&self) -> Result<u16, String> {
        let path = self.stream_port_path();
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read stream port file {}: {}", path.display(), e))?;
        content
            .trim()
            .parse::<u16>()
            .map_err(|e| format!("Failed to parse stream port '{}': {}", content.trim(), e))
    }

    /// Shut down the daemon and clean up.
    pub fn shutdown(&mut self) {
        // Try to send close command (best-effort).
        let _ = self.send_command(
            "close",
            serde_json::json!({"id": "cmux-shutdown"}),
        );

        if let Some(ref mut child) = self.daemon_process {
            // Wait up to 2 seconds, then kill.
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if start.elapsed() > std::time::Duration::from_secs(2) {
                            let _ = child.kill();
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
        }
        self.daemon_process = None;

        if let Some(task) = self.stream_task.take() {
            task.abort();
        }
        self.stream_task = None;
    }

    /// Connect to the agent-browser stream WebSocket and start forwarding
    /// decoded JPEG frames to the GTK main thread via mpsc channel.
    /// The Picture widget is updated in idle callbacks per D-02.
    pub fn start_stream(
        &mut self,
        runtime: &tokio::runtime::Handle,
        picture: gtk4::Picture,
    ) -> Result<(), String> {
        let port = self.read_stream_port()?;
        let url = format!("ws://127.0.0.1:{}", port);

        // Create mpsc channel for frame delivery (tokio -> GTK)
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        self.frame_tx = Some(frame_tx.clone());
        let page_css_size = self.page_css_size.clone();

        // Spawn tokio task: WebSocket client that reads frames
        let stream_task = runtime.spawn(async move {
            let ws_result = tokio_tungstenite::connect_async(&url).await;
            let (ws_stream, _) = match ws_result {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("cmux: browser stream WS connect failed: {}", e);
                    return;
                }
            };
            let (_write, mut read) = ws_stream.split();
            while let Some(msg_result) = read.next().await {
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("cmux: browser stream error: {}", e);
                        break;
                    }
                };
                if let tokio_tungstenite::tungstenite::Message::Text(text) = &msg {
                    if let Ok(frame) = serde_json::from_str::<serde_json::Value>(text) {
                        if frame.get("type").and_then(|t| t.as_str()) == Some("frame") {
                            if let Some(size) = frame_css_size(&frame) {
                                if let Ok(mut s) = page_css_size.lock() {
                                    *s = Some(size);
                                }
                            }
                            if let Some(data_b64) = frame.get("data").and_then(|d| d.as_str()) {
                                if let Ok(jpeg_bytes) = base64::engine::general_purpose::STANDARD.decode(data_b64) {
                                    let _ = frame_tx.send(jpeg_bytes);
                                }
                            }
                        }
                    }
                }
            }
        });
        self.stream_task = Some(stream_task);

        // Spawn GTK-side receiver: poll mpsc and update Picture widget
        let picture_clone = picture.clone();
        glib::MainContext::default().spawn_local(async move {
            let mut first_frame = true;
            while let Some(jpeg_bytes) = frame_rx.recv().await {
                let bytes = glib::Bytes::from(&jpeg_bytes);
                match gtk4::gdk::Texture::from_bytes(&bytes) {
                    Ok(texture) => {
                        picture_clone.set_paintable(Some(&texture));
                        // Hide the "No browser preview" overlay label on first frame
                        if first_frame {
                            first_frame = false;
                            if let Some(overlay) = picture_clone.parent().and_then(|p| p.downcast::<gtk4::Overlay>().ok()) {
                                if let Some(child) = overlay.first_child() {
                                    let mut sibling = child.next_sibling();
                                    while let Some(widget) = sibling {
                                        let next = widget.next_sibling();
                                        if widget.has_css_class("preview-empty") {
                                            widget.set_visible(false);
                                        }
                                        sibling = next;
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
        });

        self.preview_state = PreviewState::Streaming;
        Ok(())
    }
}

/// Widgets returned by create_preview_pane for callers to connect signals.
pub struct PreviewPaneWidgets {
    pub container: gtk4::Box,
    pub picture: gtk4::Picture,
    pub url_entry: gtk4::Entry,
    pub back_btn: gtk4::Button,
    pub forward_btn: gtk4::Button,
    pub reload_btn: gtk4::Button,
    pub go_btn: gtk4::Button,
    pub devtools_btn: gtk4::ToggleButton,
    // browser-devtools: page area (start) / devtools panel (end, while open).
    pub devtools_paned: gtk4::Paned,
    pub close_btn: gtk4::Button,
    pub zoom_btn: gtk4::Button, // pane-zoom
    /// Invisible overlay child with the Picture's allocation; its `resize`
    /// signal drives viewport updates (GtkPicture has no resize signal).
    pub viewport_sensor: gtk4::DrawingArea,
    pub pane_id: u64,
    pub uuid: Uuid,
}

/// pane-zoom: show whether the preview fills the workspace (⤡ restores the
/// side-by-side layout, ⤢ enlarges the preview).
pub fn set_zoom_button_state(btn: &gtk4::Button, zoomed: bool) {
    btn.set_label(if zoomed { "\u{2921}" } else { "\u{2922}" });
    if zoomed {
        btn.add_css_class("zoomed");
    } else {
        btn.remove_css_class("zoomed");
    }
}

/// Create a browser preview pane widget (nav bar + Picture + status overlay).
/// Returns PreviewPaneWidgets so callers can connect button signals.
pub fn create_preview_pane(next_pane_id: u64) -> PreviewPaneWidgets {
    let uuid = Uuid::new_v4();
    let picture = gtk4::Picture::new();
    picture.add_css_class("browser-preview");
    picture.set_can_shrink(true);
    // The page viewport follows the pane size (see viewport_params), so frames
    // already match the Picture 1:1 in device pixels. Fill instead of the
    // default Contain: no letterbox bands while a resize is in flight (the old
    // frame is stretched for a moment instead), and the image always covers
    // the whole widget, so widget (x, y) maps to page (x * page_w / widget_w,
    // y * page_h / widget_h).
    picture.set_content_fit(gtk4::ContentFit::Fill);
    picture.set_hexpand(true);
    picture.set_vexpand(true);

    let overlay = gtk4::Overlay::new();
    overlay.add_css_class("preview-container");
    overlay.set_child(Some(&picture));
    overlay.set_vexpand(true);

    let viewport_sensor = gtk4::DrawingArea::new();
    viewport_sensor.set_can_target(false);
    viewport_sensor.set_halign(gtk4::Align::Fill);
    viewport_sensor.set_valign(gtk4::Align::Fill);
    overlay.add_overlay(&viewport_sensor);

    // Empty state label (shown when no stream is active)
    let empty_label = gtk4::Label::new(Some("No browser preview"));
    empty_label.add_css_class("preview-empty");
    empty_label.set_halign(gtk4::Align::Center);
    empty_label.set_valign(gtk4::Align::Center);
    overlay.add_overlay(&empty_label);

    // Navigation bar buttons
    let back_btn = gtk4::Button::with_label("\u{25C0}");
    back_btn.add_css_class("browser-nav-btn");
    back_btn.set_tooltip_text(Some("Back"));

    let forward_btn = gtk4::Button::with_label("\u{25B6}");
    forward_btn.add_css_class("browser-nav-btn");
    forward_btn.set_tooltip_text(Some("Forward"));

    let reload_btn = gtk4::Button::with_label("\u{21BB}");
    reload_btn.add_css_class("browser-nav-btn");
    reload_btn.set_tooltip_text(Some("Reload"));

    // URL entry inside the nav bar
    let url_entry = gtk4::Entry::new();
    url_entry.set_placeholder_text(Some("Enter URL..."));
    url_entry.add_css_class("browser-url-bar");
    url_entry.set_hexpand(true);
    // Keep the URL bar clickable when the pane is narrow: without a minimum
    // width the nav buttons squeeze it down to a few pixels.
    url_entry.set_width_chars(16);

    let go_btn = gtk4::Button::with_label("\u{2192}");
    go_btn.add_css_class("browser-nav-btn");
    go_btn.add_css_class("browser-nav-go");
    go_btn.set_tooltip_text(Some("Go"));

    let devtools_btn = gtk4::ToggleButton::with_label("{ }");
    devtools_btn.add_css_class("browser-nav-btn");
    devtools_btn.add_css_class("browser-nav-devtools");
    devtools_btn.set_tooltip_text(Some("Developer Tools"));

    let close_btn = gtk4::Button::with_label("\u{2715}");
    close_btn.add_css_class("browser-nav-btn");
    close_btn.add_css_class("browser-nav-close");
    close_btn.set_tooltip_text(Some("Close Browser (Ctrl+Shift+Q)"));

    // pane-zoom: full pane <-> side by side; runs win.toggle-pane-zoom on the
    // active pane, which a click in the preview has just made this one.
    let zoom_btn = gtk4::Button::new();
    zoom_btn.add_css_class("browser-nav-btn");
    zoom_btn.add_css_class("browser-nav-zoom");
    zoom_btn.set_tooltip_text(Some("Toggle Full Pane (Ctrl+Shift+Enter)"));
    zoom_btn.set_action_name(Some("win.toggle-pane-zoom"));
    set_zoom_button_state(&zoom_btn, false);

    // Navigation bar: horizontal box with buttons + URL entry
    let nav_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    nav_bar.add_css_class("browser-nav-bar");
    nav_bar.append(&back_btn);
    nav_bar.append(&forward_btn);
    nav_bar.append(&reload_btn);
    nav_bar.append(&url_entry);
    nav_bar.append(&go_btn);
    nav_bar.append(&devtools_btn);
    nav_bar.append(&zoom_btn); // pane-zoom
    nav_bar.append(&close_btn);

    // Vertical box: nav bar on top, picture overlay below
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.append(&nav_bar);
    // browser-devtools: the page sits in its own box (overlay -> page box, so
    // `overlay.parent()` stays a plain Box) inside a Paned whose end child is
    // the devtools panel while it is open.
    let page_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page_box.append(&overlay);
    let devtools_paned = gtk4::Paned::new(gtk4::Orientation::Vertical);
    devtools_paned.set_start_child(Some(&page_box));
    devtools_paned.set_resize_start_child(true);
    devtools_paned.set_shrink_start_child(false);
    devtools_paned.set_resize_end_child(false);
    devtools_paned.set_shrink_end_child(false);
    devtools_paned.set_vexpand(true);
    vbox.append(&devtools_paned);
    // browser-devtools end

    PreviewPaneWidgets {
        container: vbox,
        picture,
        url_entry,
        back_btn,
        forward_btn,
        reload_btn,
        go_btn,
        devtools_btn,
        devtools_paned, // browser-devtools
        close_btn,
        zoom_btn, // pane-zoom
        viewport_sensor,
        pane_id: next_pane_id,
        uuid,
    }
}

/// `viewport` command params that make the page render at the widget's size
/// in device pixels: CSS size = widget size (logical px), deviceScaleFactor =
/// the surface scale (fractional on HiDPI). Screencast frames then map 1:1
/// onto screen pixels with no rescaling blur. None until the widget is allocated.
pub fn viewport_params(widget: &impl IsA<gtk4::Widget>) -> Option<Value> {
    let (w, h) = (widget.width(), widget.height());
    if w <= 0 || h <= 0 {
        return None;
    }
    let scale = widget
        .native()
        .and_then(|n| n.surface())
        .map(|s| s.scale())
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or_else(|| widget.scale_factor().max(1) as f64);
    Some(serde_json::json!({"width": w, "height": h, "deviceScaleFactor": scale}))
}

/// `launch` command params. Chrome only screencasts at its real device scale
/// (an emulated deviceScaleFactor changes devicePixelRatio but frames stay at
/// CSS size), so on HiDPI start Chrome at the window's scale.
pub fn launch_params(window: Option<&gtk4::Window>) -> Value {
    let scale = window
        .and_then(|w| w.surface())
        .map(|s| s.scale())
        .filter(|s| s.is_finite() && *s > 1.0);
    match scale {
        Some(scale) => serde_json::json!({
            "headless": true,
            "args": [format!("--force-device-scale-factor={scale}")],
        }),
        None => serde_json::json!({"headless": true}),
    }
}

/// Update the preview pane overlay to show the given state.
/// Removes existing status overlays and adds the appropriate label.
pub fn update_preview_overlay(overlay: &gtk4::Overlay, state: &PreviewState) {
    // Remove existing overlay children (status labels).
    // Walk siblings after the main child (Picture) and remove any status labels.
    if let Some(child) = overlay.first_child() {
        let mut sibling = child.next_sibling();
        while let Some(widget) = sibling {
            let next = widget.next_sibling();
            if widget.has_css_class("preview-empty") || widget.has_css_class("preview-error") {
                overlay.remove_overlay(&widget);
            }
            sibling = next;
        }
    }

    match state {
        PreviewState::Empty => {
            let label = gtk4::Label::new(Some("No browser preview"));
            label.add_css_class("preview-empty");
            label.set_halign(gtk4::Align::Center);
            label.set_valign(gtk4::Align::Center);
            overlay.add_overlay(&label);
        }
        PreviewState::Loading => {
            let label = gtk4::Label::new(Some("Starting browser..."));
            label.add_css_class("preview-empty");
            label.set_halign(gtk4::Align::Center);
            label.set_valign(gtk4::Align::Center);
            overlay.add_overlay(&label);
        }
        PreviewState::Connected | PreviewState::Streaming => {
            // No overlay needed -- Picture shows frames or empty background
        }
        PreviewState::Error(msg) => {
            let label = gtk4::Label::new(Some(msg.as_str()));
            label.add_css_class("preview-error");
            label.set_halign(gtk4::Align::Center);
            label.set_valign(gtk4::Align::Center);
            overlay.add_overlay(&label);
        }
    }
}

/// Spawn a tokio task that forwards mouse motion events to the agent-browser daemon.
/// Events are throttled to ~16fps (60ms) to avoid flooding the daemon (D-08).
/// The returned sender can be cloned into the GTK motion controller closure.
pub fn spawn_motion_forwarder(
    runtime: &tokio::runtime::Handle,
    daemon_socket_path: std::path::PathBuf,
) -> tokio::sync::mpsc::UnboundedSender<(i64, i64)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(i64, i64)>();
    runtime.spawn(async move {
        let mut last_sent = std::time::Instant::now();
        while let Some((x, y)) = rx.recv().await {
            let now = std::time::Instant::now();
            if now.duration_since(last_sent) < std::time::Duration::from_millis(60) {
                continue;
            }
            last_sent = now;
            let path = daemon_socket_path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&path) {
                    let req = serde_json::json!({
                        "id": "motion",
                        "action": "input_mouse",
                        "type": "mouseMoved",
                        "x": x,
                        "y": y,
                    });
                    let mut msg = serde_json::to_string(&req).unwrap_or_default();
                    msg.push('\n');
                    let _ = stream.write_all(msg.as_bytes());
                }
            }).await;
        }
    });
    tx
}

/// Find agent-browser binary in PATH or alongside the cmux binary.
fn which_agent_browser() -> Option<PathBuf> {
    // Check PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = PathBuf::from(dir).join("agent-browser");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // Check alongside cmux binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("agent-browser");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // Check /usr/lib/cmux/ (FHS install path for .deb/.rpm packages)
    let candidate = PathBuf::from("/usr/lib/cmux/agent-browser");
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

/// Simple random u64 for request IDs (no external crate needed).
fn rand_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    hasher.finish()
}

/// Page viewport size in CSS pixels from a screencast `frame` message.
fn frame_css_size(frame: &Value) -> Option<(f64, f64)> {
    let meta = frame.get("metadata")?;
    let w = meta.get("deviceWidth")?.as_f64()?;
    let h = meta.get("deviceHeight")?.as_f64()?;
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// Rectangle (x, y, width, height) where a GtkPicture of size `widget` draws a
/// paintable of intrinsic size `image` under `fit`. GtkPicture centers the
/// scaled image, so Contain/ScaleDown leave letterbox bars around it.
pub fn picture_content_rect(
    fit: gtk4::ContentFit,
    widget: (f64, f64),
    image: (f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    let ((ww, wh), (iw, ih)) = (widget, image);
    if ww <= 0.0 || wh <= 0.0 || iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let contain = (ww / iw).min(wh / ih);
    let (dw, dh) = match fit {
        gtk4::ContentFit::Fill => (ww, wh),
        gtk4::ContentFit::Cover => {
            let s = (ww / iw).max(wh / ih);
            (iw * s, ih * s)
        }
        gtk4::ContentFit::ScaleDown => {
            let s = contain.min(1.0);
            (iw * s, ih * s)
        }
        _ => (iw * contain, ih * contain),
    };
    Some(((ww - dw) / 2.0, (wh - dh) / 2.0, dw, dh))
}

/// Map a point in the preview Picture (widget coordinates) to page CSS pixels.
/// `image` is the frame's pixel size, `page` the page viewport in CSS pixels.
/// The result can lie outside the page when the point is on a letterbox bar.
pub fn widget_to_page_point(
    fit: gtk4::ContentFit,
    widget: (f64, f64),
    image: (f64, f64),
    page: (f64, f64),
    point: (f64, f64),
) -> Option<(f64, f64)> {
    let (x0, y0, dw, dh) = picture_content_rect(fit, widget, image)?;
    Some(((point.0 - x0) * page.0 / dw, (point.1 - y0) * page.1 / dh))
}

/// Query the page's current URL from the daemon (blocking, with a timeout).
fn query_page_url(daemon_socket_path: &std::path::Path) -> Option<String> {
    let mut stream = std::os::unix::net::UnixStream::connect(daemon_socket_path).ok()?;
    let timeout = Some(std::time::Duration::from_secs(2));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;
    stream.write_all(b"{\"id\":\"cmux-url\",\"action\":\"url\"}\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let resp: Value = serde_json::from_str(&line).ok()?;
    resp.get("data")?.get("url")?.as_str().map(str::to_string)
}

/// Keep `url_entry` showing the page's current URL, whatever navigated it (link
/// clicks, back/forward, `cmux browser open`). Polls the daemon off the GTK
/// thread and stops once the entry leaves the window. While the entry has focus
/// it is only updated if its text is still what the watcher (or an Enter in the
/// entry) last put there, so typing a URL is never clobbered but a focused,
/// untouched entry still follows CLI navigation and redirects.
pub fn spawn_url_watcher(
    runtime: &tokio::runtime::Handle,
    daemon_socket_path: PathBuf,
    url_entry: &gtk4::Entry,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    runtime.spawn(async move {
        while !tx.is_closed() {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let path = daemon_socket_path.clone();
            let url = tokio::task::spawn_blocking(move || query_page_url(&path)).await;
            if let Ok(Some(url)) = url {
                if tx.send(url).is_err() {
                    break;
                }
            }
        }
    });
    // Text the entry held when it was last synced; anything else is a user edit.
    let synced = std::rc::Rc::new(std::cell::RefCell::new(url_entry.text().to_string()));
    // Enter submits the typed URL: from then on it counts as synced (after the
    // navigate handler has normalized it), so the page URL may replace it.
    url_entry.connect_activate({
        let synced = synced.clone();
        move |entry| {
            let entry = entry.downgrade();
            let synced = synced.clone();
            glib::idle_add_local_once(move || {
                if let Some(entry) = entry.upgrade() {
                    *synced.borrow_mut() = entry.text().to_string();
                }
            });
        }
    });
    let entry_weak = url_entry.downgrade();
    glib::MainContext::default().spawn_local(async move {
        while let Some(url) = rx.recv().await {
            let Some(entry) = entry_weak.upgrade() else { break };
            let Some(root) = entry.root() else { break };
            let focused = root.focus().is_some_and(|f| f.is_ancestor(&entry));
            if focused && entry.text() != *synced.borrow() {
                continue;
            }
            // about:blank shows the placeholder instead.
            let text = if url == "about:blank" { "" } else { url.as_str() };
            if entry.text() != text {
                entry.set_text(text);
                if focused {
                    entry.set_position(-1);
                }
            }
            *synced.borrow_mut() = text.to_string();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::ContentFit;

    #[test]
    fn content_rect_letterboxes_wide_frame_vertically() {
        // 600x500 frame in a 300x500 widget: scaled to 300x250, centered.
        let r = picture_content_rect(ContentFit::Contain, (300.0, 500.0), (600.0, 500.0));
        assert_eq!(r, Some((0.0, 125.0, 300.0, 250.0)));
    }

    #[test]
    fn content_rect_scale_down_never_upscales() {
        let r = picture_content_rect(ContentFit::ScaleDown, (800.0, 600.0), (400.0, 300.0));
        assert_eq!(r, Some((200.0, 150.0, 400.0, 300.0)));
    }

    #[test]
    fn content_rect_rejects_empty_sizes() {
        assert_eq!(picture_content_rect(ContentFit::Contain, (0.0, 10.0), (10.0, 10.0)), None);
        assert_eq!(picture_content_rect(ContentFit::Contain, (10.0, 10.0), (10.0, 0.0)), None);
    }

    #[test]
    fn point_maps_through_letterbox_offset() {
        // Click on the image's top-left corner lands on page (0, 0).
        let p = widget_to_page_point(
            ContentFit::Contain, (300.0, 500.0), (600.0, 500.0), (600.0, 500.0), (0.0, 125.0),
        );
        assert_eq!(p, Some((0.0, 0.0)));
        // Image center -> page center.
        let p = widget_to_page_point(
            ContentFit::Contain, (300.0, 500.0), (600.0, 500.0), (600.0, 500.0), (150.0, 250.0),
        );
        assert_eq!(p, Some((300.0, 250.0)));
        // Letterbox bar above the image maps above the page.
        let p = widget_to_page_point(
            ContentFit::Contain, (300.0, 500.0), (600.0, 500.0), (600.0, 500.0), (10.0, 50.0),
        )
        .unwrap();
        assert!(p.1 < 0.0);
    }

    #[test]
    fn point_maps_device_pixels_to_css_pixels() {
        // DPR 2: 1200x1000 frame of a 600x500 CSS viewport shown 1:1 in a 600x500 widget.
        let p = widget_to_page_point(
            ContentFit::Contain, (600.0, 500.0), (1200.0, 1000.0), (600.0, 500.0), (100.0, 40.0),
        );
        assert_eq!(p, Some((100.0, 40.0)));
    }

    #[test]
    fn point_maps_with_fill() {
        let p = widget_to_page_point(
            ContentFit::Fill, (300.0, 500.0), (600.0, 500.0), (600.0, 500.0), (150.0, 100.0),
        );
        assert_eq!(p, Some((300.0, 100.0)));
    }

    #[test]
    fn frame_css_size_reads_metadata() {
        let frame = serde_json::json!({
            "type": "frame", "data": "",
            "metadata": {"deviceWidth": 640, "deviceHeight": 480, "pageScaleFactor": 1.0}
        });
        assert_eq!(frame_css_size(&frame), Some((640.0, 480.0)));
        assert_eq!(frame_css_size(&serde_json::json!({"type": "frame"})), None);
    }
}
