use super::*;

const KEYBOARD_RESIZE_STEP: i32 = 80;
const KEYBOARD_RESIZE_MIN_WIDTH: i32 = 96;
const KEYBOARD_RESIZE_MIN_HEIGHT: i32 = 72;

fn keyboard_resize_dimensions(
    width: i32,
    height: i32,
    direction: swm_config::Direction,
) -> (i32, i32) {
    match direction {
        swm_config::Direction::Left => (
            width
                .saturating_sub(KEYBOARD_RESIZE_STEP)
                .max(KEYBOARD_RESIZE_MIN_WIDTH),
            height,
        ),
        swm_config::Direction::Right => (width.saturating_add(KEYBOARD_RESIZE_STEP), height),
        swm_config::Direction::Up => (
            width,
            height
                .saturating_sub(KEYBOARD_RESIZE_STEP)
                .max(KEYBOARD_RESIZE_MIN_HEIGHT),
        ),
        swm_config::Direction::Down => (width, height.saturating_add(KEYBOARD_RESIZE_STEP)),
    }
}

fn api_error(kind: swm_ipc::ServerErrorKind, message: impl Into<String>) -> swm_ipc::Response {
    swm_ipc::Response::ApiError(swm_ipc::ServerError::new(kind, message))
}

pub fn handle_request<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    request: swm_ipc::NodeRequest,
) -> swm_ipc::Response {
    session.nodes.sync_from_space(&session.wayland.space);
    match request {
        swm_ipc::NodeRequest::List { output } => {
            let outputs = match requested_outputs(session, output.as_deref()) {
                Ok(outputs) => outputs,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            swm_ipc::Response::NodeList(swm_ipc::NodeListResponse {
                outputs: outputs
                    .into_iter()
                    .map(|output| {
                        let mut ids = session
                            .nodes
                            .records()
                            .filter(|record| record.output == output)
                            .map(|record| record.id)
                            .collect::<Vec<_>>();
                        ids.sort_by_key(|id| id.as_u64());
                        swm_ipc::NodeOutputGroup {
                            output,
                            nodes: ids
                                .into_iter()
                                .filter_map(|id| node_info(session, id))
                                .collect(),
                        }
                    })
                    .collect(),
            })
        }
        swm_ipc::NodeRequest::Info { selector, output } => {
            match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => node_info(session, id)
                    .map(swm_ipc::Response::NodeInfo)
                    .unwrap_or_else(|| {
                        api_error(swm_ipc::ServerErrorKind::NotFound, "node disappeared")
                    }),
                Err(error) => swm_ipc::Response::ApiError(error),
            }
        }
        swm_ipc::NodeRequest::Focus { selector, output } => {
            let id = match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => id,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            let focused = crate::session::activate_node(session, id, true);
            if focused {
                swm_ipc::Response::Ack
            } else {
                api_error(
                    swm_ipc::ServerErrorKind::Internal,
                    format!("failed to focus node {id}"),
                )
            }
        }
        swm_ipc::NodeRequest::Move {
            direction,
            selector,
            output,
        } => {
            let id = match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => id,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            if move_node(session, id, direction, None) {
                swm_ipc::Response::Ack
            } else {
                api_error(
                    swm_ipc::ServerErrorKind::Internal,
                    format!("failed to move node {id}"),
                )
            }
        }
        swm_ipc::NodeRequest::MoveBy {
            direction,
            distance,
            selector,
            output,
        } => {
            if distance == 0 || distance > i32::MAX as u32 {
                return api_error(
                    swm_ipc::ServerErrorKind::InvalidRequest,
                    "movement distance must be a whole number in 1..=2147483647 Field units",
                );
            }
            let id = match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => id,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            if move_node(session, id, direction, Some(distance)) {
                swm_ipc::Response::Ack
            } else {
                api_error(
                    swm_ipc::ServerErrorKind::Internal,
                    format!("failed to move node {id}"),
                )
            }
        }
        swm_ipc::NodeRequest::MoveTo {
            x,
            y,
            selector,
            output,
        } => {
            let id = match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => id,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            let origin = session.nodes.record(id).and_then(|record| {
                session
                    .wayland
                    .space
                    .outputs()
                    .find(|o| o.name() == record.output)
                    .and_then(|o| session.wayland.space.output_geometry(o))
                    .map(|g| g.loc)
            });
            let destination = origin.and_then(|origin| {
                Some(Point::from((
                    origin.x.checked_add(x)?,
                    origin.y.checked_add(y)?,
                )))
            });
            if destination.is_some_and(|destination| {
                place_node(session, id, (0, 0), false, Some(destination))
            }) {
                swm_ipc::Response::Ack
            } else {
                swm_ipc::Response::ApiError(swm_ipc::ServerError::new(
                    swm_ipc::ServerErrorKind::InvalidRequest,
                    format!(
                        "cannot place node {id}: position is blocked, out of range, or the node cannot move"
                    ),
                ))
            }
        }
        swm_ipc::NodeRequest::Close { selector, output } => {
            let id = match resolve(session, selector.as_ref(), output.as_deref()) {
                Ok(id) => id,
                Err(error) => return swm_ipc::Response::ApiError(error),
            };
            if super::close(session, id) {
                swm_ipc::Response::Ack
            } else {
                api_error(
                    swm_ipc::ServerErrorKind::NotFound,
                    format!("node {id} disappeared"),
                )
            }
        }
        swm_ipc::NodeRequest::Collapse { selector, output } => {
            change_node_state(session, selector.as_ref(), output.as_deref(), Some(true))
        }
        swm_ipc::NodeRequest::Restore { selector, output } => {
            change_node_state(session, selector.as_ref(), output.as_deref(), Some(false))
        }
        swm_ipc::NodeRequest::Toggle { selector, output } => {
            change_node_state(session, selector.as_ref(), output.as_deref(), None)
        }
    }
}

