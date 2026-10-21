use crate::ghostty::ffi;
use gtk4::prelude::*;
use uuid::Uuid;

/// Direction for pane focus navigation (Ctrl+Shift+arrows per D-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Recursive pane layout tree. Each workspace has one root SplitNode.
/// - Leaf: a single terminal pane (GtkGLArea + Ghostty surface)
/// - Split: two child subtrees separated by a GtkPaned divider
///
/// Per SPLIT-06: this is the Bonsplit Rust port — immutable-style tree where
/// split/close operations return a new root.
#[derive(Clone)]
pub enum SplitNode {
    Leaf {
        pane_id: u64,
        gl_area: gtk4::GLArea,
        surface: ffi::ghostty_surface_t,
        /// Stable UUID for session persistence and v2 socket protocol pane identity.
        uuid: Uuid,
        /// Phase 4 NOTF-01: true when this pane has unread bell activity.
        has_attention: bool,
        /// tabs: the pane's other terminal tabs. The fields above are always
        /// the ACTIVE tab (see tabs.rs).
        tabs: crate::tabs::PaneTabs,
    },
    /// Phase 8: Browser preview pane (agent-browser frame rendering).
    Preview {
        pane_id: u64,
        container: gtk4::Box,
        picture: gtk4::Picture,
        #[allow(dead_code)] // Kept alive for GTK widget tree reference counting
        url_entry: gtk4::Entry,
        uuid: Uuid,
        /// pane-zoom: nav-bar full-pane toggle; its icon follows the zoom state.
        zoom_btn: gtk4::Button,
    },
    Split {
        orientation: gtk4::Orientation,
        paned: gtk4::Paned,
        start: Box<SplitNode>,
        end: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Returns the root GTK widget for this node (GLArea for Leaf, Overlay for Preview, Paned for Split).
    pub fn widget(&self) -> gtk4::Widget {
        match self {
            SplitNode::Leaf { gl_area, tabs, .. } => match &tabs.ui {
                Some(ui) => ui.container.clone().upcast(),
                None => gl_area.clone().upcast(),
            },
            SplitNode::Preview { container, .. } => container.clone().upcast(),
            SplitNode::Split { paned, .. } => paned.clone().upcast(),
        }
    }

    /// Find the pane_id of the active (focused) leaf by checking CSS class.
    pub fn find_active_pane_id(&self) -> Option<u64> {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if gl_area.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Preview { pane_id, container, .. } => {
                if container.has_css_class("active-pane") {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => start
                .find_active_pane_id()
                .or_else(|| end.find_active_pane_id()),
        }
    }

    /// Find the UUID for a pane by pane_id. Returns None if not found.
    pub fn find_uuid_for_pane(&self, target_id: u64) -> Option<String> {
        match self {
            // tabs: background tabs keep their own pane_id/uuid.
            SplitNode::Leaf { tabs, .. } if tabs.others.iter().any(|t| t.pane_id == target_id) => {
                tabs.others.iter().find(|t| t.pane_id == target_id).map(|t| t.uuid.to_string())
            }
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                if *pane_id == target_id {
                    Some(uuid.to_string())
                } else {
                    None
                }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_uuid_for_pane(target_id)
                    .or_else(|| end.find_uuid_for_pane(target_id))
            }
        }
    }

    /// Apply the active-pane CSS class to the leaf matching active_pane_id.
    /// Removes the class from all other leaves.
    pub fn update_focus_css(&self, active_pane_id: u64) {
        match self {
            SplitNode::Leaf {
                pane_id, gl_area, ..
            } => {
                if *pane_id == active_pane_id {
                    gl_area.add_css_class("active-pane");
                } else {
                    gl_area.remove_css_class("active-pane");
                }
            }
            SplitNode::Preview { pane_id, container, .. } => {
                if *pane_id == active_pane_id {
                    container.add_css_class("active-pane");
                } else {
                    container.remove_css_class("active-pane");
                }
            }
            SplitNode::Split { start, end, .. } => {
                start.update_focus_css(active_pane_id);
                end.update_focus_css(active_pane_id);
            }
        }
    }

    /// Find a node by pane_id.
    pub fn find_node(&self, target_id: u64) -> Option<&SplitNode> {
        match self {
            SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => {
                if *pane_id == target_id { Some(self) } else { None }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_node(target_id).or_else(|| end.find_node(target_id))
            }
        }
    }

    /// Collect all leaf pane_ids into a Vec (for cleanup on workspace close).
    pub fn collect_pane_ids(&self, out: &mut Vec<u64>) {
        match self {
            SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => out.push(*pane_id),
            SplitNode::Split { start, end, .. } => {
                start.collect_pane_ids(out);
                end.collect_pane_ids(out);
            }
        }
    }

    /// Collect all surfaces into a Vec (for ghostty_surface_free on workspace close).
    /// Collect every terminal GLArea in this subtree (Preview panes have none).
    pub fn collect_gl_areas(&self, out: &mut Vec<gtk4::GLArea>) {
        match self {
            SplitNode::Leaf { gl_area, tabs, .. } => {
                out.push(gl_area.clone());
                out.extend(tabs.others.iter().map(|t| t.gl_area.clone()));
            }
            SplitNode::Preview { .. } => {}
            SplitNode::Split { start, end, .. } => {
                start.collect_gl_areas(out);
                end.collect_gl_areas(out);
            }
        }
    }

    pub fn collect_surfaces(&self, out: &mut Vec<ffi::ghostty_surface_t>) {
        match self {
            SplitNode::Leaf { surface, tabs, .. } => {
                out.push(*surface);
                out.extend(tabs.others.iter().map(|t| t.surface));
            }
            SplitNode::Preview { .. } => {} // No Ghostty surface
            SplitNode::Split { start, end, .. } => {
                start.collect_surfaces(out);
                end.collect_surfaces(out);
            }
        }
    }

    /// Find the Ghostty surface handle for a specific pane by pane_id.
    /// Used by debug.type to send text to a specific pane's surface.
    pub fn find_surface_for_pane(&self, target_id: u64) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf { pane_id, surface, tabs, .. } => {
                if *pane_id == target_id {
                    Some(*surface)
                } else {
                    tabs.others.iter().find(|t| t.pane_id == target_id).map(|t| t.surface)
                }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface
            SplitNode::Split { start, end, .. } => {
                start.find_surface_for_pane(target_id)
                    .or_else(|| end.find_surface_for_pane(target_id))
            }
        }
    }

    /// Collect (uuid, pane_id, active) for all leaves in this subtree.
    pub fn collect_pane_info(&self, out: &mut Vec<(Uuid, u64, bool)>, active_id: u64) {
        match self {
            SplitNode::Leaf { pane_id, uuid, .. } | SplitNode::Preview { pane_id, uuid, .. } => {
                out.push((*uuid, *pane_id, *pane_id == active_id));
            }
            SplitNode::Split { start, end, .. } => {
                start.collect_pane_info(out, active_id);
                end.collect_pane_info(out, active_id);
            }
        }
    }

    /// Find the ghostty surface handle for the leaf matching target_uuid (UUID string).
    pub fn find_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        match self {
            SplitNode::Leaf { uuid, surface, gl_area, tabs, .. } => {
                // tabs: resolve through the GLArea, the leaf pointer may be null.
                if uuid.to_string() == target_uuid {
                    crate::tabs::live_surface(gl_area, *surface)
                } else {
                    tabs.others
                        .iter()
                        .find(|t| t.uuid.to_string() == target_uuid)
                        .and_then(|t| crate::tabs::live_surface(&t.gl_area, t.surface))
                }
            }
            SplitNode::Preview { .. } => None, // No Ghostty surface to return
            SplitNode::Split { start, end, .. } => {
                start.find_by_uuid(target_uuid).or_else(|| end.find_by_uuid(target_uuid))
            }
        }
    }

    /// Find the pane_id for the leaf matching target_uuid (UUID string).
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        match self {
            SplitNode::Leaf { tabs, .. }
                if tabs.others.iter().any(|t| t.uuid.to_string() == target_uuid) =>
            {
                tabs.others.iter().find(|t| t.uuid.to_string() == target_uuid).map(|t| t.pane_id)
            }
            SplitNode::Leaf { uuid, pane_id, .. } | SplitNode::Preview { uuid, pane_id, .. } => {
                if uuid.to_string() == target_uuid { Some(*pane_id) } else { None }
            }
            SplitNode::Split { start, end, .. } => {
                start.find_pane_id_by_uuid(target_uuid)
                    .or_else(|| end.find_pane_id_by_uuid(target_uuid))
            }
        }
    }

    /// Set has_attention on the leaf matching pane_id. Returns true if found.
    pub fn set_attention(&mut self, target_pane_id: u64, value: bool) -> bool {
        match self {
            SplitNode::Leaf { pane_id, has_attention, tabs, .. } => {
                let found = if *pane_id == target_pane_id {
                    *has_attention = value;
                    true
                } else if let Some(t) = tabs.others.iter_mut().find(|t| t.pane_id == target_pane_id) {
                    t.has_attention = value;
                    true
                } else {
                    false
                };
                if found {
                    self.refresh_tab_bar();
                }
                found
            }
            SplitNode::Preview { .. } => false, // No attention state
            SplitNode::Split { start, end, .. } => {
                start.set_attention(target_pane_id, value) || end.set_attention(target_pane_id, value)
            }
        }
    }

    /// Returns true if any leaf in this subtree has attention.
    pub fn any_attention(&self) -> bool {
        match self {
            SplitNode::Leaf { has_attention, tabs, .. } => {
                *has_attention || tabs.others.iter().any(|t| t.has_attention)
            }
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => start.any_attention() || end.any_attention(),
        }
    }

    /// Check if a specific pane has attention.
    pub fn pane_has_attention(&self, target_pane_id: u64) -> bool {
        match self {
            SplitNode::Leaf { pane_id, has_attention, tabs, .. } => {
                (*pane_id == target_pane_id && *has_attention)
                    || tabs.others.iter().any(|t| t.pane_id == target_pane_id && t.has_attention)
            }
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => {
                start.pane_has_attention(target_pane_id) || end.pane_has_attention(target_pane_id)
            }
        }
    }

    /// Clear attention on all leaves in this subtree.
    pub fn clear_all_attention(&mut self) {
        match self {
            SplitNode::Leaf { has_attention, tabs, .. } => {
                let had = *has_attention || tabs.others.iter().any(|t| t.has_attention);
                *has_attention = false;
                tabs.others.iter_mut().for_each(|t| t.has_attention = false);
                if had {
                    self.refresh_tab_bar();
                }
            }
            SplitNode::Preview { .. } => {} // No attention state
            SplitNode::Split { start, end, .. } => {
                start.clear_all_attention();
                end.clear_all_attention();
            }
        }
    }
}

/// Attach right-click context menu to a terminal GLArea (D-08).
/// Uses button 3 (right-click only) to avoid interfering with Ghostty's mouse handling.
fn attach_terminal_context_menu(gl_area: &gtk4::GLArea) {
    let menu_model = crate::menus::build_terminal_context_menu();
    let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
    popover.set_parent(gl_area);
    popover.set_has_arrow(false);

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // Right-click only
    gesture.connect_released({
        let popover = popover.clone();
        move |_, _, x, y| {
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                x as i32, y as i32, 1, 1,
            )));
            popover.popup();
        }
    });
    gl_area.add_controller(gesture);
}

/// SplitEngine manages one workspace's pane layout tree.
pub struct SplitEngine {
    pub root: SplitNode,
    pub active_pane_id: u64,
    /// Monotonically increasing pane ID counter.
    next_pane_id: u64,
    /// GTK Application handle needed to create new GLAreas.
    app: gtk4::Application,
    /// Ghostty app handle needed to create new surfaces.
    ghostty_app: ffi::ghostty_app_t,
    /// Previously active panes, for "focus previous pane" (pane-nav).
    focus_history: FocusHistory,
    /// UUID of the owning workspace, exported to new panes as CMUX_WORKSPACE_ID.
    workspace_uuid: Uuid,
    /// pane-zoom: set while one pane fills the workspace.
    zoom: Option<PaneZoom>,
}

/// pane-zoom: a pane enlarged to fill its workspace (tmux zoom). The sibling
/// subtree at every split on the path from the root is hidden rather than
/// reparented (moving a GLArea unrealizes it and breaks its Ghostty surface):
/// a GtkPaned with one invisible child gives the other all of its space and
/// hides the handle, and keeps its divider position for unzoom.
struct PaneZoom {
    pane_id: u64,
    hidden: Vec<gtk4::Widget>,
    /// Widget carrying the `pane-zoomed` CSS hint.
    marked: gtk4::Widget,
}

