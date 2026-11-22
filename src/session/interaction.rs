use crate::input::grab::{Grab, ResizeAnchor};
use crate::input::pointer::WheelAccumulator;
use crate::input::{SuppressedButtons, SuppressedKeys};

/// Mutable state shared by compositor-owned input interactions.
pub struct InteractionState {
    grab: Grab,
    pub(crate) resize_anchor: Option<ResizeAnchor>,
    pub(crate) suppressed_buttons: SuppressedButtons,
    pub(crate) suppressed_keys: SuppressedKeys,
    pub(super) overlay_buttons: SuppressedButtons,
    pub(crate) wheel_accumulator: WheelAccumulator,
    pub(crate) zoom_fit: super::zoom_fit::ZoomFitState,
    pub(super) pointer_constraints: super::pointer::PointerConstraintLifecycle,
    pub(super) client_pointer_route: Option<super::pointer::ClientPointerRoute>,
    pub(crate) steam_close_pressed: Option<smithay::desktop::Window>,
}

impl InteractionState {
    pub(crate) fn translate_output_grab(
        &mut self,
        output: &str,
        delta: smithay::utils::Point<i32, smithay::utils::Logical>,
        nodes: &crate::nodes::NodesState,
    ) {
        let owns_window = |window: &smithay::desktop::Window| {
            crate::wayland::window_output_name(window).as_deref() == Some(output)
        };
        let owns_node = |id| {
            nodes
                .record(id)
                .is_some_and(|record| record.output == output)
        };
        let shift = |point: &mut swm_core::field::Vec2| {
            point.x += delta.x as f32;
            point.y += delta.y as f32;
        };
        match &mut self.grab {
            Grab::PendingWindowMove(pending) if pending.output == output => {
                pending.press_screen += delta.to_f64();
                pending.visual_geometry.loc += delta;
            }
            Grab::MoveWindow {
                window, last_world, ..
            } if owns_window(window) => shift(last_world),
            Grab::PendingNode {
                id, press_screen, ..
            } if owns_node(*id) => *press_screen += delta.to_f64(),
            Grab::MoveNode { id, last_world, .. } if owns_node(*id) => shift(last_world),
            Grab::ResizeWindow(resize) if owns_window(&resize.window) => {
                resize.start_rect.loc += delta;
                shift(&mut resize.start_cursor);
                resize.start_screen.0 += f64::from(delta.x);
                resize.start_screen.1 += f64::from(delta.y);
            }
            _ => {}
        }
    }

    /// The compositor pointer grab in progress, or [`Grab::None`].
    pub(crate) fn grab(&self) -> &Grab {
        &self.grab
    }

    pub(crate) fn has_grab(&self) -> bool {
        !matches!(self.grab, Grab::None)
    }

    /// Update the grab in progress, such as a drag's latest position. End a
    /// grab with `Session::end_grab` instead of overwriting it here.
    pub(crate) fn grab_mut(&mut self) -> &mut Grab {
        &mut self.grab
    }

    /// Start a compositor pointer grab, replacing any grab in progress.
    pub(crate) fn begin_grab(&mut self, grab: Grab) {
        debug_assert!(
            !matches!(grab, Grab::None),
            "end grabs with Session::end_grab so the grab cursor is dropped too"
        );
        self.grab = grab;
    }

    /// End the grab but keep the grab cursor, for a hand-off to a client
    /// move that goes on showing it. Everything else uses `Session::end_grab`.
    pub(crate) fn end_grab_keeping_cursor(&mut self) {
        self.grab = Grab::None;
    }
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            grab: Grab::None,
            resize_anchor: None,
            suppressed_buttons: SuppressedButtons::default(),
            suppressed_keys: SuppressedKeys::default(),
            overlay_buttons: SuppressedButtons::default(),
            wheel_accumulator: WheelAccumulator::default(),
            zoom_fit: super::zoom_fit::ZoomFitState::default(),
            pointer_constraints: super::pointer::PointerConstraintLifecycle::default(),
            client_pointer_route: None,
            steam_close_pressed: None,
        }
    }
}
