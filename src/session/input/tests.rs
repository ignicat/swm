use std::time::Duration;

use smithay::backend::input::{ButtonState, KeyState};
use smithay::utils::{Logical, Point};
use swm_core::field::Vec2;

use super::actions::window_action_output;
use super::keyboard::{ModalKeyRouting, modal_key_routing};
use super::{BTN_LEFT, BTN_RIGHT, PendingWindowMoveMotion};
use super::{
    drag_threshold_reached, forward_pointer_button, layer_allows_field_pan_always,
    pending_window_move_motion, pointer_move_falls_back_to_field_pan, releases_field_pan,
    releases_pending_window_move, sampled_drag_velocity, shortcut_policy_allows_bindings,
};
use crate::input::buttons::BTN_MIDDLE;
// Model the real two-phase dispatch: consume pending state before interception,
// then call the forwarding hook only for events delivered to the client.
fn steam_button(
    pending: &mut Option<u32>,
    button: u32,
    state: ButtonState,
    target: Option<u32>,
    forwarded: bool,
) -> Option<u32> {
    let previous = super::take_steam_close_press(pending, button);
    if forwarded {
        super::forwarded_steam_close_button(pending, previous, button, state, target)
    } else {
        None
    }
}

#[test]
fn steam_close_scrollbar_press_does_not_arm_close_release() {
    let mut pending = None;
    steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, None, true);
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        None
    );
}

#[test]
fn steam_close_matching_click_fires_once() {
    let mut pending = None;
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, Some(1), true),
        None
    );
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        Some(1)
    );
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        None
    );
}

#[test]
fn steam_close_release_elsewhere_cancels() {
    for target in [None, Some(2)] {
        let mut pending = None;
        steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, Some(1), true);
        assert_eq!(
            steam_button(&mut pending, BTN_LEFT, ButtonState::Released, target, true),
            None
        );
        assert_eq!(pending, None);
    }
}

#[test]
fn steam_close_intercepted_buttons_cannot_arm_or_retain_press() {
    let mut pending = None;
    steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, Some(1), false);
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        None
    );
    steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, Some(1), true);
    steam_button(
        &mut pending,
        BTN_LEFT,
        ButtonState::Released,
        Some(1),
        false,
    );
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        None
    );
}

#[test]
fn steam_close_new_press_replaces_stale_target() {
    let mut pending = Some(1);
    steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, None, true);
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        None
    );
}

#[test]
fn steam_close_other_buttons_neither_arm_nor_complete_left_click() {
    let mut pending = None;
    steam_button(&mut pending, BTN_RIGHT, ButtonState::Pressed, Some(1), true);
    assert_eq!(pending, None);
    steam_button(&mut pending, BTN_LEFT, ButtonState::Pressed, Some(1), true);
    assert_eq!(
        steam_button(
            &mut pending,
            BTN_RIGHT,
            ButtonState::Released,
            Some(1),
            true
        ),
        None
    );
    assert_eq!(
        steam_button(&mut pending, BTN_LEFT, ButtonState::Released, Some(1), true),
        Some(1)
    );
}

fn sample_constant_motion(report_hz: u32) -> Vec2 {
    let step = Duration::from_secs_f64(1.0 / f64::from(report_hz));
    let mut previous = Vec2 { x: 0.0, y: 0.0 };
    let mut velocity = Vec2 { x: 0.0, y: 0.0 };
    let mut last = Duration::ZERO;
    for index in 1..=report_hz {
        let now = step * index;
        let current = Vec2 {
            x: 400.0 * now.as_secs_f32(),
            y: -180.0 * now.as_secs_f32(),
        };
        velocity = sampled_drag_velocity(previous, current, velocity, last, now);
        previous = current;
        last = now;
    }
    velocity
}

#[test]
fn drag_velocity_is_stable_across_common_mouse_report_rates() {
    for report_hz in [125, 500, 1_000] {
        let sampled = sample_constant_motion(report_hz);
        assert!((sampled.x - 400.0).abs() < 0.1, "{report_hz} Hz");
        assert!((sampled.y + 180.0).abs() < 0.1, "{report_hz} Hz");
    }
}

#[test]
fn drag_velocity_ignores_duplicate_timestamps() {
    let previous_velocity = Vec2 { x: 25.0, y: -15.0 };
    let sampled = sampled_drag_velocity(
        Vec2 { x: 10.0, y: 20.0 },
        Vec2 { x: 50.0, y: 80.0 },
        previous_velocity,
        Duration::from_millis(5),
        Duration::from_millis(5),
    );
    assert_eq!(sampled, previous_velocity);
}

#[test]
fn maximized_titlebar_drag_ignores_double_click_jitter() {
    let press = Point::<f64, Logical>::from((400.0, 250.0));

    assert!(!drag_threshold_reached(press, (403.0, 254.0)));
    assert!(drag_threshold_reached(press, (408.0, 250.0)));
}

