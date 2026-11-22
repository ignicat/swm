use std::borrow::Cow;

use smithay::utils::{Logical, Rectangle};

use smithay::wayland::seat::WaylandFocus;

use crate::wayland;

#[path = "startup/autostart.rs"]
mod autostart;
#[path = "protocol/buffer_release.rs"]
mod buffer_release;
#[path = "windows/closing.rs"]
pub(crate) mod closing;
#[path = "protocol/commit_timing.rs"]
mod commit_timing;
#[path = "startup/config_source.rs"]
mod config_source;
mod cursor;
#[path = "startup/environment.rs"]
pub mod environment;
#[path = "windows/focus.rs"]
mod focus;
#[path = "protocol/foreign_toplevel.rs"]
mod foreign_toplevel;
pub(crate) mod gesture;
mod grab;
#[path = "capture/image_copy_capture.rs"]
mod image_copy_capture;
pub(crate) mod input;
#[path = "startup/state.rs"]
mod launch;
#[path = "windows/lifecycle.rs"]
mod lifecycle;
#[path = "workspace/navigation.rs"]
mod navigation;
#[path = "workspace/node_controls.rs"]
mod node_controls;
#[path = "windows/opening.rs"]
pub(crate) mod opening;
#[path = "display/output.rs"]
pub(crate) mod output;
#[path = "display/output_power.rs"]
mod output_power;
#[path = "display/output_rebase.rs"]
mod output_rebase;
pub(crate) mod pointer;
#[path = "protocol/handlers.rs"]
mod protocol;
#[path = "protocol/setup.rs"]
mod protocol_setup;
#[path = "display/render_completion.rs"]
mod render_completion;
#[path = "startup/spawn.rs"]
mod spawn;
#[path = "display/timing.rs"]
mod timing;
pub(crate) mod touch;
#[cfg(feature = "xwayland")]
#[path = "diagnostics/trace.rs"]
pub(crate) mod trace;
#[cfg(not(feature = "xwayland"))]
#[path = "diagnostics/trace_disabled.rs"]
pub(crate) mod trace;
#[path = "drivers/tty/mod.rs"]
pub mod tty;
#[path = "windows/window_presentation.rs"]
mod window_presentation;
#[cfg(feature = "winit")]
#[path = "drivers/winit/mod.rs"]
pub mod winit;
#[path = "workspace/zoom_fit.rs"]
pub(crate) mod zoom_fit;

mod interaction;
mod settings;
mod state;

pub(crate) use focus::{focus_window, focus_window_after_close};
#[cfg(feature = "xwayland")]
pub(crate) use grab::begin_pointer_resize;
pub(crate) use grab::{
    activate_client_pointer_move, begin_client_pointer_move, begin_pointer_edge_pan,
    begin_pointer_move, begin_window_resize, settle_window_movement,
};
pub use interaction::InteractionState;
pub(crate) use navigation::{activate_node, center_pointer_on_output};
pub(crate) use node_controls::*;
pub use settings::RuntimeSettings;
use state::SessionInit;
pub use state::{OutputDriver, RenderDriver, Session, SessionDriver};
use timing::{
    install_frame_callback_fallback_timer, install_node_decay_timer, install_overlay_timer,
};
pub(crate) use timing::{release_fifo_barriers, schedule_fifo_barriers};
pub(crate) use window_presentation::*;

/// Builds popup unconstrain inputs from disjoint `Session` fields so a later
/// `&mut session.wayland` borrow stays valid.
macro_rules! popup_unconstrain_context {
    ($session:expr) => {
        $crate::wayland::popup::UnconstrainContext {
            cameras: &$session.cameras,
            window_animations: &$session.window_animations,
            fullscreen: &$session.fullscreen,
            maximize: &$session.maximize,
            decorations: &$session.settings.decorations,
            now: $crate::frame_clock::monotonic_now(),
        }
    };
}

pub(crate) use popup_unconstrain_context;

pub(crate) fn cancel_grab_for_surface<D: SessionDriver>(
    session: &mut Session<D>,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) {
    if crate::input::grab::belongs_to_surface(session.interactions.grab(), surface) {
        cancel_compositor_grab(session);
        crate::input::grab::forget_resize_anchor(&mut session.interactions.resize_anchor, surface);
    }
}

/// Cancels compositor-owned pointer state. Modal overlays, session locking, and
/// surface destruction all use this path so a held window cannot remain
/// assigned to the wrong output.
pub(crate) fn cancel_compositor_grab<D: SessionDriver>(session: &mut Session<D>) {
    session.interactions.steam_close_pressed = None;
    if matches!(
        session.interactions.grab(),
        crate::input::grab::Grab::ResizeWindow(_)
    ) {
        crate::input::grab::release_resize_anchor(&mut session.interactions.resize_anchor);
    }
    session.end_grab();
}

pub(crate) use lifecycle::{finish_window_unmap, prepare_window_unmap};

pub(crate) fn reconcile_pointer_constraints<D: SessionDriver>(session: &mut Session<D>) {
    pointer::reconcile_state(session);
}

#[cfg(feature = "xwayland")]
pub(crate) fn has_active_pointer_confinement<D: SessionDriver>(session: &Session<D>) -> bool {
    pointer::has_active_confinement(session)
}

