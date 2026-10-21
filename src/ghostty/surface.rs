use crate::ghostty::ffi;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Whether we are inside a key-pressed/released handler, and if so whether the
/// input method was composing when the key event started. Mirrors Ghostty's GTK
/// apprt (`IMKeyEvent`): ibus calls commit THEN preedit-end, the simple context
/// calls preedit-end THEN commit, so we need the composing state captured at
/// key-event start to classify a commit correctly.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ImKeyEvent {
    False,
    NotComposing,
    Composing,
}

/// Per-surface input method (IME) state. Lets ibus/fcitx compose CJK text
/// (Korean, Japanese, Chinese) and dead keys before it reaches the terminal.
struct ImState {
    context: gtk4::IMMulticontext,
    composing: Cell<bool>,
    in_keyevent: Cell<ImKeyEvent>,
    /// Text committed by the IM during the current key event, sent with that
    /// key event so Ghostty still sees the keycode. NUL-terminated.
    buf: RefCell<[u8; 128]>,
    len: Cell<usize>,
}

/// The codepoint this physical key produces with no modifiers in the active
/// layout (e.g. 'v' for Ctrl+Shift+V). Ghostty needs it in two places: the
/// kitty keyboard protocol encodes letter keys by this codepoint (with 0 they
/// are not encoded at all, so Ctrl+V never reaches apps like Claude Code), and
/// unicode keybindings such as ctrl+shift+c fall back to it. Same idea as
/// Ghostty GTK's keyvalUnicodeUnshifted, but via translate_key, which does not
/// allocate.
fn unshifted_codepoint(ctrl: &gtk4::EventControllerKey, keycode: u32) -> u32 {
    use gtk4::gdk::prelude::*;
    use gtk4::prelude::*;

    let Some(event) = ctrl
        .current_event()
        .and_then(|e| e.downcast::<gtk4::gdk::KeyEvent>().ok())
    else {
        return 0;
    };
    let Some(widget) = ctrl.widget() else {
        return 0;
    };
    widget
        .display()
        .translate_key(keycode, gtk4::gdk::ModifierType::empty(), event.layout() as i32)
        .and_then(|(key, _, _, _)| key.to_unicode())
        .map(|ch| ch as u32)
        .unwrap_or(0)
}

const SURFACE_CELL_KEY: &str = "cmux-surface-cell";

/// Detach the Ghostty surface from its GLArea before the surface is freed.
///
/// Every handler on the GLArea (focus, key, mouse, scroll, IME, render) reads
/// the surface from this cell. GTK can still deliver events to a closed pane's
/// GLArea after the surface is gone (e.g. focus-leave while focus moves to the
/// surviving pane), so the cell must be emptied first or those handlers call
/// into freed memory.
pub fn detach_surface(gl_area: &gtk4::GLArea) {
    use gtk4::prelude::ObjectExt;
    let cell = unsafe {
        gl_area
            .data::<Rc<RefCell<Option<ffi::ghostty_surface_t>>>>(SURFACE_CELL_KEY)
            .map(|p| p.as_ref().clone())
    };
    if let Some(cell) = cell {
        // terminal-actions: drop per-surface title/search state.
        if let Some(surface) = cell.borrow_mut().take() {
            crate::header_bar::forget_surface(surface as usize);
            crate::find_bar::forget_surface(surface as usize);
        }
    }
}

/// Send IM-committed text that is not tied to a key event (e.g. a finished
/// Hangul syllable). Matches Ghostty GTK: clear the preedit, then send the text
/// as a key press with no physical key so it produces no extra PTY encoding.
fn send_committed_text(surface: ffi::ghostty_surface_t, text: &str) {
    unsafe {
        ffi::ghostty_surface_preedit(surface, std::ptr::null(), 0);
    }
    let Ok(c_text) = std::ffi::CString::new(text) else {
        return;
    };
    let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
    input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_PRESS;
    input.keycode = 0;
    input.text = c_text.as_ptr();
    input.composing = false;
    unsafe {
        ffi::ghostty_surface_key(surface, input);
    }
}

/// I/O mode for a Ghostty surface.
/// - `Exec`: normal mode — Ghostty spawns a local shell process.
/// - `Manual`: SSH remote mode — keystrokes route through io_write_cb to the SSH bridge.
#[derive(Clone)]
pub enum SurfaceIoMode {
    Exec,
    Manual {
        io_write_ctx: std::sync::Arc<crate::ssh::bridge::IoWriteContext>,
    },
}

/// agent-status: identity of a pane, exported to its shell as CMUX_WORKSPACE_ID /
/// CMUX_SURFACE_ID. `surface` is the Leaf's stable uuid in the split tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneIds {
    pub workspace: uuid::Uuid,
    pub surface: uuid::Uuid,
}

impl PaneIds {
    /// Ids for a new pane (fresh surface uuid) in `workspace`.
    pub fn new(workspace: uuid::Uuid) -> Self {
        PaneIds { workspace, surface: uuid::Uuid::new_v4() }
    }
}

/// agent-status: owned C strings backing `ghostty_surface_config_s.env_vars`.
/// Ghostty copies the strings in ghostty_surface_new, so this only has to outlive that call.
pub struct EnvVarBlock {
    _strings: Vec<std::ffi::CString>,
    vars: Vec<ffi::ghostty_env_var_s>,
}

