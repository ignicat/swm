//! Install the Wayland globals shared by both session backends.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::content_type::ContentTypeState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::DmabufState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::xdg_activation::XdgActivationState;
use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager;

use super::{Session, SessionDriver};
use crate::wayland::{WaylandGlobals, WaylandState};

impl<D: SessionDriver> Session<D> {
    pub fn create_wayland_state(display_handle: DisplayHandle, driver: &mut D) -> WaylandState {
        let capabilities = driver.dmabuf_capabilities();
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = crate::wayland::dmabuf::create_global::<Self>(
            &mut dmabuf_state,
            &display_handle,
            &capabilities,
        );
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let ext_data_control_state = DataControlState::new::<Self, _>(
            &display_handle,
            Some(&primary_selection_state),
            |client| {
                crate::wayland::permissions::allowed(
                    client,
                    crate::wayland::permissions::Capability::Clipboard,
                )
            },
        );

        crate::wayland::surface_dismissal::create_global::<Self>(&display_handle);

        WaylandState::new(WaylandGlobals {
            display_handle: display_handle.clone(),
            compositor_state: CompositorState::new_v6::<Self>(&display_handle),
            fifo_manager: smithay::wayland::fifo::FifoManagerState::new::<Self>(&display_handle),
            commit_timing_manager:
                smithay::wayland::commit_timing::CommitTimingManagerState::unmanaged::<Self>(
                    &display_handle,
                ),
            dmabuf_state,
            dmabuf_global,
            xdg_shell_state: XdgShellState::new::<Self>(&display_handle),
            xdg_dialog_state: smithay::wayland::shell::xdg::dialog::XdgDialogState::new::<Self>(
                &display_handle,
            ),
            xdg_foreign_state: smithay::wayland::xdg_foreign::XdgForeignState::new::<Self>(
                &display_handle,
            ),
            xdg_activation_state: XdgActivationState::new::<Self>(&display_handle),
            layer_shell_state: WlrLayerShellState::new::<Self>(&display_handle),
            single_pixel_buffer_state: SinglePixelBufferState::new::<Self>(&display_handle),
            content_type_state: ContentTypeState::new::<Self>(&display_handle),
            xdg_toplevel_icon_manager: XdgToplevelIconManager::new::<Self>(&display_handle),
            xdg_decoration_state: XdgDecorationState::new::<Self>(&display_handle),
            viewporter_state: ViewporterState::new::<Self>(&display_handle),
            fractional_scale_manager_state: FractionalScaleManagerState::new::<Self>(
                &display_handle,
            ),
            idle_inhibit_manager_state: IdleInhibitManagerState::new::<Self>(&display_handle),
            relative_pointer_manager_state: RelativePointerManagerState::new::<Self>(
                &display_handle,
            ),
            pointer_constraints_state: PointerConstraintsState::new::<Self>(&display_handle),
            pointer_warp_manager: smithay::wayland::pointer_warp::PointerWarpManager::new::<Self>(
                &display_handle,
            ),
            pointer_gestures_state: PointerGesturesState::new::<Self>(&display_handle),
            cursor_shape_manager_state: CursorShapeManagerState::new::<Self>(&display_handle),
            virtual_keyboard_manager_state: VirtualKeyboardManagerState::new::<Self, _>(
                &display_handle,
                |client| {
                    crate::wayland::permissions::allowed(
                        client,
                        crate::wayland::permissions::Capability::VirtualKeyboard,
                    )
                },
            ),
            text_input_manager_state: TextInputManagerState::new::<Self>(&display_handle),
            input_method_manager_state: InputMethodManagerState::new::<Self, _>(
                &display_handle,
                |client| {
                    crate::wayland::permissions::allowed(
                        client,
                        crate::wayland::permissions::Capability::InputMethod,
                    )
                },
            ),
            keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState::new::<Self>(
                &display_handle,
            ),
            shm_state: ShmState::new::<Self>(&display_handle, vec![]),
            output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&display_handle),
            wlr_output_management_state: crate::wayland::wlr_output_management::State::new::<Self>(
                &display_handle,
            ),
            wlr_gamma_control_state: crate::wayland::wlr_gamma_control::State::new::<Self>(
                &display_handle,
                D::BACKEND_KIND == crate::input::keybinds::BackendKind::Tty,
            ),
            wlr_output_power_state: crate::wayland::wlr_output_power::State::new::<Self>(
                &display_handle,
                D::BACKEND_KIND == crate::input::keybinds::BackendKind::Tty,
                |client| {
                    crate::wayland::permissions::allowed(
                        client,
                        crate::wayland::permissions::Capability::OutputPower,
                    )
                },
            ),
            wlr_screencopy_state: crate::wayland::wlr_screencopy::State::new::<Self>(
                &display_handle,
            ),
            ext_image_copy_capture_state: crate::wayland::ext_image_copy_capture::State::new::<Self>(
                &display_handle,
                |client| {
                    crate::wayland::permissions::allowed(
                        client,
                        crate::wayland::permissions::Capability::Capture,
                    )
                },
            ),
            output_globals: std::collections::HashMap::new(),
            data_device_state: DataDeviceState::new::<Self>(&display_handle),
            primary_selection_state,
            ext_data_control_state,
            foreign_toplevel_state: crate::wayland::foreign_toplevel::State::new::<Self>(
                &display_handle,
            ),
        })
    }
}
