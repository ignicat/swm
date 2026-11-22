use super::*;

// Runs before touch and gesture dispatch, so owned streams never reach clients.
pub(super) fn intercept_early<D: SessionDriver, B: InputBackend>(
    session: &mut Session<D>,
    event: &InputEvent<B>,
) -> bool {
    if session.shell.overlays.confirmation_modal_active()
        && !matches!(event, InputEvent::Keyboard { .. })
    {
        session.interactions.steam_close_pressed = None;
        match event {
            InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. } => {
                session
                    .pointer
                    .process_input_event(event, &session.wayland.space);
                session.cursor_policy.pointer_activity();
                session.request_redraw();
            }
            InputEvent::PointerButton { event } => match event.state() {
                ButtonState::Pressed => session
                    .interactions
                    .suppressed_buttons
                    .suppress(event.button_code()),
                ButtonState::Released => {
                    session
                        .interactions
                        .suppressed_buttons
                        .release_is_suppressed(event.button_code());
                }
            },
            InputEvent::PointerAxis { .. } => session.interactions.wheel_accumulator.reset_all(),
            _ => {}
        }
        return true;
    }
    false
}

pub(super) fn capture<D: SessionDriver, B: InputBackend>(
    session: &mut Session<D>,
    event: &InputEvent<B>,
    proposed_position: (f64, f64),
) -> bool {
    if session.capture.is_active() {
        match event {
            InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. } => {
                update_capture_pointer(session, proposed_position);
                session.request_redraw();
                return true;
            }
            InputEvent::PointerButton { event } => {
                if event.button_code() == BTN_LEFT {
                    update_capture_pointer(session, proposed_position);
                    match event.state() {
                        ButtonState::Pressed => {
                            session.interactions.suppressed_buttons.suppress(BTN_LEFT);
                            match session.capture.press(proposed_position) {
                                Some(crate::capture::CapturePress::ActivateSource(mode)) => {
                                    if session.capture.activate_source(mode) {
                                        update_capture_pointer(session, proposed_position);
                                    }
                                }
                                Some(crate::capture::CapturePress::Accept) => {
                                    crate::capture::accept_selected(session);
                                }
                                Some(crate::capture::CapturePress::Consumed) | None => {}
                            }
                        }
                        ButtonState::Released => {
                            session
                                .interactions
                                .suppressed_buttons
                                .release_is_suppressed(BTN_LEFT);
                            session.capture.release();
                        }
                    }
                }
                session.request_redraw();
                return true;
            }
            InputEvent::PointerAxis { .. } => return true,
            _ => {}
        }
    }
    false
}