fn change_node_state<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    selector: Option<&swm_ipc::NodeSelector>,
    output: Option<&str>,
    collapsed: Option<bool>,
) -> swm_ipc::Response {
    let id = match resolve(session, selector, output) {
        Ok(id) => id,
        Err(error) => return swm_ipc::Response::ApiError(error),
    };
    let Some(current) = session.nodes.record(id).map(|record| record.collapsed) else {
        return api_error(
            swm_ipc::ServerErrorKind::NotFound,
            format!("node {id} disappeared"),
        );
    };
    let desired = collapsed.unwrap_or(!current);
    if desired == current {
        return swm_ipc::Response::Ack;
    }
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    let changed = if desired {
        collapse(session, id, serial)
    } else {
        restore(session, id, serial)
    };
    if changed {
        swm_ipc::Response::Ack
    } else {
        let action = if desired { "collapse" } else { "restore" };
        api_error(
            swm_ipc::ServerErrorKind::Internal,
            format!("failed to {action} node {id}"),
        )
    }
}

fn requested_outputs<D: crate::session::SessionDriver>(
    session: &crate::session::Session<D>,
    requested: Option<&str>,
) -> Result<Vec<String>, swm_ipc::ServerError> {
    let mut outputs = session
        .wayland
        .space
        .outputs()
        .map(|output| output.name())
        .collect::<Vec<_>>();
    outputs.sort();
    if let Some(requested) = requested {
        if outputs.iter().any(|output| output == requested) {
            Ok(vec![requested.to_string()])
        } else {
            Err(swm_ipc::ServerError::new(
                swm_ipc::ServerErrorKind::NotFound,
                format!("unknown output {requested:?}"),
            ))
        }
    } else {
        Ok(outputs)
    }
}

