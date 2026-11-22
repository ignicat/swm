use std::ffi::OsStr;

use smithay::backend::input::{
    ButtonState, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::desktop::{Space, Window};
use smithay::input::keyboard::{FilterResult, Keysym};
use smithay::input::pointer::{ButtonEvent, RelativeMotionEvent};
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
use smithay::wayland::seat::WaylandFocus;

use super::{Session, SessionDriver};
use crate::input::pointer::{axis_frame_filtered, process_wheel_bindings};
use crate::input::{
    PointerBindingResult, match_keyboard_binding, match_pointer_bind, match_wheel_bind,
    process_pointer_binding,
};
use crate::wayland;

pub(crate) mod actions;
mod axis;
mod button;
mod dispatch;
mod hit_test;
use hit_test::*;
mod keyboard;
mod modal;
mod motion;
pub(crate) mod repeat;

use crate::input::buttons::BTN_LEFT;
#[cfg(test)]
use crate::input::buttons::BTN_RIGHT;
const NODE_DRAG_THRESHOLD_PX: f64 = 8.0;

fn sampled_drag_velocity(
    previous: swm_core::field::Vec2,
    current: swm_core::field::Vec2,
    previous_velocity: swm_core::field::Vec2,
    last_update: std::time::Duration,
    now: std::time::Duration,
) -> swm_core::field::Vec2 {
    let dt = now.saturating_sub(last_update).as_secs_f32();
    if dt <= f32::EPSILON {
        return previous_velocity;
    }
    let clamp = |value: f32| value.clamp(-800.0, 800.0);
    swm_core::field::Vec2 {
        x: previous_velocity.x * 0.35 + clamp((current.x - previous.x) / dt) * 0.65,
        y: previous_velocity.y * 0.35 + clamp((current.y - previous.y) / dt) * 0.65,
    }
}

fn drag_threshold_reached(press: Point<f64, Logical>, current: (f64, f64)) -> bool {
    let dx = current.0 - press.x;
    let dy = current.1 - press.y;
    dx.hypot(dy) >= NODE_DRAG_THRESHOLD_PX
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingWindowMoveMotion {
    Wait,
    Cancel,
    Activate,
}

fn pending_window_move_motion(
    pointer_grab_valid: bool,
    press: Point<f64, Logical>,
    current: (f64, f64),
) -> PendingWindowMoveMotion {
    if !pointer_grab_valid {
        PendingWindowMoveMotion::Cancel
    } else if drag_threshold_reached(press, current) {
        PendingWindowMoveMotion::Activate
    } else {
        PendingWindowMoveMotion::Wait
    }
}

fn releases_pending_window_move(pending_button: u32, event_button: u32, released: bool) -> bool {
    released && pending_button == event_button
}

/// A camera pan belongs to the button whose press began it: releasing any
/// other button (for example a left click made while a middle-button pan is
/// running) must neither end the drag nor be mistaken for its completion.
fn releases_field_pan(grab: &crate::input::grab::Grab, event_button: u32, released: bool) -> bool {
    released
        && matches!(
            grab,
            crate::input::grab::Grab::Pan { button, .. } if *button == event_button
        )
}

fn forward_pointer_button(intercepted: bool, finishing_client_move: bool) -> bool {
    !intercepted || finishing_client_move
}

/// Observe an outside left press for opted-in mapped layer surfaces. The
/// ordinary routing path still owns and delivers the press and its release.
fn notify_surface_outside_press<D: SessionDriver>(
    session: &Session<D>,
    route: Option<&crate::input::pointer::PointerRoute>,
    button: u32,
    state: ButtonState,
) {
    if button != BTN_LEFT || state != ButtonState::Pressed {
        return;
    }
    let focused = session
        .keyboard_handle()
        .current_focus()
        .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()));
    for output in session.wayland.space.outputs() {
        let map = smithay::desktop::layer_map_for_output(output);
        for layer in map.layers() {
            let owns_keyboard = focused.as_ref().is_some_and(|focused| {
                focused == layer.wl_surface()
                    || smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface())
                        .any(|(popup, _)| popup.wl_surface() == focused)
            });
            let inside = matches!(route.map(|route| &route.target),
                Some(crate::input::pointer::PointerTarget::Layer(target)) if target == layer);
            if owns_keyboard
                && !inside
                && !session.wayland.unmapped_layers.contains(layer.wl_surface())
            {
                wayland::surface_dismissal::outside_press(layer.wl_surface());
            }
        }
    }
}

