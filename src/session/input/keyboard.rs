use super::*;

enum KeyboardOutcome {
    Action(crate::input::keybinds::ResolvedBind),
    ExitConfirm,
    ExitCancel,
    ExitIntercept,
    AccessibilityIntercept,
    ZoomFitLeave,
    FocusCycleCancel,
    FocusCycleIntercept,
    CaptureAccept,
    CaptureCancel,
    CapturePrevious,
    CaptureNext,
    CaptureIntercept,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModalKeyRouting {
    Evaluate,
    RetireUnfocusedRelease,
    SuppressRelease,
}

pub(super) fn modal_key_routing(
    modal_active: bool,
    state: KeyState,
    release_is_suppressed: bool,
) -> ModalKeyRouting {
    if state == KeyState::Released && release_is_suppressed {
        ModalKeyRouting::SuppressRelease
    } else if modal_active && state == KeyState::Released {
        ModalKeyRouting::RetireUnfocusedRelease
    } else {
        ModalKeyRouting::Evaluate
    }
}

pub(super) fn handle<D, B>(
    session: &mut Session<D>,
    key_event: &B::KeyboardKeyEvent,
    socket_name: &OsStr,
) where
    D: SessionDriver,
    B: InputBackend,
{
    session.interactions.wheel_accumulator.reset_all();
    let keycode = key_event.key_code();
    let state = key_event.state();
    session.keyboard.side_modifiers.update(keycode, state);
    let time = key_event.time();
    if state == KeyState::Released {
        session.key_repeat.release(keycode);
    } else {
        session.key_repeat.cancel();
    }
    if state == KeyState::Pressed && session.cursor_policy.keyboard_press() {
        session.request_redraw();
    }
    let release_is_suppressed = state == KeyState::Released
        && session
            .interactions
            .suppressed_keys
            .release_is_suppressed(keycode);
    let accessibility = if session.shell.overlays.confirmation_modal_active() {
        crate::accessibility::KeyboardDisposition::Pass
    } else {
        crate::accessibility::process_key(
            session,
            std::time::Duration::from_millis(u64::from(time.millis())),
            keycode,
            state,
        )
    };
    let keyboard = session.keyboard_handle();
    let bindings_enabled = bindings_enabled(session);
    session.keyboard.delivery.begin();
    let action = keyboard.input::<KeyboardOutcome, _>(
        session,
        keycode,
        state,
        SERIAL_COUNTER.next_serial(),
        time,
        |data, modifiers, handle| {
            if data.shell.overlays.exit_modal_active() {
                if state == KeyState::Released {
                    return if release_is_suppressed {
                        FilterResult::Intercept(KeyboardOutcome::ExitIntercept)
                    } else {
                        FilterResult::Forward
                    };
                }
                return match handle.raw_latin_sym_or_raw_current_sym() {
                    Some(Keysym::Return | Keysym::KP_Enter) => {
                        FilterResult::Intercept(KeyboardOutcome::ExitConfirm)
                    }
                    Some(Keysym::Escape) => FilterResult::Intercept(KeyboardOutcome::ExitCancel),
                    _ => FilterResult::Intercept(KeyboardOutcome::ExitIntercept),
                };
            }
            // Orca's grabbed keys must not also drive compositor overlays.
            // Confirmation dialogs remain above this path so their protected
            // input policy is unchanged.
            if accessibility == crate::accessibility::KeyboardDisposition::Intercept {
                return FilterResult::Intercept(KeyboardOutcome::AccessibilityIntercept);
            }
            if state == KeyState::Pressed
                && handle.raw_latin_sym_or_raw_current_sym() == Some(Keysym::Escape)
                && crate::session::zoom_fit::showing(data)
            {
                return FilterResult::Intercept(KeyboardOutcome::ZoomFitLeave);
            }
            if data.shell.focus_cycle.is_open() {
                let sym = handle.raw_latin_sym_or_raw_current_sym();
                if state == KeyState::Released && matches!(sym, Some(Keysym::Alt_L | Keysym::Alt_R))
                {
                    return FilterResult::Forward;
                }
                if state == KeyState::Pressed && sym == Some(Keysym::Escape) {
                    return FilterResult::Intercept(KeyboardOutcome::FocusCycleCancel);
                }
                if state == KeyState::Pressed
                    && let Some(bind) = match_keyboard_binding(
                        &data.keyboard.binds,
                        modifiers,
                        data.keyboard.side_modifiers,
                        crate::input::BindingContext::default(),
                        sym,
                        keycode,
                    )
                    && matches!(bind.action, swm_config::Action::FocusCycle(_))
                {
                    return FilterResult::Intercept(KeyboardOutcome::Action(bind.clone()));
                }
                return FilterResult::Intercept(KeyboardOutcome::FocusCycleIntercept);
            }
            match modal_key_routing(data.capture.is_active(), state, release_is_suppressed) {
                ModalKeyRouting::SuppressRelease => {
                    return FilterResult::Intercept(KeyboardOutcome::CaptureIntercept);
                }
                ModalKeyRouting::RetireUnfocusedRelease => {
                    // Focus is cleared for the lifetime of the overlay. Forwarding
                    // releases here reaches no client, but lets Smithay retire keys
                    // whose presses were forwarded before the modal opened.
                    return FilterResult::Forward;
                }
                ModalKeyRouting::Evaluate => {}
            }
            if data.capture.is_active() {
                let sym = handle.raw_latin_sym_or_raw_current_sym();
                return match sym {
                    Some(Keysym::Escape) => FilterResult::Intercept(KeyboardOutcome::CaptureCancel),
                    Some(Keysym::Return | Keysym::KP_Enter) => {
                        FilterResult::Intercept(KeyboardOutcome::CaptureAccept)
                    }
                    Some(Keysym::Left | Keysym::Up) => {
                        FilterResult::Intercept(KeyboardOutcome::CapturePrevious)
                    }
                    Some(Keysym::Right | Keysym::Down) => {
                        FilterResult::Intercept(KeyboardOutcome::CaptureNext)
                    }
                    _ => FilterResult::Intercept(KeyboardOutcome::CaptureIntercept),
                };
            }
            if state != KeyState::Pressed {
                return FilterResult::Forward;
            }
            let sym = handle.raw_latin_sym_or_raw_current_sym();
            if !bindings_enabled {
                return FilterResult::Forward;
            }
            let context = super::keyboard_binding_context(data);
            match match_keyboard_binding(
                &data.keyboard.binds,
                modifiers,
                data.keyboard.side_modifiers,
                context,
                sym,
                keycode,
            ) {
                Some(bind) => FilterResult::Intercept(KeyboardOutcome::Action(bind.clone())),
                None => FilterResult::Forward,
            }
        },
    );
    crate::input::keyboard_delivery::finish(session);
    let pointer_output = session
        .wayland
        .space
        .output_under(session.pointer.position())
        .next()
        .map(Output::name);

    match action {
        Some(KeyboardOutcome::ExitConfirm) => {
            session.interactions.suppressed_keys.suppress(keycode);
            session.confirm_exit();
        }
        Some(KeyboardOutcome::ExitCancel) => {
            session.interactions.suppressed_keys.suppress(keycode);
            session.cancel_exit_confirmation();
        }
        Some(KeyboardOutcome::ExitIntercept) => {
            if state == KeyState::Pressed {
                session.interactions.suppressed_keys.suppress(keycode);
            }
        }
        Some(KeyboardOutcome::Action(bind)) => {
            session.interactions.suppressed_keys.suppress(keycode);
            actions::dispatch(
                session,
                bind.action.clone(),
                socket_name,
                pointer_output.as_deref(),
                actions::DispatchOrigin::Keyboard,
            );
            session.key_repeat.start(
                keycode,
                bind,
                session.settings.input.repeat_delay,
                session.settings.input.repeat_rate,
            );
        }
        Some(KeyboardOutcome::AccessibilityIntercept) => {}
        Some(KeyboardOutcome::ZoomFitLeave) => {
            session.interactions.suppressed_keys.suppress(keycode);
            crate::session::zoom_fit::leave(session);
        }
        Some(KeyboardOutcome::FocusCycleCancel) => {
            session.interactions.suppressed_keys.suppress(keycode);
            crate::shell::focus_cycle::cancel(session);
        }
        Some(KeyboardOutcome::FocusCycleIntercept) => {
            if state == KeyState::Pressed {
                session.interactions.suppressed_keys.suppress(keycode);
            }
        }
        Some(KeyboardOutcome::CaptureAccept) => {
            if session.capture.menu_is_active() {
                if session.capture.activate_selected_menu() {
                    update_capture_pointer(session, session.pointer.position());
                    session.request_redraw();
                }
            } else if crate::capture::accept_selected(session) {
                session.interactions.suppressed_keys.suppress(keycode);
            }
        }
        Some(KeyboardOutcome::CaptureCancel) => {
            if session.capture.return_to_menu() {
                session.request_redraw();
            } else if crate::capture::cancel_selected(session) {
                session.interactions.suppressed_keys.suppress(keycode);
            }
        }
        Some(KeyboardOutcome::CapturePrevious) => {
            if session.capture.move_menu_selection(-1)
                || crate::capture::move_source_selection(session, -1)
            {
                session.request_redraw();
            }
        }
        Some(KeyboardOutcome::CaptureNext) => {
            if session.capture.move_menu_selection(1)
                || crate::capture::move_source_selection(session, 1)
            {
                session.request_redraw();
            }
        }
        Some(KeyboardOutcome::CaptureIntercept) | None => {}
    }

    if state == KeyState::Released
        && session.shell.focus_cycle.is_open()
        && !keyboard.modifier_state().alt
    {
        crate::shell::focus_cycle::commit(session, SERIAL_COUNTER.next_serial());
    }
}
