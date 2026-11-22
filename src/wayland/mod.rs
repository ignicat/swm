mod capture;
mod data_transfer;
mod desktop;
mod input;
mod output;
mod session;
mod surface;

pub mod compositor;
pub mod permissions;

pub use capture::{ext_image_copy_capture, wlr_screencopy};
pub(crate) use data_transfer::clipboard_helper;
pub use data_transfer::{dnd, selection};
pub use desktop::{
    decoration, foreign_toplevel, fullscreen, layer_shell, popup, surface_dismissal, xdg_shell,
};
pub use input::{focus, text_input};
pub use output::display_scale;
pub use output::protocols::{wlr_gamma_control, wlr_output_management, wlr_output_power};
use output::surface_scale;
pub use session::session_lock;
pub use surface::{
    buffer_readiness, commit_timing, dmabuf, explicit_sync, fifo, frame_callbacks, presentation,
};

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use smithay::desktop::{LayerSurface, PopupManager, Space, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::content_type::ContentTypeState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::pointer_warp::PointerWarpManager;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::dialog::XdgDialogState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::xdg_activation::XdgActivationState;
use smithay::wayland::xdg_foreign::XdgForeignState;
use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager;

/// Output responsible for painting a window. Smithay's `Space` still owns
/// output geometry and pointer routing; this enforces whole-window handoff.
struct WindowOutputState {
    assigned: RwLock<String>,
    inherited: RwLock<Option<Arc<WindowOutputState>>>,
}

struct WindowOutput(Arc<WindowOutputState>);

struct WindowPresentationOwner(RwLock<Option<u32>>);

fn new_window_output(name: String) -> WindowOutput {
    WindowOutput(Arc::new(WindowOutputState {
        assigned: RwLock::new(name),
        inherited: RwLock::new(None),
    }))
}

fn window_output_state_name(state: &Arc<WindowOutputState>, depth: usize) -> String {
    if depth < 8 {
        let inherited = state
            .inherited
            .read()
            .expect("window output inheritance lock poisoned")
            .clone();
        if let Some(inherited) = inherited {
            return window_output_state_name(&inherited, depth + 1);
        }
    }
    state
        .assigned
        .read()
        .expect("window output lock poisoned")
        .clone()
}

pub fn set_window_output(window: &Window, output: &Output) {
    let owner = window
        .user_data()
        .get_or_insert_threadsafe(|| new_window_output(output.name()));
    *owner
        .0
        .assigned
        .write()
        .expect("window output lock poisoned") = output.name();
    *owner
        .0
        .inherited
        .write()
        .expect("window output inheritance lock poisoned") = None;
    display_scale::send_window(window, output);
}

/// Makes `window` follow a managed owner's output without allowing writes to
/// the child to mutate the owner. Override-redirect X11 menus use this so an
/// already-open menu follows an owner handed to another output.
#[cfg(feature = "xwayland")]
pub fn inherit_window_output(window: &Window, owner: &Window) -> bool {
    let Some(owner_output) = owner.user_data().get::<WindowOutput>() else {
        return false;
    };
    let name = window_output_state_name(&owner_output.0, 0);
    let child_output = window
        .user_data()
        .get_or_insert_threadsafe(|| new_window_output(name));
    *child_output
        .0
        .inherited
        .write()
        .expect("window output inheritance lock poisoned") = Some(owner_output.0.clone());
    true
}

pub fn window_output_name(window: &Window) -> Option<String> {
    window
        .user_data()
        .get::<WindowOutput>()
        .map(|owner| window_output_state_name(&owner.0, 0))
}

/// Associates an override-redirect X11 surface with the managed X11 window
/// whose presentation transform it must inherit.
#[cfg(feature = "xwayland")]
pub fn set_window_presentation_owner(window: &Window, owner: Option<u32>) {
    let state = window
        .user_data()
        .get_or_insert_threadsafe(|| WindowPresentationOwner(RwLock::new(owner)));
    *state
        .0
        .write()
        .expect("window presentation owner lock poisoned") = owner;
}

pub fn window_presentation_owner(window: &Window) -> Option<u32> {
    window
        .user_data()
        .get::<WindowPresentationOwner>()
        .and_then(|owner| {
            *owner
                .0
                .read()
                .expect("window presentation owner lock poisoned")
        })
}

pub fn window_is_on_output(window: &Window, output: &Output, primary: &Output) -> bool {
    window_output_name(window)
        .map(|owner| owner == output.name())
        // Windows are assigned on initial map; fall back to primary if missing.
        .unwrap_or_else(|| output == primary)
}

#[cfg(test)]
mod window_output_tests {
    use super::{new_window_output, window_output_state_name};

    #[test]
    fn inherited_output_follows_owner_without_child_writes_mutating_it() {
        let owner = new_window_output("DP-1".to_owned());
        let child = new_window_output("fallback".to_owned());
        *child
            .0
            .inherited
            .write()
            .expect("child inheritance lock poisoned") = Some(owner.0.clone());

        *owner.0.assigned.write().expect("owner lock poisoned") = "DP-2".to_owned();
        assert_eq!(window_output_state_name(&child.0, 0), "DP-2");

        *child
            .0
            .inherited
            .write()
            .expect("child inheritance lock poisoned") = None;
        *child.0.assigned.write().expect("child lock poisoned") = "DP-3".to_owned();
        assert_eq!(window_output_state_name(&child.0, 0), "DP-3");
        assert_eq!(window_output_state_name(&owner.0, 0), "DP-2");
    }
}

/// Wayland protocol and shell-model state shared by both session drivers.
///
/// Backend rendering, cameras, input devices, and redraw scheduling stay out
/// of this type. That keeps the Smithay globals and shell lifecycle together
/// without creating a compositor-wide god object.
pub struct WaylandState {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub fifo: fifo::State,
    _fifo_manager: smithay::wayland::fifo::FifoManagerState,
    _commit_timing_manager: smithay::wayland::commit_timing::CommitTimingManagerState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub xdg_shell_state: XdgShellState,
    _xdg_dialog_state: XdgDialogState,
    pub xdg_foreign_state: XdgForeignState,
    pub xdg_activation_state: XdgActivationState,
    pub layer_shell_state: WlrLayerShellState,
    _single_pixel_buffer_state: SinglePixelBufferState,
    _content_type_state: ContentTypeState,
    _xdg_toplevel_icon_manager: XdgToplevelIconManager,
    // Retained for the lifetime of its advertised global.
    _xdg_decoration_state: XdgDecorationState,
    _viewporter_state: ViewporterState,
    _fractional_scale_manager_state: FractionalScaleManagerState,
    _idle_inhibit_manager_state: IdleInhibitManagerState,
    _relative_pointer_manager_state: RelativePointerManagerState,
    _pointer_constraints_state: PointerConstraintsState,
    _pointer_warp_manager: PointerWarpManager,
    _pointer_gestures_state: PointerGesturesState,
    _cursor_shape_manager_state: CursorShapeManagerState,
    _virtual_keyboard_manager_state: VirtualKeyboardManagerState,
    _text_input_manager_state: TextInputManagerState,
    _input_method_manager_state: InputMethodManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub shm_state: ShmState,
    // Retained alongside the wl_output globals it serves.
    _output_manager_state: OutputManagerState,
    pub wlr_output_management_state: wlr_output_management::State,
    pub wlr_gamma_control_state: wlr_gamma_control::State,
    pub wlr_output_power_state: wlr_output_power::State,
    pub wlr_screencopy_state: wlr_screencopy::State,
    pub ext_image_copy_capture_state: ext_image_copy_capture::State,
    pub(crate) idle_inhibitors: HashMap<WlSurface, usize>,
    output_globals: HashMap<String, GlobalId>,
    /// Clipboard and drag-and-drop global state; Smithay passes source fds directly without buffering.
    pub data_device_state: DataDeviceState,
    pub dnd_icon: Option<dnd::DndIcon>,
    /// Middle-click primary selection state (independent from `data_device_state`).
    pub primary_selection_state: PrimarySelectionState,
    /// Standardized ext-data-control state sharing seat selections with `data_device` and `primary_selection`.
    pub ext_data_control_state: DataControlState,
    pub foreign_toplevel_state: foreign_toplevel::State,
    /// Shared popup tree across xdg-toplevel and layer-shell roots.
    pub popup_manager: PopupManager,
    /// Visible windows that have attached a buffer (see `unmapped`).
    pub space: Space<Window>,
    /// Managed-window stacking order independent of `space`, read by focus succession.
    pub managed_windows: crate::window::ManagedWindowStack,
    /// Existing toplevels awaiting their first buffer before entering `space`.
    pub unmapped: HashMap<WlSurface, Window>,
    /// Buffer-backed toplevels removed from `space` while collapsed as nodes, kept for Wayland lifecycle tracking.
    pub collapsed: HashMap<WlSurface, Window>,
    /// Last mapped position retained across a null-buffer unmap.
    pub unmapped_locations: HashMap<WlSurface, Point<i32, Logical>>,
    /// Layer surfaces without a buffer since creation or null-buffer unmap; `LayerMap` keeps the role for configures.
    pub unmapped_layers: HashSet<WlSurface>,
    /// Click-selected on-demand layer surface; exclusive layer focus is derived instead of stored.
    pub focused_layer: Option<LayerSurface>,
    /// Persistent normal-window focus, preserved across temporary layers.
    pub focused_window: Option<WlSurface>,
    /// Name of the click-focused output used when placing new toplevels.
    pub focused_output: Option<String>,
}

pub struct WaylandGlobals {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub fifo_manager: smithay::wayland::fifo::FifoManagerState,
    pub commit_timing_manager: smithay::wayland::commit_timing::CommitTimingManagerState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub xdg_shell_state: XdgShellState,
    pub xdg_dialog_state: XdgDialogState,
    pub xdg_foreign_state: XdgForeignState,
    pub xdg_activation_state: XdgActivationState,
    pub layer_shell_state: WlrLayerShellState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub content_type_state: ContentTypeState,
    pub xdg_toplevel_icon_manager: XdgToplevelIconManager,
    pub xdg_decoration_state: XdgDecorationState,
    pub viewporter_state: ViewporterState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub pointer_warp_manager: PointerWarpManager,
    pub pointer_gestures_state: PointerGesturesState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    pub virtual_keyboard_manager_state: VirtualKeyboardManagerState,
    pub text_input_manager_state: TextInputManagerState,
    pub input_method_manager_state: InputMethodManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub wlr_output_management_state: wlr_output_management::State,
    pub wlr_gamma_control_state: wlr_gamma_control::State,
    pub wlr_output_power_state: wlr_output_power::State,
    pub wlr_screencopy_state: wlr_screencopy::State,
    pub ext_image_copy_capture_state: ext_image_copy_capture::State,
    pub output_globals: HashMap<String, GlobalId>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: DataControlState,
    pub foreign_toplevel_state: foreign_toplevel::State,
}

impl WaylandState {
    pub fn new(globals: WaylandGlobals) -> Self {
        Self {
            display_handle: globals.display_handle,
            compositor_state: globals.compositor_state,
            fifo: fifo::State::default(),
            _fifo_manager: globals.fifo_manager,
            _commit_timing_manager: globals.commit_timing_manager,
            dmabuf_state: globals.dmabuf_state,
            dmabuf_global: globals.dmabuf_global,
            xdg_shell_state: globals.xdg_shell_state,
            _xdg_dialog_state: globals.xdg_dialog_state,
            xdg_foreign_state: globals.xdg_foreign_state,
            xdg_activation_state: globals.xdg_activation_state,
            layer_shell_state: globals.layer_shell_state,
            _single_pixel_buffer_state: globals.single_pixel_buffer_state,
            _content_type_state: globals.content_type_state,
            _xdg_toplevel_icon_manager: globals.xdg_toplevel_icon_manager,
            _xdg_decoration_state: globals.xdg_decoration_state,
            _viewporter_state: globals.viewporter_state,
            _fractional_scale_manager_state: globals.fractional_scale_manager_state,
            _idle_inhibit_manager_state: globals.idle_inhibit_manager_state,
            _relative_pointer_manager_state: globals.relative_pointer_manager_state,
            _pointer_constraints_state: globals.pointer_constraints_state,
            _pointer_warp_manager: globals.pointer_warp_manager,
            _pointer_gestures_state: globals.pointer_gestures_state,
            _cursor_shape_manager_state: globals.cursor_shape_manager_state,
            _virtual_keyboard_manager_state: globals.virtual_keyboard_manager_state,
            _text_input_manager_state: globals.text_input_manager_state,
            _input_method_manager_state: globals.input_method_manager_state,
            keyboard_shortcuts_inhibit_state: globals.keyboard_shortcuts_inhibit_state,
            shm_state: globals.shm_state,
            _output_manager_state: globals.output_manager_state,
            wlr_output_management_state: globals.wlr_output_management_state,
            wlr_gamma_control_state: globals.wlr_gamma_control_state,
            wlr_output_power_state: globals.wlr_output_power_state,
            wlr_screencopy_state: globals.wlr_screencopy_state,
            ext_image_copy_capture_state: globals.ext_image_copy_capture_state,
            idle_inhibitors: HashMap::new(),
            output_globals: globals.output_globals,
            data_device_state: globals.data_device_state,
            dnd_icon: None,
            primary_selection_state: globals.primary_selection_state,
            ext_data_control_state: globals.ext_data_control_state,
            foreign_toplevel_state: globals.foreign_toplevel_state,
            popup_manager: PopupManager::default(),
            space: Space::default(),
            managed_windows: crate::window::ManagedWindowStack::default(),
            unmapped: HashMap::new(),
            collapsed: HashMap::new(),
            unmapped_locations: HashMap::new(),
            unmapped_layers: HashSet::new(),
            focused_layer: None,
            focused_window: None,
            focused_output: None,
        }
    }

    pub fn ensure_output_global<D>(&mut self, output: &Output)
    where
        D: smithay::reexports::wayland_server::GlobalDispatch<
                smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
                smithay::wayland::output::WlOutputData,
            > + 'static,
    {
        self.output_globals
            .entry(output.name())
            .or_insert_with(|| output.create_global::<D>(&self.display_handle));
    }

    pub fn disable_output_global<D: 'static>(&mut self, output: &Output) {
        self.ext_image_copy_capture_state.stop_output(output);
        if let Some(global) = self.output_globals.remove(&output.name()) {
            self.display_handle.disable_global::<D>(global);
        }
    }
}

/// Per-client Wayland state - one instance per connected client.
#[derive(Default)]
pub struct ClientState {
    pub permissions: permissions::Permissions,
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        match reason {
            DisconnectReason::ConnectionClosed => {
                static CLOSED_CLIENTS: AtomicU64 = AtomicU64::new(0);
                let count = CLOSED_CLIENTS.fetch_add(1, Ordering::Relaxed) + 1;
                if count == 1 || count.is_multiple_of(256) {
                    eventline::debug!(
                        "wayland clients disconnected normally count={count} latest={client_id:?}"
                    );
                }
            }
            DisconnectReason::ProtocolError(error) => {
                eventline::warn!(
                    "wayland client {client_id:?} disconnected after protocol error: {error:?}"
                );
            }
        }
    }
}
