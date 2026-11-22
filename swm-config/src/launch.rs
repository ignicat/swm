use std::collections::BTreeMap;
use std::fmt;

use rune_cfg::RuneConfig;
use rune_cfg::ast::{ObjectItem, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Autostart {
    pub once: Vec<String>,
    pub on_reload: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfigError(String);

impl fmt::Display for LaunchConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LaunchConfigError {}

/// Parses optional environment overrides for launched apps.
/// Empty keys and values are ignored; later duplicates overwrite earlier ones.
pub fn parse_env(config: &RuneConfig) -> Result<BTreeMap<String, String>, LaunchConfigError> {
    let Some(fields) = section(config, "env")? else {
        return Ok(BTreeMap::new());
    };
    let mut env = BTreeMap::new();
    for item in fields {
        let ObjectItem::Assign(key, value) = item else {
            continue;
        };
        let Value::String(value) = value else {
            return Err(LaunchConfigError(format!("env.{key} must be a string")));
        };
        let key = key.trim();
        let value = value.trim();
        if !key.is_empty() && !value.is_empty() {
            env.insert(key.to_string(), value.to_string());
        }
    }
    Ok(env)
}

/// Parses ordered `once` and `on-reload` command declarations.
pub fn parse_autostart(config: &RuneConfig) -> Result<Autostart, LaunchConfigError> {
    let Some(fields) = section(config, "autostart")? else {
        return Ok(Autostart::default());
    };
    let mut autostart = Autostart::default();
    for item in fields {
        let ObjectItem::Assign(directive, value) = item else {
            continue;
        };
        match directive.as_str() {
            "once" | "on-reload" => {
                let Value::String(command) = value else {
                    return Err(LaunchConfigError(format!(
                        "autostart.{directive} must be a string"
                    )));
                };
                let command = command.trim();
                if command.is_empty() {
                    continue;
                }
                if directive == "once" {
                    autostart.once.push(command.to_string());
                } else {
                    autostart.on_reload.push(command.to_string());
                }
            }
            _ => {
                return Err(LaunchConfigError(format!(
                    "autostart: unsupported directive {directive:?}"
                )));
            }
        }
    }
    Ok(autostart)
}

fn section(config: &RuneConfig, name: &str) -> Result<Option<Vec<ObjectItem>>, LaunchConfigError> {
    let root = config
        .get_value("")
        .map_err(|err| LaunchConfigError(format!("launch config: {err}")))?;
    let Value::Object(fields) = root else {
        return Err(LaunchConfigError(
            "launch config root must be an object".to_string(),
        ));
    };
    let value = fields.into_iter().find_map(|item| match item {
        ObjectItem::Assign(key, value) if key == name => Some(value),
        _ => None,
    });
    match value {
        None => Ok(None),
        Some(Value::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(LaunchConfigError(format!("{name} must be a block"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> RuneConfig {
        RuneConfig::from_str(src).expect("valid Rune source")
    }

    #[test]
    fn environment_is_trimmed_and_last_value_wins() {
        let env = parse_env(&parse(
            r#"
env:
  QT_QPA_PLATFORM " wayland "
  EMPTY " "
  QT_QPA_PLATFORM "xcb"
end
"#,
        ))
        .unwrap();

        assert_eq!(
            env,
            BTreeMap::from([("QT_QPA_PLATFORM".to_string(), "xcb".to_string())])
        );
    }

    #[test]
    fn autostart_preserves_order_with_repeated_directives() {
        let autostart = parse_autostart(&parse(
            r#"
autostart:
  once "waybar"
  on-reload "notify-send reloaded"
  once "mako"
  on-reload "  "
end
"#,
        ))
        .unwrap();

        assert_eq!(autostart.once, ["waybar", "mako"]);
        assert_eq!(autostart.on_reload, ["notify-send reloaded"]);
    }
}