pub(crate) fn resolve<D: crate::session::SessionDriver>(
    session: &crate::session::Session<D>,
    selector: Option<&swm_ipc::NodeSelector>,
    output: Option<&str>,
) -> Result<NodeId, swm_ipc::ServerError> {
    if let Some(output) = output {
        requested_outputs(session, Some(output))?;
    }
    let on_output = |record: &&NodeRecord| output.is_none_or(|name| record.output == name);
    let records = session
        .nodes
        .records()
        .filter(on_output)
        .collect::<Vec<_>>();
    let direct = match selector {
        None | Some(swm_ipc::NodeSelector::Focused) => session.nodes.focused().filter(|id| {
            session
                .nodes
                .record(*id)
                .is_some_and(|record| output.is_none_or(|name| record.output == name))
        }),
        Some(swm_ipc::NodeSelector::Latest) => records
            .iter()
            .map(|record| record.id)
            .max_by_key(|id| id.as_u64()),
        Some(swm_ipc::NodeSelector::Id(raw)) => records
            .iter()
            .find(|record| record.id.as_u64() == *raw)
            .map(|record| record.id),
        Some(swm_ipc::NodeSelector::Title(text)) => {
            return unique_match(
                records
                    .iter()
                    .filter(|record| contains_case_insensitive(&record.title, text))
                    .map(|record| record.id)
                    .collect(),
                &format!("title:{text}"),
            );
        }
        Some(swm_ipc::NodeSelector::App(text)) => {
            return unique_match(
                records
                    .iter()
                    .filter(|record| {
                        record
                            .app_id
                            .as_deref()
                            .is_some_and(|app| contains_case_insensitive(app, text))
                    })
                    .map(|record| record.id)
                    .collect(),
                &format!("app:{text}"),
            );
        }
    };
    direct
        .or_else(|| {
            (selector.is_none()).then(|| {
                records
                    .iter()
                    .map(|record| record.id)
                    .max_by_key(|id| id.as_u64())
            })?
        })
        .ok_or_else(|| {
            swm_ipc::ServerError::new(
                swm_ipc::ServerErrorKind::NotFound,
                "no node matched the selector",
            )
        })
}

fn unique_match(ids: Vec<NodeId>, label: &str) -> Result<NodeId, swm_ipc::ServerError> {
    match ids.as_slice() {
        [id] => Ok(*id),
        [] => Err(swm_ipc::ServerError::new(
            swm_ipc::ServerErrorKind::NotFound,
            format!("no node matched selector {label}"),
        )),
        _ => Err(swm_ipc::ServerError::new(
            swm_ipc::ServerErrorKind::Ambiguous,
            format!("selector {label} matched multiple nodes"),
        )),
    }
}

fn contains_case_insensitive(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(&needle.to_lowercase())
}

pub(crate) fn move_selected_direction<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    direction: swm_config::Direction,
    output: Option<&str>,
) -> bool {
    move_selected(session, direction, None, output)
}

pub(crate) fn move_selected_by<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    direction: swm_config::Direction,
    distance: u32,
    output: Option<&str>,
) -> bool {
    move_selected(session, direction, Some(distance), output)
}

fn move_selected<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    direction: swm_config::Direction,
    distance: Option<u32>,
    output: Option<&str>,
) -> bool {
    session.nodes.sync_from_space(&session.wayland.space);
    let Ok(id) = resolve(session, None, output) else {
        return false;
    };
    let direction = match direction {
        swm_config::Direction::Left => swm_ipc::NodeMoveDirection::Left,
        swm_config::Direction::Right => swm_ipc::NodeMoveDirection::Right,
        swm_config::Direction::Up => swm_ipc::NodeMoveDirection::Up,
        swm_config::Direction::Down => swm_ipc::NodeMoveDirection::Down,
    };
    move_node(session, id, direction, distance)
}