impl SplitEngine {
    /// Create a new SplitEngine with a single leaf pane.
    /// The initial GLArea and surface are created by the caller (Plan 04) and passed in.
    pub fn new(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        initial_gl_area: gtk4::GLArea,
        initial_surface_cell: std::rc::Rc<std::cell::RefCell<Option<ffi::ghostty_surface_t>>>,
        pane_id: u64,
        ids: crate::ghostty::surface::PaneIds,
    ) -> Self {
        // The initial surface may not be realized yet. SplitEngine stores the cell
        // so it can read the surface pointer after realize. For focus/split operations
        // that run after realize, we read from the cell.
        // For the tree structure, we store null initially and update after realize.
        let surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&initial_gl_area);
        let _ = surface_placeholder;
        let root = new_leaf(pane_id, initial_gl_area, ids.surface);
        SplitEngine {
            root,
            active_pane_id: pane_id,
            next_pane_id: pane_id + 1,
            app,
            ghostty_app,
            focus_history: FocusHistory::default(),
            workspace_uuid: ids.workspace,
            zoom: None,
        }
    }


    /// Update the surface pointer for a leaf after realize.
    /// Recursively searches the tree (D-10: works for ALL leaves, not just root).
    pub fn set_initial_surface(&mut self, pane_id: u64, surface: ffi::ghostty_surface_t) {
        Self::set_surface_recursive(&mut self.root, pane_id, surface);
    }

    fn set_surface_recursive(node: &mut SplitNode, target_pane_id: u64, surface: ffi::ghostty_surface_t) {
        match node {
            SplitNode::Leaf { pane_id, surface: s, tabs, .. } => {
                if *pane_id == target_pane_id {
                    *s = surface;
                } else if let Some(t) = tabs.others.iter_mut().find(|t| t.pane_id == target_pane_id) {
                    t.surface = surface;
                }
            }
            SplitNode::Preview { .. } => {} // No surface to set
            SplitNode::Split { start, end, .. } => {
                Self::set_surface_recursive(start, target_pane_id, surface);
                Self::set_surface_recursive(end, target_pane_id, surface);
            }
        }
    }

    /// Reconstruct a SplitEngine from serialized SplitNodeData (D-05).
    /// Creates GTK widgets (GLArea for leaves, Paned for splits) and Ghostty surfaces.
    /// Fresh pane_ids are generated (D-06), but UUIDs are preserved from session.
    /// Returns None if tree_depth > 16 (D-14).
    pub fn from_data(
        app: gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        active_pane_uuid: Option<&str>,
        workspace_uuid: Uuid,
    ) -> Option<Self> {
        let mut next_pane_id: u64 = 1;
        let root = Self::node_from_data(&app, ghostty_app, data, &mut next_pane_id, 0, workspace_uuid)?;
        // Find active pane by saved UUID, or fall back to first leaf
        let active_id = active_pane_uuid
            .and_then(|uuid_str| root.find_pane_id_by_uuid(uuid_str))
            .unwrap_or_else(|| {
                let mut leaves = Vec::new();
                collect_leaves_in_order(&root, &mut leaves);
                leaves.first().copied().unwrap_or(1)
            });
        Some(SplitEngine {
            root,
            active_pane_id: active_id,
            next_pane_id,
            app,
            ghostty_app,
            focus_history: FocusHistory::default(),
            workspace_uuid,
            zoom: None,
        })
    }

    fn node_from_data(
        app: &gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        next_pane_id: &mut u64,
        depth: u32,
        workspace_uuid: Uuid,
    ) -> Option<SplitNode> {
        if depth > 16 {
            eprintln!("cmux: session restore tree depth > 16, falling back (D-14)");
            return None;
        }
        match data {
            SplitNodeData::Leaf { surface_uuid, cwd, tabs: saved_tabs, active_tab, .. } => {
                // tabs: every saved tab of the pane (sessions from before tabs
                // existed have none: the leaf itself is the only tab).
                let saved: Vec<crate::tabs::TabData> = if saved_tabs.is_empty() {
                    vec![crate::tabs::TabData { surface_uuid: *surface_uuid, cwd: cwd.clone() }]
                } else {
                    saved_tabs.clone()
                };
                let active = (*active_tab).min(saved.len() - 1);
                let mut entries: Vec<crate::tabs::TabEntry> = Vec::with_capacity(saved.len());
                for tab in &saved {
                    let pane_id = *next_pane_id;
                    *next_pane_id += 1;
                    // D-06: preserve UUID from session. Preview panes are saved as
                    // nil-UUID leaves; give those a fresh one so restored panes never
                    // share an identity (or a CMUX_SURFACE_ID).
                    let uuid = if tab.surface_uuid.is_nil() { Uuid::new_v4() } else { tab.surface_uuid };
                    // Create surface — realize callback will create Ghostty surface and wire registries
                    let ids = crate::ghostty::surface::PaneIds { workspace: workspace_uuid, surface: uuid };
                    let mut config = local_pane_config(None, ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_WINDOW);
                    // Every restored terminal starts in its saved working directory,
                    // if it still exists (else the default directory).
                    if !tab.cwd.is_empty() && std::path::Path::new(&tab.cwd).is_dir() {
                        config.working_directory = leak_c_string(&tab.cwd);
                    }
                    let (gl_area, _surface_cell) =
                        crate::ghostty::surface::create_surface(app, ghostty_app, Some(config), pane_id, crate::ghostty::surface::SurfaceIoMode::Exec, ids);
                    // Phase 9: Attach right-click context menu (D-08)
                    attach_terminal_context_menu(&gl_area);
                    entries.push(crate::tabs::TabEntry {
                        pane_id,
                        gl_area,
                        surface: std::ptr::null_mut(),
                        uuid,
                        has_attention: false,
                    });
                }
                let current = entries.remove(active);
                let mut leaf = new_leaf(current.pane_id, current.gl_area, current.uuid);
                if let SplitNode::Leaf { tabs, gl_area, .. } = &mut leaf {
                    if !entries.is_empty() {
                        let ui = ensure_tabs_ui(gl_area, tabs);
                        for t in &entries {
                            ui.stack.add_child(&t.gl_area);
                        }
                        ui.stack.set_visible_child(gl_area);
                    }
                    tabs.others = entries;
                    tabs.active = active;
                }
                leaf.refresh_tab_bar();
                Some(leaf)
            }
            SplitNodeData::Split { orientation, ratio, start, end } => {
                let start_node = Self::node_from_data(app, ghostty_app, start, next_pane_id, depth + 1, workspace_uuid)?;
                let end_node = Self::node_from_data(app, ghostty_app, end, next_pane_id, depth + 1, workspace_uuid)?;
                let gtk_orientation = match orientation.as_str() {
                    "vertical" => gtk4::Orientation::Vertical,
                    _ => gtk4::Orientation::Horizontal,
                };
                let paned = gtk4::Paned::new(gtk_orientation);
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                paned.set_wide_handle(true);
                paned.set_start_child(Some(&start_node.widget()));
                paned.set_end_child(Some(&end_node.widget()));
                // D-03: restore ratio after layout pass
                let saved_ratio = *ratio;
                let paned_ref = paned.clone();
                let orient = gtk_orientation;
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orient == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position((size as f64 * saved_ratio) as i32);
                    }
                });
                Some(SplitNode::Split {
                    orientation: gtk_orientation,
                    paned,
                    start: Box::new(start_node),
                    end: Box::new(end_node),
                })
            }
        }
    }

    /// Sync null surface pointers in the tree from GL_TO_SURFACE registry.
    /// Called after restore to wire surfaces that were created during GLArea realize.
    pub fn sync_surfaces_from_registry(&mut self) {
        Self::sync_surfaces_recursive(&mut self.root);
    }

    fn sync_surfaces_recursive(node: &mut SplitNode) {
        match node {
            SplitNode::Leaf { gl_area, surface, tabs, .. } => {
                if surface.is_null() {
                    *surface = crate::tabs::live_surface(gl_area, *surface).unwrap_or(std::ptr::null_mut());
                }
                for t in tabs.others.iter_mut().filter(|t| t.surface.is_null()) {
                    t.surface = crate::tabs::live_surface(&t.gl_area, t.surface).unwrap_or(std::ptr::null_mut());
                }
            }
            SplitNode::Preview { .. } => {} // No surface to sync
            SplitNode::Split { start, end, .. } => {
                Self::sync_surfaces_recursive(start);
                Self::sync_surfaces_recursive(end);
            }
        }
    }

    /// Returns the root widget of this workspace's split tree.
    pub fn root_widget(&self) -> gtk4::Widget {
        self.root.widget()
    }

    /// Grab GTK keyboard focus for the active pane's GLArea.
    /// Called after workspace switch so key events route to Ghostty, not the sidebar.
    pub fn grab_active_focus(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
    }

    /// Returns the UUID of the currently active pane, if found.
    pub fn active_pane_uuid(&self) -> Option<String> {
        self.root.find_uuid_for_pane(self.active_pane_id)
    }

    /// Grab GTK keyboard focus AND notify Ghostty of focus for the active pane.
    /// Use this after any operation that may have moved focus away from the terminal
    /// (sidebar toggle, workspace switch, etc.). grab_active_focus() only handles the
    /// GTK side; this method ensures Ghostty's internal focused state is also updated.
    pub fn focus_active_surface(&self) {
        if let Some(gl_area) = self.find_gl_area(self.active_pane_id) {
            gl_area.grab_focus();
        }
        // Call ghostty_surface_set_focus(true) on the active surface via registry lookup.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                for area_ptr in areas.iter() {
                    let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                        unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                    if area.has_css_class("active-pane") {
                        if let Some(&surface_ptr) = gl_to_surface.get(&(area_ptr.0 as usize)) {
                            unsafe {
                                crate::ghostty::ffi::ghostty_surface_set_focus(
                                    surface_ptr as ffi::ghostty_surface_t,
                                    true,
                                );
                            }
                        }
                        break;
                    }
                }
            }
        }
        // Kick the render loop to repaint after focus restore.
        if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if area.is_realized() {
                    area.queue_render();
                }
            }
        }
    }

    /// Split the active pane to the right (Ctrl+D per D-10).
    /// Replaces the active Leaf with a Split(Horizontal) containing the old leaf + new leaf.
    /// Per D-08: new surface inherits CWD via ghostty_surface_inherited_config.
    /// Per D-09: initial split ratio is 50/50 (set in paned.connect_realize).
    /// Per SPLIT-07: new pane receives focus immediately.
    pub fn split_right(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Horizontal)
    }

    /// Split the active pane downward (Ctrl+Shift+D per D-10).
    pub fn split_down(&mut self) -> Option<u64> {
        self.split_active(gtk4::Orientation::Vertical)
    }

    pub fn split_active(&mut self, orientation: gtk4::Orientation) -> Option<u64> {
        self.unzoom(); // pane-zoom: a new split shows the whole layout again
        self.front_active_tab();
        let active_id = self.active_pane_id;
        let new_pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        // When the root is a Leaf (first split), the GLArea is a direct child of the GtkStack
        // page. The replacer will remove it from the Stack (via remove_widget_from_parent) and
        // place it inside the new Paned. We then need to add the Paned to the Stack page.
        // Only capture this for Leaf roots — for nested splits the outer Paned stays in the Stack.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> =
            if matches!(self.root, SplitNode::Leaf { .. } | SplitNode::Preview { .. }) {
                old_root_widget
                    .parent()
                    .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                    .and_then(|stack| {
                        let name = stack.page(&old_root_widget).name()?.to_string();
                        Some((stack, name))
                    })
            } else {
                None
            };

        // Find the active leaf's surface for inherited config.
        let inherited_surface = self.find_surface(active_id)?;

        // Unfocus the old surface before the split — Ghostty routes input by focus state,
        // so without this the old pane continues receiving keystrokes after the new pane
        // is created (SPLIT-07).
        unsafe {
            ffi::ghostty_surface_set_focus(inherited_surface, false);
        }

        // Get inherited config from the active surface (for CWD inheritance per D-08).
        // Pass by value (ghostty_surface_config_s is Copy) — avoids dangling pointer
        // in the GLArea realize callback, which fires asynchronously after this returns.
        let new_uuid = Uuid::new_v4();
        let active_uuid = self.root.find_uuid_for_pane(active_id);
        let inherited_config = local_pane_config(
            Some((inherited_surface, active_uuid)),
            ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_SPLIT,
        );

        // Create new GLArea + surface for the new pane.
        let new_ids = crate::ghostty::surface::PaneIds { workspace: self.workspace_uuid, surface: new_uuid };
        eprintln!(
            "cmux: split_active calling create_surface for new_pane_id={}",
            new_pane_id
        );
        let (new_gl_area, _surface_cell) = crate::ghostty::surface::create_surface(
            &self.app,
            self.ghostty_app,
            Some(inherited_config),
            new_pane_id,
            crate::ghostty::surface::SurfaceIoMode::Exec,
            new_ids,
        );
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&new_gl_area);

        // Replace the active leaf in the tree with a Split node.
        let new_surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        let _ = new_surface_placeholder; // updated after realize via SURFACE_REGISTRY
        let new_leaf = new_leaf(new_pane_id, new_gl_area.clone(), new_uuid);

        let _replaced = self.replace_leaf_with_split(active_id, new_leaf, orientation)?;

        // If the root was a Leaf, it's now a Split whose Paned has no parent.
        // Re-parent the new Paned root into the GtkStack page we saved above.
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // After realize, update active focus to the new pane.
        self.focus_history.record(active_id, new_pane_id);
        self.active_pane_id = new_pane_id;
        self.root.update_focus_css(new_pane_id);

        // Focus the new GLArea widget so it receives keyboard events.
        new_gl_area.grab_focus();

        Some(new_pane_id)
    }

    /// Allocate the next available pane ID (used by browser preview pane creation).
    pub fn allocate_pane_id(&mut self) -> u64 {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }

    /// Split the active pane vertically and insert a Preview node on the right.
    /// Returns (new_pane_id, picture_widget, url_entry) so the caller can wire streaming and URL navigation.
    /// The active terminal pane stays on the left and retains focus.
    pub fn split_active_with_preview(&mut self) -> Option<crate::browser::PreviewPaneWidgets> {
        self.unzoom(); // pane-zoom
        self.front_active_tab();
        let active_id = self.active_pane_id;
        let new_pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        // Same stack re-parenting guard as split_active:
        // When root is a Leaf or Preview, capture the GtkStack parent so we can
        // re-parent the new Paned into the Stack after the split.
        let old_root_widget = self.root.widget();
        let stack_slot: Option<(gtk4::Stack, String)> =
            if matches!(self.root, SplitNode::Leaf { .. } | SplitNode::Preview { .. }) {
                old_root_widget
                    .parent()
                    .and_then(|p| p.downcast::<gtk4::Stack>().ok())
                    .and_then(|stack| {
                        let name = stack.page(&old_root_widget).name()?.to_string();
                        Some((stack, name))
                    })
            } else {
                None
            };

        // Create preview pane widgets
        let widgets = crate::browser::create_preview_pane(new_pane_id);

        // Phase 9: Attach right-click context menu to browser preview (D-09)
        {
            let menu_model = crate::menus::build_browser_context_menu();
            let popover = gtk4::PopoverMenu::from_model(Some(&menu_model));
            popover.set_parent(&widgets.container);
            popover.set_has_arrow(false);

            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right-click only
            gesture.connect_released({
                let popover = popover.clone();
                move |_, _, x, y| {
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
                        x as i32, y as i32, 1, 1,
                    )));
                    popover.popup();
                }
            });
            widgets.container.add_controller(gesture);
        }

        let preview_node = SplitNode::Preview {
            pane_id: new_pane_id,
            container: widgets.container.clone(),
            picture: widgets.picture.clone(),
            url_entry: widgets.url_entry.clone(),
            uuid: widgets.uuid,
            zoom_btn: widgets.zoom_btn.clone(),
        };

        // Replace active leaf with Split(active_leaf, preview_node) -- vertical, preview on right
        let _replaced = self.replace_leaf_with_split(
            active_id,
            preview_node,
            gtk4::Orientation::Horizontal, // Horizontal paned = side-by-side (left terminal, right preview)
        )?;

        // Re-parent new root Paned into GtkStack if root was a single Leaf
        if let Some((stack, name)) = stack_slot {
            let new_root = self.root.widget();
            stack.add_named(&new_root, Some(&name));
            stack.set_visible_child_name(&name);
        }

        // Keep focus on the original terminal pane (do NOT change active_pane_id)
        // The terminal keeps keyboard input, preview is passive display only.
        self.root.update_focus_css(active_id);

        Some(widgets)
    }

    /// Replace the leaf with `target_pane_id` with a Split(orientation) node.
    /// Returns Some(()) on success, None if the leaf was not found.
    fn replace_leaf_with_split(
        &mut self,
        target_pane_id: u64,
        new_leaf: SplitNode,
        orientation: gtk4::Orientation,
    ) -> Option<()> {
        let orientation_cap = orientation;
        let mut replacer = Some(|old_leaf: SplitNode| {
            let old_widget = old_leaf.widget();
            let new_widget = new_leaf.widget();

            // GTK4 requires a widget to have no parent before set_start/end_child.
            // old_widget may be parented to the Stack (first split) or an outer Paned (nested).
            remove_widget_from_parent(&old_widget);

            let paned = gtk4::Paned::new(orientation_cap);
            // Both children must be allowed to resize — GTK4 default for resize_end_child
            // is TRUE but be explicit to ensure drag works in both directions.
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            // Prevent children from collapsing to 0px when dragging to an extreme.
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            // Wide handle makes the divider grabable (default is ~5px, hard to click).
            paned.set_wide_handle(true);

            paned.set_start_child(Some(&old_widget));
            paned.set_end_child(Some(&new_widget));

            // Set 50/50 position after the first layout pass (per D-09 and RESEARCH Pitfall 2).
            // connect_realize fires before GTK allocates sizes, so p.width() is 0 there.
            // idle_add_local_once defers to the next main-loop idle, after layout completes.
            {
                let paned_ref = paned.clone();
                gtk4::glib::idle_add_local_once(move || {
                    let size = if orientation_cap == gtk4::Orientation::Horizontal {
                        paned_ref.width()
                    } else {
                        paned_ref.height()
                    };
                    if size > 0 {
                        paned_ref.set_position(size / 2);
                    }
                });
            }

            // Restore GTK focus AND Ghostty focus after GtkPaned drag ends (Gap 1A).
            //
            // The divider drag temporarily moves GTK focus to the separator handle,
            // which causes Ghostty's cursor blink and keyboard input to stop.
            //
            // IMPORTANT: We must NOT restore focus on every `notify::position` change
            // during the drag — that fires on every pixel of movement and causes:
            //   1. grab_focus() thrashing that fights the active gesture
            //   2. ghostty_surface_set_focus(true) message storms to the render thread
            //   3. GL_AREA_REGISTRY lock contention with the resize idle handler
            // Instead, we detect the drag lifecycle via the paned's internal GestureDrag
            // controller and restore focus only once when the drag ends.
            {
                // Find the GtkPaned's internal GestureDrag on its separator handle.
                // GTK4's Paned uses a GestureDrag controller internally — we observe
                // the controller list and connect to its drag-end signal.
                //
                // The gesture lives on the separator handle widget, not the Paned
                // itself. Walk the Paned's children to find the separator, then
                // inspect its controllers.
                let mut found_gesture = false;

                // First try: controllers on the Paned itself
                let controllers = paned.observe_controllers();
                let n = controllers.n_items();
                eprintln!("cmux: Paned has {} controllers", n);
                for i in 0..n {
                    if let Some(obj) = controllers.item(i) {
                        eprintln!("cmux:   controller[{}]: {}", i, obj.type_().name());
                        if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                            eprintln!("cmux:   -> found GestureDrag on Paned, connecting drag-end");
                            gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                eprintln!("cmux: GestureDrag drag-end fired on Paned — deferring focus restore to idle");
                                // Defer to idle so GTK has time to fully release the gesture
                                // and clean up the event sequence before we move focus.
                                gtk4::glib::idle_add_local_once(|| {
                                    eprintln!("cmux: drag-end idle: restoring focus now");
                                    restore_active_pane_focus();
                                });
                            });
                            found_gesture = true;
                            break;
                        }
                    }
                }

                // Second try: walk children to find the separator handle widget
                if !found_gesture {
                    let mut child = paned.first_child();
                    while let Some(ref widget) = child {
                        let type_name = widget.type_().name();
                        eprintln!("cmux: Paned child: {}", type_name);
                        let ctrl_list = widget.observe_controllers();
                        let cn = ctrl_list.n_items();
                        for i in 0..cn {
                            if let Some(obj) = ctrl_list.item(i) {
                                eprintln!(
                                    "cmux:   child controller[{}]: {}",
                                    i,
                                    obj.type_().name()
                                );
                                if let Ok(gesture) = obj.downcast::<gtk4::GestureDrag>() {
                                    eprintln!(
                                        "cmux:   -> found GestureDrag on {}, connecting drag-end",
                                        type_name
                                    );
                                    gesture.connect_drag_end(|_gesture, _offset_x, _offset_y| {
                                        eprintln!("cmux: GestureDrag drag-end fired on separator — deferring focus restore to idle");
                                        gtk4::glib::idle_add_local_once(|| {
                                            eprintln!("cmux: separator drag-end idle: restoring focus now");
                                            restore_active_pane_focus();
                                        });
                                    });
                                    found_gesture = true;
                                    break;
                                }
                            }
                        }
                        if found_gesture {
                            break;
                        }
                        child = widget.next_sibling();
                    }
                }

                if !found_gesture {
                    eprintln!("cmux: WARNING — no GestureDrag found on Paned or its children, falling back to notify::position");
                    // Fallback: use notify::position but debounced via idle.
                    // connect_notify requires Send+Sync, so use AtomicBool instead of Rc<Cell>.
                    let restore_pending =
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    paned.connect_notify(Some("position"), move |_paned, _pspec| {
                        if restore_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        let pending = restore_pending.clone();
                        gtk4::glib::idle_add_once(move || {
                            pending.store(false, std::sync::atomic::Ordering::SeqCst);
                            restore_active_pane_focus();
                        });
                    });
                }
            }

            SplitNode::Split {
                orientation: orientation_cap,
                paned: paned.clone(),
                start: Box::new(old_leaf),
                end: Box::new(new_leaf),
            }
        });
        replace_in_tree(&mut self.root, target_pane_id, &mut replacer)
    }

    /// Close the active pane (Ctrl+Shift+X per UI-SPEC).
    /// Removes the active leaf, replaces its parent Split with the surviving sibling.
    /// Returns the new active pane_id, or None if this was the last pane.
    pub fn close_active(&mut self) -> Option<u64> {
        let active_id = self.active_pane_id;

        // pane-zoom: closing a pane (or the zoomed pane's last tab) restores the layout.
        if self.tab_location(active_id).is_none_or(|(_, _, count)| count <= 1) {
            self.unzoom();
        }

        // tabs: a pane with several tabs loses only its active tab.
        if self.tab_location(active_id).is_some_and(|(_, _, count)| count > 1) {
            return self.close_tab(active_id).then_some(self.active_pane_id);
        }

        // Cannot close the last pane — workspace close is handled at AppState level.
        let is_single_pane = match &self.root {
            SplitNode::Leaf { pane_id, .. } if *pane_id == active_id => true,
            SplitNode::Preview { pane_id, .. } if *pane_id == active_id => true,
            _ => false,
        };
        if is_single_pane {
            return None; // Signal to AppState: close the workspace instead
        }

        // A browser preview has no Ghostty surface to free.
        if matches!(self.root.find_node(active_id), Some(SplitNode::Preview { .. })) {
            return self.close_preview(active_id);
        }

        // Don't close the last terminal pane if only a Preview would survive.
        // A Preview-only workspace has no terminal and the Ghostty surface free
        // can crash when no terminal remains to receive focus.
        let terminal_count = count_terminals(&self.root);
        let active_is_terminal = matches!(
            self.root.find_node(active_id),
            Some(SplitNode::Leaf { .. })
        );
        if active_is_terminal && terminal_count <= 1 {
            return None; // Prevent closing last terminal; close workspace instead
        }

        // Find the surface before removing it from the tree.
        let surface_to_free = self.find_surface(active_id)?;

        // Capture the raw GLArea pointer BEFORE the tree removal drops the GObject.
        // GL_AREA_REGISTRY holds raw pointers; once GTK finalizes the GObject the
        // pointer becomes dangling. Remove it here while the GLArea is still alive.
        let closed_gl_area = self.find_gl_area(active_id);
        let raw_gl_area: Option<*mut gtk4::ffi::GtkGLArea> = self
            .find_gl_area(active_id)
            .map(|a| a.as_ptr() as *mut gtk4::ffi::GtkGLArea);

        // Remove the leaf from the tree and get the surviving sibling's pane_id.
        let surviving_id = remove_leaf_from_tree(&mut self.root, active_id)?;

        // Remove the now-dropped GLArea from GL_AREA_REGISTRY before any further
        // callbacks can dereference the dangling pointer (Gap 2 fix).
        if let Some(raw_ptr) = raw_gl_area {
            if let Ok(mut areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                areas.retain(|p| p.0 != raw_ptr);
            }
            // Also remove from GL_TO_SURFACE mapping.
            if let Ok(mut gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                gl_to_surface.remove(&(raw_ptr as usize));
            }
        }

        // Update focus to the surviving pane.
        self.focus_history.forget(active_id);
        self.focus_history.forget(surviving_id);
        self.active_pane_id = surviving_id;
        self.root.update_focus_css(surviving_id);

        // Call ghostty_surface_set_focus on the surviving surface (SPLIT-07).
        if let Some(surface) = self.find_surface(surviving_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(surface, true);
            }
        }

        // Grab GTK focus on the surviving pane's widget (GLArea or URL entry).
        if let Some(gl_area) = self.find_gl_area(surviving_id) {
            gl_area.grab_focus();
        } else if let Some(entry) = find_url_entry_in_tree(&self.root, surviving_id) {
            entry.grab_focus();
        }

        // Deregister from SURFACE_REGISTRY and free the surface. This must come
        // after the focus move above: grab_focus emits focus-leave on the closed
        // pane's GLArea, whose handler calls ghostty_surface_set_focus on this
        // surface. Detach it from the GLArea first so any later event on the
        // closed pane is a no-op instead of a use-after-free.
        if let Some(area) = &closed_gl_area {
            crate::ghostty::surface::detach_surface(area);
        }
        unsafe {
            ffi::ghostty_surface_free(surface_to_free);
        }
        if let Ok(mut registry) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
            registry.remove(&(surface_to_free as usize));
        }
        // agent-status
        if let Ok(mut map) = crate::ghostty::callbacks::SURFACE_WORKSPACE.lock() {
            map.remove(&(surface_to_free as usize));
        }

        Some(surviving_id)
    }

    /// Remove the browser preview pane `pane_id`. If it was the active pane, the
    /// surviving sibling becomes active and takes focus. Returns the active pane.
    pub fn close_preview(&mut self, pane_id: u64) -> Option<u64> {
        if !matches!(self.root.find_node(pane_id), Some(SplitNode::Preview { .. })) {
            return None;
        }
        self.unzoom(); // pane-zoom
        let surviving_id = remove_leaf_from_tree(&mut self.root, pane_id)?;
        self.focus_history.forget(pane_id);
        if self.active_pane_id == pane_id {
            self.focus_history.forget(surviving_id);
            self.active_pane_id = surviving_id;
        }
        self.root.update_focus_css(self.active_pane_id);
        self.focus_active_surface();
        Some(self.active_pane_id)
    }

    /// Pane ids of every browser preview in this workspace.
    pub fn preview_pane_ids(&self) -> Vec<u64> {
        let mut ids = Vec::new();
        self.root.collect_pane_ids(&mut ids);
        ids.retain(|id| matches!(self.root.find_node(*id), Some(SplitNode::Preview { .. })));
        ids
    }

    /// Make the browser preview `pane_id` the active pane after a click inside it,
    /// without moving GTK focus: the click itself focuses the URL bar or the page.
    pub fn activate_preview(&mut self, pane_id: u64) -> bool {
        if pane_id == self.active_pane_id
            || !matches!(self.root.find_node(pane_id), Some(SplitNode::Preview { .. }))
        {
            return false;
        }
        self.focus_history.record(self.active_pane_id, pane_id);
        if let Some(old_surface) = self.find_surface(self.active_pane_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(old_surface, false);
            }
        }
        self.active_pane_id = pane_id;
        self.root.update_focus_css(pane_id);
        true
    }

    /// Navigate focus to the pane adjacent in `direction` (Ctrl+Alt+arrows per D-10).
    pub fn focus_next_in_direction(&mut self, direction: FocusDirection) -> bool {
        if let Some(new_id) = find_adjacent(&self.root, self.active_pane_id, direction) {
            self.set_active_pane(new_id);
            true
        } else {
            false
        }
    }

    /// Make the leaf whose GLArea is `gl_area_ptr` the active pane (click-to-focus).
    /// Looks the pane up by widget rather than pane_id, which is only unique
    /// within one workspace. Returns false if the GLArea is not in this engine.
    pub fn activate_gl_area(&mut self, gl_area_ptr: usize) -> bool {
        let Some(pane_id) = find_pane_for_gl_area(&self.root, gl_area_ptr) else {
            return false;
        };
        if pane_id != self.active_pane_id {
            self.set_active_pane(pane_id);
        }
        true
    }

    /// Move the active pane to `new_id`: Ghostty focus, the active-pane CSS class
    /// (which focus restoration after divider drags relies on) and GTK focus.
    fn set_active_pane(&mut self, new_id: u64) {
        // pane-zoom: like tmux, moving to another pane leaves zoom.
        if new_id != self.active_pane_id {
            self.unzoom();
        }
        self.focus_history.record(self.active_pane_id, new_id);
        // Unfocus old surface.
        if let Some(old_surface) = self.find_surface(self.active_pane_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(old_surface, false);
            }
        }
        self.active_pane_id = new_id;
        self.root.update_focus_css(new_id);
        // Focus new surface or Preview URL entry.
        if let Some(new_surface) = self.find_surface(new_id) {
            unsafe {
                ffi::ghostty_surface_set_focus(new_surface, true);
            }
        }
        if let Some(gl_area) = self.find_gl_area(new_id) {
            gl_area.grab_focus();
        } else if let Some(entry) = find_url_entry_in_tree(&self.root, new_id) {
            entry.grab_focus();
        }
    }

    // ── pane-nav ─────────────────────────────────────────────────────────────

    /// Make `pane_id` the active pane (socket pane.focus / surface.focus).
    /// Returns false if the pane is not in this engine. Re-focusing the already
    /// active pane only re-grabs GTK focus: toggling Ghostty focus off and on
    /// again would kill the cursor blink timer (see restore_active_pane_focus).
    pub fn focus_pane(&mut self, pane_id: u64) -> bool {
        // tabs: a terminal in a background tab: bring its tab to the front.
        let pane_id = match self.tab_location(pane_id) {
            Some((leaf_id, index, _)) if leaf_id != pane_id => {
                self.select_tab_in(leaf_id, index).unwrap_or(pane_id)
            }
            _ => pane_id,
        };
        if self.root.find_node(pane_id).is_none() {
            return false;
        }
        if pane_id == self.active_pane_id {
            self.grab_active_focus();
        } else {
            self.set_active_pane(pane_id);
        }
        true
    }

    /// Focus the `n`th (1-based) terminal pane in reading order.
    /// Returns the focused pane_id, or None if there is no such pane.
    pub fn focus_pane_number(&mut self, n: usize) -> Option<u64> {
        let target = pane_for_number(&self.root, n)?;
        self.focus_pane(target);
        Some(target)
    }

    /// Jump back to the previously focused pane that still exists.
    pub fn focus_last_pane(&mut self) -> Option<u64> {
        let root = &self.root;
        let target = self
            .focus_history
            .previous(|id| root.find_node(id).is_some())?;
        self.focus_pane(target);
        Some(target)
    }

    /// Move the divider of the nearest ancestor split along `direction`'s axis
    /// by `fraction` of that split's size (Right/Down move it right/down).
    /// Returns false when the active pane has no such ancestor.
    pub fn resize_active(&mut self, direction: FocusDirection, fraction: f64) -> bool {
        self.unzoom(); // pane-zoom: dividers are hidden while zoomed
        let horizontal = matches!(direction, FocusDirection::Left | FocusDirection::Right);
        let Some(path) = nearest_split_along(&self.root, self.active_pane_id, horizontal) else {
            return false;
        };
        let Some(SplitNode::Split { paned, .. }) = node_at_path(&self.root, &path) else {
            return false;
        };
        let size = if horizontal { paned.width() } else { paned.height() };
        if size <= 0 {
            return false;
        }
        let forward = matches!(direction, FocusDirection::Right | FocusDirection::Down);
        paned.set_position(resized_position(paned.position(), size, fraction, forward));
        schedule_resize_recovery();
        true
    }

    /// Set every divider so panes share space equally along each split's axis.
    pub fn equalize(&mut self) {
        if !matches!(self.root, SplitNode::Split { .. }) {
            return;
        }
        self.unzoom(); // pane-zoom
        equalize_node(&self.root);
        schedule_resize_recovery();
    }

    // ── pane-zoom ────────────────────────────────────────────────────────────

    /// The pane filling the workspace, if any. (tabs: the id of the tab that
    /// was in front when the pane was zoomed.)
    pub fn zoomed_pane(&self) -> Option<u64> {
        self.zoom.as_ref().map(|z| z.pane_id)
    }

    /// Enlarge `pane_id` to fill the workspace, making it the active pane.
    /// Returns false when there is nothing to hide (a single pane) or no such pane.
    pub fn zoom_pane(&mut self, pane_id: u64) -> bool {
        if self.zoom.as_ref().is_some_and(|z| z.pane_id == pane_id) {
            return true;
        }
        let Some(paths) = zoom_hidden_paths(&self.root, pane_id) else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        // Switching panes unzooms; the zoomed pane has to be active so input
        // never goes to a hidden one.
        if pane_id != self.active_pane_id {
            self.set_active_pane(pane_id);
        }
        self.unzoom();
        let hidden: Vec<gtk4::Widget> = paths
            .iter()
            .filter_map(|path| node_at_path(&self.root, path))
            .map(|node| node.widget())
            .collect();
        for widget in &hidden {
            widget.set_visible(false);
        }
        let Some(target) = self.root.find_node(pane_id) else {
            return false;
        };
        let marked = target.widget();
        marked.add_css_class("pane-zoomed");
        self.zoom = Some(PaneZoom { pane_id, hidden, marked });
        self.sync_zoom_buttons();
        schedule_resize_recovery();
        true
    }

    /// Show the whole layout again. Returns false when nothing was zoomed.
    pub fn unzoom(&mut self) -> bool {
        let Some(zoom) = self.zoom.take() else {
            return false;
        };
        for widget in &zoom.hidden {
            widget.set_visible(true);
        }
        zoom.marked.remove_css_class("pane-zoomed");
        self.sync_zoom_buttons();
        schedule_resize_recovery();
        true
    }

    /// Zoom (`Some(true)`), unzoom (`Some(false)`) or toggle (`None`) the
    /// active pane. Returns whether the workspace is zoomed afterwards.
    pub fn set_zoom(&mut self, on: Option<bool>) -> bool {
        let on = on.unwrap_or(self.zoom.is_none());
        if on {
            self.zoom_pane(self.active_pane_id);
        } else {
            self.unzoom();
        }
        self.zoom.is_some()
    }

    /// Point every browser preview's full-pane button at the current state.
    fn sync_zoom_buttons(&self) {
        let zoomed = self.zoom.is_some();
        let mut ids = Vec::new();
        self.root.collect_pane_ids(&mut ids);
        for id in ids {
            if let Some(SplitNode::Preview { zoom_btn, .. }) = self.root.find_node(id) {
                crate::browser::set_zoom_button_state(zoom_btn, zoomed);
            }
        }
    }

    /// The active pane's Ghostty surface and UUID, used as the source for
    /// working-directory inheritance when opening a new workspace.
    pub fn active_terminal_source(&self) -> Option<(ffi::ghostty_surface_t, Option<String>)> {
        if !matches!(self.root.find_node(self.active_pane_id), Some(SplitNode::Leaf { .. })) {
            return None;
        }
        let surface = self.find_surface(self.active_pane_id)?;
        Some((surface, self.root.find_uuid_for_pane(self.active_pane_id)))
    }

    /// Update the surface pointer for a pane after its GLArea realize callback fires.
    /// Called by Plan 04 wiring after SURFACE_REGISTRY is populated.
    pub fn update_surface(&mut self, pane_id: u64, surface: ffi::ghostty_surface_t) {
        update_surface_in_tree(&mut self.root, pane_id, surface);
    }

    /// Live Ghostty surface of a pane in THIS engine. The tree pointer is filled
    /// lazily after realize, so fall back to the pane's GLArea in GL_TO_SURFACE.
    /// (socket-cli: pane_id is only unique per workspace, so never resolve it via a
    /// global pane_id scan — that could hit another workspace's surface.)
    pub fn find_surface(&self, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
        find_surface_in_tree(&self.root, pane_id).or_else(|| {
            let key = self.find_gl_area(pane_id)?.as_ptr() as usize;
            let map = crate::ghostty::callbacks::GL_TO_SURFACE.lock().ok()?;
            map.get(&key).map(|&ptr| ptr as ffi::ghostty_surface_t)
        })
    }

    fn find_gl_area(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }

    /// Returns all leaf panes in this engine as (uuid, pane_id, active) tuples.
    pub fn all_panes(&self) -> Vec<(Uuid, u64, bool)> {
        let mut panes = Vec::new();
        self.root.collect_pane_info(&mut panes, self.active_pane_id);
        panes
    }

    /// Look up a surface by its UUID string. Returns the ghostty surface handle if found.
    pub fn find_surface_by_uuid(&self, target_uuid: &str) -> Option<ffi::ghostty_surface_t> {
        self.root.find_by_uuid(target_uuid)
    }

    /// Look up a pane_id by its UUID string.
    pub fn find_pane_id_by_uuid(&self, target_uuid: &str) -> Option<u64> {
        self.root.find_pane_id_by_uuid(target_uuid)
    }

    /// Look up a GLArea by pane_id (public wrapper for socket handlers).
    pub fn gl_area_for_pane(&self, pane_id: u64) -> Option<gtk4::GLArea> {
        find_gl_area_in_tree(&self.root, pane_id)
    }
}

