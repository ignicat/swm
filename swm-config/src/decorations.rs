use rune_cfg::RuneConfig;

/// Opaque RGB color for window borders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Primary compositor window border styling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decorations {
    pub border_width_px: i32,
    pub border_color_focused: BorderColor,
    pub border_color_unfocused: BorderColor,
    pub resize_using_border: bool,
    pub screencast_indicator_size: i32,
    pub screencast_indicator_color: BorderColor,
}

impl Default for Decorations {
    fn default() -> Self {
        Self {
            border_width_px: 3,
            border_color_focused: BorderColor {
                r: 0xf4 as f32 / 255.0,
                g: 0xf5 as f32 / 255.0,
                b: 0xf7 as f32 / 255.0,
            },
            border_color_unfocused: BorderColor {
                r: 0x47 as f32 / 255.0,
                g: 0x4d as f32 / 255.0,
                b: 0x59 as f32 / 255.0,
            },
            resize_using_border: true,
            screencast_indicator_size: 4,
            screencast_indicator_color: BorderColor {
                r: 0.94,
                g: 0.20,
                b: 0.24,
            },
        }
    }
}

/// Parses `decorations.border`, falling back to defaults for omitted keys.
/// Tries both `colour` and `color` explicitly.
pub fn parse_decorations(config: &RuneConfig) -> Decorations {
    let defaults = Decorations::default();

    let border_width_px = config
        .get_or("decorations.border.size", defaults.border_width_px)
        .clamp(0, 64);
    let border_color_focused = parse_color(
        config,
        "decorations.border.colour-focused",
        defaults.border_color_focused,
    );
    let border_color_unfocused = parse_color(
        config,
        "decorations.border.colour-unfocused",
        defaults.border_color_unfocused,
    );

    Decorations {
        border_width_px,
        border_color_focused,
        border_color_unfocused,
        resize_using_border: config.get_or(
            "decorations.resize-using-border",
            defaults.resize_using_border,
        ),
        screencast_indicator_size: config
            .get_or(
                "decorations.screencast-indicator.size",
                defaults.screencast_indicator_size,
            )
            .clamp(1, 32),
        screencast_indicator_color: parse_color(
            config,
            "decorations.screencast-indicator.colour",
            defaults.screencast_indicator_color,
        ),
    }
}

fn parse_color(config: &RuneConfig, key: &str, default: BorderColor) -> BorderColor {
    let Some(raw) = crate::aliases::spellings(key)
        .iter()
        .find_map(|path| config.get_optional::<String>(path).ok().flatten())
    else {
        return default;
    };
    crate::color::parse_hex_rgb(&raw)
        .map(|[r, g, b]| BorderColor { r, g, b })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screencast_indicator_parses_size_and_color() {
        let config = RuneConfig::from_str("decorations:\n  screencast-indicator:\n    size 7\n    colour \"#112233\"\n  end\nend\n").unwrap();
        let decorations = parse_decorations(&config);
        assert_eq!(decorations.screencast_indicator_size, 7);
        assert_eq!(
            decorations.screencast_indicator_color,
            BorderColor {
                r: 17.0 / 255.0,
                g: 34.0 / 255.0,
                b: 51.0 / 255.0
            }
        );
    }
}
