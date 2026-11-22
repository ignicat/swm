use swm_api::{Direction, MonitorTarget};

use super::Action;

pub(super) fn parse_monitor(args: &[String]) -> Result<Action, String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => Ok(Action::MonitorHelp),
        Some("transfer") => Ok(Action::WindowTransfer(parse_direction_args(&args[1..])?)),
        Some("focus") => {
            let target = args
                .get(1)
                .ok_or_else(|| "monitor focus requires a direction or output name".to_string())?;
            if let Some(unexpected) = args.get(2) {
                return Err(format!("unexpected monitor focus argument {unexpected:?}"));
            }
            let target = match target.as_str() {
                "left" => MonitorTarget::Direction(Direction::Left),
                "right" => MonitorTarget::Direction(Direction::Right),
                "up" => MonitorTarget::Direction(Direction::Up),
                "down" => MonitorTarget::Direction(Direction::Down),
                output => MonitorTarget::Output(output.to_string()),
            };
            Ok(Action::MonitorFocus(target))
        }
        Some(other) => Err(format!("unknown monitor command {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_monitor_controls() {
        assert_eq!(
            parse_monitor(&args(&["focus", "DP-2"])),
            Ok(Action::MonitorFocus(MonitorTarget::Output("DP-2".into())))
        );
    }
}

fn parse_direction_args(args: &[String]) -> Result<Direction, String> {
    if args.len() != 1 {
        return Err("expected exactly one direction: left, right, up, or down".into());
    }
    match args[0].as_str() {
        "left" => Ok(Direction::Left),
        "right" => Ok(Direction::Right),
        "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        _ => Err("expected direction: left, right, up, or down".into()),
    }
}
pub(super) fn parse_pan(args: &[String]) -> Result<Action, String> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        return Ok(Action::PanHelp);
    }
    Ok(Action::PanField(parse_direction_args(args)?))
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[test]
    fn directional_commands_validate_arguments() {
        for name in ["left", "right", "up", "down"] {
            assert!(matches!(parse_pan(&[name.into()]), Ok(Action::PanField(_))));
            assert!(matches!(
                parse_monitor(&["transfer".into(), name.into()]),
                Ok(Action::WindowTransfer(_))
            ));
        }
        assert!(parse_pan(&["sideways".into()]).is_err());
        assert!(parse_monitor(&["transfer".into(), "left".into(), "extra".into()]).is_err());
    }
}
