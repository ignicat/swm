mod control;
mod node;
pub mod portal;
mod trail;

use std::path::PathBuf;

use swm_api::{DpmsCommand, MonitorTarget};

pub use node::NodeCommand;
pub use portal::PortalCommand;
pub use trail::TrailCommand;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Outputs,
    Reload,
    Dpms {
        command: DpmsCommand,
        output: Option<String>,
    },
    DpmsHelp,
    Node {
        request: NodeCommand,
        output: NodeOutput,
    },
    NodeHelp,
    Trail {
        request: TrailCommand,
        output: TrailOutput,
    },
    TrailHelp,
    MonitorFocus(MonitorTarget),
    WindowTransfer(swm_api::Direction),
    PanField(swm_api::Direction),
    PanHelp,
    MonitorHelp,
    Portal {
        command: PortalCommand,
        json: bool,
    },
    PortalHelp,
    ConfigEdit(Option<PathBuf>),
    ConfigVerify(Option<PathBuf>),
    ConfigHelp,
    Quit,
    Version,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeOutput {
    List { json: bool },
    Info { json: bool },
    Ack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrailOutput {
    List { json: bool },
    Ack,
}

pub fn parse(args: &[String]) -> Result<Action, String> {
    let action = match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => Action::Help,
        Some("--version") | Some("-V") => Action::Version,
        Some("outputs") => {
            if let Some(unexpected) = args.get(1) {
                return Err(format!("unexpected argument {unexpected:?}"));
            }
            Action::Outputs
        }
        Some("reload") => Action::Reload,
        Some("dpms") => return parse_dpms(&args[1..]),
        Some("node") => return node::parse(&args[1..]),
        Some("trail") => return trail::parse(&args[1..]),
        Some("pan") => return control::parse_pan(&args[1..]),
        Some("monitor") => return control::parse_monitor(&args[1..]),
        Some("portal") => return portal::parse(&args[1..]),
        Some("config") => return parse_config(&args[1..]),
        Some("quit") => Action::Quit,
        Some(other) => return Err(format!("unknown command {other:?}")),
    };

    if let Some(unexpected) = args.get(1) {
        return Err(format!("unexpected argument {unexpected:?}"));
    }
    Ok(action)
}

fn parse_dpms(args: &[String]) -> Result<Action, String> {
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(Action::DpmsHelp);
    }

    let mut command = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "dpms output option requires a connector name".to_string())?;
                if output.replace(value.clone()).is_some() {
                    return Err("dpms output option was specified more than once".to_string());
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown dpms option {value:?}"));
            }
            value if command.is_none() => command = Some(value),
            value => return Err(format!("unexpected dpms argument {value:?}")),
        }
        index += 1;
    }

    let command = match command {
        Some("off") => DpmsCommand::Off,
        Some("on") => DpmsCommand::On,
        Some("toggle") => DpmsCommand::Toggle,
        Some(other) => return Err(format!("unknown dpms command {other:?}")),
        None => return Ok(Action::DpmsHelp),
    };
    Ok(Action::Dpms { command, output })
}

fn parse_config(args: &[String]) -> Result<Action, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Action::ConfigHelp);
    };
    if command == "-h" || command == "--help" {
        return Ok(Action::ConfigHelp);
    }
    if command != "edit" && command != "verify" {
        return Err(format!("unknown config command {command:?}"));
    }
    let mut path = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(Action::ConfigHelp),
            "-c" | "--config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("config {command} path option requires a path"))?;
                set_config_path(&mut path, value, command)?;
            }
            value if value.starts_with("--config=") => {
                set_config_path(&mut path, &value["--config=".len()..], command)?;
            }
            value => return Err(format!("unexpected config {command} argument {value:?}")),
        }
        index += 1;
    }
    Ok(match command {
        "edit" => Action::ConfigEdit(path),
        "verify" => Action::ConfigVerify(path),
        _ => unreachable!("config command checked above"),
    })
}