// ── Tree traversal helpers ───────────────────────────────────────────────────

/// Replace the leaf with `target_id` using `replacer` function. Returns Some(()) if found.
fn replace_in_tree<F>(node: &mut SplitNode, target_id: u64, replacer: &mut Option<F>) -> Option<()>
where
    F: FnOnce(SplitNode) -> SplitNode,
{
    match node {
        SplitNode::Leaf { pane_id, .. } if *pane_id == target_id => {
            if let Some(r) = replacer.take() {
                // Take ownership of the old node to pass to replacer.
                let old = std::mem::replace(
                    node,
                    SplitNode::Leaf {
                        pane_id: 0,
                        gl_area: gtk4::GLArea::new(),
                        surface: std::ptr::null_mut(),
                        uuid: Uuid::new_v4(),
                        has_attention: false,
                        tabs: Default::default(),
                    },
                );
                *node = r(old);
                Some(())
            } else {
                None
            }
        }
        SplitNode::Leaf { .. } => None,
        SplitNode::Preview { .. } => None, // Cannot split a preview pane
        SplitNode::Split {
            start, end, paned, ..
        } => {
            if let Some(()) = replace_in_tree(start, target_id, replacer) {
                // Update paned start child to new widget.
                paned.set_start_child(Some(&start.widget()));
                Some(())
            } else if let Some(()) = replace_in_tree(end, target_id, replacer) {
                paned.set_end_child(Some(&end.widget()));
                Some(())
            } else {
                None
            }
        }
    }
}

