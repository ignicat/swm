/// The base `mod` key a keybind chord is built on. Generic and physical-side
/// variants are retained so `mod "lsuper"` remains distinct from `super`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKey {
    Super,
    LeftSuper,
    RightSuper,
    Alt,
    LeftAlt,
    RightAlt,
    Ctrl,
    LeftCtrl,
    RightCtrl,
    Shift,
    LeftShift,
    RightShift,
}

/// A resolved modifier combination for a single keybind (the base `mod` plus
/// any extra modifiers in the chord, e.g. `mod+shift+e`).
///
/// Generic flags match either physical side. Side flags require that exact
/// key, using the compositor's raw evdev/XKB keycode tracking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub left_shift: bool,
    pub right_shift: bool,
    pub ctrl: bool,
    pub left_ctrl: bool,
    pub right_ctrl: bool,
    pub alt: bool,
    pub left_alt: bool,
    pub right_alt: bool,
    pub super_key: bool,
    pub left_super: bool,
    pub right_super: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BindingScope {
    #[default]
    Global,
    Field,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusCycleDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrailDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonitorTarget {
    Direction(Direction),
    Output(String),
}

/// Compositor or shell action bound to a key chord.
///
/// `ZoomIn` steps back toward 1.0x zoom; `src/input/zoom.rs` enforces the 1.0x ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    CloseFocusedWindow,
    ToggleFullscreen,
    ToggleFieldMaximize,
    ToggleState,
    ToggleFocusedPin,
    /// Zoom every Field monitor out to fit everything on it, or zoom back.
    ZoomFit,
    FocusCycle(FocusCycleDirection),
    Trail(TrailDirection),
    FocusDirection(Direction),
    /// Move the focused or most-recent Field node by one placement step.
    MoveNode(Direction),
    /// Move along the requested axis by up to a positive number of Field units.
    MoveNodeBy {
        direction: Direction,
        distance: u32,
    },
    TransferWindow(Direction),
    PanField(Direction),
    /// Resize the focused Field window by one placement step. Left and up
    /// shrink; right and down grow.
    ResizeWindow(Direction),
    /// Begin an interactive compositor move for the window under the pointer,
    /// falling back to Field panning when the grab starts on empty background.
    PointerMoveWindow,
    /// Begin an interactive compositor resize for the window under the pointer.
    PointerResizeWindow,
    /// Pan the Field from an empty-background pointer drag.
    PointerPanField,
    /// Pan the Field from a pointer drag that may begin anywhere on an output:
    /// over windows, borders, and collapsed nodes as well as empty
    /// background. It declines (and the press reaches the client) only while
    /// fullscreen or Field maximize owns the camera, over shell layers such as
    /// panels and overlays, and while a client holds a pointer lock or confinement.
    PointerPanFieldAlways,
    /// Keep a grabbed window on its output and pan that Field after dwelling
    /// against an edge.
    PointerDragPan,
    CenterLastFocused,
    MonitorFocus(MonitorTarget),
    Reload,
    OpenTerminal,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// A user-provided command line. This is the fallback for every action
    /// string that is not one of SWM's compositor actions.
    Spawn(String),
}

impl Action {
    /// Whether a compact keybind repeats when it does not provide an explicit
    /// `repeat` override. Continuous navigation and geometry actions repeat;
    /// destructive, modal, toggle, and process-launching actions do not.
    pub fn repeats_by_default(&self) -> bool {
        matches!(
            self,
            Self::FocusCycle(_)
                | Self::Trail(_)
                | Self::FocusDirection(_)
                | Self::MoveNode(_)
                | Self::MoveNodeBy { .. }
                | Self::PanField(_)
                | Self::ResizeWindow(_)
                | Self::MonitorFocus(_)
                | Self::ZoomIn
                | Self::ZoomOut
        )
    }

    pub fn default_scope(&self) -> BindingScope {
        match self {
            Self::MoveNode(_)
            | Self::MoveNodeBy { .. }
            | Self::TransferWindow(_)
            | Self::PanField(_)
            | Self::ResizeWindow(_)
            | Self::ToggleFocusedPin => BindingScope::Field,
            Self::PointerPanField | Self::PointerPanFieldAlways | Self::PointerDragPan => {
                BindingScope::Field
            }
            _ => BindingScope::Global,
        }
    }
}

/// A single parsed keybind, including the presentation context in which it is
/// active. Duplicate chords are valid when their scopes do not overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keybind {
    pub scope: BindingScope,
    pub modifiers: Modifiers,
    pub key: String,
    pub action: Action,
    /// Whether holding this keyboard trigger repeats the action using the
    /// compositor's configured input repeat delay and rate.
    pub repeat: bool,
}

/// Parsed `keybinds` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keybinds {
    pub modifier: ModifierKey,
    pub binds: Vec<Keybind>,
}

impl Default for Keybinds {
    /// Parse the same template written by the bootstrap path so the runtime
    /// fallback and fresh-install config cannot drift apart.
    fn default() -> Self {
        let config = rune_cfg::RuneConfig::from_str(crate::bootstrap::DEFAULT_CONFIG)
            .expect("embedded default config must remain valid");
        crate::parse::parse_keybinds(&config).expect("embedded default keybinds must remain valid")
    }
}
