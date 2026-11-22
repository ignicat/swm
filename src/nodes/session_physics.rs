//! Coordinate node physics and landmark movement with the live session.

use super::*;

pub fn reconcile_landmarks<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    only_output: Option<&str>,
) {
    reconcile_landmarks_inner(session, only_output);
}

fn reconcile_landmarks_inner<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    only_output: Option<&str>,
) {
    session.nodes.sync_from_space(&session.wayland.space);
    let candidates = session
        .nodes
        .records()
        .filter(|record| {
            record.collapsed
                && record.attached
                && only_output.is_none_or(|output| record.output == output)
        })
        .filter_map(|record| {
            session
                .nodes
                .field
                .node(record.id)
                .map(|node| (record.id, record.output.clone(), node.pos))
        })
        .collect::<Vec<_>>();
    for (id, output, current) in candidates {
        // Placement reflow answers real geometry changes, such as a window
        // growing onto a node, so it measures at native size. Overlaps that
        // exist only because the camera is zoomed out belong to the zoom
        // reflow, which moves nodes temporarily and brings them home. Measuring
        // here at the zoomed-out size disagreed with that solver, so every
        // client repaint while zoomed out (panning moves the pointer over
        // windows) pushed nodes further away and erased their way home.
        let scale = 1.0;
        let destination = session.nodes.nearest_free_position(
            id,
            current,
            scale,
            PlacementContext {
                decorations: &session.settings.decorations,
                fullscreen: &session.fullscreen,
                maximize: &session.maximize,
            },
        );
        if destination == current {
            continue;
        }
        // Placement reflow is a real move, not a reversible zoom offset: the
        // shared path commits this landmark's pre-zoom home as it applies.
        apply_landmark_move(
            session,
            id,
            destination,
            LandmarkMovement::Persistent,
            &output,
        );
    }
}

fn dynamics_bodies<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
) -> Vec<dynamics::Body> {
    dynamics_bodies_at_scale(session, None)
}

fn dynamics_bodies_at_scale<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    scale_override: Option<(&str, f32)>,
) -> Vec<dynamics::Body> {
    session.nodes.sync_from_space(&session.wayland.space);
    super::dynamics_inputs::collect_dynamics_bodies(
        super::dynamics_inputs::DynamicsBodyContext {
            nodes: &session.nodes,
            cameras: &session.cameras,
            placement: PlacementContext {
                decorations: &session.settings.decorations,
                fullscreen: &session.fullscreen,
                maximize: &session.maximize,
            },
        },
        scale_override,
    )
}

/// Where a landmark move came from. Camera zoom may be reverted while the
/// presented footprint shrinks again; anything physical or user-driven rebases
/// the landmark permanently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LandmarkMovement {
    ZoomTemporary,
    Persistent,
}

fn landmark_moved(origin: Vec2, destination: Vec2) -> bool {
    (destination.x - origin.x).abs() > dynamics::LANDMARK_POSITION_EPSILON
        || (destination.y - origin.y).abs() > dynamics::LANDMARK_POSITION_EPSILON
}

/// Applies one landmark destination in the Field scene, preserving the slide
/// animation and retargeting it from the position actually on screen. Temporary zoom movement remembers the pre-zoom home on
/// the first displacement; persistent movement discards it, so anything a
/// physical or user-driven move touches is rebased together.
fn apply_landmark_move<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    id: NodeId,
    destination: Vec2,
    movement: LandmarkMovement,
    output: &str,
) -> bool {
    let now = crate::frame_clock::monotonic_now();
    let Some((current, collapsed, attached, node_output)) =
        session.nodes.record(id).and_then(|record| {
            session.nodes.field.node(id).map(|node| {
                (
                    node.pos,
                    record.collapsed,
                    record.attached,
                    record.output.clone(),
                )
            })
        })
    else {
        return false;
    };
    if !collapsed || !attached || node_output != output {
        return false;
    }
    if !landmark_moved(destination, current) {
        return false;
    }
    if movement == LandmarkMovement::ZoomTemporary {
        session.nodes.remember_zoom_home(id, current);
    } else {
        session.nodes.commit_zoom_home(id);
    }
    let from = session.nodes.landmark_position(id, current, now);
    if let Some(node) = session.nodes.field.node_mut(id) {
        node.pos = destination;
    }
    session.nodes.physics_velocity.remove(&id);
    session
        .nodes
        .start_landmark_slide(id, from, destination, now);
    true
}