#[test]
fn move_window_grab_becomes_field_pan_on_background() {
    assert!(pointer_move_falls_back_to_field_pan(
        &crate::input::pointer::PointerTarget::Background
    ));
}

#[test]
fn camera_pan_ends_only_with_the_button_that_began_it() {
    use crate::input::grab::Grab;

    let pan = Grab::Pan {
        output: "DP-1".into(),
        button: BTN_MIDDLE,
    };
    assert!(releases_field_pan(&pan, BTN_MIDDLE, true));
    assert!(
        !releases_field_pan(&pan, BTN_MIDDLE, false),
        "the press that began the pan must not end it"
    );
    assert!(
        !releases_field_pan(&pan, BTN_LEFT, true),
        "releasing another button must not drop a middle-button pan"
    );
    assert!(!releases_field_pan(&pan, BTN_RIGHT, true));

    // A left-button pan still ends on the left release.
    let left = Grab::Pan {
        output: "DP-1".into(),
        button: BTN_LEFT,
    };
    assert!(releases_field_pan(&left, BTN_LEFT, true));
    assert!(!releases_field_pan(&left, BTN_MIDDLE, true));
    assert!(!releases_field_pan(&Grab::None, BTN_LEFT, true));
}

#[test]
fn always_pan_covers_desktop_layers_but_not_panels_or_overlays() {
    use smithay::wayland::shell::wlr_layer::Layer;

    assert!(layer_allows_field_pan_always(Layer::Background));
    assert!(layer_allows_field_pan_always(Layer::Bottom));
    assert!(!layer_allows_field_pan_always(Layer::Top));
    assert!(!layer_allows_field_pan_always(Layer::Overlay));
}

#[test]
fn client_titlebar_move_forwards_its_consumed_release() {
    assert!(forward_pointer_button(true, true));
    assert!(!forward_pointer_button(true, false));
    assert!(forward_pointer_button(false, false));
}

#[test]
fn client_titlebar_click_releases_without_activating_move() {
    assert!(releases_pending_window_move(BTN_LEFT, BTN_LEFT, true));
    assert!(!releases_pending_window_move(BTN_LEFT, BTN_LEFT, false));
    assert!(!releases_pending_window_move(BTN_LEFT, BTN_RIGHT, true));
}

#[test]
fn client_titlebar_move_activates_only_after_valid_threshold_motion() {
    let press = Point::<f64, Logical>::from((400.0, 250.0));

    assert_eq!(
        pending_window_move_motion(true, press, (403.0, 254.0)),
        PendingWindowMoveMotion::Wait,
    );
    assert_eq!(
        pending_window_move_motion(true, press, (408.0, 250.0)),
        PendingWindowMoveMotion::Activate,
    );
    assert_eq!(
        pending_window_move_motion(false, press, (420.0, 250.0)),
        PendingWindowMoveMotion::Cancel,
    );
}

#[test]
fn shortcut_policy_respects_session_lock_and_client_inhibition() {
    assert!(shortcut_policy_allows_bindings(false, false));
    assert!(!shortcut_policy_allows_bindings(true, false));
    assert!(!shortcut_policy_allows_bindings(false, true));
    assert!(!shortcut_policy_allows_bindings(true, true));
}

#[test]
fn keyboard_actions_ignore_stationary_pointer_in_both_focus_modes() {
    for mode in [swm_config::FocusMode::Hover, swm_config::FocusMode::Click] {
        assert_eq!(
            window_action_output(
                super::actions::DispatchOrigin::Keyboard,
                mode,
                Some("left"),
                Some("right")
            ),
            Some("right".into())
        );
        assert_eq!(
            window_action_output(
                super::actions::DispatchOrigin::Keyboard,
                mode,
                Some("left"),
                None
            ),
            None
        );
    }
}

#[test]
fn window_actions_follow_pointer_only_in_hover_mode() {
    assert_eq!(
        window_action_output(
            super::actions::DispatchOrigin::Other,
            swm_config::FocusMode::Hover,
            Some("right"),
            Some("left"),
        ),
        Some("right".to_string())
    );
    assert_eq!(
        window_action_output(
            super::actions::DispatchOrigin::Other,
            swm_config::FocusMode::Click,
            Some("right"),
            Some("left"),
        ),
        Some("left".to_string())
    );
    assert_eq!(
        window_action_output(
            super::actions::DispatchOrigin::Other,
            swm_config::FocusMode::Hover,
            None,
            Some("left")
        ),
        Some("left".to_string())
    );
}

#[test]
fn modal_releases_retire_preexisting_forwarded_keys() {
    assert_eq!(
        modal_key_routing(true, KeyState::Released, false),
        ModalKeyRouting::RetireUnfocusedRelease
    );
    assert_eq!(
        modal_key_routing(false, KeyState::Released, true),
        ModalKeyRouting::SuppressRelease
    );
    assert_eq!(
        modal_key_routing(true, KeyState::Pressed, false),
        ModalKeyRouting::Evaluate
    );
}