impl EnvVarBlock {
    /// Build from key/value pairs; pairs containing an interior NUL are skipped.
    pub fn new(pairs: &[(String, String)]) -> Self {
        let mut strings = Vec::with_capacity(pairs.len() * 2);
        let mut vars = Vec::with_capacity(pairs.len());
        for (k, v) in pairs {
            let (Ok(k), Ok(v)) = (std::ffi::CString::new(k.as_str()), std::ffi::CString::new(v.as_str())) else {
                continue;
            };
            // The CString heap buffers do not move when the CStrings move into `strings`.
            vars.push(ffi::ghostty_env_var_s { key: k.as_ptr(), value: v.as_ptr() });
            strings.push(k);
            strings.push(v);
        }
        EnvVarBlock { _strings: strings, vars }
    }

    /// Point `config.env_vars` at this block (valid while `self` is alive).
    pub fn apply(&mut self, config: &mut ffi::ghostty_surface_config_s) {
        config.env_vars = if self.vars.is_empty() { std::ptr::null_mut() } else { self.vars.as_mut_ptr() };
        config.env_var_count = self.vars.len();
    }
}

/// Call glGetError() via FFI to check for GL errors after ghostty calls.
/// Returns 0 if no error, or the GL error code otherwise.
fn gl_get_error() -> u32 {
    extern "C" {
        fn glGetError() -> u32;
    }
    unsafe { glGetError() }
}

extern "C" {
    fn glGetIntegerv(pname: u32, params: *mut i32);
}
const GL_VIEWPORT: u32 = 0x0BA2;
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