// Retire the previous press before any input interception can return early.
// Only the final client-forwarding path may arm a new press or use this release.
fn take_steam_close_press<T>(pending: &mut Option<T>, button: u32) -> Option<T> {
    if button == BTN_LEFT {
        pending.take()
    } else {
        None
    }
}

fn forwarded_steam_close_button<T: PartialEq>(
    pending: &mut Option<T>,
    previous: Option<T>,
    button: u32,
    state: ButtonState,
    target: Option<T>,
) -> Option<T> {
    if button != BTN_LEFT {
        return None;
    }
    match state {
        ButtonState::Pressed => {
            *pending = target;
            None
        }
        ButtonState::Released => target.filter(|target| previous.as_ref() == Some(target)),
    }
}

fn steam_client_close_target(
    route: Option<&crate::input::pointer::PointerRoute>,
) -> Option<Window> {
    let route = route?;
    let crate::input::pointer::PointerTarget::Window(window) = &route.target else {
        return None;
    };
    let (_, surface_origin) = route.focus.as_ref()?;
    let local = route.location - *surface_origin;
    crate::xwayland::is_steam_client_close_hit(window, local).then(|| window.clone())
}

fn shortcut_policy_allows_bindings(session_locked: bool, inhibitor_active: bool) -> bool {
    !session_locked && !inhibitor_active
}

pub(super) fn bindings_enabled<D: SessionDriver>(session: &Session<D>) -> bool {
    let focus = wayland::focus::current(
        &session.wayland,
        &session.fullscreen,
        crate::frame_clock::monotonic_now(),
    );
    let inhibitor_active = focus
        .map(|focus| focus.surface())
        .and_then(|surface| {
            session
                .seat
                .keyboard_shortcuts_inhibitor_for_surface(&surface)
        })
        .is_some_and(|inhibitor| inhibitor.is_active());
    // Exclusive layer focus owns forwarded keys, but does not itself inhibit
    // compositor shortcuts. Lock-screen isolation and explicit client
    // inhibitors remain authoritative.
    shortcut_policy_allows_bindings(session.session_lock.active(), inhibitor_active)
}

pub(super) fn binding_context_for_output<D: SessionDriver>(
    _session: &Session<D>,
    output_name: Option<&str>,
) -> crate::input::BindingContext {
    let Some(_output_name) = output_name else {
        return crate::input::BindingContext::default();
    };
    crate::input::BindingContext::field()
}

pub(super) fn keyboard_binding_context<D: SessionDriver>(
    session: &Session<D>,
) -> crate::input::BindingContext {
    let selected =
        crate::wayland::focus::selected_output(&session.wayland).map(|output| output.name());
    let output = selected;
    binding_context_for_output(session, output.as_deref())
}

