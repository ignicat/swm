//! Translate configured actions into session commands.

use std::ffi::OsStr;

use swm_config::Action;
use swm_core::camera::Camera;

use crate::session::{environment, spawn};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SessionControl {
    Continue,
    Quit,
    CloseFocusedWindow,
    ToggleFullscreen,
    ToggleFieldMaximize,
    ToggleState,
    ToggleFocusedPin,
    ZoomFit,
    FocusCycle(swm_config::FocusCycleDirection),
    Trail(swm_config::TrailDirection),
    FocusDirection(swm_config::Direction),
    MoveNode(swm_config::Direction),
    MoveNodeBy {
        direction: swm_config::Direction,
        distance: u32,
    },
    TransferWindow(swm_config::Direction),
    PanField(swm_config::Direction),
    ResizeWindow(swm_config::Direction),
    CenterLastFocused,
    MonitorFocus(swm_config::MonitorTarget),
    Reload,
}

#[derive(Clone, Copy)]
pub(super) struct SpawnContext<'a> {
    pub(super) socket_name: &'a OsStr,
    pub(super) x11_display: Option<&'a OsStr>,
    pub(super) cursor_size: u8,
    pub(super) environment: &'a environment::LaunchEnvironment,
}

/// Interprets every configured action once for both session backends.
/// Backends provide the camera selected by their own output routing and
/// translate the returned quit request into their loop's native mechanism.
pub(super) fn dispatch_action(
    action: Action,
    terminal_command: Option<&str>,
    spawn_context: SpawnContext<'_>,
    camera: Option<&mut Camera>,
    zoom: &swm_config::Zoom,
) -> SessionControl {
    match action {
        Action::Quit => return SessionControl::Quit,
        Action::CloseFocusedWindow => return SessionControl::CloseFocusedWindow,
        Action::ToggleFullscreen => return SessionControl::ToggleFullscreen,
        Action::ToggleFieldMaximize => return SessionControl::ToggleFieldMaximize,
        Action::ToggleState => return SessionControl::ToggleState,
        Action::ToggleFocusedPin => return SessionControl::ToggleFocusedPin,
        Action::ZoomFit => return SessionControl::ZoomFit,
        Action::FocusCycle(direction) => return SessionControl::FocusCycle(direction),
        Action::Trail(direction) => return SessionControl::Trail(direction),
        Action::FocusDirection(direction) => return SessionControl::FocusDirection(direction),
        Action::TransferWindow(direction) => return SessionControl::TransferWindow(direction),
        Action::PanField(direction) => return SessionControl::PanField(direction),
        Action::MoveNode(direction) => return SessionControl::MoveNode(direction),
        Action::MoveNodeBy {
            direction,
            distance,
        } => {
            return SessionControl::MoveNodeBy {
                direction,
                distance,
            };
        }
        Action::ResizeWindow(direction) => return SessionControl::ResizeWindow(direction),
        Action::PointerMoveWindow
        | Action::PointerResizeWindow
        | Action::PointerPanField
        | Action::PointerPanFieldAlways
        | Action::PointerDragPan => {
            eventline::warn!("keybinds: pointer grab action used outside a pointer-button binding")
        }
        Action::CenterLastFocused => return SessionControl::CenterLastFocused,
        Action::MonitorFocus(direction) => return SessionControl::MonitorFocus(direction),
        Action::Reload => return SessionControl::Reload,
        Action::OpenTerminal => match terminal_command {
            Some(command) => spawn::spawn_detached(
                command,
                spawn_context.socket_name,
                spawn_context.x11_display,
                spawn_context.cursor_size,
                spawn_context.environment,
            ),
            None => eventline::warn!("keybinds: no terminal configured or found on PATH"),
        },
        Action::ZoomOut => {
            if let Some(camera) = camera {
                crate::input::zoom::zoom_out(camera, zoom);
            }
        }
        Action::ZoomIn => {
            if let Some(camera) = camera {
                crate::input::zoom::zoom_in(camera, zoom);
            }
        }
        Action::ZoomReset => {
            if let Some(camera) = camera {
                camera.reset_zoom_target();
            }
        }
        Action::Spawn(command) => spawn::spawn_detached(
            &command,
            spawn_context.socket_name,
            spawn_context.x11_display,
            spawn_context.cursor_size,
            spawn_context.environment,
        ),
    }
    SessionControl::Continue
}