/// Creates and returns a GtkGLArea with a Ghostty terminal surface wired up.
/// Initializes ghostty_app_t, then defers ghostty_surface_t creation to the
/// GtkGLArea realize signal — when the GL context is guaranteed to exist.
pub fn create_surface(
    _app: &gtk4::Application,
    ghostty_app: ffi::ghostty_app_t,
    inherited_config: Option<ffi::ghostty_surface_config_s>,
    pane_id: u64,
    io_mode: SurfaceIoMode,
    ids: PaneIds,
) -> (gtk4::GLArea, Rc<RefCell<Option<ffi::ghostty_surface_t>>>) {
    use gtk4::prelude::*;
    use std::sync::atomic::Ordering;

    use crate::ghostty::callbacks::{self, SURFACE_PTR};

    eprintln!(
        "cmux: create_surface called for pane_id={}, inherited_config={}",
        pane_id,
        inherited_config.is_some()
    );

    let gl_area = gtk4::GLArea::new();
    eprintln!(
        "cmux: created GLArea {:p} for pane_id={}",
        gl_area.as_ptr(),
        pane_id
    );
    // Per Pitfall 1: require OpenGL 4.3 before the area is realized.
    gl_area.set_required_version(4, 3);
    // Manual render mode: only render when wakeup_cb schedules queue_render().
    // An independent render loop adds input latency (per CLAUDE.md pitfall).
    gl_area.set_auto_render(false);
    // Must be focusable to receive keyboard events via EventControllerKey.
    gl_area.set_focusable(true);
    // Every terminal reserves the focus border (transparent until focused), so
    // moving focus recolors it instead of resizing the terminals.
    gl_area.add_css_class("cmux-pane");
    // Grab keyboard focus when the user clicks inside the terminal.
    gl_area.set_focus_on_click(true);
    // Expand to fill available space — required for GtkPaned to distribute space evenly.
    // Without this, GLArea has natural size 0 and the Paned gives all space to end child.
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);

    // Shared cell for the surface pointer — created in realize (after GL context exists),
    // then used in render, resize, input, and scale-factor callbacks.
    // Rc<RefCell<...>> is safe here: all callbacks run on the GLib main thread.
    let surface_cell: Rc<RefCell<Option<ffi::ghostty_surface_t>>> = Rc::new(RefCell::new(None));
    // Keep the cell reachable from the widget so detach_surface can clear it
    // before the surface is freed.
    unsafe {
        use gtk4::prelude::ObjectExt;
        gl_area.set_data(SURFACE_CELL_KEY, surface_cell.clone());
    }

    // ── GtkGLArea::realize ───────────────────────────────────────────────────
    // GL context is now valid. Create the surface HERE so ghostty can access
    // the GL context immediately after creation (fixes segfault in set_content_scale).
    //
    // IMPORTANT: GTK may re-realize the widget when reparenting (e.g., moving from
    // GtkStack into GtkPaned during split). We must check if the surface already
    // exists and reuse it, otherwise we create orphaned surfaces that never render.
    let pane_id_for_log = pane_id;
    gl_area.connect_realize({
        let cell = surface_cell.clone();
        let io_mode = io_mode;
        move |area| {
            eprintln!(
                "cmux: GLArea {:p} realize for pane_id={} — making GL context current",
                area.as_ptr(),
                pane_id_for_log
            );
            area.make_current();
            if let Some(err) = area.error() {
                eprintln!("cmux: GLArea realize error: {err}");
                std::process::exit(1); // Per D-09: no GUI error dialog in Phase 1
            }

            // Check if surface already exists (re-realize after reparent).
            // If so, just update the size/scale and refresh — don't create a new surface.
            //
            // DO NOT restore focus here. During a split, the old pane is reparented into
            // a GtkPaned — it should NOT regain focus. The new pane gets focus via its own
            // fresh realize path (set_focus(true) + grab_focus). EventControllerFocus
            // handles focus restoration automatically when GTK gives this widget focus back
            // (via `enter` signal). Calling set_focus(true) here incorrectly marks the old
            // pane as focused, causing both panes to have focused=true simultaneously and
            // triggering Ghostty's early-return guard on the new pane's subsequent focus calls.
            if let Some(existing_surface) = *cell.borrow() {
                eprintln!(
                    "cmux: GLArea {:p} re-realized — reinitializing GL resources for surface {:p}",
                    area.as_ptr(),
                    existing_surface
                );
                // Reinitialize renderer GL resources (shaders, swap chain) for the
                // new GL context. This matches Ghostty's own GTK apprt glareaRealize
                // which calls displayRealized() after reparent/display-move.
                unsafe {
                    ffi::ghostty_surface_display_realized(existing_surface);
                }
                let scale = area.scale_factor() as f64;
                let w = area.width();
                let h = area.height();
                unsafe {
                    let phys_w = (w as f64 * scale) as u32;
                    let phys_h = (h as f64 * scale) as u32;
                    if phys_w > 0 && phys_h > 0 {
                        ffi::ghostty_surface_set_size(existing_surface, phys_w, phys_h);
                    }
                    ffi::ghostty_surface_set_content_scale(existing_surface, scale, scale);
                    ffi::ghostty_surface_refresh(existing_surface);
                }
                area.queue_render();
                return;
            }

            eprintln!("cmux: GL context made current, no error");
            eprintln!(
                "cmux: GL area size at realize: {}x{}",
                area.width(),
                area.height()
            );
            eprintln!("cmux: GL scale factor at realize: {}", area.scale_factor());

            // Log GL version and renderer info for diagnostics.
            if let Some(ctx) = area.context() {
                let (major, minor) = ctx.version();
                eprintln!("cmux: GL context version: {major}.{minor}");
                eprintln!("cmux: GL context is_legacy: {}", ctx.is_legacy());
            }

            // Create the ghostty surface now that GL context is current.
            let surface = unsafe {
                let gl_area_ptr = area.as_ptr() as *mut std::ffi::c_void;

                let platform = ffi::ghostty_platform_u {
                    gtk4: ffi::ghostty_platform_gtk4_s {
                        gl_area: gl_area_ptr,
                    },
                };

                let mut surface_config = if let Some(ic) = inherited_config {
                    ic // already owned by value — no dangling pointer
                } else {
                    unsafe { ffi::ghostty_surface_config_new() }
                };

                surface_config.platform_tag = ffi::ghostty_platform_e_GHOSTTY_PLATFORM_GTK4;
                surface_config.platform = platform;
                // Ghostty passes this back to the clipboard callbacks; it is how
                // they find the surface (see surface_from_userdata).
                surface_config.userdata = gl_area_ptr;
                surface_config.scale_factor = area.scale_factor() as f64;

                // Set manual I/O mode for SSH remote surfaces.
                if let SurfaceIoMode::Manual { ref io_write_ctx } = io_mode {
                    surface_config.io_mode = ffi::ghostty_surface_io_mode_e_GHOSTTY_SURFACE_IO_MANUAL;
                    surface_config.io_write_cb = Some(crate::ssh::bridge::ssh_io_write_cb);
                    // Increment Arc refcount for the raw pointer — the matching
                    // Arc::from_raw happens in unrealize/cleanup.
                    let ctx_raw = std::sync::Arc::into_raw(io_write_ctx.clone()) as *mut std::ffi::c_void;
                    surface_config.io_write_userdata = ctx_raw;
                }

                // agent-status: export CMUX_WORKSPACE_ID / CMUX_SURFACE_ID / CMUX_SOCKET_PATH
                // into the pane's shell. env_block must outlive ghostty_surface_new.
                let mut env_block = EnvVarBlock::new(&crate::agent_status::pane_env_vars(
                    &ids.workspace.to_string(),
                    &ids.surface.to_string(),
                    &crate::socket::socket_path().to_string_lossy(),
                ));
                env_block.apply(&mut surface_config);

                eprintln!("cmux: calling ghostty_surface_new");
                let s = ffi::ghostty_surface_new(ghostty_app, &surface_config);
                drop(env_block);
                if s.is_null() {
                    eprintln!("cmux: FATAL — ghostty_surface_new returned null");
                    std::process::exit(1);
                }
                eprintln!("cmux: ghostty_surface_new succeeded: {:p}", s);
                // Check GL error state after surface creation.
                let gl_err = gl_get_error();
                if gl_err != 0 {
                    eprintln!("cmux: GL error after ghostty_surface_new: 0x{gl_err:x}");
                } else {
                    eprintln!("cmux: GL error state after ghostty_surface_new: OK");
                }
                s
            };

            if let Ok(mut registry) = callbacks::SURFACE_REGISTRY.lock() {
                registry.insert(surface as usize, pane_id);
            }
            // terminal-actions: a recycled surface pointer (e.g. after a
            // workspace close, which does not detach) must not inherit a title
            // or become the find bar's target.
            crate::header_bar::forget_surface(surface as usize);
            crate::find_bar::forget_surface(surface as usize);
            // agent-status: route this surface's progress reports to its workspace.
            if let Ok(mut map) = callbacks::SURFACE_WORKSPACE.lock() {
                map.insert(surface as usize, ids.workspace);
            }

            // Set initial size and scale after surface creation.
            let scale = area.scale_factor() as f64;
            let w = area.width();
            let h = area.height();
            unsafe {
                // Per Pitfall 5: convert logical→physical pixels.
                // Guard against calling set_size(0,0) at realize time — widget has not been
                // allocated its real size yet. connect_resize will fire with the correct size.
                let phys_w = (w as f64 * scale) as u32;
                let phys_h = (h as f64 * scale) as u32;
                if phys_w > 0 && phys_h > 0 {
                    ffi::ghostty_surface_set_size(surface, phys_w, phys_h);
                    eprintln!("cmux: ghostty_surface_set_size({}, {})", phys_w, phys_h);
                } else {
                    eprintln!(
                        "cmux: ghostty_surface_set_size skipped at realize time (size 0x0) — connect_resize will provide real size"
                    );
                }
                ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                eprintln!("cmux: ghostty_surface_set_content_scale({scale})");
                ffi::ghostty_surface_set_focus(surface, true);
                eprintln!("cmux: ghostty_surface_set_focus(true)");
            }

            // Store the surface pointer BEFORE grab_focus so that EventControllerFocus
            // `enter` can call set_focus(true) when the widget receives GTK keyboard focus.
            // If we store after, the enter handler finds cell=None and is a no-op.
            *cell.borrow_mut() = Some(surface);

            // Grab GTK keyboard focus so the terminal receives key events immediately
            // without requiring a mouse click (belt-and-suspenders with switch_to_index).
            area.grab_focus();
            // Also store in global for read_clipboard_cb (which has no surface arg).
            SURFACE_PTR.store(surface as usize, Ordering::SeqCst);

            // Register this GLArea in the multi-surface registry for wakeup_cb
            if let Ok(mut areas) = callbacks::GL_AREA_REGISTRY.lock() {
                areas.push(callbacks::GtkGLAreaPtr(
                    area.as_ptr() as *mut gtk4::ffi::GtkGLArea
                ));
            }
            // Register GLArea → surface mapping for notify::position focus restore
            if let Ok(mut gl_to_surface) = callbacks::GL_TO_SURFACE.lock() {
                gl_to_surface.insert(area.as_ptr() as usize, surface as usize);
            }

            // Request first render.
            area.queue_render();
        }
    });

    // ── GtkGLArea::unrealize — free renderer GL resources before context dies
    {
        let pane_id_unrealize = pane_id;
        let cell_unrealize = surface_cell.clone();
        gl_area.connect_unrealize(move |area| {
            eprintln!(
                "cmux: GLArea {:p} pane={} UNREALIZE — freeing GL resources",
                area.as_ptr(),
                pane_id_unrealize,
            );
            // Make GL context current so Ghostty can properly free GL objects.
            area.make_current();
            if let Some(surface) = *cell_unrealize.borrow() {
                unsafe {
                    ffi::ghostty_surface_display_unrealized(surface);
                }
                eprintln!(
                    "cmux: display_unrealized called for surface {:p}",
                    surface
                );
            }
        });
    }

    // ── GtkGLArea::render ────────────────────────────────────────────────────
    // Called by GTK frame clock when queue_render() was requested.
    gl_area.connect_render({
        let cell = surface_cell.clone();
        let render_count = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let pane_id_render = pane_id;
        move |area, _ctx| {
            let count = render_count.get() + 1;
            render_count.set(count);
            // Log every render — keep the session short!
            if true {
                eprintln!(
                    "cmux: render #{} pane={} area={:p} size={}x{} err={:?}",
                    count,
                    pane_id_render,
                    area.as_ptr(),
                    area.width(),
                    area.height(),
                    area.error()
                );
            }
            if let Some(surface) = *cell.borrow() {
                // Log GL state before draw to diagnose render stalls.
                if count % 60 == 1 || count <= 5 {
                    let mut viewport = [0i32; 4];
                    let mut draw_fbo = 0i32;
                    unsafe {
                        glGetIntegerv(GL_VIEWPORT, viewport.as_mut_ptr());
                        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut draw_fbo);
                    }
                    eprintln!(
                        "cmux: render GL state pane={}: viewport={}x{}+{}+{} draw_fbo={}",
                        pane_id_render,
                        viewport[2],
                        viewport[3],
                        viewport[0],
                        viewport[1],
                        draw_fbo
                    );
                }
                unsafe {
                    ffi::ghostty_surface_draw(surface);
                }
            } else {
                eprintln!("cmux: render callback — surface not yet initialized, skipping draw");
            }
            gtk4::glib::Propagation::Stop // suppress GTK default render
        }
    });

    // ── GtkGLArea::resize ────────────────────────────────────────────────────
    // GTK provides logical (CSS) pixels; Ghostty needs physical pixels (Pitfall 5).
    //
    // CRITICAL: ghostty_surface_set_size must be called SYNCHRONOUSLY in this
    // signal handler, not deferred to an idle. Ghostty's renderer anti-flicker
    // guard in drawFrame() compares GL_VIEWPORT (the actual widget size) against
    // the renderer's cached screen size. If we defer set_size to an idle, the
    // next drawFrame(true) — triggered by queue_render — sees a size mismatch
    // and re-presents the last frame forever (the guard returns before updating
    // the renderer's cached size). This matches Ghostty's own GTK apprt which
    // calls sizeCallback directly in glareaResize, not deferred.
    //
    // sizeCallback early-returns when the size hasn't changed, so redundant
    // calls during rapid drag are cheap (just a comparison, no reflow).
    //
    // Do NOT bounce focus (false→true) here. The cursor blink timer is
    // independent of resize — ghostty_surface_set_size only calls setScreenSize
    // on the renderer; it does not cancel the blink timer. A false→true bounce
    // kills the timer via an async cancel race (see restore_active_pane_focus).
    {
        let cell = surface_cell.clone();
        gl_area.connect_resize(move |area, logical_w, logical_h| {
            let scale = area.scale_factor();
            let phys_w = (logical_w * scale) as u32;
            let phys_h = (logical_h * scale) as u32;

            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_size(surface, phys_w, phys_h);
                }
            }

            // Drive the render loop directly — wakeup idles can be starved during
            // rapid resize events (sustained mouse drag floods the GLib main loop
            // with motion events at DEFAULT priority, delaying DEFAULT_IDLE wakeup
            // idles). Calling app_tick + queue_render here ensures the terminal
            // reflows and re-renders even during a sustained resize drag.
            let app_ptr =
                crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
            if app_ptr != 0 {
                unsafe {
                    let app = app_ptr as ffi::ghostty_app_t;
                    ffi::ghostty_app_tick(app);
                }
            }
            if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                for area_ptr in areas.iter() {
                    let area: glib::translate::Borrowed<gtk4::GLArea> =
                        unsafe { glib::translate::from_glib_borrow(area_ptr.0) };
                    if area.is_realized() {
                        area.queue_render();
                        area.queue_draw(); // Gap 1B: repaints CSS border
                    }
                }
            }
        });
    }

    // ── notify::scale-factor (GHOST-06) ─────────────────────────────────────
    // Fires when the window moves to a monitor with a different DPI.
    // Must use connect_notify_local: ghostty_surface_t is *mut c_void (not Send+Sync).
    // connect_notify_local only requires 'static, and runs on the GLib main thread.
    gl_area.connect_notify_local(Some("scale-factor"), {
        let cell = surface_cell.clone();
        move |widget, _| {
            if let Some(surface) = *cell.borrow() {
                // Prefer fractional scale from GdkSurface (Wayland fractional scaling)
                // over Widget::scale_factor() which returns the integer ceiling.
                let scale = widget.native()
                    .and_then(|n| n.surface())
                    .map(|s| s.scale())
                    .unwrap_or(widget.scale_factor() as f64);
                eprintln!("cmux: scale-factor changed to {} for surface {:p}", scale, surface);
                unsafe {
                    ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                    ffi::ghostty_surface_refresh(surface); // trigger redraw at new scale
                }
            }
        }
    });

    // ── Input method (IME) ───────────────────────────────────────────────────────
    // Follows Ghostty's GTK apprt (src/apprt/gtk/class/surface.zig): key events
    // are filtered through a GtkIMMulticontext first; preedit text is shown via
    // ghostty_surface_preedit and committed text is sent as key input.
    let im = Rc::new(ImState {
        context: gtk4::IMMulticontext::new(),
        composing: Cell::new(false),
        in_keyevent: Cell::new(ImKeyEvent::False),
        buf: RefCell::new([0u8; 128]),
        len: Cell::new(0),
    });
    {
        use gtk4::prelude::*;
        im.context.set_client_widget(Some(&gl_area));

        im.context.connect_preedit_start({
            let im = im.clone();
            move |_ctx| {
                im.composing.set(true);
                im.len.set(0);
            }
        });
        im.context.connect_preedit_changed({
            let im = im.clone();
            let cell = surface_cell.clone();
            move |ctx| {
                // fcitx5-hangul can commit and then immediately start a new
                // preedit without preedit-start, so always mark composing here.
                im.composing.set(true);
                let Some(surface) = *cell.borrow() else {
                    return;
                };
                let (preedit, _, _) = ctx.preedit_string();
                unsafe {
                    ffi::ghostty_surface_preedit(
                        surface,
                        preedit.as_ptr() as *const std::os::raw::c_char,
                        preedit.len(),
                    );
                }
            }
        });
        im.context.connect_preedit_end({
            let im = im.clone();
            let cell = surface_cell.clone();
            move |_ctx| {
                im.composing.set(false);
                if let Some(surface) = *cell.borrow() {
                    unsafe {
                        ffi::ghostty_surface_preedit(surface, std::ptr::null(), 0);
                    }
                }
            }
        });
        im.context.connect_commit({
            let im = im.clone();
            let cell = surface_cell.clone();
            move |_ctx, text| {
                // A plain (non-composed) key like "a" also arrives as a commit
                // during the key event. Buffer it so it is sent together with
                // the key event and Ghostty keeps the keycode.
                if im.in_keyevent.get() == ImKeyEvent::NotComposing {
                    let bytes = text.as_bytes();
                    let mut buf = im.buf.borrow_mut();
                    if bytes.len() >= buf.len() {
                        eprintln!("cmux: IM commit too long for buffer ({} bytes)", bytes.len());
                        return;
                    }
                    buf[..bytes.len()].copy_from_slice(bytes);
                    buf[bytes.len()] = 0;
                    im.len.set(bytes.len());
                    return;
                }

                // Composed text (or a commit from outside a key event): send it now.
                im.composing.set(false);
                if let Some(surface) = *cell.borrow() {
                    send_committed_text(surface, text);
                }
            }
        });
    }

    // ── Key input (GHOST-03) ─────────────────────────────────────────────────────
    // EventControllerKey fires key-pressed and key-released events.
    // CRITICAL: no allocations in this path — per CLAUDE.md typing-latency-sensitive paths.
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed({
        let cell = surface_cell.clone();
        let im = im.clone();
        move |ctrl, keyval, keycode, state| {
            use crate::ghostty::input::map_mods;
            use gtk4::prelude::*;

            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return gtk4::glib::Propagation::Proceed,
            };

            // Tell the IM where the cursor is so its candidate window shows up there.
            let (mut x, mut y, mut w, mut h) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            unsafe {
                ffi::ghostty_surface_ime_point(surface, &mut x, &mut y, &mut w, &mut h);
            }
            im.context
                .set_cursor_location(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1));

            // Let the IM filter the key first. Commit/preedit signals may fire
            // synchronously inside filter_keypress, so no RefCell borrows here.
            im.in_keyevent.set(if im.composing.get() {
                ImKeyEvent::Composing
            } else {
                ImKeyEvent::NotComposing
            });
            let im_handled = ctrl
                .current_event()
                .map(|event| im.context.filter_keypress(&event))
                .unwrap_or(false);
            let was_composing = im.in_keyevent.get() == ImKeyEvent::Composing;
            im.in_keyevent.set(ImKeyEvent::False);

            if crate::ghostty::input::im_swallows_key(
                im_handled,
                im.composing.get(),
                was_composing,
                im.len.get(),
            ) {
                im.len.set(0);
                return gtk4::glib::Propagation::Stop;
            }

            // text field: UTF-8 from the keyval (what the key produces with modifiers applied).
            // Must be a C string. Use a stack-allocated buffer to avoid heap allocation.
            // Prefer text the IM committed for this key event, if any.
            let im_buf = im.buf.borrow();
            let unicode = keyval.to_unicode();
            let mut text_buf = [0u8; 8]; // UTF-8: max 4 bytes + null
            let text_ptr = if im.len.get() > 0 {
                im_buf.as_ptr() as *const i8
            } else if let Some(ch) = unicode {
                let mut s = [0u8; 5];
                let encoded = ch.encode_utf8(&mut s[..4]);
                let len = encoded.len();
                text_buf[..len].copy_from_slice(encoded.as_bytes());
                text_buf[len] = 0;
                text_buf.as_ptr() as *const i8
            } else {
                std::ptr::null()
            };

            let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
            // keycode must be the raw GTK hardware keycode (XKB scancode).
            // Ghostty looks this up in its own native keycodes table to resolve the physical key.
            // Do NOT translate to ghostty_input_key_e here — that is an entirely different type.
            input.keycode = keycode;
            input.mods = map_mods(state);
            input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_PRESS;
            input.text = text_ptr;
            input.consumed_mods = 0; // Not used in Phase 1
            input.unshifted_codepoint = unshifted_codepoint(ctrl, keycode);
            input.composing = im.composing.get();

            let consumed = unsafe { ffi::ghostty_surface_key(surface, input) };
            drop(im_buf);
            im.len.set(0);

            // A keybinding consumed the key mid-composition: drop the preedit.
            if consumed && im.composing.get() {
                im.context.reset();
                unsafe {
                    ffi::ghostty_surface_preedit(surface, std::ptr::null(), 0);
                }
            }
            gtk4::glib::Propagation::Stop // Inhibit: prevent GTK from handling the key
        }
    });
    key_controller.connect_key_released({
        let cell = surface_cell.clone();
        let im = im.clone();
        move |ctrl, _keyval, keycode, state| {
            use crate::ghostty::input::map_mods;
            use gtk4::prelude::*;

            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };

            // Releases go through the IM too; don't encode them while composing.
            im.in_keyevent.set(if im.composing.get() {
                ImKeyEvent::Composing
            } else {
                ImKeyEvent::NotComposing
            });
            let im_handled = ctrl
                .current_event()
                .map(|event| im.context.filter_keypress(&event))
                .unwrap_or(false);
            im.in_keyevent.set(ImKeyEvent::False);
            im.len.set(0);
            if im_handled && im.composing.get() {
                return;
            }

            let mut input = unsafe { std::mem::zeroed::<ffi::ghostty_input_key_s>() };
            input.keycode = keycode;
            input.mods = map_mods(state);
            input.action = ffi::ghostty_input_action_e_GHOSTTY_ACTION_RELEASE;
            input.text = std::ptr::null();
            input.consumed_mods = 0; // Not used in Phase 1
            input.unshifted_codepoint = unshifted_codepoint(ctrl, keycode);
            unsafe {
                ffi::ghostty_surface_key(surface, input);
            }
        }
    });
    gl_area.add_controller(key_controller);

    // ── Mouse button input (GHOST-04) ────────────────────────────────────────────
    let click_gesture = gtk4::GestureClick::new();
    click_gesture.set_button(0); // 0 = listen to all mouse buttons
    click_gesture.connect_pressed({
        let cell = surface_cell.clone();
        move |gesture, _n_press, _x, _y| {
            // Click-to-focus: GLArea does not take focus on its own, and the split
            // engine's active pane must follow, or focus restoration (keyed on the
            // active-pane CSS class) snaps focus back to the previous pane.
            if let Some(widget) = gesture.widget() {
                widget.grab_focus();
                callbacks::CLICK_FOCUS_GL_AREA.store(
                    widget.as_ptr() as usize,
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            if std::env::var_os("CMUX_DEBUG_INPUT").is_some() {
                eprintln!("cmux-input: mouse press button={} surface={:p}", gesture.current_button(), surface);
            }
            let button = match gesture.current_button() {
                1 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_LEFT,
                2 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_MIDDLE,
                3 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_RIGHT,
                _ => return,
            };
            let mods = crate::ghostty::input::map_mods(gesture.current_event_state());
            unsafe {
                ffi::ghostty_surface_mouse_button(
                    surface,
                    ffi::ghostty_input_mouse_state_e_GHOSTTY_MOUSE_PRESS,
                    button,
                    mods,
                );
            }
        }
    });
    click_gesture.connect_released({
        let cell = surface_cell.clone();
        move |gesture, _n_press, _x, _y| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            let button = match gesture.current_button() {
                1 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_LEFT,
                2 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_MIDDLE,
                3 => ffi::ghostty_input_mouse_button_e_GHOSTTY_MOUSE_RIGHT,
                _ => return,
            };
            if std::env::var_os("CMUX_DEBUG_INPUT").is_some() {
                eprintln!("cmux-input: mouse release button={} surface={:p}", gesture.current_button(), surface);
            }
            let mods = crate::ghostty::input::map_mods(gesture.current_event_state());
            unsafe {
                ffi::ghostty_surface_mouse_button(
                    surface,
                    ffi::ghostty_input_mouse_state_e_GHOSTTY_MOUSE_RELEASE,
                    button,
                    mods,
                );
            }
        }
    });
    if std::env::var_os("CMUX_DEBUG_INPUT").is_some() {
        click_gesture.connect_stopped(|_| eprintln!("cmux-input: click gesture stopped"));
        click_gesture.connect_cancel(|_, _| eprintln!("cmux-input: click gesture cancelled"));
    }
    gl_area.add_controller(click_gesture);

    // ── Mouse motion ─────────────────────────────────────────────────────────────
    let motion_controller = gtk4::EventControllerMotion::new();
    motion_controller.connect_motion({
        let cell = surface_cell.clone();
        move |ctrl, x, y| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return,
            };
            let mods = crate::ghostty::input::map_mods(ctrl.current_event_state());
            unsafe {
                crate::ghostty::ffi::ghostty_surface_mouse_pos(surface, x, y, mods);
            }
        }
    });
    gl_area.add_controller(motion_controller);

    // ── Focus tracking (GHOST-05) ─────────────────────────────────────────────
    // EventControllerFocus fires enter/leave when GTK keyboard focus enters/leaves the widget.
    // This ensures ghostty_surface_set_focus() stays in sync with GTK focus routing —
    // critical after GtkPaned drags (separator steals focus) and sidebar show/hide.
    // Without this, Ghostty's internal focused flag diverges from GTK reality, and
    // subsequent set_focus(true) calls hit the early-return guard (if self.focused == focused { return; }).
    let focus_controller = gtk4::EventControllerFocus::new();
    focus_controller.connect_enter({
        let cell = surface_cell.clone();
        let gl_area_for_focus = gl_area.clone();
        let im = im.clone();
        move |_ctrl| {
            use gtk4::prelude::*;
            im.context.focus_in();
            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, true);
                    // Kick the render loop so the cursor becomes visible immediately
                    // rather than waiting up to one blink interval (~500ms). The
                    // renderer thread processes the focused=true message asynchronously;
                    // without a refresh+queue_render here, GTK renders that happen
                    // before the message is processed show the stale (invisible) cursor.
                    ffi::ghostty_surface_refresh(surface);
                }
                gl_area_for_focus.queue_render();
            }
        }
    });
    focus_controller.connect_leave({
        let cell = surface_cell.clone();
        let im = im.clone();
        move |_ctrl| {
            use gtk4::prelude::*;
            im.context.focus_out();
            if let Some(surface) = *cell.borrow() {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, false);
                }
            }
        }
    });
    gl_area.add_controller(focus_controller);

    // ── Scroll input ─────────────────────────────────────────────────────────────
    let scroll_controller = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::DISCRETE,
    );
    scroll_controller.connect_scroll({
        let cell = surface_cell.clone();
        move |ctrl, dx, dy| {
            let surface = match *cell.borrow() {
                Some(s) => s,
                None => return gtk4::glib::Propagation::Proceed,
            };
            // Detect if this is pixel-precise (touchpad) or discrete (mouse wheel)
            let is_pixel = ctrl
                .current_event()
                .and_then(|e| e.downcast::<gtk4::gdk::ScrollEvent>().ok())
                .map(|se| se.direction() == gtk4::gdk::ScrollDirection::Smooth)
                .unwrap_or(false);

            // ghostty_input_scroll_mods_t is a bitmask:
            // bit 0: scroll_is_pixel (1 if touchpad, 0 if mouse wheel)
            // bit 1: momentum (1 if momentum scrolling)
            let scroll_mods = if is_pixel { 1 } else { 0 };

            // Match Ghostty's GTK apprt (ecMouseScrollVertical): GTK deltas are
            // inverted for Ghostty's scroll callback, and precision (touchpad)
            // deltas are scaled by 10 for a usable scroll speed.
            let multiplier = if is_pixel { 10.0 } else { 1.0 };
            unsafe {
                ffi::ghostty_surface_mouse_scroll(
                    surface,
                    -dx * multiplier,
                    -dy * multiplier,
                    scroll_mods,
                );
            }
            gtk4::glib::Propagation::Stop
        }
    });
    gl_area.add_controller(scroll_controller);

    (gl_area, surface_cell)
}

