use std::fmt;

use rune_cfg::RuneConfig;

use crate::aliases::lookup;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackgroundMode {
    #[default]
    None,
    Classic,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackgroundFit {
    #[default]
    Cover,
    Contain,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub mode: BackgroundMode,
    pub path: String,
    pub fit: BackgroundFit,
    pub intensity: f32,
    pub color: BackgroundColor,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            mode: BackgroundMode::None,
            path: String::new(),
            fit: BackgroundFit::Cover,
            intensity: 1.0,
            color: BackgroundColor {
                r: 0x18 as f32 / 255.0,
                g: 0x1a as f32 / 255.0,
                b: 0x26 as f32 / 255.0,
            },
        }
    }
}

#[derive(Debug)]
pub enum BackgroundParseError {
    Rune(rune_cfg::RuneError),
    InvalidValue { path: &'static str, value: String },
}

impl fmt::Display for BackgroundParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rune(error) => write!(f, "{error}"),
            Self::InvalidValue { path, value } => {
                write!(f, "invalid value {value:?} for {path}")
            }
        }
    }
}

impl std::error::Error for BackgroundParseError {}

impl From<rune_cfg::RuneError> for BackgroundParseError {
    fn from(value: rune_cfg::RuneError) -> Self {
        Self::Rune(value)
    }
}

pub fn parse_background(config: &RuneConfig) -> Result<Background, BackgroundParseError> {
    let defaults = Background::default();
    let mode = match lookup::<String>(config, "wallpaper.mode")?
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("none") => BackgroundMode::None,
        Some("classic") => BackgroundMode::Classic,
        Some(value) => {
            return Err(BackgroundParseError::InvalidValue {
                path: "wallpaper.mode",
                value: value.to_string(),
            });
        }
    };
    let fit = match lookup::<String>(config, "wallpaper.fit")?
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("cover") => BackgroundFit::Cover,
        Some("contain") => BackgroundFit::Contain,
        Some("stretch") => BackgroundFit::Stretch,
        Some(value) => {
            return Err(BackgroundParseError::InvalidValue {
                path: "wallpaper.fit",
                value: value.to_string(),
            });
        }
    };
    let intensity = lookup::<f32>(config, "wallpaper.intensity")?.unwrap_or(1.0);
    if !intensity.is_finite() || intensity < 0.0 {
        return Err(BackgroundParseError::InvalidValue {
            path: "wallpaper.intensity",
            value: intensity.to_string(),
        });
    }

    Ok(Background {
        mode,
        path: lookup::<String>(config, "wallpaper.path")?.unwrap_or_default(),
        fit,
        intensity,
        color: optional_color(config, "wallpaper.colour", defaults.color)?,
    })
}

fn optional_color(
    config: &RuneConfig,
    key: &'static str,
    default: BackgroundColor,
) -> Result<BackgroundColor, BackgroundParseError> {
    let Some(value) = lookup::<String>(config, key)? else {
        return Ok(default);
    };
    crate::color::parse_hex_rgb(&value)
        .map(|[r, g, b]| BackgroundColor { r, g, b })
        .ok_or(BackgroundParseError::InvalidValue { path: key, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_wallpaper_section() {
        let config = RuneConfig::from_str(
            r##"
wallpaper:
  mode "classic"
  path "/tmp/bg.png"
  fit "contain"
  colour "#202233"
  intensity 1.35
end
"##,
        )
        .unwrap();
        let background = parse_background(&config).unwrap();
        assert_eq!(background.mode, BackgroundMode::Classic);
        assert_eq!(background.fit, BackgroundFit::Contain);
        assert_eq!(background.color.r, 0x20 as f32 / 255.0);
        assert_eq!(background.intensity, 1.35);
    }

    #[test]
    fn accepts_legacy_background_and_gesso_aliases() {
        let background = RuneConfig::from_str("background:\n  mode \"classic\"\nend\n").unwrap();
        assert_eq!(
            parse_background(&background).unwrap().mode,
            BackgroundMode::Classic
        );

        let gesso = RuneConfig::from_str("gesso:\n  mode \"classic\"\nend\n").unwrap();
        assert_eq!(
            parse_background(&gesso).unwrap().mode,
            BackgroundMode::Classic
        );
    }
}
