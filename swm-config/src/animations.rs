use rune_cfg::RuneConfig;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnimationCurve {
    #[default]
    Linear,
    EaseInOutCubic,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
    Elastic,
}

impl AnimationCurve {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "linear" => Some(Self::Linear),
            "ease-in-out-cubic" => Some(Self::EaseInOutCubic),
            "ease-out-quad" => Some(Self::EaseOutQuad),
            "ease-out-cubic" => Some(Self::EaseOutCubic),
            "ease-out-expo" => Some(Self::EaseOutExpo),
            "elastic" => Some(Self::Elastic),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EasingMotion {
    pub duration_ms: u32,
    pub curve: AnimationCurve,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringMotion {
    pub damping_ratio: f64,
    pub stiffness: f64,
}

impl Default for SpringMotion {
    fn default() -> Self {
        Self {
            damping_ratio: 1.0,
            stiffness: 800.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimationMotion {
    Easing(EasingMotion),
    Spring(SpringMotion),
}

impl AnimationMotion {
    fn parse(config: &RuneConfig, path: &str, default: Self) -> Self {
        let kind = config
            .get_optional::<String>(&format!("{path}.motion"))
            .ok()
            .flatten();

        match kind.as_deref() {
            Some("spring") => {
                let defaults = match default {
                    Self::Spring(defaults) => defaults,
                    Self::Easing(_) => SpringMotion::default(),
                };
                Self::Spring(SpringMotion {
                    damping_ratio: finite_clamp(
                        config.get_or(&format!("{path}.damping-ratio"), defaults.damping_ratio),
                        0.1,
                        10.0,
                        defaults.damping_ratio,
                    ),
                    stiffness: finite_clamp(
                        config.get_or(&format!("{path}.stiffness"), defaults.stiffness),
                        1.0,
                        100_000.0,
                        defaults.stiffness,
                    ),
                })
            }
            Some("easing") => {
                let defaults = match default {
                    Self::Easing(defaults) => defaults,
                    Self::Spring(_) => EasingMotion {
                        duration_ms: 250,
                        curve: AnimationCurve::EaseOutCubic,
                    },
                };
                Self::Easing(EasingMotion {
                    duration_ms: config
                        .get_or(&format!("{path}.duration-ms"), defaults.duration_ms),
                    curve: config
                        .get_optional::<String>(&format!("{path}.curve"))
                        .ok()
                        .flatten()
                        .and_then(|curve| AnimationCurve::parse(&curve))
                        .unwrap_or(defaults.curve),
                })
            }
            _ => default,
        }
    }
}

fn finite_clamp(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowOpenAnimationType {
    #[default]
    CenterOut,
    Fade,
    Launch,
}

impl WindowOpenAnimationType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "center-out" => Some(Self::CenterOut),
            "fade" => Some(Self::Fade),
            "launch" => Some(Self::Launch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowOpenAnimation {
    pub enabled: bool,
    pub animation_type: WindowOpenAnimationType,
    pub motion: AnimationMotion,
    /// Optional fragment-shader path. Relative paths resolve from the
    /// directory that contains `swm.rune`. Empty or omitted means the
    /// configured `type` draws the pixels.
    pub custom_shader: Option<String>,
}

impl Default for WindowOpenAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            animation_type: WindowOpenAnimationType::default(),
            motion: AnimationMotion::Easing(EasingMotion {
                duration_ms: 300,
                curve: AnimationCurve::Linear,
            }),
            custom_shader: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowCloseAnimationType {
    #[default]
    Shrink,
    Fade,
    Retract,
}

impl WindowCloseAnimationType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "shrink" => Some(Self::Shrink),
            "fade" => Some(Self::Fade),
            "retract" => Some(Self::Retract),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowCloseAnimation {
    pub enabled: bool,
    pub animation_type: WindowCloseAnimationType,
    pub duration_ms: u32,
    /// Optional fragment-shader path. Relative paths resolve from the
    /// directory that contains `swm.rune`. Empty or omitted means the
    /// configured `type` draws the pixels.
    pub custom_shader: Option<String>,
}

impl Default for WindowCloseAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            animation_type: WindowCloseAnimationType::default(),
            duration_ms: 270,
            custom_shader: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullscreenAnimation {
    pub enabled: bool,
    pub motion: AnimationMotion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeAnimation {
    pub enabled: bool,
    pub duration_ms: u32,
    pub collapse_duration_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmoothResizeAnimation {
    pub enabled: bool,
    pub duration_ms: u32,
}

impl Default for SmoothResizeAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: 90,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaximizeAnimation {
    pub enabled: bool,
    pub motion: AnimationMotion,
}

impl Default for MaximizeAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            motion: AnimationMotion::Easing(EasingMotion {
                duration_ms: 240,
                curve: AnimationCurve::EaseInOutCubic,
            }),
        }
    }
}

/// Ordinary programmatic Field movement; pointer dragging stays direct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMoveAnimation {
    pub enabled: bool,
    pub motion: AnimationMotion,
}

impl Default for WindowMoveAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            motion: AnimationMotion::Easing(EasingMotion {
                duration_ms: 250,
                curve: AnimationCurve::EaseOutCubic,
            }),
        }
    }
}

impl Default for NodeAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: 280,
            collapse_duration_ms: 280,
        }
    }
}