/// Reconcile the fixed-pixel footprint of collapsed landmarks at a newly
/// presented zoom scale. This is the zoom-out direction: active windows are
/// stationary blockers, while collapsed nodes move under the shared collision
/// policy. The first displacement of each landmark remembers its pre-zoom home
/// so zooming back in can revert it.
pub(crate) fn reconcile_landmarks_for_zoom<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    output: &str,
    scale: f32,
) -> bool {
    let bodies = dynamics_bodies_at_scale(session, Some((output, scale)))
        .into_iter()
        .filter(|body| body.output == output)
        .collect::<Vec<_>>();
    let positions = dynamics::solve_zoom_reflow(&bodies);
    let mut targets = positions.into_iter().collect::<Vec<_>>();
    targets.sort_by_key(|(id, _)| id.as_u64());
    let mut changed = false;
    for (id, destination) in targets {
        changed |= apply_landmark_move(
            session,
            id,
            destination,
            LandmarkMovement::ZoomTemporary,
            output,
        );
    }
    session.nodes.retire_zoom_homes_without_field_node();
    changed
}

/// Returns temporarily displaced landmarks toward their remembered pre-zoom
/// homes as the presented footprint grows again. This is the zoom-in direction
/// of the same reconciliation: a home is forgotten once its landmark reaches
/// it, and a still-blocked landmark keeps its home for a later attempt.
pub(crate) fn restore_landmarks_for_zoom<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    output: &str,
    scale: f32,
) -> bool {
    let bodies = dynamics_bodies_at_scale(session, Some((output, scale)))
        .into_iter()
        .filter(|body| body.output == output)
        .collect::<Vec<_>>();
    let homes = bodies
        .iter()
        .filter_map(|body| session.nodes.zoom_home(body.id).map(|home| (body.id, home)))
        .collect::<HashMap<_, _>>();
    if homes.is_empty() {
        return false;
    }
    let positions = dynamics::solve_zoom_return(&bodies, &homes);
    let mut targets = positions
        .into_iter()
        .filter(|(id, _)| homes.contains_key(id))
        .collect::<Vec<_>>();
    targets.sort_by_key(|(id, _)| id.as_u64());
    let mut changed = false;
    for (id, destination) in targets {
        let home = homes[&id];
        if !landmark_moved(destination, home) {
            // The landmark is home again, so the temporary displacement is over.
            changed |=
                apply_landmark_move(session, id, home, LandmarkMovement::ZoomTemporary, output);
            session.nodes.commit_zoom_home(id);
            continue;
        }
        changed |= apply_landmark_move(
            session,
            id,
            destination,
            LandmarkMovement::ZoomTemporary,
            output,
        );
    }
    session.nodes.retire_zoom_homes_without_field_node();
    changed
}

pub(super) fn apply_dynamics_positions<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    positions: HashMap<NodeId, Vec2>,
    authority: Option<NodeId>,
) -> HashSet<String> {
    let changes = positions
        .iter()
        .filter_map(|(id, position)| {
            let record = session.nodes.record(*id)?;
            let current = session.nodes.field.node(*id)?.pos;
            ((position.x - current.x).abs() > 0.001 || (position.y - current.y).abs() > 0.001).then(
                || {
                    (
                        *id,
                        *position,
                        record.collapsed,
                        record.output.clone(),
                        record.window.clone(),
                        record.geometry.size,
                    )
                },
            )
        })
        .collect::<Vec<_>>();
    let mut outputs = HashSet::new();
    for (id, position, collapsed, output, window, size) in changes {
        outputs.insert(output);
        session.nodes.commit_zoom_home(id);
        if let Some(node) = session.nodes.field.node_mut(id) {
            node.pos = position;
        }
        session.nodes.landmark_slides.borrow_mut().remove(&id);
        if collapsed {
            continue;
        }
        let location = Point::<i32, Logical>::from((
            (position.x - size.w as f32 * 0.5).round() as i32,
            (position.y - size.h as f32 * 0.5).round() as i32,
        ));
        // `Space::map_element` always moves an existing element to the top of
        // its z-index, even with `activate = false`. Physics must only change
        // position: otherwise a node pushing a window makes that window jump
        // layers on every solver frame.
        session.wayland.space.relocate_element(&window, location);
        if let Some(record) = session.nodes.record_mut(id) {
            record.geometry = Rectangle::new(location, size);
        }
        if crate::xwayland::is_x11(&window) {
            crate::xwayland::configure_window(session, &window, Rectangle::new(location, size));
        }
        if authority != Some(id) {
            // A physically displaced Wayland window needs only compositor
            // placement; its client-controlled size remains unchanged.
            crate::wayland::popup::update_reactive_for_window(
                &session.wayland,
                crate::session::popup_unconstrain_context!(session),
                &window,
            );
        }
    }
    outputs
}