// ── Clipboard callbacks ──────────────────────────────────────────────────────

/// Resolve the surface for a Ghostty runtime callback. Each surface's userdata is
/// its GtkGLArea pointer (set in create_surface), and GL_TO_SURFACE maps that to
/// the live surface. Returns None if the surface has since been closed.
fn surface_from_userdata(userdata: *mut std::ffi::c_void) -> Option<ffi::ghostty_surface_t> {
    if userdata.is_null() {
        return None;
    }
    let map = crate::ghostty::callbacks::GL_TO_SURFACE.lock().ok()?;
    map.get(&(userdata as usize))
        .map(|&s| s as ffi::ghostty_surface_t)
}

/// Ghostty's embedded runtime declares this as returning bool ("started"); if it
/// returns false Ghostty frees the request. The bundled ghostty.h still says void,
/// so the signature here follows src/apprt/embedded.zig instead.
pub(crate) unsafe extern "C" fn read_clipboard_cb(
    userdata: *mut std::ffi::c_void,
    clipboard_type: crate::ghostty::ffi::ghostty_clipboard_e,
    request: *mut std::ffi::c_void,
) -> bool {
    use gtk4::prelude::*;

    if surface_from_userdata(userdata).is_none() {
        return false;
    }
    let display = match gtk4::gdk::Display::default() {
        Some(d) => d,
        None => return false,
    };
    let clipboard = if clipboard_type == ffi::ghostty_clipboard_e_GHOSTTY_CLIPBOARD_SELECTION {
        display.primary_clipboard()
    } else {
        display.clipboard()
    };

    // Read asynchronously and complete the request later, like Ghostty's GTK
    // apprt. Blocking the main loop here re-enters GTK from inside a Ghostty
    // callback. Ghostty keeps `request` alive until it is completed.
    let userdata = userdata as usize;
    let request = request as usize;
    glib::spawn_future_local(async move {
        let text = clipboard
            .read_text_future()
            .await
            .ok()
            .flatten()
            .map(|s| s.replace('\0', ""))
            .unwrap_or_default();
        let Ok(c_text) = std::ffi::CString::new(text) else {
            return;
        };
        // The surface may have been closed while the read was pending.
        let Some(surface) = surface_from_userdata(userdata as *mut std::ffi::c_void) else {
            return;
        };
        unsafe {
            // Phase 1: auto-confirm all clipboard reads without a dialog (per D-09).
            ffi::ghostty_surface_complete_clipboard_request(
                surface,
                c_text.as_ptr(),
                request as *mut std::ffi::c_void,
                true,
            );
        }
    });
    true
}