pub(crate) fn cursor_visible<D: SessionDriver>(session: &Session<D>) -> bool {
    pointer::cursor_visible(session)
}

pub(crate) fn cursor_override<D: SessionDriver>(
    session: &Session<D>,
) -> Option<smithay::input::pointer::CursorIcon> {
    pointer::cursor_override(session)
}

pub(crate) fn note_pointer_activity<D: SessionDriver>(session: &mut Session<D>) {
    session.cursor_policy.pointer_activity();
}

/// Requests that a client close a managed window.
///
/// X11 clients may replace their contents before withdrawing, so retain their
/// visible frame before sending the request. Native XDG close requests are
/// advisory and capture only at the authoritative buffer-removal boundary;
/// this keeps a client live if it presents and then cancels a confirmation.
pub(crate) fn request_window_close<D: SessionDriver>(
    session: &mut Session<D>,
    window: &smithay::desktop::Window,
) {
    if crate::window::closing::pending(window) {
        return;
    }
    let frozen = closing::capture_before_close_request(session, window);
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_close();
    } else {
        session.xwayland.note_close_requested(window);
        crate::xwayland::close_window(window);
        closing::start_shortcut_close(session, window, frozen);
    }
    if frozen {
        session.request_redraw();
    }
}

pub(crate) fn sync_keyboard_focus<D: SessionDriver>(
    session: &mut Session<D>,
    serial: smithay::utils::Serial,
) {
    if session.session_lock.active() {
        if session.window_rules.set_focus(None) {
            session.refresh_dynamic_window_rules();
        }
        let focused = session
            .session_lock
            .focused_surface()
            .map(crate::xwayland::KeyboardFocusTarget::from);
        let keyboard = session.keyboard_handle();
        let has_keyboard_focus = focused.is_some();
        pointer::prepare_keyboard_focus_change(session, None);
        keyboard.set_focus(session, focused, serial);
        session
            .xwayland
            .sync_active_window(None, has_keyboard_focus);
        return;
    }
    wayland::focus::refresh_selected_layer(&mut session.wayland);
    focus::reconcile_modal_focus(session);
    if let Some(surface) = session.wayland.focused_window.clone() {
        session
            .nodes
            .focus_surface(&surface, session.start_time.elapsed().as_millis() as u64);
    } else if session.nodes.focused().is_some_and(|id| {
        session
            .nodes
            .record(id)
            .is_some_and(|record| !record.collapsed)
    }) {
        session.nodes.focus(None, 0);
    }
    let focused = wayland::focus::current(
        &session.wayland,
        &session.fullscreen,
        crate::frame_clock::monotonic_now(),
    )
    .and_then(|focus| match focus {
        wayland::focus::KeyboardFocus::Window(surface) => session
            .wayland
            .space
            .elements()
            .find(|window| {
                window
                    .wl_surface()
                    .is_some_and(|candidate| candidate.as_ref() == &surface)
            })
            .and_then(crate::xwayland::KeyboardFocusTarget::for_window)
            .or_else(|| Some(surface.into())),
        wayland::focus::KeyboardFocus::ExclusiveLayer(surface)
        | wayland::focus::KeyboardFocus::OnDemandLayer(surface) => Some(surface.into()),
    });
    let keyboard = session.keyboard_handle();
    let next_constraint_root = focused
        .as_ref()
        .and_then(|target| target.wl_surface().map(Cow::into_owned))
        .filter(|surface| session.wayland.focused_window.as_ref() == Some(surface));
    if session.window_rules.set_focus(next_constraint_root.clone()) {
        session.refresh_dynamic_window_rules();
    }
    let active_x11_window = focused
        .as_ref()
        .and_then(crate::xwayland::KeyboardFocusTarget::x11_window_id);
    let globally_active_x11_window = focused
        .as_ref()
        .and_then(crate::xwayland::KeyboardFocusTarget::globally_active_x11_window_id);
    let has_keyboard_focus = focused.is_some();
    if let Some(focused) = focused.as_ref() {
        focused.acknowledge_attention();
    }
    // Commit keyboard/X focus before retiring the old pointer constraint.
    // Xwayland releases a locked X pointer when it receives `unlocked`; doing
    // that while the old game still owns core X focus lets its final cursor
    // re-anchor affect the camera. The seat update is synchronous from the
    // compositor's perspective, so no input can reach the new target between
    // these two operations.
    keyboard.set_focus(session, focused, serial);
    session
        .xwayland
        .sync_active_window(active_x11_window, has_keyboard_focus);
    session
        .xwayland
        .focus_globally_active_window(globally_active_x11_window);
    pointer::prepare_keyboard_focus_change(session, next_constraint_root.as_ref());
    // Map, unmap, destroy and raise all funnel through here, so this is the one
    // place the X server's stack can drift from the compositor's.
    crate::window::enforce_dialog_stacking(&mut session.wayland);
    session.xwayland.sync_stacking_order(&session.wayland.space);
    // Activation is when clients inspect root coordinates for menus and hit tests.
    if let Some(xid) = active_x11_window
        && let Some(window) = crate::xwayland::window_for_xid(&session.wayland.space, xid)
    {
        crate::xwayland::sync_position(session, &window);
    }
    pointer::reconcile_state(session);
}