pub(super) fn physics_frame_delta(last: Duration, now: Duration) -> f32 {
    now.saturating_sub(last).as_secs_f32().min(1.0 / 30.0)
}

pub(crate) fn move_grabbed_body_rigid<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    id: NodeId,
    desired: Vec2,
) -> bool {
    let bodies = dynamics_bodies(session);
    if !bodies.iter().any(|body| body.id == id) {
        return false;
    }
    session.nodes.physics_velocity.clear();
    let positions = dynamics::solve_static_swept(bodies, id, desired);
    let changed = !apply_dynamics_positions(session, positions, Some(id)).is_empty();
    if changed {
        session.request_redraw();
    }
    changed
}

pub(crate) fn tick_physics<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    now: Duration,
) -> bool {
    if !session.nodes.physics.enabled {
        session.nodes.physics_velocity.clear();
        session.nodes.release_locks.clear();
        session.nodes.physics_last_tick = now;
        return false;
    }
    let authority = match session.interactions.grab() {
        crate::input::grab::Grab::MoveWindow {
            id: Some(id),
            last_world,
            velocity,
            ..
        } => Some((*id, *last_world, *velocity)),
        crate::input::grab::Grab::MoveNode {
            id,
            last_world,
            velocity,
            ..
        } => Some((*id, *last_world, *velocity)),
        _ => None,
    };
    let expired_lock = session.nodes.expire_release_locks(now);
    if session.nodes.physics_velocity.is_empty()
        && authority.is_none()
        && session.nodes.release_locks.is_empty()
        && !expired_lock
    {
        session.nodes.physics_last_tick = now;
        return false;
    }
    let bodies = dynamics_bodies(session);
    let live = bodies.iter().map(|body| body.id).collect::<HashSet<_>>();
    session
        .nodes
        .physics_velocity
        .retain(|id, _| live.contains(id));
    let dt = physics_frame_delta(session.nodes.physics_last_tick, now);
    session.nodes.physics_last_tick = now;
    let positions = if let Some(authority) = authority {
        dynamics::solve_physics_swept(
            bodies,
            &mut session.nodes.physics_velocity,
            authority,
            dt,
            session.nodes.physics.damping,
        )
    } else {
        if dt <= f32::EPSILON && !expired_lock {
            return !session.nodes.physics_velocity.is_empty()
                || !session.nodes.release_locks.is_empty();
        }
        dynamics::solve_physics(
            &bodies,
            &mut session.nodes.physics_velocity,
            None,
            dt,
            session.nodes.physics.damping,
        )
    };
    let _ = apply_dynamics_positions(session, positions, authority.map(|(id, _, _)| id));
    authority.is_some()
        || !session.nodes.physics_velocity.is_empty()
        || !session.nodes.release_locks.is_empty()
}

pub(crate) fn displace_landmarks_for_new_window<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    id: NodeId,
) -> bool {
    let Some(position) = session.nodes.field.node(id).map(|node| node.pos) else {
        return false;
    };
    let bodies = dynamics_bodies(session);
    if !bodies.iter().any(|body| body.id == id) {
        return false;
    }
    let now = crate::frame_clock::monotonic_now();
    let landmarks = bodies
        .iter()
        .filter(|body| body.kind == dynamics::BodyKind::Node)
        .map(|body| {
            (
                body.id,
                body.pos,
                session.nodes.landmark_position(body.id, body.pos, now),
            )
        })
        .collect::<Vec<_>>();
    session.nodes.physics_velocity.clear();
    let positions = dynamics::solve_static_swept(bodies, id, position);
    let changed = !apply_dynamics_positions(session, positions, Some(id)).is_empty();
    if !changed {
        return false;
    }

    for (landmark, old_target, from) in landmarks {
        let Some(to) = session.nodes.field.node(landmark).map(|node| node.pos) else {
            continue;
        };
        if (to.x - old_target.x).abs() <= 0.001 && (to.y - old_target.y).abs() <= 0.001 {
            continue;
        }
        session.nodes.start_landmark_slide(landmark, from, to, now);
    }
    session.request_redraw();
    true
}