fn dispatch_pointer_grab_action<D: SessionDriver>(
    session: &mut Session<D>,
    action: &swm_config::Action,
    route: Option<&crate::input::pointer::PointerRoute>,
    button: u32,
    serial: smithay::utils::Serial,
) -> bool {
    match action {
        swm_config::Action::PointerMoveWindow => {
            let Some(route) = route else {
                return false;
            };
            if pointer_move_falls_back_to_field_pan(&route.target) {
                return begin_field_pan(session, route, button, serial);
            }
            let window = match &route.target {
                crate::input::pointer::PointerTarget::Window(window)
                | crate::input::pointer::PointerTarget::Decoration { window, .. } => window,
                _ => return false,
            };
            if crate::window::accepts_popup_move(window) {
                let geometry = route.visual_geometry.unwrap_or_else(|| window.geometry());
                let pointer = session.pointer.position();
                session
                    .interactions
                    .begin_grab(crate::input::grab::Grab::MovePopup {
                        window: window.clone(),
                        button,
                        offset: (
                            f64::from(geometry.loc.x) - pointer.0,
                            f64::from(geometry.loc.y) - pointer.1,
                        )
                            .into(),
                    });
                session.cursor.set_override(
                    crate::cursor::OverrideSource::Grab,
                    Some(smithay::input::pointer::CursorIcon::Grabbing),
                );
                return true;
            }
            if !crate::window::accepts_compositor_grab(window)
                || window.wl_surface().is_some_and(|surface| {
                    session
                        .fullscreen
                        .is_fullscreen_or_pending(surface.as_ref())
                })
            {
                return false;
            }
            wayland::focus::select_output(&mut session.wayland, &route.output);
            super::begin_pointer_move(session, window, serial, button)
        }
        swm_config::Action::PointerResizeWindow => {
            let Some(route) = route else {
                return false;
            };
            let (window, border_handle) = match &route.target {
                crate::input::pointer::PointerTarget::Window(window) => (window, None),
                crate::input::pointer::PointerTarget::Decoration {
                    window,
                    hit: crate::window_chrome::Hit::Resize(handle),
                } => (window, Some(*handle)),
                _ => return false,
            };
            if !crate::window::accepts_compositor_grab(window)
                || window.wl_surface().is_some_and(|surface| {
                    session
                        .fullscreen
                        .is_fullscreen_or_pending(surface.as_ref())
                })
            {
                return false;
            }
            let Some(start_rect) = session.wayland.space.element_geometry(window) else {
                return false;
            };
            let world = swm_core::field::Vec2 {
                x: route.location.x as f32,
                y: route.location.y as f32,
            };
            let handle = border_handle.unwrap_or_else(|| {
                crate::input::grab::handle_from_press_position(start_rect, world)
            });
            wayland::focus::select_output(&mut session.wayland, &route.output);
            super::begin_window_resize(
                session,
                window,
                handle,
                button,
                world,
                route.visual_geometry.unwrap_or(start_rect),
                serial,
            )
        }
        swm_config::Action::PointerPanField => {
            let Some(route) = route else {
                return false;
            };
            begin_field_pan(session, route, button, serial)
        }
        swm_config::Action::PointerPanFieldAlways => {
            let Some(route) = route else {
                return false;
            };
            begin_field_pan_always(session, route, button, serial)
        }
        swm_config::Action::PointerDragPan => {
            let Some(route) = route else {
                return false;
            };
            let window = match &route.target {
                crate::input::pointer::PointerTarget::Window(window)
                | crate::input::pointer::PointerTarget::Decoration { window, .. } => window,
                _ => return false,
            };
            if !crate::window::accepts_compositor_grab(window)
                || window.wl_surface().is_some_and(|surface| {
                    session
                        .fullscreen
                        .is_fullscreen_or_pending(surface.as_ref())
                })
            {
                return false;
            }
            wayland::focus::select_output(&mut session.wayland, &route.output);
            super::begin_pointer_edge_pan(session, window, serial, button)
        }
        _ => false,
    }
}

fn pointer_move_falls_back_to_field_pan(target: &crate::input::pointer::PointerTarget) -> bool {
    matches!(target, crate::input::pointer::PointerTarget::Background)
}

fn begin_field_pan<D: SessionDriver>(
    session: &mut Session<D>,
    route: &crate::input::pointer::PointerRoute,
    button: u32,
    serial: smithay::utils::Serial,
) -> bool {
    if !matches!(
        &route.target,
        crate::input::pointer::PointerTarget::Background
    ) || node_at_pointer(session).is_some()
    {
        return false;
    }
    if !start_field_pan_grab(session, route, button) {
        return false;
    }
    wayland::focus::select_output(&mut session.wayland, &route.output);
    super::focus::focus_layer(session, None, serial);
    true
}

/// Layer-shell surfaces beneath the windows (wallpaper daemons, desktop
/// widgets) are part of the desktop surface, so `pan-field-always` pans over
/// them. Panels, launchers, notifications, and lock screens live on the Top and
/// Overlay layers: they are screen-fixed UI that must keep their own clicks.
fn layer_allows_field_pan_always(layer: smithay::wayland::shell::wlr_layer::Layer) -> bool {
    use smithay::wayland::shell::wlr_layer::Layer;
    matches!(layer, Layer::Background | Layer::Bottom)
}

/// Windows `pan-field-always` may begin over. It follows `drag-pan`'s policy
/// for a window the compositor may grab (X11 override-redirect menus and
/// tooltips are excluded) and additionally leaves fullscreen windows, including
/// ones whose fullscreen request is still pending, entirely to the client.
fn window_allows_field_pan_always<D: SessionDriver>(session: &Session<D>, window: &Window) -> bool {
    crate::window::accepts_compositor_grab(window)
        && !crate::xwayland::is_fullscreen(window)
        && !window.wl_surface().is_some_and(|surface| {
            session
                .fullscreen
                .is_fullscreen_or_pending(surface.as_ref())
        })
}

