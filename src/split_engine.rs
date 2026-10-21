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
    },
    /// Phase 8: Browser preview pane (agent-browser frame rendering).
    Preview {
        pane_id: u64,
        container: gtk4::Box,
        picture: gtk4::Picture,
        #[allow(dead_code)] // Kept alive for GTK widget tree reference counting
        url_entry: gtk4::Entry,
        uuid: Uuid,
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
            SplitNode::Leaf { gl_area, .. } => gl_area.clone().upcast(),
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
            SplitNode::Leaf { gl_area, .. } => out.push(gl_area.clone()),
            SplitNode::Preview { .. } => {}
            SplitNode::Split { start, end, .. } => {
                start.collect_gl_areas(out);
                end.collect_gl_areas(out);
            }
        }
    }

    pub fn collect_surfaces(&self, out: &mut Vec<ffi::ghostty_surface_t>) {
        match self {
            SplitNode::Leaf { surface, .. } => out.push(*surface),
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
            SplitNode::Leaf { pane_id, surface, .. } => {
                if *pane_id == target_id { Some(*surface) } else { None }
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
            SplitNode::Leaf { uuid, surface, .. } => {
                if uuid.to_string() == target_uuid { Some(*surface) } else { None }
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
            SplitNode::Leaf { pane_id, has_attention, .. } => {
                if *pane_id == target_pane_id {
                    *has_attention = value;
                    true
                } else {
                    false
                }
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
            SplitNode::Leaf { has_attention, .. } => *has_attention,
            SplitNode::Preview { .. } => false,
            SplitNode::Split { start, end, .. } => start.any_attention() || end.any_attention(),
        }
    }

    /// Check if a specific pane has attention.
    pub fn pane_has_attention(&self, target_pane_id: u64) -> bool {
        match self {
            SplitNode::Leaf { pane_id, has_attention, .. } => {
                *pane_id == target_pane_id && *has_attention
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
            SplitNode::Leaf { has_attention, .. } => *has_attention = false,
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
    ) -> Self {
        // The initial surface may not be realized yet. SplitEngine stores the cell
        // so it can read the surface pointer after realize. For focus/split operations
        // that run after realize, we read from the cell.
        // For the tree structure, we store null initially and update after realize.
        let surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&initial_gl_area);
        let root = SplitNode::Leaf {
            pane_id,
            gl_area: initial_gl_area,
            surface: surface_placeholder,
            uuid: Uuid::new_v4(),
            has_attention: false,
        };
        SplitEngine {
            root,
            active_pane_id: pane_id,
            next_pane_id: pane_id + 1,
            app,
            ghostty_app,
            focus_history: FocusHistory::default(),
        }
    }

    /// Replace the UUID of the initial (root) leaf. Used when the caller already
    /// tagged the pane's surface with a UUID (see `local_pane_config`).
    pub fn with_root_uuid(mut self, new_uuid: Uuid) -> Self {
        if let SplitNode::Leaf { uuid, .. } = &mut self.root {
            *uuid = new_uuid;
        }
        self
    }

    /// Update the surface pointer for a leaf after realize.
    /// Recursively searches the tree (D-10: works for ALL leaves, not just root).
    pub fn set_initial_surface(&mut self, pane_id: u64, surface: ffi::ghostty_surface_t) {
        Self::set_surface_recursive(&mut self.root, pane_id, surface);
    }

    fn set_surface_recursive(node: &mut SplitNode, target_pane_id: u64, surface: ffi::ghostty_surface_t) {
        match node {
            SplitNode::Leaf { pane_id, surface: s, .. } => {
                if *pane_id == target_pane_id {
                    *s = surface;
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
    ) -> Option<Self> {
        let mut next_pane_id: u64 = 1;
        let root = Self::node_from_data(&app, ghostty_app, data, &mut next_pane_id, 0)?;
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
        })
    }

    fn node_from_data(
        app: &gtk4::Application,
        ghostty_app: ffi::ghostty_app_t,
        data: &SplitNodeData,
        next_pane_id: &mut u64,
        depth: u32,
    ) -> Option<SplitNode> {
        if depth > 16 {
            eprintln!("cmux: session restore tree depth > 16, falling back (D-14)");
            return None;
        }
        match data {
            SplitNodeData::Leaf { surface_uuid, .. } => {
                let pane_id = *next_pane_id;
                *next_pane_id += 1;
                // D-06: preserve UUID from session. Preview panes are saved as
                // nil-UUID leaves; give those a fresh one so restored panes never
                // share an identity (or a CMUX_SURFACE_ID).
                let uuid = if surface_uuid.is_nil() { Uuid::new_v4() } else { *surface_uuid };
                // Create surface — realize callback will create Ghostty surface and wire registries
                let config = local_pane_config(None, ffi::ghostty_surface_context_e_GHOSTTY_SURFACE_CONTEXT_WINDOW, uuid);
                let (gl_area, _surface_cell) =
                    crate::ghostty::surface::create_surface(app, ghostty_app, Some(config), pane_id, crate::ghostty::surface::SurfaceIoMode::Exec);
                // Phase 9: Attach right-click context menu (D-08)
                attach_terminal_context_menu(&gl_area);
                let surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
                Some(SplitNode::Leaf {
                    pane_id,
                    gl_area,
                    surface: surface_placeholder,
                    uuid,
                    has_attention: false,
                })
            }
            SplitNodeData::Split { orientation, ratio, start, end } => {
                let start_node = Self::node_from_data(app, ghostty_app, start, next_pane_id, depth + 1)?;
                let end_node = Self::node_from_data(app, ghostty_app, end, next_pane_id, depth + 1)?;
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
            SplitNode::Leaf { gl_area, surface, .. } => {
                if surface.is_null() {
                    if let Ok(gl_to_surface) = crate::ghostty::callbacks::GL_TO_SURFACE.lock() {
                        if let Some(&s) = gl_to_surface.get(&(gl_area.as_ptr() as usize)) {
                            *surface = s as ffi::ghostty_surface_t;
                        }
                    }
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
            new_uuid,
        );

        // Create new GLArea + surface for the new pane.
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
        );
        // Phase 9: Attach right-click context menu (D-08)
        attach_terminal_context_menu(&new_gl_area);

        // Replace the active leaf in the tree with a Split node.
        let new_surface_placeholder: ffi::ghostty_surface_t = std::ptr::null_mut();
        let new_leaf = SplitNode::Leaf {
            pane_id: new_pane_id,
            gl_area: new_gl_area.clone(),
            surface: new_surface_placeholder, // updated after realize via SURFACE_REGISTRY
            uuid: new_uuid,
            has_attention: false,
        };

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

        // Cannot close the last pane — workspace close is handled at AppState level.
        let is_single_pane = match &self.root {
            SplitNode::Leaf { pane_id, .. } if *pane_id == active_id => true,
            SplitNode::Preview { pane_id, .. } if *pane_id == active_id => true,
            _ => false,
        };
        if is_single_pane {
            return None; // Signal to AppState: close the workspace instead
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

        Some(surviving_id)
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
        equalize_node(&self.root);
        schedule_resize_recovery();
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
        SplitNode::Leaf { .. } => None,
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
        SplitNode::Leaf { .. } => None,
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
        SplitNode::Leaf { .. } => {}
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

/// Best-effort CWD capture for a Ghostty surface via /proc.
/// Walks /proc looking for child processes of the current process (cmux),
/// then reads /proc/{pid}/cwd for the foreground shell.
/// Never panics — falls back to $HOME or empty string.
fn get_surface_cwd(surface: ffi::ghostty_surface_t) -> String {
    if surface.is_null() {
        return String::new();
    }
    // Try to find child shell processes by scanning /proc for children of our PID.
    // Each Ghostty surface spawns a child shell — we look for processes whose
    // parent is the cmux process and read their CWD.
    let our_pid = std::process::id();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        // Collect candidate child PIDs (children of our process)
        let mut candidates: Vec<u32> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
                // Read /proc/{pid}/stat to check parent PID
                if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                    // Format: pid (comm) state ppid ...
                    // Find the closing paren then parse ppid
                    if let Some(after_comm) = stat.rfind(')') {
                        let fields: Vec<&str> = stat[after_comm + 2..].split_whitespace().collect();
                        if fields.len() >= 2 {
                            if let Ok(ppid) = fields[1].parse::<u32>() {
                                if ppid == our_pid {
                                    candidates.push(pid);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Use the last (most recent) child process CWD as best guess.
        // In practice, each surface has one direct child shell.
        for pid in candidates.iter().rev() {
            if let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
                let cwd_str = cwd.to_string_lossy().to_string();
                if !cwd_str.is_empty() {
                    return cwd_str;
                }
            }
        }
    }
    // Fallback to $HOME
    std::env::var("HOME").unwrap_or_default()
}

impl SplitNode {
    /// Produce a serializable snapshot of this node's tree structure.
    /// `shell` and `cwd` are best-effort: Plan 05 fills these via /proc.
    /// Falls back to empty strings if /proc is unavailable or the pid is unknown.
    pub fn to_data(&self) -> SplitNodeData {
        match self {
            SplitNode::Leaf { pane_id, uuid, surface, .. } => {
                let cwd = get_surface_cwd(*surface);
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                SplitNodeData::Leaf {
                    pane_id: *pane_id,
                    surface_uuid: *uuid,
                    shell,
                    cwd,
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
const SURFACE_ID_ENV: &str = "CMUX_SURFACE_ID";
const SURFACE_ID_ENV_C: &std::ffi::CStr = c"CMUX_SURFACE_ID";

/// Leak a C string for a Ghostty surface config. The config is consumed at
/// GLArea realize time, after the caller returns, and Ghostty copies what it
/// keeps, so these few bytes per pane are never reclaimed.
fn leak_c_string(s: &str) -> *const std::os::raw::c_char {
    match std::ffi::CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null(),
    }
}

/// Surface config for a new local terminal pane with UUID `new_uuid`.
/// When `source` (the pane it is opened from, plus its UUID) is given, the new
/// pane starts in the source's working directory: Ghostty's own value (learned
/// from OSC 7) if it names a local directory, else the cwd of the source
/// pane's shell read from /proc.
pub(crate) fn local_pane_config(
    source: Option<(ffi::ghostty_surface_t, Option<String>)>,
    context: ffi::ghostty_surface_context_e,
    new_uuid: Uuid,
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
    let env = Box::new(ffi::ghostty_env_var_s {
        key: SURFACE_ID_ENV_C.as_ptr(),
        value: leak_c_string(&new_uuid.to_string()),
    });
    config.env_vars = Box::into_raw(env);
    config.env_var_count = 1;
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
            }),
            end: Box::new(SplitNodeData::Leaf {
                pane_id: 2,
                surface_uuid: Uuid::new_v4(),
                shell: String::new(),
                cwd: String::new(),
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
        }
    }

    fn preview(id: u64) -> SplitNodeData {
        SplitNodeData::Leaf {
            pane_id: id,
            surface_uuid: Uuid::nil(),
            shell: String::new(),
            cwd: String::new(),
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