pub(crate) unsafe extern "C" fn confirm_read_clipboard_cb(
    userdata: *mut std::ffi::c_void,
    value: *const std::os::raw::c_char,
    request: *mut std::ffi::c_void,
    _request_type: crate::ghostty::ffi::ghostty_clipboard_request_e,
) {
    // Phase 1: auto-confirm all clipboard reads without a dialog (per D-09).
    // The third argument is the pending clipboard request (not the surface);
    // it must be handed back to complete_clipboard_request.
    let Some(surface) = surface_from_userdata(userdata) else {
        return;
    };
    unsafe {
        ffi::ghostty_surface_complete_clipboard_request(surface, value, request, true);
    }
}

pub(crate) unsafe extern "C" fn write_clipboard_cb(
    _userdata: *mut std::ffi::c_void,
    clipboard_type: crate::ghostty::ffi::ghostty_clipboard_e,
    content: *const crate::ghostty::ffi::ghostty_clipboard_content_s,
    _len: usize,
    _confirm: bool,
) {
    use gtk4::prelude::*;

    if content.is_null() {
        return;
    }
    let item = &*content;
    let text = if !item.data.is_null() {
        match std::ffi::CStr::from_ptr(item.data).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    } else {
        return;
    };

    let display = match gtk4::gdk::Display::default() {
        Some(d) => d,
        None => return,
    };
    let clipboard = if clipboard_type == ffi::ghostty_clipboard_e_GHOSTTY_CLIPBOARD_SELECTION {
        display.primary_clipboard()
    } else {
        display.clipboard()
    };
    clipboard.set_text(&text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_vars(config: &ffi::ghostty_surface_config_s) -> Vec<(String, String)> {
        if config.env_var_count == 0 {
            return Vec::new();
        }
        let vars = unsafe { std::slice::from_raw_parts(config.env_vars, config.env_var_count) };
        vars.iter()
            .map(|v| unsafe {
                (
                    std::ffi::CStr::from_ptr(v.key).to_string_lossy().into_owned(),
                    std::ffi::CStr::from_ptr(v.value).to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn env_var_block_sets_surface_config() {
        let ids = PaneIds::new(uuid::Uuid::new_v4());
        let pairs = crate::agent_status::pane_env_vars(
            &ids.workspace.to_string(),
            &ids.surface.to_string(),
            "/run/user/1000/cmux/cmux.sock",
        );
        let mut block = EnvVarBlock::new(&pairs);
        let mut config: ffi::ghostty_surface_config_s = unsafe { std::mem::zeroed() };
        block.apply(&mut config);
        assert_eq!(read_vars(&config), pairs);
    }

    #[test]
    fn env_var_block_skips_nul_and_handles_empty() {
        let pairs = vec![
            ("BAD\0KEY".to_string(), "x".to_string()),
            ("CMUX_MSG".to_string(), "한글 값".to_string()),
        ];
        let mut block = EnvVarBlock::new(&pairs);
        let mut config: ffi::ghostty_surface_config_s = unsafe { std::mem::zeroed() };
        block.apply(&mut config);
        assert_eq!(read_vars(&config), vec![("CMUX_MSG".to_string(), "한글 값".to_string())]);

        let mut empty = EnvVarBlock::new(&[]);
        empty.apply(&mut config);
        assert!(config.env_vars.is_null());
        assert_eq!(config.env_var_count, 0);
    }
}
