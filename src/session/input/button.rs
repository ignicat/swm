use smithay::input::pointer::PointerHandle;
use smithay::utils::Serial;

use super::*;

/// The facts about one button event every stage below reads.
#[derive(Clone, Copy)]
struct Click {
    button: u32,
    state: ButtonState,
    time: u32,
    serial: Serial,
    /// Pointer position after this frame's motion, in global coordinates.
    position_after: (f64, f64),
}

// Resolve press/release ownership before forwarding anything to a client.
//
// The stages run in a fixed order. The first group each either consume the
// event outright (returning `true`, after which the frame is finished) or
// leave it for later stages. The second group pass `intercepted` along: once
// a stage consumes the button, later ones mostly stand aside, and a button
// nobody consumed is forwarded to the client.
pub(super) fn handle<D: SessionDriver, B: InputBackend>(
    session: &mut Session<D>,
    event: &InputEvent<B>,
    steam_close_press: Option<Window>,
    socket_name: &OsStr,
    position_after: (f64, f64),
) {
    let pointer_handle = session.pointer_handle();
    let InputEvent::PointerButton {
        event: button_event,
    } = event
    else {
        return;
    };
    let click = Click {
        button: button_event.button_code(),
        state: button_event.state(),
        time: button_event.time().millis(),
        serial: SERIAL_COUNTER.next_serial(),
        position_after,
    };
    let Click {
        button,
        state,
        time,
        ..
    } = click;
    let finish = |session: &mut Session<D>| {
        super::super::pointer::finish_frame(session, &pointer_handle);
    };

    if finish_popup_move(session, click) {
        return finish(session);
    }
    release_unclaimed_client_move(session, click);
    if finish_left_release_drag(session, click) {
        return finish(session);
    }
    let mut route = super::super::pointer::route_for_discrete_input(session, time);
    if route_through_popup_grab(session, click, &pointer_handle, &mut route) {
        return finish(session);
    }
    notify_surface_outside_press(session, route.as_ref(), button, state);
    if decoration_pointer_grab(session, click, &route)
        || border_resize_click(session, click, &route)
    {
        return finish(session);
    }
    if button == BTN_LEFT
        && state == ButtonState::Pressed
        && let Some(route) = route.as_ref()
    {
        wayland::focus::select_output(&mut session.wayland, &route.output);
    }
    let mut intercepted = false;
    let modifiers = session.keyboard_handle().modifier_state();
    intercepted = pointer_binding(session, click, intercepted, socket_name, &route, modifiers);
    intercepted = node_press(session, click, intercepted, modifiers);
    let (consumed, finishing_client_move) = field_left_button(session, click, intercepted, &route);
    intercepted = consumed;
    intercepted = release_ends_grab(session, click, intercepted);
    forward_to_client(
        session,
        click,
        &pointer_handle,
        intercepted,
        finishing_client_move,
        steam_close_press,
        &route,
    );
    finish(session);
}

/// A compositor-driven popup move owns every button until its own button
/// is released.
fn finish_popup_move<D: SessionDriver>(session: &mut Session<D>, click: Click) -> bool {
    let Click {
        button,
        state,
        time,
        ..
    } = click;
    if let crate::input::grab::Grab::MovePopup { button: owner, .. } = session.interactions.grab() {
        if *owner == button && state == ButtonState::Released {
            session.end_grab();
            super::super::pointer::update_client_state(session, time);
            session.request_redraw();
        }
        // The app received neither the initiating press nor any held motion.
        return true;
    }
    false
}

/// A client-owned pending move that never became a compositor move leaves
/// its release to the client.
fn release_unclaimed_client_move<D: SessionDriver>(session: &mut Session<D>, click: Click) {
    let Click { button, state, .. } = click;
    if matches!(
        session.interactions.grab(),
        crate::input::grab::Grab::PendingWindowMove(pending)
            if pending.client_owned && releases_pending_window_move(
                pending.button,
                button,
                state == ButtonState::Released,
            )
    ) {
        // No compositor move began, so leave this release unconsumed. It
        // completes the client's click and lets its double-click handler
        // decide whether to toggle maximize.
        session.end_grab();
    }
}

