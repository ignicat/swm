use super::*;

// Applies constraints and routes motion to the current grab or client.
// Returning `None` means the motion owner consumed the event before discrete dispatch.
pub(super) fn handle<D: SessionDriver, B: InputBackend>(
    session: &mut Session<D>,
    event: &InputEvent<B>,
    position_before: (f64, f64),
    proposed_position: (f64, f64),
) -> Option<(f64, f64)> {
    let pointer_handle = session.pointer_handle();
    let motion = match event {
        InputEvent::PointerMotion { event } => Some((
            event.delta(),
            event.delta_unaccel(),
            event.time(),
            event.time().millis(),
        )),
        InputEvent::PointerMotionAbsolute { event } => {
            let delta = Point::<f64, Logical>::from((
                proposed_position.0 - position_before.0,
                proposed_position.1 - position_before.1,
            ));
            Some((delta, delta, event.time(), event.time().millis()))
        }
        _ => None,
    };
    let constrained_motion = super::super::pointer::constrain_motion(session, &pointer_handle);

    if let super::super::pointer::ConstrainedMotion::RelativeOnly { surface, origin } =
        &constrained_motion
        && let Some((delta, delta_unaccel, time, _)) = motion
    {
        session.pointer.set_position(position_before);
        crate::session::trace::surface_sampled_event(
            session,
            surface,
            "locked-relative-motion",
            format_args!(
                "delta={delta:?} delta_unaccel={delta_unaccel:?} anchor={position_before:?} origin={origin:?}"
            ),
        );
        pointer_handle.relative_motion(
            session,
            Some((surface.clone(), *origin)),
            &RelativeMotionEvent {
                delta,
                delta_unaccel,
                time,
            },
        );
        super::super::pointer::finish_frame(session, &pointer_handle);
        return None;
    }

    if let super::super::pointer::ConstrainedMotion::Clamp(position) = constrained_motion {
        session.pointer.set_position((position.x, position.y));
    }
    if let crate::input::grab::Grab::MoveWindow {
        edge_pan: Some(edge_pan),
        ..
    } = session.interactions.grab()
        && let Some(output) = session
            .wayland
            .space
            .outputs()
            .find(|output| output.name() == edge_pan.output)
        && let Some(geometry) = session.wayland.space.output_geometry(output)
    {
        let position = session.pointer.position();
        let right = f64::from(geometry.loc.x + geometry.size.w) - 0.001;
        let bottom = f64::from(geometry.loc.y + geometry.size.h) - 0.001;
        session.pointer.set_position((
            position.0.clamp(f64::from(geometry.loc.x), right),
            position.1.clamp(f64::from(geometry.loc.y), bottom),
        ));
    }
    let position_after = session.pointer.position();
    session.request_redraw();

    if motion.is_some()
        && let crate::input::grab::Grab::PendingNode {
            id,
            surface,
            press_screen,
            screen_offset,
            ..
        } = session.interactions.grab()
    {
        let dx = position_after.0 - press_screen.x;
        let dy = position_after.1 - press_screen.y;
        if dx.hypot(dy) >= NODE_DRAG_THRESHOLD_PX && !crate::session::node_user_pinned(session, *id)
        {
            let id = *id;
            let surface = surface.clone();
            let screen_offset = *screen_offset;
            if let Some((output, output_geometry)) =
                output_at_pointer(&session.wayland.space, position_after)
                && let Some(camera) = session.cameras.get(&output.name())
            {
                let pointer_world = crate::input::grab::screen_to_world_on_output(
                    position_after,
                    camera,
                    output_geometry,
                );
                let offset = crate::input::grab::screen_offset_to_world(screen_offset, camera);
                let desired = swm_core::field::Vec2 {
                    x: pointer_world.x + offset.x,
                    y: pointer_world.y + offset.y,
                };
                crate::nodes::set_collapsed_output(session, id, &output);
                session.nodes.clear_direct_motion(id);
                session
                    .interactions
                    .begin_grab(crate::input::grab::Grab::MoveNode {
                        id,
                        surface,
                        screen_offset,
                        last_world: desired,
                        last_update: crate::frame_clock::monotonic_now(),
                        velocity: swm_core::field::Vec2 { x: 0.0, y: 0.0 },
                    });
                session.cursor.set_override(
                    crate::cursor::OverrideSource::Grab,
                    Some(smithay::input::pointer::CursorIcon::Grabbing),
                );
            }
        }
    }

    if motion.is_some()
        && let crate::input::grab::Grab::PendingWindowMove(pending) = session.interactions.grab()
    {
        let pending = pending.clone();
        match pending_window_move_motion(
            !pending.client_owned || pointer_handle.has_grab(pending.serial),
            pending.press_screen,
            position_after,
        ) {
            PendingWindowMoveMotion::Wait => {}
            PendingWindowMoveMotion::Cancel => {
                session.end_grab();
            }
            PendingWindowMoveMotion::Activate => {
                // A successful hand-off to the client move keeps the grab
                // cursor, which that move goes on to use.
                session.interactions.end_grab_keeping_cursor();
                if !super::super::activate_client_pointer_move(session, pending) {
                    session
                        .cursor
                        .set_override(crate::cursor::OverrideSource::Grab, None);
                }
            }
        }
    }

    let mut resize_preview_changed = false;
    if motion.is_some()
        && let crate::input::grab::Grab::ResizeWindow(state) = session.interactions.grab_mut()
    {
        let world = crate::input::grab::resize_cursor_from_screen(state, position_after);
        let requested_size = crate::input::grab::resize_target_size(
            state.handle,
            state.start_rect,
            state.start_cursor,
            world,
        );
        state.target_size = if crate::xwayland::is_x11(&state.window) {
            crate::xwayland::constrain_window_size(&state.window, requested_size)
        } else {
            requested_size
        };
        let animations = &session.settings.animations;
        resize_preview_changed = crate::input::grab::advance_resize_preview(
            state,
            crate::frame_clock::monotonic_now(),
            animations.enabled && animations.smooth_resize.enabled,
            animations.smooth_resize.duration_ms,
        );
    }
    match session.interactions.grab() {
        crate::input::grab::Grab::MovePopup { window, offset, .. } if motion.is_some() => {
            let window = window.clone();
            let location: Point<i32, Logical> = (
                (position_after.0 + offset.x).round() as i32,
                (position_after.1 + offset.y).round() as i32,
            )
                .into();
            #[cfg(feature = "xwayland")]
            if let Some(surface) = window.x11_surface() {
                session.xwayland.move_override_redirect(surface, location);
            }
            session.wayland.space.relocate_element(&window, location);
            if let Some((output, _)) = output_at_pointer(&session.wayland.space, position_after) {
                wayland::set_window_output(&window, &output);
            }
            session.request_redraw();
            super::super::pointer::finish_frame(session, &pointer_handle);
            return None;
        }
        crate::input::grab::Grab::MoveWindow {
            id,
            window,
            drag_size,
            anchor,
            edge_pan,
            last_world,
            last_update,
            velocity,
            ..
        } => {
            let id = *id;
            let window = window.clone();
            let drag_size = *drag_size;
            let anchor = *anchor;
            let edge_pan = edge_pan.clone();
            let previous = *last_world;
            let last_update = *last_update;
            let previous_velocity = *velocity;
            let drag_output = edge_pan
                .as_ref()
                .and_then(|edge_pan| {
                    let output = session
                        .wayland
                        .space
                        .outputs()
                        .find(|output| output.name() == edge_pan.output)?
                        .clone();
                    let geometry = session.wayland.space.output_geometry(&output)?;
                    Some((output, geometry))
                })
                .or_else(|| output_at_pointer(&session.wayland.space, position_after));
            if let Some((output, output_geometry)) = drag_output {
                let output_name = output.name();
                let camera = session.cameras.get(&output_name)?;
                let output_changed = edge_pan.is_none()
                    && wayland::window_output_name(&window).as_deref()
                        != Some(output_name.as_str());
                let size = drag_size.unwrap_or_else(|| {
                    session
                        .wayland
                        .space
                        .element_geometry(&window)
                        .map(|geometry| geometry.size)
                        .unwrap_or((1, 1).into())
                });
                let edge_placement = edge_pan.as_ref().and_then(|_| {
                    let frame_extents =
                        crate::window_chrome::frame_extents(&window, &session.settings.decorations);
                    crate::input::grab::window_edge_pan_placement(
                        position_after,
                        anchor,
                        size,
                        frame_extents,
                        camera,
                        output_geometry,
                    )
                });
                let desired_location = edge_placement
                    .map(|placement| placement.location)
                    .unwrap_or_else(|| {
                        anchor.world_location(position_after, camera, output_geometry)
                    });
                let desired_center = edge_placement.map(|placement| placement.center).unwrap_or(
                    swm_core::field::Vec2 {
                        x: desired_location.x as f32 + size.w as f32 * 0.5,
                        y: desired_location.y as f32 + size.h as f32 * 0.5,
                    },
                );
                let now = crate::frame_clock::monotonic_now();
                if let Some(placement) = edge_placement
                    && let crate::input::grab::Grab::MoveWindow {
                        edge_pan: Some(live),
                        ..
                    } = session.interactions.grab_mut()
                {
                    live.update_contact(placement.contact, now);
                }
                let sampled = if edge_pan.is_some() {
                    swm_core::field::Vec2 { x: 0.0, y: 0.0 }
                } else {
                    sampled_drag_velocity(
                        previous,
                        desired_center,
                        previous_velocity,
                        last_update,
                        now,
                    )
                };
                if output_changed {
                    if let Some(id) = id {
                        crate::nodes::set_collapsed_output(session, id, &output);
                    } else {
                        wayland::set_window_output(&window, &output);
                    }
                }
                if let Some(id) = id {
                    if !session.nodes.physics_enabled() {
                        let _ = crate::nodes::move_grabbed_body_rigid(session, id, desired_center);
                    }
                } else {
                    session
                        .wayland
                        .space
                        .map_element(window.clone(), desired_location, false);
                }
                if let crate::input::grab::Grab::MoveWindow {
                    last_world,
                    last_update,
                    velocity,
                    ..
                } = session.interactions.grab_mut()
                {
                    *last_world = desired_center;
                    *last_update = now;
                    *velocity = sampled;
                }
                if id.is_some() && session.nodes.physics_enabled() {
                    session.request_redraw();
                }
                if output_changed {
                    wayland::popup::update_reactive_for_window(
                        &session.wayland,
                        crate::session::popup_unconstrain_context!(session),
                        &window,
                    );
                }
            }
        }
        crate::input::grab::Grab::MoveNode {
            id,
            screen_offset,
            last_world,
            last_update,
            velocity,
            ..
        } => {
            let id = *id;
            let screen_offset = *screen_offset;
            let previous = *last_world;
            let last_update = *last_update;
            let previous_velocity = *velocity;
            if let Some((output, output_geometry)) =
                output_at_pointer(&session.wayland.space, position_after)
                && let Some(camera) = session.cameras.get(&output.name())
            {
                let pointer_world = crate::input::grab::screen_to_world_on_output(
                    position_after,
                    camera,
                    output_geometry,
                );
                let offset = crate::input::grab::screen_offset_to_world(screen_offset, camera);
                let desired = swm_core::field::Vec2 {
                    x: pointer_world.x + offset.x,
                    y: pointer_world.y + offset.y,
                };
                let now = crate::frame_clock::monotonic_now();
                let sampled =
                    sampled_drag_velocity(previous, desired, previous_velocity, last_update, now);
                crate::nodes::set_collapsed_output(session, id, &output);
                if !session.nodes.physics_enabled() {
                    let _ = crate::nodes::move_grabbed_body_rigid(session, id, desired);
                }
                if let crate::input::grab::Grab::MoveNode {
                    last_world,
                    last_update,
                    velocity,
                    ..
                } = session.interactions.grab_mut()
                {
                    *last_world = desired;
                    *last_update = now;
                    *velocity = sampled;
                }
                if session.nodes.physics_enabled() {
                    session.request_redraw();
                }
            }
        }
        crate::input::grab::Grab::Pan { output, .. } => {
            let dx = position_after.0 - position_before.0;
            let dy = position_after.1 - position_before.1;
            if let Some(camera) = session.cameras.get_mut_for_pan(output) {
                let delta = crate::input::grab::screen_delta_to_world(dx, dy, camera);
                camera.pan_target(swm_core::field::Vec2 {
                    x: -delta.x,
                    y: -delta.y,
                });
            }
        }
        crate::input::grab::Grab::ResizeWindow(_) => {}
        crate::input::grab::Grab::None
        | crate::input::grab::Grab::MovePopup { .. }
        | crate::input::grab::Grab::PendingWindowMove(_)
        | crate::input::grab::Grab::PendingNode { .. } => {}
    }

    if resize_preview_changed {
        apply_active_resize(session);
    }

    if let Some((delta, delta_unaccel, time, time_msec)) = motion
        && !super::super::pointer::constraint_suspended_for_grab(session)
    {
        let route = super::super::pointer::route_for_motion(session, time_msec);
        if super::super::pointer::relative_motion_allowed(session, route.as_ref()) {
            pointer_handle.relative_motion(
                session,
                route.as_ref().and_then(|route| route.focus.clone()),
                &RelativeMotionEvent {
                    delta,
                    delta_unaccel,
                    time,
                },
            );
        }
        super::super::pointer::finish_frame(session, &pointer_handle);
        // Keep shell hover cursors and landmarks from replacing the client's
        // cursor while its implicit grab owns pointer delivery.
        if super::super::pointer::client_click_grab_active(session) {
            return None;
        }
        if let Some(route) = route.as_ref() {
            super::super::focus::update_hover(session, route, SERIAL_COUNTER.next_serial());
        }
        let border_resize_hover = route.as_ref().and_then(|route| match &route.target {
            crate::input::pointer::PointerTarget::Decoration {
                hit: crate::window_chrome::Hit::Resize(handle),
                ..
            } => Some(*handle),
            _ => None,
        });
        let node_grab_active = session.interactions.grab().landmark_active();
        let now = crate::frame_clock::monotonic_now();
        let hovered_node = (!node_grab_active)
            .then(|| node_at_pointer(session))
            .flatten();
        if let Some((id, output)) = hovered_node.as_ref() {
            super::super::focus::focus_node_from_hover(
                session,
                *id,
                output,
                SERIAL_COUNTER.next_serial(),
            );
        }
        let hovered = hovered_node.map(|(id, _)| id);
        let node_changed = session.nodes.set_hovered(hovered, now);
        if let Some(handle) = border_resize_hover {
            session.cursor.set_override(
                crate::cursor::OverrideSource::Hover,
                Some(handle.cursor_icon()),
            );
        } else if !node_grab_active {
            session
                .cursor
                .set_override(crate::cursor::OverrideSource::Hover, None);
        }
        if node_changed {
            session.request_redraw();
        }
    }
    if motion.is_some() && super::super::pointer::constraint_suspended_for_grab(session) {
        super::super::pointer::finish_frame(session, &pointer_handle);
    }

    Some(position_after)
}