pub(crate) fn resize_selected_direction<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    direction: swm_config::Direction,
    output: Option<&str>,
) -> bool {
    session.nodes.sync_from_space(&session.wayland.space);
    let Ok(id) = resolve(session, None, output) else {
        return false;
    };
    let Some(record) = session.nodes.record(id).cloned() else {
        return false;
    };
    if record.collapsed
        || crate::session::node_user_pinned(session, id)
        || session.fullscreen.is_fullscreen_or_pending(&record.surface)
        || session.maximize.contains(&record.surface)
    {
        return false;
    }
    let Some(current) = session.wayland.space.element_geometry(&record.window) else {
        return false;
    };
    let requested = keyboard_resize_dimensions(current.size.w, current.size.h, direction).into();
    let size = if crate::xwayland::is_x11(&record.window) {
        crate::xwayland::constrain_window_size(&record.window, requested)
    } else {
        requested
    };
    if size == current.size {
        return false;
    }
    // Resize from the visible position, just as a pointer resize does. Changing
    // size would otherwise invalidate the move animation and expose its endpoint.
    crate::session::settle_window_movement(session, &record.window);
    let Some(current) = session.wayland.space.element_geometry(&record.window) else {
        return false;
    };
    if let Some(toplevel) = record.window.toplevel() {
        toplevel.with_pending_state(|pending| pending.size = Some(size));
        toplevel.send_pending_configure();
    } else {
        crate::xwayland::configure_window(
            session,
            &record.window,
            Rectangle::new(current.loc, size),
        );
    }
    session.request_redraw();
    true
}

fn move_node<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    id: NodeId,
    direction: swm_ipc::NodeMoveDirection,
    distance: Option<u32>,
) -> bool {
    if distance.is_some_and(|distance| distance == 0 || distance > i32::MAX as u32) {
        return false;
    }
    let step = distance.unwrap_or(80) as i32;
    let delta = match direction {
        swm_ipc::NodeMoveDirection::Left => (-step, 0),
        swm_ipc::NodeMoveDirection::Right => (step, 0),
        swm_ipc::NodeMoveDirection::Up => (0, -step),
        swm_ipc::NodeMoveDirection::Down => (0, step),
    };
    place_node(session, id, delta, distance.is_some(), None)
}