/// Left releases that end a node press or drag.
fn finish_left_release_drag<D: SessionDriver>(session: &mut Session<D>, click: Click) -> bool {
    let Click {
        button,
        state,
        serial,
        ..
    } = click;
    if button == BTN_LEFT && state == ButtonState::Released {
        match session.interactions.grab() {
            crate::input::grab::Grab::PendingNode { id, .. } => {
                let id = *id;
                session.end_grab();
                let _ = crate::nodes::restore(session, id, serial);
                return true;
            }
            crate::input::grab::Grab::MoveNode { id, .. } => {
                let id = *id;
                let now = crate::frame_clock::monotonic_now();
                if session.nodes.physics_enabled() {
                    let _ = crate::nodes::tick_physics(session, now);
                }
                session.end_grab();
                session.nodes.clear_direct_motion(id);
                session.request_redraw();
                return true;
            }
            _ => {}
        }
    }
    false
}

/// While a native popup grab is active the popup owns presses inside it.
/// An outside press ends the grab and re-routes the click to what is under
/// the pointer instead.
fn route_through_popup_grab<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    pointer_handle: &PointerHandle<Session<D>>,
    route: &mut Option<crate::input::pointer::PointerRoute>,
) -> bool {
    let Click {
        button,
        state,
        time,
        serial,
        ..
    } = click;
    let native_popup_grab = pointer_handle
        .with_grab(|_, grab| {
            grab.downcast_ref::<smithay::desktop::PopupPointerGrab<Session<D>>>()
                .is_some()
        })
        .unwrap_or(false);
    if native_popup_grab && let Some(mut grab) = session.popup_grab.take() {
        let popup_roots = grab
            .pointer_grab_start_data()
            .focus
            .as_ref()
            .map(|(root, _)| {
                smithay::desktop::PopupManager::popups_for_surface(root)
                    .map(|(popup, _)| popup.wl_surface().clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let target_root = route
            .as_ref()
            .and_then(|route| route.focus.as_ref())
            .map(|(surface, _)| wayland::compositor::root_surface(surface));
        if !grab.has_ended()
            && wayland::popup::outside_popup_press(
                state == ButtonState::Pressed,
                target_root.as_ref(),
                &popup_roots,
            )
        {
            grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
            // Motion retires Smithay's ended pointer/keyboard grabs and
            // restores the actual outside target before normal dispatch.
            // Do not deliver a synthetic button or consume the real click.
            *route = super::super::pointer::route_for_motion(session, time);
            session.request_redraw();
        } else if !grab.has_ended() {
            session.popup_grab = Some(grab);
            // The popup owns this event, including releases. Compositor
            // bindings and desktop interactions must not steal it.
            pointer_handle.button(
                session,
                &ButtonEvent {
                    serial,
                    time: smithay::backend::input::InputTime::from_millis(time),
                    button,
                    state,
                },
            );
            return true;
        }
    } else if !native_popup_grab {
        session.popup_grab = None;
    }
    false
}

/// A move, resize or pan chord pressed on a server-side decoration starts
/// its grab before the decoration sees the press.
fn decoration_pointer_grab<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    route: &Option<crate::input::pointer::PointerRoute>,
) -> bool {
    let Click {
        button,
        state,
        serial,
        ..
    } = click;
    if state == ButtonState::Pressed
        && matches!(
            route.as_ref().map(|route| &route.target),
            Some(crate::input::pointer::PointerTarget::Decoration { .. })
        )
        && bindings_enabled(session)
    {
        let modifiers = session.keyboard_handle().modifier_state();
        let context = binding_context_for_output(
            session,
            route.as_ref().map(|route| route.output.name()).as_deref(),
        );
        let action = match_pointer_bind(
            &session.keyboard.binds,
            &modifiers,
            session.keyboard.side_modifiers,
            context,
            button,
        );
        if action.as_ref().is_some_and(|action| {
            matches!(
                action,
                swm_config::Action::PointerMoveWindow
                    | swm_config::Action::PointerResizeWindow
                    | swm_config::Action::PointerPanFieldAlways
            )
        }) && dispatch_pointer_grab_action(
            session,
            action.as_ref().expect("checked above"),
            route.as_ref(),
            button,
            serial,
        ) {
            return true;
        }
    }
    false
}

/// Left presses on compositor window borders: focus and start resizing.
fn border_resize_click<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    route: &Option<crate::input::pointer::PointerRoute>,
) -> bool {
    let Click {
        button,
        state,
        serial,
        ..
    } = click;
    if button == BTN_LEFT
        && state == ButtonState::Pressed
        && let Some(crate::input::pointer::PointerRoute {
            target:
                crate::input::pointer::PointerTarget::Decoration {
                    window,
                    hit: crate::window_chrome::Hit::Resize(handle),
                },
            location,
            visual_geometry,
            ..
        }) = route.as_ref()
    {
        super::super::focus::focus_window_from_pointer(session, window, serial);
        let world = swm_core::field::Vec2 {
            x: location.x as f32,
            y: location.y as f32,
        };
        let visual_geometry = (*visual_geometry)
            .or_else(|| session.wayland.space.element_geometry(window))
            .unwrap_or_else(|| window.geometry());
        let _ = super::super::begin_window_resize(
            session,
            window,
            *handle,
            button,
            world,
            visual_geometry,
            serial,
        );
        session.request_redraw();
        return true;
    }
    false
}

/// Pointer-button keybinds, including move, resize and pan grabs.
fn pointer_binding<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    intercepted: bool,
    socket_name: &OsStr,
    route: &Option<crate::input::pointer::PointerRoute>,
    modifiers: smithay::input::keyboard::ModifiersState,
) -> bool {
    let Click {
        button,
        state,
        serial,
        ..
    } = click;
    let mut intercepted = intercepted;
    let bindings_enabled = bindings_enabled(session);
    let binding_context = binding_context_for_output(
        session,
        route.as_ref().map(|route| route.output.name()).as_deref(),
    );
    if !intercepted {
        match process_pointer_binding(
            &session.keyboard.binds,
            crate::input::PointerBindingInput {
                mods: &modifiers,
                sides: session.keyboard.side_modifiers,
                context: binding_context,
                button,
                state,
                bindings_enabled,
            },
            &mut session.interactions.suppressed_buttons,
        ) {
            PointerBindingResult::Action(action) => {
                let pointer_grab = matches!(
                    action,
                    swm_config::Action::PointerMoveWindow
                        | swm_config::Action::PointerResizeWindow
                        | swm_config::Action::PointerPanField
                        | swm_config::Action::PointerPanFieldAlways
                        | swm_config::Action::PointerDragPan
                );
                let handled = if pointer_grab {
                    dispatch_pointer_grab_action(session, &action, route.as_ref(), button, serial)
                } else {
                    let output_name = route.as_ref().map(|route| route.output.name().to_string());
                    actions::dispatch(
                        session,
                        action,
                        socket_name,
                        output_name.as_deref(),
                        actions::DispatchOrigin::Other,
                    );
                    true
                };
                if pointer_grab || !handled {
                    session
                        .interactions
                        .suppressed_buttons
                        .release_is_suppressed(button);
                }
                intercepted = handled;
            }
            PointerBindingResult::SuppressedRelease => intercepted = true,
            PointerBindingResult::Unhandled => {}
        }
    }
    intercepted
}

