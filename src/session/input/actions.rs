use std::ffi::OsStr;

use smithay::output::Output;
use smithay::utils::SERIAL_COUNTER;

use super::{Session, SessionDriver, focus_output_target, toggle_focused_node};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchOrigin {
    Keyboard,
    Other,
}

fn action_hides_cursor_for_keyboard_navigation(action: &swm_config::Action) -> bool {
    matches!(
        action,
        swm_config::Action::FocusCycle(_)
            | swm_config::Action::Trail(_)
            | swm_config::Action::FocusDirection(_)
            | swm_config::Action::CenterLastFocused
            | swm_config::Action::MonitorFocus(_)
    )
}

pub(super) fn window_action_output(
    origin: DispatchOrigin,
    focus_mode: swm_config::FocusMode,
    pointer_output: Option<&str>,
    selected_output: Option<&str>,
) -> Option<String> {
    if origin == DispatchOrigin::Keyboard {
        return selected_output.map(str::to_owned);
    }
    match focus_mode {
        swm_config::FocusMode::Hover => pointer_output.or(selected_output).map(str::to_owned),
        swm_config::FocusMode::Click => selected_output.map(str::to_owned),
    }
}

pub(crate) fn dispatch<D: SessionDriver>(
    session: &mut Session<D>,
    action: swm_config::Action,
    socket_name: &OsStr,
    output_name: Option<&str>,
    origin: DispatchOrigin,
) {
    if origin == DispatchOrigin::Keyboard
        && action_hides_cursor_for_keyboard_navigation(&action)
        && session.cursor_policy.keyboard_navigation()
    {
        session.request_redraw();
    }
    let zoom_action = matches!(
        &action,
        swm_config::Action::ZoomIn | swm_config::Action::ZoomOut | swm_config::Action::ZoomReset
    );
    let selected_output =
        crate::wayland::focus::selected_output(&session.wayland).map(Output::name);
    let action_output = window_action_output(
        origin,
        session.settings.input.focus_mode,
        output_name,
        selected_output.as_deref(),
    );
    let output_name = if origin == DispatchOrigin::Keyboard {
        selected_output.as_deref()
    } else {
        output_name
    };
    let x11_display = session.xwayland.display_name();
    let camera = output_name.and_then(|name| session.cameras.get_mut(name));
    let zoom_camera_available = camera.is_some();
    match super::dispatch::dispatch_action(
        action,
        session.keyboard.terminal_command(),
        super::dispatch::SpawnContext {
            socket_name,
            x11_display: x11_display.as_deref(),
            cursor_size: session.cursor.size(),
            environment: session.launch.environment(),
        },
        camera,
        &session.settings.zoom,
    ) {
        super::dispatch::SessionControl::Continue => {}
        super::dispatch::SessionControl::Quit => session.show_exit_confirmation(),
        super::dispatch::SessionControl::CloseFocusedWindow => {
            crate::nodes::close_focused_on_output(session, action_output.as_deref());
        }
        super::dispatch::SessionControl::ToggleFullscreen => {
            super::super::toggle_focused_fullscreen(session, action_output.as_deref())
        }
        super::dispatch::SessionControl::ToggleFieldMaximize => {
            super::super::toggle_focused_field_maximize(session, action_output.as_deref())
        }
        super::dispatch::SessionControl::ToggleState => toggle_focused_node(
            session,
            action_output.as_deref(),
            SERIAL_COUNTER.next_serial(),
        ),
        super::dispatch::SessionControl::ToggleFocusedPin => {
            super::super::toggle_focused_pin(session, action_output.as_deref());
        }
        super::dispatch::SessionControl::ZoomFit => {
            crate::session::zoom_fit::toggle(session);
        }
        super::dispatch::SessionControl::FocusCycle(direction) => {
            crate::shell::focus_cycle::start_or_step(session, direction);
        }
        super::dispatch::SessionControl::Trail(direction) => {
            if let Err(error) = crate::trail::navigate(session, direction, action_output.as_deref())
            {
                eventline::debug!("trail: {}", error.message);
            }
        }
        super::dispatch::SessionControl::FocusDirection(direction) => {
            if let Some(output) = action_output {
                super::super::navigation::focus_directional_field(session, &output, direction);
            }
        }
        super::dispatch::SessionControl::TransferWindow(direction) => {
            if let Err(error) = crate::nodes::transfer_window(session, direction) {
                eventline::debug!("window transfer: {error}");
            }
        }
        super::dispatch::SessionControl::PanField(direction) => {
            if let Err(error) = crate::nodes::pan_field(session, direction) {
                eventline::debug!("pan field: {error}");
            }
        }
        super::dispatch::SessionControl::MoveNode(direction) => {
            if let Some(output) = action_output {
                crate::nodes::move_selected_direction(session, direction, Some(&output));
            }
        }
        super::dispatch::SessionControl::MoveNodeBy {
            direction,
            distance,
        } => {
            if let Some(output) = action_output {
                crate::nodes::ipc::move_selected_by(session, direction, distance, Some(&output));
            }
        }
        super::dispatch::SessionControl::ResizeWindow(direction) => {
            if let Some(output) = action_output {
                crate::nodes::resize_selected_direction(session, direction, Some(&output));
            }
        }
        super::dispatch::SessionControl::CenterLastFocused => {
            if let Some(output) = action_output {
                super::super::navigation::center_last_focused(session, &output);
            }
        }
        super::dispatch::SessionControl::MonitorFocus(target) => {
            focus_output_target(session, target)
        }
        super::dispatch::SessionControl::Reload => {
            if !session.config_source.request_reload() {
                eventline::warn!("config: manual reload requested without a watched file");
            }
        }
    }
    if zoom_action
        && zoom_camera_available
        && let Some(output_name) = output_name
    {
        if session.settings.zoom.enabled
            && let Some(live_scale) = session
                .cameras
                .get(output_name)
                .map(crate::input::zoom::scale)
        {
            session.shell.overlays.show_zoom_indicator(
                output_name,
                live_scale,
                &session.settings.overlays.zoom_indicator,
                crate::frame_clock::monotonic_now(),
            );
        }
        session.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_navigation_actions_hide_the_cursor() {
        assert!(action_hides_cursor_for_keyboard_navigation(
            &swm_config::Action::Trail(swm_config::TrailDirection::Previous)
        ));
        assert!(action_hides_cursor_for_keyboard_navigation(
            &swm_config::Action::MonitorFocus(swm_config::MonitorTarget::Direction(
                swm_config::Direction::Left,
            ))
        ));
        assert!(!action_hides_cursor_for_keyboard_navigation(
            &swm_config::Action::ResizeWindow(swm_config::Direction::Left)
        ));
        assert!(!action_hides_cursor_for_keyboard_navigation(
            &swm_config::Action::ZoomOut
        ));
    }
}