/// Remove leaf `target_id` from the tree. Returns the surviving sibling's pane_id.
/// Replaces the parent Split with the surviving sibling in the GTK widget tree.
fn remove_leaf_from_tree(node: &mut SplitNode, target_id: u64) -> Option<u64> {
    match node {
        SplitNode::Leaf { .. } | SplitNode::Preview { .. } => None, // Caller ensures we never remove the root leaf
        SplitNode::Split {
            start, end, paned, ..
        } => {
            // Check if start is the target (Leaf or Preview).
            let start_is_target = match start.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id == target_id,
                _ => false,
            };
            if start_is_target {
                // Surviving sibling is end. Replace this Split with end in the GTK tree.
                let surviving = *end.clone();
                let surviving_widget = surviving.widget();
                // Find the paned's parent and replace it with the surviving widget.
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(
                        &parent,
                        &paned.clone().upcast(),
                        &surviving_widget,
                    );
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Check if end is the target (Leaf or Preview).
            let end_is_target = match end.as_ref() {
                SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id == target_id,
                _ => false,
            };
            if end_is_target {
                let surviving = *start.clone();
                let surviving_widget = surviving.widget();
                if let Some(parent) = paned.parent() {
                    replace_child_in_parent(
                        &parent,
                        &paned.clone().upcast(),
                        &surviving_widget,
                    );
                }
                let surviving_id = first_pane_id(&surviving);
                *node = surviving;
                return Some(surviving_id);
            }
            // Recurse into start subtree.
            if let Some(id) = remove_leaf_from_tree(start, target_id) {
                paned.set_start_child(Some(&start.widget()));
                return Some(id);
            }
            // Recurse into end subtree.
            if let Some(id) = remove_leaf_from_tree(end, target_id) {
                paned.set_end_child(Some(&end.widget()));
                return Some(id);
            }
            None
        }
    }
}

/// Replace `old_widget` with `new_widget` in `parent`. Handles GtkPaned children and GtkStack pages.
fn replace_child_in_parent(
    parent: &gtk4::Widget,
    old_widget: &gtk4::Widget,
    new_widget: &gtk4::Widget,
) {
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        let is_start = paned
            .start_child()
            .as_ref()
            .map(|w| w == old_widget)
            .unwrap_or(false);
        // new_widget is usually still a child of old_widget (the Paned being
        // collapsed); a widget can't have two parents, so without unparenting
        // first the swap fails and the closed pane stays on screen, frozen.
        remove_widget_from_parent(new_widget);
        if is_start {
            paned.set_start_child(Some(new_widget));
        } else {
            paned.set_end_child(Some(new_widget));
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        let page = stack.page(old_widget);
        if let Some(name) = page.name() {
            let name_str = name.to_string();
            stack.remove(old_widget);
            // new_widget may still be parented to the Paned we're replacing; unparent first.
            remove_widget_from_parent(new_widget);
            stack.add_named(new_widget, Some(&name_str));
            stack.set_visible_child_name(&name_str);
        } else {
            stack.remove(old_widget);
        }
    }
    // If parent is something else, the widget swap is a no-op (should not happen in Phase 2).
}

/// Return the first (leftmost/topmost) pane_id in a subtree.
fn first_pane_id(node: &SplitNode) -> u64 {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => *pane_id,
        SplitNode::Split { start, .. } => first_pane_id(start),
    }
}

fn find_surface_in_tree(node: &SplitNode, pane_id: u64) -> Option<ffi::ghostty_surface_t> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            surface,
            ..
        } if *id == pane_id => {
            if surface.is_null() {
                None
            } else {
                Some(*surface)
            }
        }
        SplitNode::Leaf { tabs, .. } => tabs
            .others
            .iter()
            .find(|t| t.pane_id == pane_id && !t.surface.is_null())
            .map(|t| t.surface),
        SplitNode::Preview { .. } => None, // No Ghostty surface
        SplitNode::Split { start, end, .. } => {
            find_surface_in_tree(start, pane_id).or_else(|| find_surface_in_tree(end, pane_id))
        }
    }
}

fn find_gl_area_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::GLArea> {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            gl_area,
            ..
        } if *id == pane_id => Some(gl_area.clone()),
        SplitNode::Leaf { tabs, .. } => tabs
            .others
            .iter()
            .find(|t| t.pane_id == pane_id)
            .map(|t| t.gl_area.clone()),
        SplitNode::Preview { .. } => None, // Preview uses Picture, not GLArea
        SplitNode::Split { start, end, .. } => {
            find_gl_area_in_tree(start, pane_id).or_else(|| find_gl_area_in_tree(end, pane_id))
        }
    }
}

fn find_pane_for_gl_area(node: &SplitNode, gl_area_ptr: usize) -> Option<u64> {
    match node {
        SplitNode::Leaf { pane_id, gl_area, .. }
            if gl_area.as_ptr() as usize == gl_area_ptr => Some(*pane_id),
        SplitNode::Leaf { .. } | SplitNode::Preview { .. } => None,
        SplitNode::Split { start, end, .. } => find_pane_for_gl_area(start, gl_area_ptr)
            .or_else(|| find_pane_for_gl_area(end, gl_area_ptr)),
    }
}