/// A left press on a collapsed node: Mod moves it, a plain press waits to
/// see whether it becomes a click.
fn node_press<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    intercepted: bool,
    modifiers: smithay::input::keyboard::ModifiersState,
) -> bool {
    let Click {
        button,
        state,
        serial,
        position_after,
        ..
    } = click;
    let mut intercepted = intercepted;
    if !intercepted
        && button == BTN_LEFT
        && state == ButtonState::Pressed
        && let Some((id, output)) = node_at_pointer(session)
        && let Some(record) = session.nodes.record(id).cloned()
        && let Some(node_position) = session.nodes.field().node(id).map(|node| node.pos)
        && let Some(output_geometry) = session.wayland.space.output_geometry(&output)
        && let Some(camera) = session.cameras.get(&output.name())
    {
        wayland::focus::select_output(&mut session.wayland, &output);
        let center = crate::nodes::screen_from_world(node_position, camera, output_geometry);
        let screen_offset = swm_core::field::Vec2 {
            x: center.x as f32 - position_after.0 as f32,
            y: center.y as f32 - position_after.1 as f32,
        };
        session
            .nodes
            .clear_hover(crate::frame_clock::monotonic_now());
        super::super::focus::focus_node_from_pointer(session, id, &output, serial);
        let mod_held = crate::input::mod_key_held(
            &modifiers,
            session.keyboard.side_modifiers,
            session.keyboard.effective_mod,
        );
        if mod_held {
            if !crate::session::node_user_pinned(session, id) {
                session.nodes.clear_direct_motion(id);
                session
                    .interactions
                    .begin_grab(crate::input::grab::Grab::MoveNode {
                        id,
                        surface: record.surface,
                        screen_offset,
                        last_world: node_position,
                        last_update: crate::frame_clock::monotonic_now(),
                        velocity: swm_core::field::Vec2 { x: 0.0, y: 0.0 },
                    });
                session.cursor.set_override(
                    crate::cursor::OverrideSource::Grab,
                    Some(smithay::input::pointer::CursorIcon::Grabbing),
                );
            }
        } else {
            session
                .interactions
                .begin_grab(crate::input::grab::Grab::PendingNode {
                    id,
                    surface: record.surface,
                    press_screen: Point::<f64, Logical>::from(position_after),
                    screen_offset,
                });
        }
        intercepted = true;
    }
    intercepted
}

