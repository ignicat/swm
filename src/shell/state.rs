/// Renderer-independent state owned by the compositor shell.
///
/// Shell features remain separate state machines, but their shared ownership
/// is explicit at the session boundary instead of appearing as unrelated
/// global fields.
pub struct ShellState {
    pub(crate) overlays: super::overlay::OverlayManager,
    pub(crate) focus_cycle: super::focus_cycle::FocusCycleState,
}

impl ShellState {
    pub fn new() -> Self {
        Self {
            overlays: super::overlay::OverlayManager::default(),
            focus_cycle: super::focus_cycle::FocusCycleState::default(),
        }
    }
}