/// Count terminal (Leaf) panes in the tree.
fn count_terminals(node: &SplitNode) -> usize {
    match node {
        SplitNode::Leaf { .. } => 1,
        SplitNode::Preview { .. } => 0,
        SplitNode::Split { start, end, .. } => count_terminals(start) + count_terminals(end),
    }
}

fn find_url_entry_in_tree(node: &SplitNode, pane_id: u64) -> Option<gtk4::Entry> {
    match node {
        SplitNode::Preview { pane_id: id, url_entry, .. } if *id == pane_id => Some(url_entry.clone()),
        SplitNode::Preview { .. } | SplitNode::Leaf { .. } => None,
        SplitNode::Split { start, end, .. } => {
            find_url_entry_in_tree(start, pane_id).or_else(|| find_url_entry_in_tree(end, pane_id))
        }
    }
}

fn update_surface_in_tree(node: &mut SplitNode, pane_id: u64, surface: ffi::ghostty_surface_t) {
    match node {
        SplitNode::Leaf {
            pane_id: id,
            surface: s,
            ..
        } if *id == pane_id => *s = surface,
        SplitNode::Leaf { tabs, .. } => {
            if let Some(t) = tabs.others.iter_mut().find(|t| t.pane_id == pane_id) {
                t.surface = surface;
            }
        }
        SplitNode::Preview { .. } => {} // No surface to update
        SplitNode::Split { start, end, .. } => {
            update_surface_in_tree(start, pane_id, surface);
            update_surface_in_tree(end, pane_id, surface);
        }
    }
}

/// Find the pane adjacent to `active_id` in `direction`.
/// Strategy: collect ordered leaf positions and find the neighbor.
/// This is a directional approximation: Left/Up = previous leaf, Right/Down = next leaf.
/// A full spatial algorithm (comparing widget coordinates) can be added in a future phase.
fn find_adjacent(root: &SplitNode, active_id: u64, direction: FocusDirection) -> Option<u64> {
    let mut leaves = Vec::new();
    collect_leaves_in_order(root, &mut leaves);
    let pos = leaves.iter().position(|&id| id == active_id)?;
    match direction {
        FocusDirection::Left | FocusDirection::Up => {
            if pos > 0 {
                Some(leaves[pos - 1])
            } else {
                None
            }
        }
        FocusDirection::Right | FocusDirection::Down => {
            if pos + 1 < leaves.len() {
                Some(leaves[pos + 1])
            } else {
                None
            }
        }
    }
}

/// Remove `widget` from its current GTK parent so it can be reparented.
/// GTK4 requires `gtk_widget_get_parent(child) == NULL` before set_start/end_child.
fn remove_widget_from_parent(widget: &gtk4::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        if paned
            .start_child()
            .as_ref()
            .map(|w| w == widget)
            .unwrap_or(false)
        {
            paned.set_start_child(None::<&gtk4::Widget>);
        } else {
            paned.set_end_child(None::<&gtk4::Widget>);
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        stack.remove(widget);
    }
}

/// Restore GTK keyboard focus and Ghostty surface focus to the active pane.
/// Called once when a GtkPaned drag ends — NOT on every pixel of movement.
/// Re-syncs each surface's cached size with the GLArea's current allocation to
/// break any anti-flicker stall in Ghostty's drawFrame() guard, then kicks
/// the render thread with ghostty_surface_refresh + queue_render.
///
/// Does NOT touch focus state. The cursor blink timer runs independently of
/// resize. Calling set_focus(false→true) here kills the timer due to an async
/// cancel race in Ghostty's renderer thread: the false message enqueues a timer
/// cancel, but the true message is processed before the cancel callback fires,
/// so the guard `if cursor_c.state() != .active` sees `.active` and skips the
/// restart. The cancel then completes, leaving the timer permanently dead.
fn restore_active_pane_focus() {
    resync_surfaces(true);
}

/// Body of restore_active_pane_focus. With `grab_focus` false it only re-syncs
/// sizes and redraws, for programmatic divider moves that must not take focus.
fn resync_surfaces(grab_focus: bool) {
    // Re-set size + refresh ALL surfaces to break the anti-flicker stall.
    if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
            for area_ptr in areas.iter() {
                let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                    unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                if let Some(&surface_ptr) = gl_to_surface.get(&(area_ptr.0 as usize)) {
                    let scale = area.scale_factor();
                    let w = (area.width() * scale) as u32;
                    let h = (area.height() * scale) as u32;
                    if w > 0 && h > 0 {
                        unsafe {
                            let surface = surface_ptr as ffi::ghostty_surface_t;
                            ffi::ghostty_surface_set_size(surface, w, h);
                            ffi::ghostty_surface_refresh(surface);
                        }
                    }
                }
                if area.is_realized() {
                    area.queue_render();
                    area.queue_draw();
                }
            }
        }
    }

    // Drive the app tick to process any pending mailbox messages (redraw_surface, etc.)
    let app_ptr = crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
    if app_ptr != 0 {
        unsafe {
            let app = app_ptr as ffi::ghostty_app_t;
            ffi::ghostty_app_tick(app);
        }
    }

    // Restore GTK keyboard focus to the active pane.
    let areas = if grab_focus {
        crate::ghostty::callbacks::GL_AREA_REGISTRY.lock().ok()
    } else {
        None
    };
    if let Some(areas) = areas {
        for area_ptr in areas.iter() {
            let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
            if area.has_css_class("active-pane") {
                area.grab_focus();
                area.queue_render();
                break;
            }
        }
    }

    // The set_size → IO thread → render thread → updateFrame → cells rebuild pipeline
    // is asynchronous. The immediate queue_render above may still draw stale content
    // because cells haven't been rebuilt yet. Schedule follow-up recovery ticks to
    // give the pipeline time to converge (50ms, 150ms, 300ms).
    for delay_ms in [50u32, 150, 300] {
        gtk4::glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay_ms as u64),
            move || {
                let app_ptr =
                    crate::ghostty::callbacks::APP_PTR.load(std::sync::atomic::Ordering::SeqCst);
                if app_ptr != 0 {
                    unsafe {
                        ffi::ghostty_app_tick(app_ptr as ffi::ghostty_app_t);
                    }
                }
                if let Ok(areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
                    for area_ptr in areas.iter() {
                        let area: gtk4::glib::translate::Borrowed<gtk4::GLArea> =
                            unsafe { gtk4::glib::translate::from_glib_borrow(area_ptr.0) };
                        if area.is_realized() {
                            area.queue_render();
                        }
                    }
                }
            },
        );
    }
}

fn collect_leaves_in_order(node: &SplitNode, out: &mut Vec<u64>) {
    match node {
        SplitNode::Leaf { pane_id, .. } | SplitNode::Preview { pane_id, .. } => out.push(*pane_id),
        SplitNode::Split { start, end, .. } => {
            collect_leaves_in_order(start, out);
            collect_leaves_in_order(end, out);
        }
    }
}

/// Serde-friendly mirror of SplitNode for session persistence.
/// GTK widget references (GLArea, Paned) cannot be serialized — this parallel type holds
/// only the data needed to reconstruct the tree on restore.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SplitNodeData {
    Leaf {
        pane_id: u64,
        surface_uuid: Uuid,
        /// Shell executable path, e.g. "/bin/zsh" or "/bin/bash"
        shell: String,
        /// Absolute working directory path (best-effort; may be empty if /proc unavailable)
        cwd: String,
        /// tabs: every tab of the pane in tab order, for panes with 2+ tabs.
        /// `surface_uuid`/`cwd` above are the active tab's, so older cmux
        /// versions still restore that one. Absent in older session files.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tabs: Vec<crate::tabs::TabData>,
        /// tabs: index of the active tab in `tabs`.
        #[serde(default, skip_serializing_if = "is_zero")]
        active_tab: usize,
    },
    Split {
        /// "horizontal" or "vertical"
        orientation: String,
        /// Divider position as fraction 0.0-1.0 relative to parent size (D-03).
        #[serde(default = "default_ratio")]
        ratio: f64,
        start: Box<SplitNodeData>,
        end: Box<SplitNodeData>,
    },
}

fn default_ratio() -> f64 {
    0.5
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl SplitNode {
    /// Produce a serializable snapshot of this node's tree structure.
    /// `shell` and `cwd` are best-effort: Plan 05 fills these via /proc.
    /// Falls back to empty strings if /proc is unavailable or the pid is unknown.
    pub fn to_data(&self) -> SplitNodeData {
        match self {
            SplitNode::Leaf { pane_id, uuid, tabs, .. } => {
                // The cwd of this pane's own shell (found by its CMUX_SURFACE_ID),
                // not just any child of cmux: every pane used to save the same path.
                let cwd = tab_cwd(*uuid);
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                // tabs: list every tab when there is more than one.
                let tab_list = if tabs.others.is_empty() {
                    Vec::new()
                } else {
                    let mut list: Vec<crate::tabs::TabData> = tabs
                        .others
                        .iter()
                        .map(|t| crate::tabs::TabData {
                            surface_uuid: t.uuid,
                            cwd: tab_cwd(t.uuid),
                        })
                        .collect();
                    list.insert(tabs.active.min(list.len()), crate::tabs::TabData { surface_uuid: *uuid, cwd: tab_cwd(*uuid) });
                    list
                };
                SplitNodeData::Leaf {
                    pane_id: *pane_id,
                    surface_uuid: *uuid,
                    shell,
                    cwd,
                    active_tab: if tab_list.is_empty() { 0 } else { tabs.active },
                    tabs: tab_list,
                }
            },
            SplitNode::Preview { .. } => {
                // Preview panes are ephemeral; skip in session serialization.
                // Return a dummy leaf that will be ignored during restore.
                SplitNodeData::Leaf {
                    pane_id: 0,
                    surface_uuid: Uuid::nil(),
                    shell: String::new(),
                    cwd: String::new(),
                    tabs: Vec::new(),
                    active_tab: 0,
                }
            },
            SplitNode::Split { orientation, paned, start, end, .. } => {
                let total_size = if *orientation == gtk4::Orientation::Horizontal {
                    paned.width()
                } else {
                    paned.height()
                };
                let ratio = if total_size > 0 {
                    (paned.position() as f64) / (total_size as f64)
                } else {
                    0.5 // default if not yet laid out
                };
                SplitNodeData::Split {
                    orientation: match orientation {
                        gtk4::Orientation::Horizontal => "horizontal".to_string(),
                        gtk4::Orientation::Vertical => "vertical".to_string(),
                        _ => "horizontal".to_string(),
                    },
                    ratio,
                    start: Box::new(start.to_data()),
                    end: Box::new(end.to_data()),
                }
            },
        }
    }
}

// ── pane-nav: pure layout logic ──────────────────────────────────────────────
//
// Numbering, equalize weights, resize targets and focus history are kept free
// of GTK so they can be unit tested on SplitNodeData trees.

/// Which child of a split a tree path step descends into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Start,
    End,
}

/// One node of a pane tree, abstracted over SplitNode and SplitNodeData.
pub(crate) enum NodeShape<'a, T> {
    Leaf { pane_id: u64, terminal: bool },
    Split { horizontal: bool, start: &'a T, end: &'a T },
}

pub(crate) trait PaneTree: Sized {
    fn shape(&self) -> NodeShape<'_, Self>;
}

impl PaneTree for SplitNode {
    fn shape(&self) -> NodeShape<'_, Self> {
        match self {
            SplitNode::Leaf { pane_id, .. } => NodeShape::Leaf { pane_id: *pane_id, terminal: true },
            SplitNode::Preview { pane_id, .. } => NodeShape::Leaf { pane_id: *pane_id, terminal: false },
            SplitNode::Split { orientation, start, end, .. } => NodeShape::Split {
                horizontal: *orientation == gtk4::Orientation::Horizontal,
                start,
                end,
            },
        }
    }
}

impl PaneTree for SplitNodeData {
    fn shape(&self) -> NodeShape<'_, Self> {
        match self {
            // Preview panes are serialized as nil-UUID leaves (see to_data).
            SplitNodeData::Leaf { pane_id, surface_uuid, .. } => NodeShape::Leaf {
                pane_id: *pane_id,
                terminal: !surface_uuid.is_nil(),
            },
            SplitNodeData::Split { orientation, start, end, .. } => NodeShape::Split {
                horizontal: orientation != "vertical",
                start,
                end,
            },
        }
    }
}