/// Unclaimed left presses focus what they hit; the release ends a window
/// move. Returns whether the button was consumed and whether a client-owned
/// move just finished.
fn field_left_button<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    intercepted: bool,
    route: &Option<crate::input::pointer::PointerRoute>,
) -> (bool, bool) {
    let Click {
        button,
        state,
        serial,
        ..
    } = click;
    let mut intercepted = intercepted;
    let mut finishing_client_move = false;
    if !intercepted && button == BTN_LEFT {
        match state {
            ButtonState::Pressed => match route.as_ref().map(|route| &route.target) {
                Some(crate::input::pointer::PointerTarget::Window(window)) => {
                    super::super::focus::focus_window_from_pointer(session, window, serial);
                }
                Some(crate::input::pointer::PointerTarget::Layer(layer)) => {
                    super::super::focus::focus_layer(session, Some(layer.clone()), serial);
                }
                Some(crate::input::pointer::PointerTarget::Decoration { .. }) => {
                    intercepted = true;
                }
                Some(crate::input::pointer::PointerTarget::Background) => {
                    super::super::focus::focus_layer(session, None, serial);
                }
                None => {}
            },
            ButtonState::Released => {
                let released_window = match session.interactions.grab() {
                    crate::input::grab::Grab::MoveWindow {
                        id: Some(id),
                        button: move_button,
                        client_owned,
                        ..
                    } if *move_button == button => Some((*id, *client_owned)),
                    _ => None,
                };
                if let Some((id, client_owned)) = released_window {
                    finishing_client_move = client_owned;
                    let now = crate::frame_clock::monotonic_now();
                    if session.nodes.physics_enabled() {
                        let _ = crate::nodes::tick_physics(session, now);
                    }
                    session.end_grab();
                    session.nodes.clear_direct_motion(id);
                    if session.nodes.physics_enabled() {
                        session.nodes.lock_released_window(id, now);
                    }
                    session.request_redraw();
                    intercepted = true;
                }
            }
        }
    }
    (intercepted, finishing_client_move)
}

/// The release of the button that began a camera pan or a window resize
/// ends it.
fn release_ends_grab<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    intercepted: bool,
) -> bool {
    let Click { button, state, .. } = click;
    let mut intercepted = intercepted;
    // A camera pan ends with the release of the button that began it, which
    // may be any pointer button a `pan-field`/`pan-field-always` chord uses.
    if !intercepted
        && releases_field_pan(
            session.interactions.grab(),
            button,
            state == ButtonState::Released,
        )
    {
        session.end_grab();
        intercepted = true;
    }

    if !intercepted
        && state == ButtonState::Released
        && matches!(
            session.interactions.grab(),
            crate::input::grab::Grab::ResizeWindow(resize) if resize.button == button
        )
    {
        crate::input::grab::release_resize_anchor(&mut session.interactions.resize_anchor);
        session.end_grab();
        intercepted = true;
    }
    intercepted
}

/// Deliver the button to the client under the pointer, unless a stage
/// above consumed it.
fn forward_to_client<D: SessionDriver>(
    session: &mut Session<D>,
    click: Click,
    pointer_handle: &PointerHandle<Session<D>>,
    intercepted: bool,
    finishing_client_move: bool,
    steam_close_press: Option<Window>,
    route: &Option<crate::input::pointer::PointerRoute>,
) {
    let Click {
        button,
        state,
        time,
        serial,
        ..
    } = click;
    if forward_pointer_button(intercepted, finishing_client_move) {
        if let Some(window) = forwarded_steam_close_button(
            &mut session.interactions.steam_close_pressed,
            steam_close_press,
            button,
            state,
            steam_client_close_target(route.as_ref()),
        ) {
            super::super::closing::start_steam_client_close_control(session, &window);
        }
        let had_click_grab = super::super::pointer::client_click_grab_active(session);
        pointer_handle.button(
            session,
            &ButtonEvent {
                serial,
                time: smithay::backend::input::InputTime::from_millis(time),
                button,
                state,
            },
        );
        if had_click_grab && !super::super::pointer::client_click_grab_active(session) {
            // The last release goes to the owner, then normal hit-testing
            // resumes even if the physical pointer stops moving.
            super::super::pointer::route_for_motion(session, time);
        }
    }
}