impl Default for FullscreenAnimation {
    fn default() -> Self {
        Self {
            enabled: true,
            motion: AnimationMotion::Spring(SpringMotion::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animations {
    pub enabled: bool,
    pub window_open: WindowOpenAnimation,
    pub window_close: WindowCloseAnimation,
    pub window_move: WindowMoveAnimation,
    pub fullscreen: FullscreenAnimation,
    pub maximize: MaximizeAnimation,
    pub smooth_resize: SmoothResizeAnimation,
    pub node: NodeAnimation,
}

impl Default for Animations {
    fn default() -> Self {
        Self {
            enabled: true,
            window_open: WindowOpenAnimation::default(),
            window_close: WindowCloseAnimation::default(),
            window_move: WindowMoveAnimation::default(),
            fullscreen: FullscreenAnimation::default(),
            maximize: MaximizeAnimation::default(),
            smooth_resize: SmoothResizeAnimation::default(),
            node: NodeAnimation::default(),
        }
    }
}

pub fn parse_animations(config: &RuneConfig) -> Animations {
    let defaults = Animations::default();
    let animation_type = config
        .get_optional::<String>("animations.window-open.type")
        .ok()
        .flatten()
        .and_then(|value| WindowOpenAnimationType::parse(&value))
        .unwrap_or(defaults.window_open.animation_type);
    let default_easing = match defaults.window_open.motion {
        AnimationMotion::Easing(easing) => easing,
        AnimationMotion::Spring(_) => unreachable!("window-open defaults use easing motion"),
    };
    let curve = config
        .get_optional::<String>("animations.window-open.curve")
        .ok()
        .flatten()
        .and_then(|curve| AnimationCurve::parse(&curve))
        .unwrap_or(default_easing.curve);
    let configured_easing = AnimationMotion::Easing(EasingMotion {
        duration_ms: config.get_or(
            "animations.window-open.duration-ms",
            default_easing.duration_ms,
        ),
        curve,
    });
    let window_open_motion =
        AnimationMotion::parse(config, "animations.window-open", configured_easing);

    Animations {
        enabled: config.get_or("animations.enabled", defaults.enabled),
        window_open: WindowOpenAnimation {
            enabled: config.get_or(
                "animations.window-open.enabled",
                defaults.window_open.enabled,
            ),
            animation_type,
            motion: window_open_motion,
            custom_shader: optional_shader_path(config, "animations.window-open.custom-shader"),
        },
        window_close: WindowCloseAnimation {
            enabled: config.get_or(
                "animations.window-close.enabled",
                defaults.window_close.enabled,
            ),
            animation_type: config
                .get_optional::<String>("animations.window-close.type")
                .ok()
                .flatten()
                .and_then(|value| WindowCloseAnimationType::parse(&value))
                .unwrap_or(defaults.window_close.animation_type),
            duration_ms: config.get_or(
                "animations.window-close.duration-ms",
                defaults.window_close.duration_ms,
            ),
            custom_shader: optional_shader_path(config, "animations.window-close.custom-shader"),
        },
        fullscreen: FullscreenAnimation {
            enabled: config.get_or("animations.fullscreen.enabled", defaults.fullscreen.enabled),
            motion: AnimationMotion::parse(
                config,
                "animations.fullscreen",
                defaults.fullscreen.motion,
            ),
        },
        maximize: {
            let default_easing = match defaults.maximize.motion {
                AnimationMotion::Easing(easing) => easing,
                AnimationMotion::Spring(_) => unreachable!("maximize defaults use easing motion"),
            };
            let configured_easing = AnimationMotion::Easing(EasingMotion {
                duration_ms: config.get_or(
                    "animations.maximize.duration-ms",
                    default_easing.duration_ms,
                ),
                curve: config
                    .get_optional::<String>("animations.maximize.curve")
                    .ok()
                    .flatten()
                    .and_then(|curve| AnimationCurve::parse(&curve))
                    .unwrap_or(default_easing.curve),
            });
            MaximizeAnimation {
                enabled: config.get_or("animations.maximize.enabled", defaults.maximize.enabled),
                motion: AnimationMotion::parse(config, "animations.maximize", configured_easing),
            }
        },
        window_move: {
            let configured_easing = AnimationMotion::Easing(EasingMotion {
                duration_ms: config.get_or("animations.window-move.duration-ms", 250),
                curve: config
                    .get_optional::<String>("animations.window-move.curve")
                    .ok()
                    .flatten()
                    .and_then(|curve| AnimationCurve::parse(&curve))
                    .unwrap_or(AnimationCurve::EaseOutCubic),
            });
            WindowMoveAnimation {
                enabled: config.get_or(
                    "animations.window-move.enabled",
                    defaults.window_move.enabled,
                ),
                motion: AnimationMotion::parse(config, "animations.window-move", configured_easing),
            }
        },
        smooth_resize: SmoothResizeAnimation {
            enabled: config.get_or(
                "animations.smooth-resize.enabled",
                defaults.smooth_resize.enabled,
            ),
            duration_ms: config
                .get_or(
                    "animations.smooth-resize.duration-ms",
                    defaults.smooth_resize.duration_ms,
                )
                .clamp(1, 2_000),
        },
        node: NodeAnimation {
            enabled: config.get_or("animations.node.enabled", defaults.node.enabled),
            duration_ms: config.get_or("animations.node.duration-ms", defaults.node.duration_ms),
            collapse_duration_ms: config.get_or(
                "animations.node.collapse-duration-ms",
                defaults.node.collapse_duration_ms,
            ),
        },
    }
}

fn optional_shader_path(config: &RuneConfig, path: &str) -> Option<String> {
    config
        .get_optional::<String>(path)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_movement_supports_easing_spring_and_zero_duration() {
        for duration in [0, 900] {
            let config = RuneConfig::from_str(&format!("animations:\n  window-move:\n    enabled false\n    duration-ms {duration}\n    curve \"linear\"\n  end\nend\n")).unwrap();
            assert_eq!(
                parse_animations(&config).window_move,
                WindowMoveAnimation {
                    enabled: false,
                    motion: AnimationMotion::Easing(EasingMotion {
                        duration_ms: duration,
                        curve: AnimationCurve::Linear
                    })
                }
            );
        }
        let config = RuneConfig::from_str("animations:\n  window-move:\n    motion \"spring\"\n    damping-ratio 0.8\n    stiffness 950.0\n  end\nend\n").unwrap();
        assert_eq!(
            parse_animations(&config).window_move.motion,
            AnimationMotion::Spring(SpringMotion {
                damping_ratio: 0.8,
                stiffness: 950.0
            })
        );
    }
}
