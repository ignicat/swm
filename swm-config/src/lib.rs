mod aliases;
pub mod animations;
pub mod background;
pub mod bootstrap;
pub mod chord;
mod color;
pub mod cursor;
pub mod decorations;
pub mod diagnostic;
pub mod field;
pub mod font;
pub mod input;
pub mod keybinds;
pub mod launch;
pub mod nodes;
pub mod output;
pub mod overlays;
pub mod parse;
pub mod physics;
pub mod rules;
pub mod runtime;
pub mod terminal;
pub mod trail;
pub mod view;
pub mod zoom;

pub use animations::{
    AnimationCurve, AnimationMotion, Animations, EasingMotion, FullscreenAnimation,
    MaximizeAnimation, NodeAnimation, SmoothResizeAnimation, SpringMotion, WindowCloseAnimation,
    WindowCloseAnimationType, WindowMoveAnimation, WindowOpenAnimation, WindowOpenAnimationType,
    parse_animations,
};
pub use background::{
    Background, BackgroundColor, BackgroundFit, BackgroundMode, BackgroundParseError,
    parse_background,
};
pub use bootstrap::{
    DEFAULT_CONFIG, bootstrap_default_config, bootstrap_default_config_at, config_path,
};
pub use chord::parse_chord;
pub use cursor::{Cursor, parse_cursor};
pub use decorations::{BorderColor, Decorations, parse_decorations};
pub use diagnostic::ConfigDiagnostic;
pub use field::{
    CloseRestorePan, Field, FieldParseError, PinBadgeCorner, Pins, parse_field, parse_field_checked,
};
pub use font::{Font, parse_font};
pub use input::{
    AccelProfile, ClickMethod, DeviceKind, DeviceOverride, DeviceSettings, FocusMode,
    GestureAction, GestureBinding, GestureHoldBinding, GestureModifier, GestureScope,
    GestureSettings, GestureSwipeDirection, Input, InputParseError, KeyboardConfig, MouseSettings,
    ScrollMethod, ScrollPanMode, TapButtonMap, parse_input,
};
pub use keybinds::{
    Action, BindingScope, Direction, FocusCycleDirection, Keybind, Keybinds, ModifierKey,
    Modifiers, MonitorTarget, TrailDirection,
};
pub use launch::{Autostart, LaunchConfigError, parse_autostart, parse_env};
pub use nodes::{
    Debug, Decay, LandmarkPlacement, NodeBackgroundColor, NodeDisplayPolicy, NodeParseError,
    NodeShape, Nodes, RestoreCentering, parse_debug, parse_decay, parse_nodes_checked,
};
pub use output::{OutputConfig, Vrr};
pub use overlays::{
    NotificationPosition, Notifications, OverlayColorMode, OverlayParseError, Overlays,
    SystemColorScheme, ZoomIndicator, parse_overlays_checked,
};
pub use parse::{ParseError, parse_keybinds};
pub use physics::{Physics, parse_physics};
pub use rules::{
    LayerRule, LayerShellLayer, RulePattern, Rules, WindowRule, WindowRuleParseError,
    WindowRulePattern, WindowSpawnPlacement, parse_rules, parse_window_rules,
};
pub use runtime::{
    RuntimeConfig, RuntimeConfigError, load_runtime_config_at, load_runtime_config_diagnostic_at,
    parse_runtime_config,
};
pub use terminal::{TERMINAL_PRIORITY, resolve_default_terminal, resolve_default_terminal_in_path};
pub use trail::{Trail, parse_trail};
pub use view::{ViewConfig, ViewParseError, parse_view_checked};
pub use zoom::Zoom;