fn place_node<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    id: NodeId,
    (dx, dy): (i32, i32),
    explicit_distance: bool,
    exact: Option<Point<i32, Logical>>,
) -> bool {
    let Some(record) = session.nodes.record(id).cloned() else {
        return false;
    };
    if crate::session::node_user_pinned(session, id)
        || !record.attached
        || session.fullscreen.is_fullscreen_or_pending(&record.surface)
        || session.maximize.contains(&record.surface)
        || crate::input::grab::belongs_to_surface(session.interactions.grab(), &record.surface)
    {
        return false;
    }
    if let Some(desired) = exact {
        let scale = session
            .cameras
            .get(&record.output)
            .map(crate::presentation::camera::scale)
            .unwrap_or(1.0);
        if !session.nodes.exact_position_is_free(
            id,
            desired,
            scale,
            super::PlacementContext {
                decorations: &session.settings.decorations,
                fullscreen: &session.fullscreen,
                maximize: &session.maximize,
            },
        ) {
            return false;
        }
    }
    if record.collapsed {
        let Some(current) = session.nodes.field.node(id).map(|node| node.pos) else {
            return false;
        };
        let desired = exact.map_or(
            Vec2 {
                x: current.x + dx as f32,
                y: current.y + dy as f32,
            },
            |point| Vec2 {
                x: point.x as f32,
                y: point.y as f32,
            },
        );
        let scale = session
            .cameras
            .get(&record.output)
            .map(crate::presentation::camera::scale)
            .unwrap_or(1.0);
        let chrome = super::PlacementContext {
            decorations: &session.settings.decorations,
            fullscreen: &session.fullscreen,
            maximize: &session.maximize,
        };
        let destination = if exact.is_some() {
            desired
        } else if explicit_distance {
            session
                .nodes
                .directional_landmark_position(id, current, (dx, dy), scale, chrome)
        } else {
            session
                .nodes
                .nearest_free_position(id, desired, scale, chrome)
        };
        if exact.is_some() && destination != desired {
            return false;
        }
        let now = crate::frame_clock::monotonic_now();
        let visible = session.nodes.landmark_position(id, current, now);
        if let Some(node) = session.nodes.field.node_mut(id) {
            node.pos = destination;
        }
        // A discrete move is an explicit placement, not a reversible zoom offset.
        session.nodes.commit_zoom_home(id);
        session
            .nodes
            .start_landmark_slide(id, visible, destination, now);
    } else {
        let Some(location) = session.wayland.space.element_location(&record.window) else {
            return false;
        };
        let desired = exact.unwrap_or_else(|| {
            Point::from((location.x.saturating_add(dx), location.y.saturating_add(dy)))
        });
        if desired.x.checked_add(record.geometry.size.w).is_none()
            || desired.y.checked_add(record.geometry.size.h).is_none()
        {
            return false;
        }
        let scale = session
            .cameras
            .get(&record.output)
            .map(crate::presentation::camera::scale)
            .unwrap_or(1.0);
        let chrome = super::PlacementContext {
            decorations: &session.settings.decorations,
            fullscreen: &session.fullscreen,
            maximize: &session.maximize,
        };
        let next = if exact.is_some() {
            desired
        } else if explicit_distance {
            session
                .nodes
                .directional_active_rect(
                    id,
                    Rectangle::new(location, record.geometry.size),
                    (dx, dy),
                    scale,
                    chrome,
                )
                .loc
        } else {
            session
                .nodes
                .nearest_free_active_rect(
                    id,
                    Rectangle::new(desired, record.geometry.size),
                    &record.output,
                    scale,
                    chrome,
                )
                .loc
        };
        if exact.is_some() && next != desired {
            return false;
        }
        if next != location {
            let current = session
                .wayland
                .space
                .element_geometry(&record.window)
                .unwrap_or(record.geometry);
            session.window_animations.move_window(
                record.surface.clone(),
                crate::frame_clock::monotonic_now(),
                current,
                Rectangle::new(next, current.size),
            );
        }
        session.wayland.space.relocate_element(&record.window, next);
        if crate::xwayland::is_x11(&record.window) {
            crate::xwayland::configure_window(
                session,
                &record.window,
                Rectangle::new(next, record.geometry.size),
            );
        }
    }
    session.request_redraw();
    true
}

pub(crate) fn node_info<D: crate::session::SessionDriver>(
    session: &crate::session::Session<D>,
    id: NodeId,
) -> Option<swm_ipc::NodeInfo> {
    let record = session.nodes.record(id)?;
    let node = session.nodes.field.node(id)?;
    let latest = session
        .nodes
        .records()
        .filter(|candidate| candidate.output == record.output)
        .map(|candidate| candidate.id)
        .max_by_key(|candidate| candidate.as_u64())
        == Some(id);
    let (role, family, modal, parent) = relation_metadata(session, record);
    Some(swm_ipc::NodeInfo {
        id: id.as_u64(),
        title: record.title.clone(),
        app_id: record.app_id.clone(),
        output: Some(record.output.clone()),
        kind: swm_ipc::NodeKind::Surface,
        state: if record.collapsed {
            swm_ipc::NodeState::Node
        } else {
            swm_ipc::NodeState::Active
        },
        visible: record.attached,
        focused: session.nodes.focused() == Some(id),
        latest,
        pinned: crate::session::node_user_pinned(session, id),
        role,
        protocol_family: family,
        modal,
        parent: parent.clone(),
        transient_for: parent,
        child_popup_count: PopupManager::popups_for_surface(&record.surface).count(),
        pos_x: node.pos.x,
        pos_y: node.pos.y,
        width: node.intrinsic_size.x,
        height: node.intrinsic_size.y,
        screen: node_screen_info(session, record),
    })
}