/// Terminal panes in reading order: start (left/top) subtree before end.
pub(crate) fn terminal_panes_in_order<T: PaneTree>(node: &T) -> Vec<u64> {
    fn walk<T: PaneTree>(node: &T, out: &mut Vec<u64>) {
        match node.shape() {
            NodeShape::Leaf { pane_id, terminal } => {
                if terminal {
                    out.push(pane_id);
                }
            }
            NodeShape::Split { start, end, .. } => {
                walk(start, out);
                walk(end, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

/// The pane_id of the `n`th (1-based) terminal pane in reading order.
pub(crate) fn pane_for_number<T: PaneTree>(root: &T, n: usize) -> Option<u64> {
    if n == 0 {
        return None;
    }
    terminal_panes_in_order(root).get(n - 1).copied()
}

/// How many panes sit side by side along an axis (columns when `horizontal`,
/// rows otherwise). A same-axis split adds its children's spans; a cross-axis
/// split is as wide as its wider child.
pub(crate) fn span_along<T: PaneTree>(node: &T, horizontal: bool) -> usize {
    match node.shape() {
        NodeShape::Leaf { .. } => 1,
        NodeShape::Split { horizontal: h, start, end } => {
            let (a, b) = (span_along(start, horizontal), span_along(end, horizontal));
            if h == horizontal { a + b } else { a.max(b) }
        }
    }
}

/// Fraction of a split's space the start child gets when equalizing, so that
/// every pane along the split's axis ends up the same size.
pub(crate) fn equalize_fraction<T: PaneTree>(start: &T, end: &T, horizontal: bool) -> f64 {
    let a = span_along(start, horizontal) as f64;
    let b = span_along(end, horizontal) as f64;
    a / (a + b)
}

/// Path from `node` to the leaf `pane_id`, appended to `path`.
fn path_to_pane<T: PaneTree>(node: &T, pane_id: u64, path: &mut Vec<Side>) -> bool {
    match node.shape() {
        NodeShape::Leaf { pane_id: id, .. } => id == pane_id,
        NodeShape::Split { start, end, .. } => {
            path.push(Side::Start);
            if path_to_pane(start, pane_id, path) {
                return true;
            }
            path.pop();
            path.push(Side::End);
            if path_to_pane(end, pane_id, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

/// Path to the closest ancestor split of `pane_id` whose divider moves along
/// the requested axis (`horizontal` = left/right).
pub(crate) fn nearest_split_along<T: PaneTree>(root: &T, pane_id: u64, horizontal: bool) -> Option<Vec<Side>> {
    let mut path = Vec::new();
    if !path_to_pane(root, pane_id, &mut path) {
        return None;
    }
    for depth in (0..path.len()).rev() {
        let prefix = &path[..depth];
        if let Some(NodeShape::Split { horizontal: h, .. }) = node_at_path(root, prefix).map(|n| n.shape()) {
            if h == horizontal {
                return Some(prefix.to_vec());
            }
        }
    }
    None
}

/// Follow `path` from `root`.
pub(crate) fn node_at_path<'a, T: PaneTree>(root: &'a T, path: &[Side]) -> Option<&'a T> {
    let mut node = root;
    for side in path {
        match node.shape() {
            NodeShape::Split { start, end, .. } => {
                node = if *side == Side::Start { start } else { end };
            }
            NodeShape::Leaf { .. } => return None,
        }
    }
    Some(node)
}

/// pane-zoom: paths of the subtrees to hide so that `pane_id` fills the tree:
/// at every split on the way down, the child that does not lead to the pane.
/// Empty when `pane_id` is the root itself; None when it is not in the tree.
pub(crate) fn zoom_hidden_paths<T: PaneTree>(root: &T, pane_id: u64) -> Option<Vec<Vec<Side>>> {
    let mut path = Vec::new();
    if !path_to_pane(root, pane_id, &mut path) {
        return None;
    }
    Some(
        (0..path.len())
            .map(|depth| {
                let mut sibling = path[..depth].to_vec();
                sibling.push(match path[depth] {
                    Side::Start => Side::End,
                    Side::End => Side::Start,
                });
                sibling
            })
            .collect(),
    )
}

/// New divider position after moving it by `fraction` of `size` (at least one
/// pixel), forward (right/down) or backward, clamped to the split.
pub(crate) fn resized_position(position: i32, size: i32, fraction: f64, forward: bool) -> i32 {
    let delta = ((size as f64) * fraction).round().max(1.0) as i32;
    let moved = if forward { position + delta } else { position - delta };
    moved.clamp(0, size.max(0))
}

/// Previously active panes of one workspace, most recent last.
#[derive(Debug, Default, Clone)]
pub(crate) struct FocusHistory {
    stack: Vec<u64>,
}

const FOCUS_HISTORY_LEN: usize = 16;

impl FocusHistory {
    /// Record that focus moved from `old` to `new`. Each pane appears once,
    /// so repeated "focus previous" toggles between two panes.
    pub fn record(&mut self, old: u64, new: u64) {
        if old == new {
            return;
        }
        self.stack.retain(|&id| id != old && id != new);
        self.stack.push(old);
        if self.stack.len() > FOCUS_HISTORY_LEN {
            self.stack.remove(0);
        }
    }

    /// Drop a pane (closed, or now active) from the history.
    pub fn forget(&mut self, pane_id: u64) {
        self.stack.retain(|&id| id != pane_id);
    }

    /// tabs: a pane is known by its active tab's id; follow tab switches.
    pub fn rename(&mut self, old: u64, new: u64) {
        if old == new {
            return;
        }
        self.stack.retain(|&id| id != new);
        for id in self.stack.iter_mut().filter(|id| **id == old) {
            *id = new;
        }
    }

    /// The most recently active pane for which `exists` holds.
    pub fn previous(&self, exists: impl Fn(u64) -> bool) -> Option<u64> {
        self.stack.iter().rev().copied().find(|&id| exists(id))
    }
}

// ── pane-nav: GTK side of resize / equalize ─────────────────────────────────

/// Set every divider in `node` to its equal-share position.
///
/// Positions are computed against each Paned's *current* allocation, even for
/// nested Paneds about to be resized by their parent: when a Paned whose
/// position was set is re-allocated, GTK scales the position by
/// new/old size (both children resize), so the share is preserved.
fn equalize_node(node: &SplitNode) {
    let SplitNode::Split { orientation, paned, start, end } = node else {
        return;
    };
    let horizontal = *orientation == gtk4::Orientation::Horizontal;
    let (size, start_size, end_size) = if horizontal {
        (paned.width(), start.widget().width(), end.widget().width())
    } else {
        (paned.height(), start.widget().height(), end.widget().height())
    };
    // Space shared by the children: the Paned minus its handle.
    let avail = start_size + end_size;
    if size > 0 && avail > 0 {
        let fraction = equalize_fraction(start.as_ref(), end.as_ref(), horizontal);
        paned.set_position(((avail as f64) * fraction).round() as i32);
    }
    equalize_node(start);
    equalize_node(end);
}

/// After a programmatic divider move, re-sync surface sizes once layout has
/// settled: the recovery a mouse drag gets on drag-end, minus the focus grab
/// (pane.resize / pane.equalize must not move focus).
fn schedule_resize_recovery() {
    gtk4::glib::idle_add_local_once(|| resync_surfaces(false));
}

// ── pane-nav: working-directory inheritance ─────────────────────────────────

/// Environment variable carrying a pane's UUID into its shell (same name as
/// cmux on macOS). Used to find a pane's shell in /proc and read its cwd.
/// create_surface exports it (with the other CMUX_* vars, see
/// agent_status::pane_env_vars) from the pane's PaneIds.
const SURFACE_ID_ENV: &str = crate::agent_status::ENV_SURFACE_ID;

/// Leak a C string for a Ghostty surface config. The config is consumed at
/// GLArea realize time, after the caller returns, and Ghostty copies what it
/// keeps, so these few bytes per pane are never reclaimed.
fn leak_c_string(s: &str) -> *const std::os::raw::c_char {
    match std::ffi::CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null(),
    }
}

/// Surface config for a new local terminal pane.
/// When `source` (the pane it is opened from, plus its UUID) is given, the new
/// pane starts in the source's working directory: Ghostty's own value (learned
/// from OSC 7) if it names a local directory, else the cwd of the source
/// pane's shell read from /proc.
pub(crate) fn local_pane_config(
    source: Option<(ffi::ghostty_surface_t, Option<String>)>,
    context: ffi::ghostty_surface_context_e,
) -> ffi::ghostty_surface_config_s {
    let mut config = unsafe { ffi::ghostty_surface_config_new() };
    if let Some((surface, source_uuid)) = source.filter(|(s, _)| !s.is_null()) {
        let inherited = unsafe { ffi::ghostty_surface_inherited_config(surface, context) };
        if context == ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_SPLIT {
            // Splits keep the full inherited config (font size etc.), as before.
            config = inherited;
        }
        // An SSH pane (manual I/O) reports remote paths; never apply them locally.
        let remote = inherited.io_mode == ffi::ghostty_surface_io_mode_e_GHOSTTY_SURFACE_IO_MANUAL;
        let ghostty_wd = if remote || inherited.working_directory.is_null() {
            None
        } else {
            unsafe { std::ffi::CStr::from_ptr(inherited.working_directory) }
                .to_str()
                .ok()
                .map(str::to_string)
        };
        let wd = ghostty_wd
            .filter(|d| std::path::Path::new(d).is_dir())
            .or_else(|| source_uuid.as_deref().filter(|_| !remote).and_then(shell_cwd_for_pane));
        config.working_directory = wd.as_deref().map(leak_c_string).unwrap_or(std::ptr::null());
    }
    config.context = context;
    config
}

/// Field `index` after the command name in /proc/{pid}/stat
/// ("pid (comm) state ppid pgrp session tty_nr tpgid ..."; state is 0).
fn proc_stat_field(pid: u32, index: usize) -> Option<i64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rfind(')')?;
    stat.get(after_comm + 2..)?.split_whitespace().nth(index)?.parse().ok()
}

/// Direct children of `pid`, from /proc/{pid}/task/*/children (each thread
/// lists the children it forked).
fn child_pids(pid: u32) -> Vec<u32> {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|list| {
            list.split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

/// The CMUX_SURFACE_ID a process was started with, if any.
fn surface_id_of(pid: u32) -> Option<String> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = format!("{SURFACE_ID_ENV}=");
    env.split(|&b| b == 0)
        .find_map(|var| var.strip_prefix(prefix.as_bytes()))
        .map(|value| String::from_utf8_lossy(value).into_owned())
}

fn proc_cwd(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Working directory of the pane tagged `pane_uuid`. Finds the top-most
/// descendant of this process carrying CMUX_SURFACE_ID=<uuid> (Ghostty may
/// wrap the shell in `/bin/sh -c`), then, like kitty, prefers the terminal's
/// foreground process group: the interactive shell itself when it is idle.
fn shell_cwd_for_pane(pane_uuid: &str) -> Option<String> {
    // Breadth-first over our descendants only, not descending into other
    // panes' process trees. A pane's shell is a direct child (or a grandchild
    // behind Ghostty's `sh -c`), so this stops early.
    let mut queue: std::collections::VecDeque<u32> = child_pids(std::process::id()).into();
    while let Some(pid) = queue.pop_front() {
        match surface_id_of(pid) {
            Some(id) if id == pane_uuid => {
                let foreground = proc_stat_field(pid, 5)
                    .and_then(|tpgid| u32::try_from(tpgid).ok())
                    .filter(|&fg| fg > 0 && surface_id_of(fg).as_deref() == Some(pane_uuid));
                return foreground.and_then(proc_cwd).or_else(|| proc_cwd(pid));
            }
            Some(_) => {}
            None => queue.extend(child_pids(pid)),
        }
    }
    None
}

// ── tabs: terminal tabs inside a pane ────────────────────────────────────────
//
// See tabs.rs for the model: a Leaf's own fields are its active tab, its
// background tabs live in `tabs.others`. Tab positions below are indices in
// tab order (0-based, left to right).

/// One tab of a pane, for socket listings.
pub struct TabInfo {
    pub uuid: Uuid,
    pub index: usize,
    /// The pane's visible tab.
    pub active: bool,
    /// The pane is the workspace's active pane.
    pub pane_active: bool,
    pub has_attention: bool,
    pub title: String,
}

/// One terminal of a workspace, for surface.list.
pub struct SurfaceInfo {
    pub uuid: Uuid,
    pub pane_id: u64,
    /// None for a browser preview pane.
    pub gl_area: Option<gtk4::GLArea>,
    /// Visible tab of the workspace's active pane.
    pub active: bool,
    pub tab_index: usize,
    pub tab_count: usize,
}

/// A new single-tab leaf. With `[ui] always_show_tabs` it gets its tab bar
/// right away; otherwise the pane widget stays a bare GLArea.
fn new_leaf(pane_id: u64, gl_area: gtk4::GLArea, uuid: Uuid) -> SplitNode {
    let mut tabs = crate::tabs::PaneTabs::default();
    if crate::tabs::always_show_tabs() {
        ensure_tabs_ui(&gl_area, &mut tabs);
    }
    let leaf = SplitNode::Leaf {
        pane_id,
        gl_area,
        surface: std::ptr::null_mut(),
        uuid,
        has_attention: false,
        tabs,
    };
    leaf.refresh_tab_bar();
    leaf
}

/// Give a pane its tab bar + Stack, putting the container where the GLArea
/// was (a Paned side or a workspace Stack page) and the GLArea in the Stack.
fn ensure_tabs_ui(gl_area: &gtk4::GLArea, tabs: &mut crate::tabs::PaneTabs) -> crate::tabs::PaneTabsUi {
    if let Some(ui) = &tabs.ui {
        return ui.clone();
    }
    let ui = crate::tabs::PaneTabsUi::new();
    if let Some(parent) = gl_area.parent() {
        swap_child(&parent, gl_area.upcast_ref(), ui.container.upcast_ref());
    }
    ui.stack.add_child(gl_area);
    tabs.ui = Some(ui.clone());
    ui
}

/// Put `new` in `old`'s place. Unlike replace_child_in_parent this keeps the
/// workspace Stack's visible page (the pane may be in a background workspace).
fn swap_child(parent: &gtk4::Widget, old: &gtk4::Widget, new: &gtk4::Widget) {
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        // set_*_child unparents the old child.
        if paned.start_child().as_ref() == Some(old) {
            paned.set_start_child(Some(new));
        } else {
            paned.set_end_child(Some(new));
        }
    } else if let Some(stack) = parent.downcast_ref::<gtk4::Stack>() {
        let name = stack.page(old).name().map(|n| n.to_string());
        let visible = stack.visible_child_name().map(|n| n.to_string());
        stack.remove(old);
        stack.add_named(new, name.as_deref());
        if let Some(v) = visible {
            stack.set_visible_child_name(&v);
        }
    } else {
        eprintln!("cmux: tabs: unexpected pane parent {}", parent.type_().name());
        old.unparent();
    }
}

/// Free a closed background tab, in the same order as close_active: remove
/// the widget (its unrealize releases GL resources while the surface is
/// alive), drop it from the registries, detach, then free the surface.
fn free_tab(entry: crate::tabs::TabEntry, ui: Option<&crate::tabs::PaneTabsUi>) {
    let ptr = entry.gl_area.as_ptr() as usize;
    let surface = crate::tabs::live_surface(&entry.gl_area, entry.surface);
    if let Some(ui) = ui {
        ui.stack.remove(&entry.gl_area);
    }
    if let Ok(mut areas) = crate::ghostty::callbacks::GL_AREA_REGISTRY.lock() {
        areas.retain(|p| p.0 as usize != ptr);
    }
    if let Ok(mut gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
        gl_to_surface.remove(&ptr);
    }
    crate::ghostty::surface::detach_surface(&entry.gl_area);
    if let Some(surface) = surface {
        unsafe {
            ffi::ghostty_surface_free(surface);
        }
        if let Ok(mut registry) = crate::ghostty::callbacks::SURFACE_REGISTRY.lock() {
            registry.remove(&(surface as usize));
        }
        if let Ok(mut map) = crate::ghostty::callbacks::SURFACE_WORKSPACE.lock() {
            map.remove(&(surface as usize));
        }
    }
}

/// Working directory of a tab's shell, for the session (empty if unknown).
impl SplitEngine {
    /// Working directory of every terminal in this workspace (all tabs of all panes).
    pub fn terminal_cwds(&self) -> Vec<(Uuid, String)> {
        self.all_surfaces()
            .into_iter()
            .filter(|s| s.gl_area.is_some())
            .map(|s| (s.uuid, tab_cwd(s.uuid)))
            .collect()
    }
}

fn tab_cwd(uuid: Uuid) -> String {
    shell_cwd_for_pane(&uuid.to_string()).unwrap_or_default()
}

/// The leaf whose active tab is `leaf_id`.
fn leaf_mut(node: &mut SplitNode, leaf_id: u64) -> Option<&mut SplitNode> {
    if matches!(node, SplitNode::Leaf { pane_id, .. } if *pane_id == leaf_id) {
        return Some(node);
    }
    match node {
        SplitNode::Split { start, end, .. } => {
            if let Some(found) = leaf_mut(start, leaf_id) {
                return Some(found);
            }
            leaf_mut(end, leaf_id)
        }
        _ => None,
    }
}

fn leaf_ref(node: &SplitNode, leaf_id: u64) -> Option<&SplitNode> {
    match node.find_node(leaf_id)? {
        leaf @ SplitNode::Leaf { .. } => Some(leaf),
        _ => None,
    }
}

/// (leaf id, tab position, tab count) of terminal `id`.
fn tab_location_in(node: &SplitNode, id: u64) -> Option<(u64, usize, usize)> {
    match node {
        SplitNode::Leaf { pane_id, tabs, .. } => {
            if *pane_id == id {
                return Some((*pane_id, tabs.active, tabs.count()));
            }
            tabs.others
                .iter()
                .position(|t| t.pane_id == id)
                .map(|i| (*pane_id, tabs.order_index(i), tabs.count()))
        }
        SplitNode::Preview { .. } => None,
        SplitNode::Split { start, end, .. } => {
            tab_location_in(start, id).or_else(|| tab_location_in(end, id))
        }
    }
}

fn terminal_for_gl_area_in(node: &SplitNode, gl_ptr: usize) -> Option<u64> {
    match node {
        SplitNode::Leaf { pane_id, gl_area, tabs, .. } => {
            if gl_area.as_ptr() as usize == gl_ptr {
                Some(*pane_id)
            } else {
                tabs.others
                    .iter()
                    .find(|t| t.gl_area.as_ptr() as usize == gl_ptr)
                    .map(|t| t.pane_id)
            }
        }
        SplitNode::Preview { .. } => None,
        SplitNode::Split { start, end, .. } => terminal_for_gl_area_in(start, gl_ptr)
            .or_else(|| terminal_for_gl_area_in(end, gl_ptr)),
    }
}

/// Every terminal of a leaf in tab order: (pane_id, uuid, gl_area, attention, active).
fn leaf_tabs_in_order(node: &SplitNode) -> Vec<(u64, Uuid, gtk4::GLArea, bool, bool)> {
    let SplitNode::Leaf { pane_id, gl_area, uuid, has_attention, tabs, .. } = node else {
        return Vec::new();
    };
    let mut out: Vec<_> = tabs
        .others
        .iter()
        .map(|t| (t.pane_id, t.uuid, t.gl_area.clone(), t.has_attention, false))
        .collect();
    out.insert(tabs.active.min(out.len()), (*pane_id, *uuid, gl_area.clone(), *has_attention, true));
    out
}

fn refresh_tab_bars_in(node: &SplitNode) {
    match node {
        SplitNode::Leaf { .. } => node.refresh_tab_bar(),
        SplitNode::Preview { .. } => {}
        SplitNode::Split { start, end, .. } => {
            refresh_tab_bars_in(start);
            refresh_tab_bars_in(end);
        }
    }
}

impl SplitNode {
    /// tabs: re-render this leaf's tab bar (titles, active tab, attention).
    pub fn refresh_tab_bar(&self) {
        let SplitNode::Leaf { tabs, .. } = self else { return };
        let Some(ui) = &tabs.ui else { return };
        let views: Vec<crate::tabs::TabView> = leaf_tabs_in_order(self)
            .iter()
            .enumerate()
            .map(|(i, (_, uuid, area, attention, active))| crate::tabs::TabView {
                gl_ptr: area.as_ptr() as usize,
                uuid: *uuid,
                title: crate::tabs::title_for(area, uuid, i),
                custom: crate::tabs::custom_title(uuid).is_some(),
                active: *active,
                attention: *attention,
            })
            .collect();
        ui.render(&views);
    }
}

impl SplitEngine {
    /// (leaf id, tab position, tab count) of terminal `id`; the leaf id is the
    /// pane's active tab, so `leaf id != id` means `id` is a background tab.
    pub fn tab_location(&self, id: u64) -> Option<(u64, usize, usize)> {
        tab_location_in(&self.root, id)
    }

    /// tabs: socket handlers may set active_pane_id to any terminal, including
    /// a background tab; bring that tab to the front so tree operations (which
    /// match each pane by its visible tab) find it.
    fn front_active_tab(&mut self) {
        let id = self.active_pane_id;
        if let Some((leaf, index, _)) = self.tab_location(id).filter(|(leaf, _, _)| *leaf != id) {
            self.active_pane_id = leaf;
            self.select_tab_in(leaf, index);
        }
    }

    /// True if terminal `id` is a background tab of its pane.
    pub fn is_background_tab(&self, id: u64) -> bool {
        self.tab_location(id).is_some_and(|(leaf, _, _)| leaf != id)
    }

    /// The terminal (tab) whose GLArea is `gl_ptr`.
    pub fn terminal_for_gl_area(&self, gl_ptr: usize) -> Option<u64> {
        terminal_for_gl_area_in(&self.root, gl_ptr)
    }

    /// UUID of terminal `id` (active or background tab).
    pub fn find_uuid_for_pane(&self, id: u64) -> Option<String> {
        self.root.find_uuid_for_pane(id)
    }

    pub fn refresh_tab_bars(&self) {
        refresh_tab_bars_in(&self.root);
    }

    /// The tab bar of the pane holding terminal `tab_id` (created if the pane is
    /// still a bare GLArea) and that terminal's GLArea pointer, for editing its
    /// tab in place. None for a preview pane or an unknown id.
    pub fn tab_bar_for(&mut self, tab_id: u64) -> Option<(crate::tabs::PaneTabsUi, usize)> {
        let (leaf_id, _, _) = self.tab_location(tab_id)?;
        let leaf = leaf_mut(&mut self.root, leaf_id)?;
        let SplitNode::Leaf { pane_id, gl_area, tabs, .. } = leaf else { return None };
        let gl_ptr = if *pane_id == tab_id {
            gl_area.as_ptr() as usize
        } else {
            tabs.others.iter().find(|t| t.pane_id == tab_id)?.gl_area.as_ptr() as usize
        };
        let ui = ensure_tabs_ui(gl_area, tabs);
        leaf.refresh_tab_bar();
        Some((ui, gl_ptr))
    }

    /// Open a new tab right of the active tab of the active pane, in that
    /// terminal's working directory, and focus it. Returns its pane_id.
    pub fn new_tab(&mut self) -> Option<u64> {
        self.front_active_tab();
        let active_id = self.active_pane_id;
        if !matches!(self.root.find_node(active_id), Some(SplitNode::Leaf { .. })) {
            return None; // browser preview
        }
        // Same cwd inheritance as splits, but only the working directory
        // (TAB context), like a new workspace.
        let source = self.active_terminal_source();
        if let Some((surface, _)) = &source {
            unsafe {
                ffi::ghostty_surface_set_focus(*surface, false);
            }
        }
        let new_id = self.allocate_pane_id();
        let new_uuid = Uuid::new_v4();
        let config = local_pane_config(source, ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_TAB);
        // Each tab is its own surface: own CMUX_SURFACE_ID (pane_env_vars).
        let new_ids = crate::ghostty::surface::PaneIds { workspace: self.workspace_uuid, surface: new_uuid };
        let (new_area, _surface_cell) = crate::ghostty::surface::create_surface(
            &self.app,
            self.ghostty_app,
            Some(config),
            new_id,
            crate::ghostty::surface::SurfaceIoMode::Exec,
            new_ids,
        );
        attach_terminal_context_menu(&new_area);

        let old_area = {
            let Some(SplitNode::Leaf { pane_id, gl_area, surface, uuid, has_attention, tabs }) =
                leaf_mut(&mut self.root, active_id)
            else {
                return None;
            };
            let ui = ensure_tabs_ui(gl_area, tabs);
            ui.stack.add_child(&new_area);
            ui.stack.set_visible_child(&new_area);
            let outgoing = crate::tabs::TabEntry {
                pane_id: *pane_id,
                gl_area: gl_area.clone(),
                surface: *surface,
                uuid: *uuid,
                has_attention: *has_attention,
            };
            tabs.others.insert(tabs.active.min(tabs.others.len()), outgoing);
            tabs.active += 1;
            *pane_id = new_id;
            *surface = std::ptr::null_mut(); // set after realize (see find_surface)
            *uuid = new_uuid;
            *has_attention = false;
            std::mem::replace(gl_area, new_area.clone())
        };
        old_area.remove_css_class("active-pane");
        self.focus_history.rename(active_id, new_id);
        self.active_pane_id = new_id;
        self.root.update_focus_css(new_id);
        new_area.grab_focus();
        self.refresh_tab_bars();
        Some(new_id)
    }

    /// Bring tab `index` of the leaf `leaf_id` to the front. Moves Ghostty and
    /// GTK focus only if that leaf is the active pane. Returns the leaf's new
    /// id (the selected tab's pane_id).
    fn select_tab_in(&mut self, leaf_id: u64, index: usize) -> Option<u64> {
        let focus = leaf_id == self.active_pane_id;
        let old_surface = if focus { self.find_surface(leaf_id) } else { None };
        let (new_id, new_area, old_area) = {
            let Some(SplitNode::Leaf { pane_id, gl_area, surface, uuid, has_attention, tabs }) =
                leaf_mut(&mut self.root, leaf_id)
            else {
                return None;
            };
            if index >= tabs.count() {
                return None;
            }
            if index == tabs.active {
                return Some(leaf_id);
            }
            let outgoing = crate::tabs::TabEntry {
                pane_id: *pane_id,
                gl_area: gl_area.clone(),
                surface: *surface,
                uuid: *uuid,
                has_attention: *has_attention,
            };
            let incoming = crate::tabs::swap_in(&mut tabs.others, tabs.active, outgoing, index);
            tabs.active = index;
            *pane_id = incoming.pane_id;
            *surface = incoming.surface;
            *uuid = incoming.uuid;
            // Looking at a tab clears its attention marker.
            *has_attention = incoming.has_attention && !focus;
            let old_area = std::mem::replace(gl_area, incoming.gl_area);
            if let Some(ui) = &tabs.ui {
                ui.stack.set_visible_child(gl_area);
            }
            (*pane_id, gl_area.clone(), old_area)
        };
        old_area.remove_css_class("active-pane");
        self.focus_history.rename(leaf_id, new_id);
        if focus {
            if let Some(surface) = old_surface {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, false);
                }
            }
            self.active_pane_id = new_id;
            self.root.update_focus_css(new_id);
            if let Some(surface) = self.find_surface(new_id) {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, true);
                }
            }
            new_area.grab_focus();
        }
        self.refresh_tab_bars();
        Some(new_id)
    }

    /// Select tab `index` (0-based) of the active pane.
    pub fn select_tab(&mut self, index: usize) -> bool {
        match self.tab_location(self.active_pane_id) {
            Some((leaf, _, count)) if index < count => self.select_tab_in(leaf, index).is_some(),
            _ => false,
        }
    }

    /// Next/previous tab of the active pane, wrapping. False with one tab.
    pub fn cycle_tab(&mut self, forward: bool) -> bool {
        match self.tab_location(self.active_pane_id) {
            Some((leaf, index, count)) if count > 1 => self
                .select_tab_in(leaf, crate::tabs::cycle_index(index, count, forward))
                .is_some(),
            _ => false,
        }
    }

    /// Close terminal tab `tab_id` if its pane has other tabs (the pane's
    /// visible tab is replaced by its right, else left, neighbor). Does not
    /// move focus to another pane. False if it is its pane's only tab.
    pub fn close_tab(&mut self, tab_id: u64) -> bool {
        let Some((mut leaf_id, index, count)) = self.tab_location(tab_id) else {
            return false;
        };
        if count < 2 {
            return false;
        }
        if leaf_id == tab_id {
            let Some(next) = crate::tabs::neighbor_for_close(index, count) else {
                return false;
            };
            match self.select_tab_in(leaf_id, next) {
                Some(id) => leaf_id = id,
                None => return false,
            }
        }
        // tab_id is now a background tab of `leaf_id`.
        let (entry, ui) = {
            let Some(SplitNode::Leaf { tabs, .. }) = leaf_mut(&mut self.root, leaf_id) else {
                return false;
            };
            let Some(pos) = tabs.others.iter().position(|t| t.pane_id == tab_id) else {
                return false;
            };
            let order = tabs.order_index(pos);
            let Some(entry) = crate::tabs::remove_background(&mut tabs.others, &mut tabs.active, order) else {
                return false;
            };
            (entry, tabs.ui.clone())
        };
        self.focus_history.forget(tab_id);
        free_tab(entry, ui.as_ref());
        self.refresh_tab_bars();
        true
    }

    /// Move tab `tab_id` to position `to` within its pane.
    pub fn move_tab(&mut self, tab_id: u64, to: usize) -> bool {
        let Some((leaf_id, from, count)) = self.tab_location(tab_id) else {
            return false;
        };
        if to >= count {
            return false;
        }
        if let Some(SplitNode::Leaf { tabs, .. }) = leaf_mut(&mut self.root, leaf_id) {
            crate::tabs::move_in_order(&mut tabs.others, &mut tabs.active, from, to);
        }
        self.refresh_tab_bars();
        true
    }

    /// Every tab of the pane holding terminal `tab_id`, in tab order.
    pub fn pane_tabs(&self, tab_id: u64) -> Vec<TabInfo> {
        let Some((leaf_id, _, _)) = self.tab_location(tab_id) else {
            return Vec::new();
        };
        let Some(leaf) = leaf_ref(&self.root, leaf_id) else {
            return Vec::new();
        };
        leaf_tabs_in_order(leaf)
            .into_iter()
            .enumerate()
            .map(|(index, (_, uuid, area, has_attention, active))| TabInfo {
                uuid,
                index,
                active,
                pane_active: leaf_id == self.active_pane_id,
                has_attention,
                title: crate::tabs::title_for(&area, &uuid, index),
            })
            .collect()
    }

    /// Every terminal of this workspace (all tabs of all panes), in pane
    /// reading order then tab order.
    pub fn all_surfaces(&self) -> Vec<SurfaceInfo> {
        let mut leaves = Vec::new();
        collect_leaves_in_order(&self.root, &mut leaves);
        let mut out = Vec::new();
        for leaf_id in leaves {
            if let Some(SplitNode::Preview { uuid, .. }) = self.root.find_node(leaf_id) {
                out.push(SurfaceInfo {
                    uuid: *uuid,
                    pane_id: leaf_id,
                    gl_area: None,
                    active: leaf_id == self.active_pane_id,
                    tab_index: 0,
                    tab_count: 1,
                });
                continue;
            }
            let Some(leaf) = leaf_ref(&self.root, leaf_id) else { continue };
            let tabs = leaf_tabs_in_order(leaf);
            let count = tabs.len();
            for (index, (pane_id, uuid, gl_area, _, active)) in tabs.into_iter().enumerate() {
                out.push(SurfaceInfo {
                    uuid,
                    pane_id,
                    gl_area: Some(gl_area),
                    active: active && leaf_id == self.active_pane_id,
                    tab_index: index,
                    tab_count: count,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_node_data_leaf_has_surface_uuid() {
        // Build a minimal SplitNodeData::Leaf directly and verify surface_uuid field exists.
        let id = Uuid::new_v4();
        let data = SplitNodeData::Leaf {
            pane_id: 42,
            surface_uuid: id,
            shell: "/bin/bash".to_string(),
            cwd: "/home/user".to_string(),
            tabs: Vec::new(),
            active_tab: 0,
        };
        if let SplitNodeData::Leaf { surface_uuid, pane_id, .. } = data {
            assert_eq!(surface_uuid, id);
            assert_eq!(pane_id, 42);
        } else {
            panic!("Expected SplitNodeData::Leaf");
        }
    }

    #[test]
    fn split_node_data_roundtrip_json() {
        // Verify SplitNodeData serializes and deserializes via serde_json.
        let leaf = SplitNodeData::Leaf {
            pane_id: 1,
            surface_uuid: Uuid::new_v4(),
            shell: "/bin/zsh".to_string(),
            cwd: "/tmp".to_string(),
            tabs: Vec::new(),
            active_tab: 0,
        };
        let json = serde_json::to_string(&leaf).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let (
            SplitNodeData::Leaf { pane_id: p1, surface_uuid: u1, .. },
            SplitNodeData::Leaf { pane_id: p2, surface_uuid: u2, .. },
        ) = (&leaf, &restored)
        {
            assert_eq!(p1, p2);
            assert_eq!(u1, u2);
        } else {
            panic!("Roundtrip changed variant");
        }
    }

    #[test]
    fn split_node_data_split_roundtrip_json() {
        // Verify nested SplitNodeData serializes correctly with ratio field.
        let split = SplitNodeData::Split {
            orientation: "horizontal".to_string(),
            ratio: 0.35,
            start: Box::new(SplitNodeData::Leaf {
                pane_id: 1,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
                tabs: Vec::new(),
                active_tab: 0,
            }),
            end: Box::new(SplitNodeData::Leaf {
                pane_id: 2,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
                tabs: Vec::new(),
                active_tab: 0,
            }),
        };
        let json = serde_json::to_string(&split).expect("serialize failed");
        let restored: SplitNodeData = serde_json::from_str(&json).expect("deserialize failed");
        if let SplitNodeData::Split { orientation, ratio, .. } = restored {
            assert_eq!(orientation, "horizontal");
            assert!((ratio - 0.35).abs() < f64::EPSILON, "ratio not preserved in roundtrip");
        } else {
            panic!("Roundtrip changed variant to non-Split");
        }

        // Verify v1-compat: Split without ratio field deserializes with default 0.5
        let v1_json = r#"{"type":"Split","orientation":"vertical","start":{"type":"Leaf","pane_id":1,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""},"end":{"type":"Leaf","pane_id":2,"surface_uuid":"00000000-0000-0000-0000-000000000000","shell":"","cwd":""}}"#;
        let v1_restored: SplitNodeData = serde_json::from_str(v1_json).expect("v1 deserialize failed");
        if let SplitNodeData::Split { ratio, .. } = v1_restored {
            assert!((ratio - 0.5).abs() < f64::EPSILON, "v1 missing ratio should default to 0.5");
        } else {
            panic!("v1 deserialize changed variant");
        }
    }

    // ── pane-nav ────────────────────────────────────────────────────────────

    fn leaf(id: u64) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id: id,
            surface_uuid: Uuid::new_v4(),
            shell: String::new(),
            cwd: String::new(),
            tabs: Vec::new(),
            active_tab: 0,
        }
    }

    fn preview(id: u64) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id: id,
            surface_uuid: Uuid::nil(),
            shell: String::new(),
            cwd: String::new(),
            tabs: Vec::new(),
            active_tab: 0,
        }
    }

    fn node(orientation: &str, start: SplitNodeData, end: SplitNodeData) -> SplitNodeData {
        SplitNodeData::Split {
            orientation: orientation.to_string(),
            ratio: 0.5,
            start: Box::new(start),
            end: Box::new(end),
        }
    }

    /// ┌───┬───┐
    /// │ 1 │ 2 │
    /// │   ├───┤
    /// │   │ 3 │
    /// ├───┴───┤
    /// │   4   │
    /// └───────┘
    fn sample_tree() -> SplitNodeData {
        node(
            "vertical",
            node("horizontal", leaf(1), node("vertical", leaf(2), leaf(3))),
            leaf(4),
        )
    }

    #[test]
    fn pane_numbers_follow_reading_order() {
        let tree = sample_tree();
        assert_eq!(terminal_panes_in_order(&tree), vec![1, 2, 3, 4]);
        assert_eq!(pane_for_number(&tree, 1), Some(1));
        assert_eq!(pane_for_number(&tree, 3), Some(3));
        assert_eq!(pane_for_number(&tree, 4), Some(4));
        assert_eq!(pane_for_number(&tree, 5), None);
        assert_eq!(pane_for_number(&tree, 0), None);
        // A single pane is pane 1.
        assert_eq!(pane_for_number(&leaf(7), 1), Some(7));
    }

    #[test]
    fn pane_numbers_skip_browser_previews() {
        let tree = node("horizontal", leaf(1), node("horizontal", preview(2), leaf(3)));
        assert_eq!(terminal_panes_in_order(&tree), vec![1, 3]);
        assert_eq!(pane_for_number(&tree, 2), Some(3));
    }

    #[test]
    fn equalize_weights_count_panes_along_the_axis() {
        // Three columns made of nested horizontal splits: 1/3 then 1/2.
        let (a, b) = (leaf(1), node("horizontal", leaf(2), leaf(3)));
        assert!((equalize_fraction(&a, &b, true) - 1.0 / 3.0).abs() < 1e-9);
        // A cross-axis subtree counts as one column, however many rows it has.
        let (a, b) = (leaf(1), node("vertical", leaf(2), leaf(3)));
        assert!((equalize_fraction(&a, &b, true) - 0.5).abs() < 1e-9);
        // Two columns on each side of the root split: 50/50.
        let a = node("horizontal", leaf(1), leaf(2));
        let b = node("vertical", node("horizontal", leaf(3), leaf(4)), leaf(5));
        assert_eq!(span_along(&b, true), 2);
        assert_eq!(span_along(&b, false), 2);
        assert!((equalize_fraction(&a, &b, true) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn resize_targets_nearest_split_on_the_axis() {
        let tree = sample_tree();
        // Pane 3: nearest up/down split is its parent [Start, End];
        // nearest left/right split is [Start].
        assert_eq!(
            nearest_split_along(&tree, 3, false),
            Some(vec![Side::Start, Side::End])
        );
        assert_eq!(nearest_split_along(&tree, 3, true), Some(vec![Side::Start]));
        // Pane 4 has no left/right divider; up/down moves the root divider.
        assert_eq!(nearest_split_along(&tree, 4, true), None);
        assert_eq!(nearest_split_along(&tree, 4, false), Some(vec![]));
        // Unknown pane.
        assert_eq!(nearest_split_along(&tree, 99, true), None);
        // node_at_path resolves the returned path to that split.
        let found = node_at_path(&tree, &[Side::Start, Side::End]).unwrap();
        assert!(matches!(found, SplitNodeData::Split { orientation, .. } if orientation == "vertical"));
    }

    // pane-zoom
    #[test]
    fn zoom_hides_the_sibling_at_every_split_on_the_path() {
        use Side::{End, Start};
        let tree = sample_tree();
        // Pane 3 = root.Start.End.End: hide pane 4, pane 1 and pane 2.
        assert_eq!(
            zoom_hidden_paths(&tree, 3),
            Some(vec![vec![End], vec![Start, Start], vec![Start, End, Start]])
        );
        // Pane 4 sits right under the root: only the other half is hidden.
        assert_eq!(zoom_hidden_paths(&tree, 4), Some(vec![vec![Start]]));
        let hidden: Vec<u64> = zoom_hidden_paths(&tree, 1)
            .unwrap()
            .iter()
            .flat_map(|p| terminal_panes_in_order(node_at_path(&tree, p).unwrap()))
            .collect();
        assert_eq!(hidden, vec![4, 2, 3]);
        // A lone pane has nothing to hide; unknown panes cannot be zoomed.
        assert_eq!(zoom_hidden_paths(&leaf(7), 7), Some(vec![]));
        assert_eq!(zoom_hidden_paths(&tree, 99), None);
        // Browser previews zoom like terminals.
        let with_preview = node("horizontal", leaf(1), preview(2));
        assert_eq!(zoom_hidden_paths(&with_preview, 2), Some(vec![vec![Start]]));
    }

    #[test]
    fn resized_position_moves_and_clamps() {
        assert_eq!(resized_position(500, 1000, 0.05, true), 550);
        assert_eq!(resized_position(500, 1000, 0.05, false), 450);
        assert_eq!(resized_position(990, 1000, 0.05, true), 1000);
        assert_eq!(resized_position(10, 1000, 0.05, false), 0);
        // Tiny fractions still move by at least one pixel.
        assert_eq!(resized_position(10, 100, 0.001, true), 11);
    }

    #[test]
    fn focus_history_toggles_and_forgets() {
        let mut h = FocusHistory::default();
        let all = |_: u64| true;
        assert_eq!(h.previous(all), None);
        h.record(1, 2); // 1 -> 2
        assert_eq!(h.previous(all), Some(1));
        h.record(2, 1); // "focus previous" back to 1
        assert_eq!(h.previous(all), Some(2));
        h.record(1, 3); // 1 -> 3
        assert_eq!(h.previous(all), Some(1));
        h.record(3, 3); // no-op
        assert_eq!(h.previous(all), Some(1));
        // Panes that no longer exist are skipped, and can be forgotten.
        assert_eq!(h.previous(|id| id != 1), Some(2));
        h.forget(1);
        assert_eq!(h.previous(all), Some(2));
    }

    // ── tabs ────────────────────────────────────────────────────────────────

    #[test]
    fn tabs_session_round_trip() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let tab = |u: Uuid, cwd: &str| crate::tabs::TabData { surface_uuid: u, cwd: cwd.to_string() };
        let data = SplitNodeData::Leaf {
            pane_id: 1,
            surface_uuid: b,
            shell: "/bin/sh".to_string(),
            cwd: "/tmp".to_string(),
            tabs: vec![tab(a, "/"), tab(b, "/tmp"), tab(c, "/home/한글")],
            active_tab: 1,
        };
        let json = serde_json::to_string(&data).unwrap();
        let back: SplitNodeData = serde_json::from_str(&json).unwrap();
        let SplitNodeData::Leaf { surface_uuid, tabs, active_tab, .. } = back else {
            panic!("variant changed");
        };
        assert_eq!(surface_uuid, b);
        assert_eq!(active_tab, 1);
        assert_eq!(tabs, vec![tab(a, "/"), tab(b, "/tmp"), tab(c, "/home/한글")]);
    }

    #[test]
    fn single_tab_panes_serialize_as_before() {
        // No tab list / active index in the JSON of a single-tab pane, so files
        // stay readable (and identical) for cmux builds without tabs.
        let json = serde_json::to_value(leaf(3)).unwrap();
        assert!(json.get("tabs").is_none());
        assert!(json.get("active_tab").is_none());
    }

    #[test]
    fn sessions_without_tabs_still_load() {
        // A session.json leaf written before tabs existed.
        let old = r#"{"type":"Split","orientation":"horizontal","ratio":0.5,
            "start":{"type":"Leaf","pane_id":1,"surface_uuid":"11111111-1111-1111-1111-111111111111","shell":"/bin/zsh","cwd":"/tmp"},
            "end":{"type":"Leaf","pane_id":2,"surface_uuid":"22222222-2222-2222-2222-222222222222","shell":"/bin/zsh","cwd":""}}"#;
        let data: SplitNodeData = serde_json::from_str(old).unwrap();
        let SplitNodeData::Split { start, .. } = data else { panic!("not a split") };
        let SplitNodeData::Leaf { tabs, active_tab, cwd, .. } = *start else { panic!("not a leaf") };
        assert!(tabs.is_empty());
        assert_eq!(active_tab, 0);
        assert_eq!(cwd, "/tmp");
        // A tab entry without cwd loads too.
        let t: crate::tabs::TabData =
            serde_json::from_str(r#"{"surface_uuid":"11111111-1111-1111-1111-111111111111"}"#).unwrap();
        assert!(t.cwd.is_empty());
    }

    #[test]
    fn focus_history_follows_tab_switches() {
        let mut h = FocusHistory::default();
        h.record(1, 2); // pane 1 -> pane 2
        // Pane 1 switches its visible tab from terminal 1 to terminal 7.
        h.rename(1, 7);
        assert_eq!(h.previous(|_| true), Some(7));
        h.rename(5, 9); // unknown id: no-op
        assert_eq!(h.previous(|_| true), Some(7));
    }

    #[test]
    fn focus_history_is_bounded() {
        let mut h = FocusHistory::default();
        for id in 0..100u64 {
            h.record(id, id + 1);
        }
        assert_eq!(h.previous(|_| true), Some(99));
        assert!(h.stack.len() <= FOCUS_HISTORY_LEN);
        assert!(!h.stack.contains(&0));
    }
}