/// Starts a camera pan for `pan-field-always` (works over windows, nodes, and desktop layers;
/// refuses without side effects when fullscreen or pointer locks own the target).
fn begin_field_pan_always<D: SessionDriver>(
    session: &mut Session<D>,
    route: &crate::input::pointer::PointerRoute,
    button: u32,
    serial: smithay::utils::Serial,
) -> bool {
    let target_allowed = match &route.target {
        crate::input::pointer::PointerTarget::Background => true,
        crate::input::pointer::PointerTarget::Layer(layer) => {
            layer_allows_field_pan_always(layer.layer())
        }
        crate::input::pointer::PointerTarget::Window(window)
        | crate::input::pointer::PointerTarget::Decoration { window, .. } => {
            window_allows_field_pan_always(session, window)
        }
    };
    if !target_allowed {
        return false;
    }
    // Fullscreen and Field maximize own the camera. Starting a grab the motion
    // handler cannot honor would only eat the click.
    if !session.cameras.is_pannable(&route.output.name()) {
        return false;
    }
    // A client that is mid-drag or holds a pointer lock/confinement keeps the
    // pointer; a pan started now would freeze or fight it.
    if session
        .seat
        .get_pointer()
        .is_some_and(|pointer| pointer.is_grabbed())
        || super::pointer::has_active_constraint(session)
    {
        return false;
    }
    if !start_field_pan_grab(session, route, button) {
        return false;
    }
    wayland::focus::select_output(&mut session.wayland, &route.output);
    // Dragging empty background drops layer keyboard focus, exactly as
    // `pan-field` does. Over a window the drag is navigation, not interaction,
    // so keyboard focus stays where it was.
    if matches!(
        &route.target,
        crate::input::pointer::PointerTarget::Background
    ) {
        super::focus::focus_layer(session, None, serial);
    }
    true
}

fn start_field_pan_grab<D: SessionDriver>(
    session: &mut Session<D>,
    route: &crate::input::pointer::PointerRoute,
    button: u32,
) -> bool {
    if !session
        .interactions
        .grab_mut()
        .try_begin_pan(route.output.name(), button)
    {
        return false;
    }
    session.cursor.set_override(
        crate::cursor::OverrideSource::Grab,
        Some(smithay::input::pointer::CursorIcon::Grabbing),
    );
    true
}

pub(crate) fn tick_grabbed_window_edge_pan<D: SessionDriver>(
    session: &mut Session<D>,
    now: std::time::Duration,
) -> Option<String> {
    let (id, window, drag_size, anchor, edge_pan) = match session.interactions.grab() {
        crate::input::grab::Grab::MoveWindow {
            id: Some(id),
            window,
            drag_size,
            anchor,
            edge_pan: Some(edge_pan),
            ..
        } => (*id, window.clone(), *drag_size, *anchor, edge_pan.clone()),
        _ => return None,
    };
    let output = session
        .wayland
        .space
        .outputs()
        .find(|output| output.name() == edge_pan.output)?
        .clone();
    let output_geometry = session.wayland.space.output_geometry(&output)?;
    let camera = *session.cameras.get(&edge_pan.output)?;
    let size = drag_size.unwrap_or_else(|| {
        session
            .wayland
            .space
            .element_geometry(&window)
            .map(|geometry| geometry.size)
            .unwrap_or((1, 1).into())
    });
    let frame_extents = crate::window_chrome::frame_extents(&window, &session.settings.decorations);
    let placement = crate::input::grab::window_edge_pan_placement(
        session.pointer.position(),
        anchor,
        size,
        frame_extents,
        &camera,
        output_geometry,
    );
    let Some(placement) = placement else {
        if let crate::input::grab::Grab::MoveWindow {
            edge_pan: Some(live),
            ..
        } = session.interactions.grab_mut()
        {
            live.update_contact(swm_core::field::Vec2 { x: 0.0, y: 0.0 }, now);
            live.last_tick = now;
        }
        return None;
    };

    let (ready, dt, contact) = match session.interactions.grab_mut() {
        crate::input::grab::Grab::MoveWindow {
            edge_pan: Some(live),
            last_world,
            last_update,
            velocity,
            ..
        } => {
            live.update_contact(placement.contact, now);
            let ready = live.ready(now);
            let dt = now
                .saturating_sub(live.last_tick)
                .as_secs_f32()
                .min(1.0 / 20.0);
            live.last_tick = now;
            *last_world = placement.center;
            *last_update = now;
            *velocity = swm_core::field::Vec2 { x: 0.0, y: 0.0 };
            (ready, dt, live.contact)
        }
        _ => return None,
    };
    if contact.x == 0.0 && contact.y == 0.0 {
        return None;
    }

    if !session.nodes.physics_enabled() {
        let _ = crate::nodes::move_grabbed_body_rigid(session, id, placement.center);
    }
    if ready && dt > 0.0 {
        let speed = crate::input::grab::WINDOW_EDGE_PAN_SPEED_PXPS
            / crate::presentation::camera::scale(&camera).max(0.05);
        if let Some(camera) = session.cameras.get_mut(&edge_pan.output) {
            camera.pan_target(swm_core::field::Vec2 {
                x: contact.x * speed * dt,
                y: contact.y * speed * dt,
            });
        }
    }
    Some(edge_pan.output)
}