fn node_screen_info<D: crate::session::SessionDriver>(
    session: &crate::session::Session<D>,
    record: &NodeRecord,
) -> Option<swm_ipc::NodeScreenInfo> {
    if !record.attached
        || record.collapsed
        || session
            .window_rules
            .applied(&record.surface)
            .block_screencast
    {
        return None;
    }
    let output = session
        .wayland
        .space
        .outputs()
        .find(|output| output.name() == record.output)?;
    let output_geometry = session.wayland.space.output_geometry(output)?;
    let presentation = crate::presentation::window::WindowPresentation::for_window(
        crate::presentation::window::WindowPresentationDesktop {
            space: &session.wayland.space,
            cameras: &session.cameras,
            window_animations: &session.window_animations,
            fullscreen: &session.fullscreen,
            maximize: &session.maximize,
            decorations: &session.settings.decorations,
        },
        &record.window,
        output,
        crate::frame_clock::monotonic_now(),
    )?;
    let geometry = session
        .window_capture_context()
        .visual_geometry(&record.window, presentation.visual_geometry());
    geometry.intersection(output_geometry)?;
    let size = session.window_capture_context().size(&record.window);
    let stack_order = session
        .wayland
        .space
        .elements()
        .position(|element| element == &record.window)
        .unwrap_or(0) as u32;
    Some(swm_ipc::NodeScreenInfo {
        x: geometry.loc.x,
        y: geometry.loc.y,
        width: geometry.size.w,
        height: geometry.size.h,
        buffer_width: size.w,
        buffer_height: size.h,
        stack_order,
    })
}

fn relation_metadata<D: crate::session::SessionDriver>(
    session: &crate::session::Session<D>,
    record: &NodeRecord,
) -> (
    swm_ipc::NodeRole,
    swm_ipc::NodeProtocolFamily,
    bool,
    Option<swm_ipc::NodeRelationInfo>,
) {
    if crate::xwayland::is_x11(&record.window) {
        return (
            swm_ipc::NodeRole::NormalToplevel,
            swm_ipc::NodeProtocolFamily::Xwayland,
            false,
            None,
        );
    }
    let (parent, modal) = with_states(&record.surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .map(|data| {
                (
                    data.parent.clone(),
                    data.dialog_hint == ToplevelDialogHint::Modal,
                )
            })
            .unwrap_or((None, false))
    });
    let relation = parent.map(|parent| swm_ipc::NodeRelationInfo {
        node_id: session
            .nodes
            .id_for_surface(&crate::wayland::compositor::root_surface(&parent))
            .map(NodeId::as_u64),
    });
    (
        if relation.is_some() || modal {
            swm_ipc::NodeRole::Dialog
        } else {
            swm_ipc::NodeRole::NormalToplevel
        },
        swm_ipc::NodeProtocolFamily::XdgToplevel,
        modal,
        relation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_resize_uses_directional_axes_and_safe_minima() {
        assert_eq!(
            keyboard_resize_dimensions(800, 600, swm_config::Direction::Left),
            (720, 600)
        );
        assert_eq!(
            keyboard_resize_dimensions(800, 600, swm_config::Direction::Right),
            (880, 600)
        );
        assert_eq!(
            keyboard_resize_dimensions(800, 600, swm_config::Direction::Up),
            (800, 520)
        );
        assert_eq!(
            keyboard_resize_dimensions(800, 600, swm_config::Direction::Down),
            (800, 680)
        );
        assert_eq!(
            keyboard_resize_dimensions(100, 80, swm_config::Direction::Left),
            (KEYBOARD_RESIZE_MIN_WIDTH, 80)
        );
        assert_eq!(
            keyboard_resize_dimensions(100, 80, swm_config::Direction::Up),
            (100, KEYBOARD_RESIZE_MIN_HEIGHT)
        );
    }

    #[test]
    fn unique_match_classifies_missing_and_ambiguous_selectors() {
        let missing = unique_match(Vec::new(), "app:term").unwrap_err();
        assert_eq!(missing.kind, swm_ipc::ServerErrorKind::NotFound);

        let ambiguous = unique_match(vec![NodeId::new(1), NodeId::new(2)], "app:term").unwrap_err();
        assert_eq!(ambiguous.kind, swm_ipc::ServerErrorKind::Ambiguous);
    }
}