fn set_config_path(path: &mut Option<PathBuf>, value: &str, command: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("config {command} path cannot be empty"));
    }
    if path.replace(PathBuf::from(value)).is_some() {
        return Err(format!(
            "config {command} path was specified more than once"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use swm_api::{NodeMoveDirection, NodeSelector};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parser_accepts_top_level_commands() {
        assert_eq!(parse(&[]), Ok(Action::Help));
        assert_eq!(parse(&args(&["outputs"])), Ok(Action::Outputs));
        assert_eq!(parse(&args(&["--help"])), Ok(Action::Help));
        assert_eq!(parse(&args(&["--version"])), Ok(Action::Version));
        assert_eq!(parse(&args(&["quit"])), Ok(Action::Quit));
        assert!(parse(&args(&["output"])).is_err());
        assert!(parse(&args(&["outputs", "extra"])).is_err());
    }

    #[test]
    fn parser_covers_dpms_and_config() {
        assert_eq!(
            parse(&args(&["dpms", "-o", "DP-1", "on"])),
            Ok(Action::Dpms {
                command: DpmsCommand::On,
                output: Some("DP-1".to_string()),
            })
        );
        assert_eq!(
            parse(&args(&["config", "verify", "--config=/tmp/two.rune"])),
            Ok(Action::ConfigVerify(Some("/tmp/two.rune".into())))
        );
        assert_eq!(
            parse(&args(&["config", "edit"])),
            Ok(Action::ConfigEdit(None))
        );
        assert_eq!(
            parse(&args(&["config", "edit", "-c", "/tmp/two.rune"])),
            Ok(Action::ConfigEdit(Some("/tmp/two.rune".into())))
        );
        assert!(parse(&args(&["config", "migrate"])).is_err());
        assert!(
            parse(&args(&[
                "config",
                "migrate",
                "--dry-run",
                "-c",
                "/tmp/two.rune"
            ]))
            .is_err()
        );
        assert!(parse(&args(&["dpms", "sleep"])).is_err());
        assert!(parse(&args(&["config", "verify", "-c"])).is_err());
        assert!(parse(&args(&["config", "edit", "--config="])).is_err());
        assert!(parse(&args(&["config", "verify", "--dry-run"])).is_err());
    }

    #[test]
    fn parser_covers_node_state_controls() {
        assert_eq!(
            parse(&args(&["node", "list", "-o", "DP-1", "--json"])),
            Ok(Action::Node {
                request: NodeCommand::List {
                    output: Some("DP-1".into())
                },
                output: NodeOutput::List { json: true },
            })
        );
        assert_eq!(
            parse(&args(&["node", "collapse", "focused"])),
            Ok(Action::Node {
                request: NodeCommand::Collapse {
                    selector: Some(NodeSelector::Focused),
                    output: None,
                },
                output: NodeOutput::Ack,
            })
        );
        assert_eq!(
            parse(&args(&["node", "move", "left", "app:firefox"])),
            Ok(Action::Node {
                request: NodeCommand::Move {
                    direction: NodeMoveDirection::Left,
                    distance: None,
                    selector: Some(NodeSelector::App("firefox".into())),
                    output: None,
                },
                output: NodeOutput::Ack,
            })
        );
        assert!(parse(&args(&["node", "collapse", "--json"])).is_err());
    }

    #[test]
    fn node_move_distance_retains_selectors_and_output_in_any_order() {
        for arguments in [
            vec![
                "node",
                "move",
                "right",
                "--distance",
                "10",
                "app:firefox",
                "-o",
                "DP-1",
            ],
            vec![
                "node",
                "move",
                "right",
                "app:firefox",
                "--output",
                "DP-1",
                "--distance=10",
            ],
        ] {
            assert_eq!(
                parse(&args(&arguments)),
                Ok(Action::Node {
                    request: NodeCommand::Move {
                        direction: NodeMoveDirection::Right,
                        distance: Some(10),
                        selector: Some(NodeSelector::App("firefox".into())),
                        output: Some("DP-1".into()),
                    },
                    output: NodeOutput::Ack,
                })
            );
        }
        for value in ["0", "-1", "1.5", "abc", "2147483648", ""] {
            assert!(parse(&args(&["node", "move", "right", "--distance", value])).is_err());
        }
        for arguments in [
            vec!["node", "move", "right", "--distance"],
            vec!["node", "move", "right", "--distance", "10", "--distance=20"],
            vec!["node", "focus", "--distance", "10"],
            vec!["node", "move", "right", "--distance=10", "--json"],
        ] {
            assert!(parse(&args(&arguments)).is_err());
        }
    }
}
