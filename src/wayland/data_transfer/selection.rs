use smithay::input::dnd::{DnDGrab, DndFocus, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::Serial;
use smithay::wayland::selection::data_device::{DataDeviceHandler, set_data_device_focus};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, set_primary_focus};

/// Points clipboard and primary selections at the focused surface's client.
///
/// Smithay does not update selection ownership inside `KeyboardHandle::set_focus`;
/// skipping this leaves clipboard globals advertised while silently breaking
/// copy and paste.
pub fn sync_selection_focus<D>(
    display_handle: &DisplayHandle,
    seat: &Seat<D>,
    focused: Option<&WlSurface>,
) where
    D: SeatHandler + DataDeviceHandler + PrimarySelectionHandler + 'static,
{
    let client = focused.and_then(|surface| surface.client());
    set_data_device_focus(display_handle, seat, client.clone());
    set_primary_focus(display_handle, seat, client);
}

/// Promotes a client's implicit pointer or touch grab into a server-side
/// drag-and-drop grab for `wl_data_device.start_drag`.
///
/// `WaylandDndGrabHandler`'s default implementation cancels the source, which
/// silently kills client-initiated drags.
pub fn start_dnd_grab<D, S>(
    state: &mut D,
    display_handle: &DisplayHandle,
    source: S,
    seat: Seat<D>,
    serial: Serial,
    type_: GrabType,
) -> bool
where
    D: SeatHandler + DndGrabHandler + 'static,
    D::PointerFocus: DndFocus<D>,
    D::TouchFocus: DndFocus<D>,
    S: Source,
{
    match type_ {
        GrabType::Pointer => {
            let start = seat.get_pointer().and_then(|pointer| {
                pointer
                    .grab_start_data()
                    .map(|start_data| (pointer, start_data))
            });
            match start {
                Some((pointer, start_data)) => {
                    let grab = DnDGrab::new_pointer(display_handle, start_data, source, seat);
                    // Keep the drag bound to the press surface; re-resolving focus
                    // mid-drag would transfer it under whatever is hovered next.
                    pointer.set_grab(state, grab, serial, Focus::Keep);
                    true
                }
                // No active press matches the serial; canceling notifies the client.
                None => {
                    source.cancel();
                    false
                }
            }
        }
        GrabType::Touch => {
            let start = seat.get_touch().and_then(|touch| {
                touch
                    .grab_start_data()
                    .map(|start_data| (touch, start_data))
            });
            match start {
                Some((touch, start_data)) => {
                    let grab = DnDGrab::new_touch(display_handle, start_data, source, seat);
                    touch.set_grab(state, grab, serial);
                    true
                }
                None => {
                    source.cancel();
                    false
                }
            }
        }
    }
}