pub(crate) fn grabbed_window_edge_pan_active_on<D: SessionDriver>(
    session: &Session<D>,
    output_name: &str,
) -> bool {
    matches!(
        session.interactions.grab(),
        crate::input::grab::Grab::MoveWindow {
            edge_pan: Some(edge_pan),
            ..
        } if edge_pan.output == output_name
            && (edge_pan.contact.x != 0.0 || edge_pan.contact.y != 0.0)
    )
}

fn apply_active_resize<D: SessionDriver>(session: &mut Session<D>) -> bool {
    let crate::input::grab::Grab::ResizeWindow(state) = session.interactions.grab() else {
        return false;
    };
    let state = state.clone();
    let size = crate::input::grab::resize_preview_size(&state);
    if let Some(toplevel) = state.window.toplevel() {
        toplevel.with_pending_state(|pending| pending.size = Some(size));
        let serial = toplevel.send_pending_configure();
        crate::input::grab::note_resize_configure(&mut session.interactions.resize_anchor, serial);
    } else {
        let location = crate::input::grab::resize_location_after_commit(
            state.handle,
            state.start_rect.loc,
            state.start_rect.size,
            size,
        );
        let target = Rectangle::new(location, size);
        crate::xwayland::configure_window(session, &state.window, target);
    }
    true
}

pub(crate) fn wakeup_smooth_resize<D: SessionDriver>(
    session: &mut Session<D>,
    now: std::time::Duration,
) -> bool {
    let animations = &session.settings.animations;
    let changed = match session.interactions.grab_mut() {
        crate::input::grab::Grab::ResizeWindow(state) => {
            crate::input::grab::advance_resize_preview(
                state,
                now,
                animations.enabled && animations.smooth_resize.enabled,
                animations.smooth_resize.duration_ms,
            )
        }
        _ => false,
    };
    changed && apply_active_resize(session)
}

fn focus_output_target<D: SessionDriver>(
    session: &mut Session<D>,
    target: swm_config::MonitorTarget,
) {
    let target = match target {
        swm_config::MonitorTarget::Direction(direction) => {
            let Some(current) = crate::wayland::focus::selected_output(&session.wayland).cloned()
            else {
                return;
            };
            let Some(target) =
                crate::wayland::focus::adjacent_output(&session.wayland.space, &current, direction)
            else {
                return;
            };
            target
        }
        swm_config::MonitorTarget::Output(name) => {
            let Some(target) = session
                .wayland
                .space
                .outputs()
                .find(|output| output.name() == name)
                .cloned()
            else {
                eventline::warn!("monitor focus: unknown output {name:?}");
                return;
            };
            target
        }
    };
    crate::wayland::focus::select_output(&mut session.wayland, &target);

    let target_id = session.nodes.focused_on_output(&target.name());
    let target_record = target_id.and_then(|id| session.nodes.record(id)).cloned();
    match (target_id, target_record) {
        (Some(id), Some(record)) if record.collapsed => {
            crate::window::clear_focus(&mut session.wayland);
            session
                .nodes
                .focus(Some(id), session.start_time.elapsed().as_millis() as u64);
            super::sync_keyboard_focus(session, SERIAL_COUNTER.next_serial());
        }
        (_, Some(record)) => {
            super::focus_window(session, &record.window, SERIAL_COUNTER.next_serial());
        }
        _ => {
            crate::window::clear_focus(&mut session.wayland);
            session
                .nodes
                .focus(None, session.start_time.elapsed().as_millis() as u64);
            super::sync_keyboard_focus(session, SERIAL_COUNTER.next_serial());
        }
    }
    session.request_redraw();
}

fn toggle_focused_node<D: SessionDriver>(
    session: &mut Session<D>,
    output_name: Option<&str>,
    serial: smithay::utils::Serial,
) {
    crate::nodes::toggle_focused_on_output(session, output_name, serial);
}

