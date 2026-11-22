mod cmd;
mod config;
mod help;
mod print;

use std::process::ExitCode;

use cmd::{Action, NodeCommand, TrailCommand};
use swm_api::Client;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match cmd::parse(&args) {
        Ok(Action::Outputs) => with_client(|client| Ok(print::outputs(client.outputs()?))),
        Ok(Action::Reload) => with_client(|client| {
            client.reload_config()?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::Dpms { command, output }) => with_client(|client| {
            client.set_dpms(command, output.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::DpmsHelp) => show(help::DPMS_HELP),
        Ok(Action::Node {
            request,
            output: presentation,
        }) => with_client(|client| {
            match request {
                NodeCommand::List { output } => {
                    return Ok(print::node_list(
                        client.nodes(output.as_deref())?,
                        presentation,
                    ));
                }
                NodeCommand::Info { selector, output } => {
                    return Ok(print::node_info(
                        client.node_info(selector, output.as_deref())?,
                        presentation,
                    ));
                }
                NodeCommand::Focus { selector, output } => {
                    client.focus_node(selector, output.as_deref())?;
                }
                NodeCommand::Collapse { selector, output } => {
                    client.collapse_node(selector, output.as_deref())?;
                }
                NodeCommand::Restore { selector, output } => {
                    client.restore_node(selector, output.as_deref())?;
                }
                NodeCommand::Toggle { selector, output } => {
                    client.toggle_node(selector, output.as_deref())?;
                }
                NodeCommand::Close { selector, output } => {
                    client.close_node(selector, output.as_deref())?;
                }
                NodeCommand::MoveTo {
                    x,
                    y,
                    selector,
                    output,
                } => {
                    client.move_node_to(x, y, selector, output.as_deref())?;
                }
                NodeCommand::Move {
                    direction,
                    distance,
                    selector,
                    output,
                } => match distance {
                    Some(distance) => {
                        client.move_node_by(direction, distance, selector, output.as_deref())?
                    }
                    None => client.move_node(direction, selector, output.as_deref())?,
                },
            }
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::NodeHelp) => show(help::NODE_HELP),
        Ok(Action::Trail { request, output }) => with_client(|client| match request {
            TrailCommand::Navigate { direction, output } => {
                client.navigate_trail(direction, output.as_deref())?;
                Ok(ExitCode::SUCCESS)
            }
            TrailCommand::List { output: connector } => {
                Ok(print::trail(client.trail(connector.as_deref())?, output))
            }
            TrailCommand::Goto { target, output } => {
                client.goto_trail(target, output.as_deref())?;
                Ok(ExitCode::SUCCESS)
            }
        }),
        Ok(Action::TrailHelp) => show(help::TRAIL_HELP),
        Ok(Action::WindowTransfer(direction)) => with_client(|client| {
            client.transfer_window(direction)?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::PanHelp) => show("Usage: swmctl pan left|right|up|down\n"),
        Ok(Action::PanField(direction)) => with_client(|client| {
            client.pan_field(direction)?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::MonitorFocus(target)) => with_client(|client| {
            client.focus_monitor(target)?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::MonitorHelp) => show(help::MONITOR_HELP),
        Ok(Action::Portal { command, json }) => cmd::portal::run(command, json),
        Ok(Action::PortalHelp) => show(help::PORTAL_HELP),
        Ok(Action::ConfigEdit(path)) => config::edit(path),
        Ok(Action::ConfigVerify(path)) => config::verify(path),
        Ok(Action::ConfigHelp) => show(help::CONFIG_HELP),
        Ok(Action::Quit) => with_client(|client| {
            client.request_quit()?;
            Ok(ExitCode::SUCCESS)
        }),
        Ok(Action::Version) => print::version(),
        Ok(Action::Help) => show(help::HELP),
        Err(err) => {
            eprintln!("swmctl: {err}\n\n{}", help::HELP);
            ExitCode::from(2)
        }
    }
}

fn with_client(operation: impl FnOnce(&Client) -> swm_api::Result<ExitCode>) -> ExitCode {
    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("swmctl: failed to reach the running compositor: {error}");
            return ExitCode::FAILURE;
        }
    };
    match operation(&client) {
        Ok(code) => code,
        Err(error) => {
            eprintln!(
                "swmctl: compositor request failed ({:?}): {}",
                error.kind(),
                error.message()
            );
            ExitCode::FAILURE
        }
    }
}

fn show(text: &str) -> ExitCode {
    print!("{text}");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #[test]
    fn help_separates_commands_from_options() {
        let commands = crate::help::HELP
            .split_once("Commands:\n")
            .and_then(|(_, rest)| rest.split_once("\nOptions:"))
            .map(|(commands, _)| commands)
            .expect("help has Commands followed by Options");

        assert!(commands.contains("outputs"));
        assert!(commands.contains("reload"));
        assert!(!commands.contains("capture"));
        assert!(commands.contains("dpms"));
        assert!(commands.contains("node"));
        assert!(commands.contains("trail"));
        assert!(commands.contains("monitor"));
        assert!(!commands.contains("tile"));
        assert!(commands.contains("portal"));
        assert!(!commands.contains("gamescope"));
        assert!(commands.contains("config"));
        assert!(!commands.contains("basics"));
        assert!(commands.contains("quit"));
        assert!(!commands.contains("--help"));
        assert!(!commands.contains("--version"));
    }
}