pub fn handle<D, B>(session: &mut Session<D>, event: &InputEvent<B>, socket_name: &OsStr)
where
    D: SessionDriver,
    B: InputBackend,
{
    if !crate::input::virtual_keyboard::restore(session) {
        return;
    }
    let steam_close_press = match event {
        InputEvent::PointerButton { event } => take_steam_close_press(
            &mut session.interactions.steam_close_pressed,
            event.button_code(),
        ),
        InputEvent::DeviceRemoved { .. } => {
            session.interactions.steam_close_pressed = None;
            None
        }
        _ => None,
    };
    if is_user_activity(event) {
        let seat = session.seat.clone();
        session.idle_notifier_state.notify_activity(&seat);
    }
    if session.session_lock.active() {
        session.interactions.steam_close_pressed = None;
        crate::wayland::session_lock::handle_input(session, event);
        return;
    }
    if let InputEvent::PointerButton { event } = event
        && event.state() == ButtonState::Released
        && session
            .interactions
            .overlay_buttons
            .release_is_suppressed(event.button_code())
    {
        return;
    }
    if modal::intercept_early(session, event) {
        return;
    }
    if super::touch::handle(session, event) || super::gesture::handle(session, event) {
        return;
    }
    let position_before = session.pointer.position();
    if matches!(
        event,
        InputEvent::PointerMotion { .. }
            | InputEvent::PointerMotionAbsolute { .. }
            | InputEvent::PointerButton { .. }
            | InputEvent::PointerAxis { .. }
    ) && session.cursor_policy.pointer_activity()
    {
        session.request_redraw();
    }
    let pointer_handle = session.pointer_handle();
    super::pointer::finish_frame(session, &pointer_handle);
    session
        .pointer
        .process_input_event(event, &session.wayland.space);
    let proposed_position = session.pointer.position();
    // Modal owners claim pointer events in this order before grabs.
    if modal::capture(session, event, proposed_position) {
        return;
    }
    if super::zoom_fit::handle_pointer(session, event) {
        return;
    }
    let Some(position_after) = motion::handle(session, event, position_before, proposed_position)
    else {
        return;
    };

    if matches!(event, InputEvent::PointerButton { .. }) {
        button::handle(
            session,
            event,
            steam_close_press,
            socket_name,
            position_after,
        );
        return;
    }

    if matches!(event, InputEvent::PointerAxis { .. }) {
        axis::handle(session, event, socket_name);
        return;
    }

    if let InputEvent::Keyboard { event: key_event } = event {
        keyboard::handle::<D, B>(session, key_event, socket_name);
    }
}

fn is_user_activity<B: InputBackend>(event: &InputEvent<B>) -> bool {
    !matches!(
        event,
        InputEvent::DeviceAdded { .. }
            | InputEvent::DeviceRemoved { .. }
            | InputEvent::SwitchToggle { .. }
            | InputEvent::Special(_)
    )
}

fn update_capture_pointer<D: SessionDriver>(session: &mut Session<D>, position: (f64, f64)) {
    if !session.capture.is_active() {
        return;
    }
    if session.capture.menu_is_active() {
        session.capture.motion(position);
        return;
    }
    let Some(route) = super::pointer::route_client(session) else {
        return;
    };
    let Some(output_geometry) = session.wayland.space.output_geometry(&route.output) else {
        return;
    };
    let monitor = swm_ipc::CaptureSource::Monitor {
        name: route.output.name(),
        x: output_geometry.loc.x,
        y: output_geometry.loc.y,
        width: output_geometry.size.w,
        height: output_geometry.size.h,
    };
    let window = match route.target {
        crate::input::pointer::PointerTarget::Window(window)
        | crate::input::pointer::PointerTarget::Decoration { window, .. } => {
            window.wl_surface().and_then(|surface| {
                let geometry = session
                    .window_capture_context()
                    .visual_geometry(&window, route.visual_geometry?);
                let size = session.window_capture_context().size(&window);
                Some((
                    swm_ipc::CaptureSource::Window {
                        window_id: session.nodes.id_for_surface(surface.as_ref())?.as_u64(),
                        width: size.w,
                        height: size.h,
                    },
                    geometry,
                ))
            })
        }
        _ => None,
    };
    session
        .capture
        .hover_source(monitor, window, output_geometry);
}

#[cfg(test)]
mod tests;
