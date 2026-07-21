#[cfg(target_os = "linux")]
mod client;
mod config;
pub mod ipc;
#[cfg(target_os = "linux")]
pub mod pty;
#[cfg(target_os = "linux")]
mod render;
#[cfg(target_os = "linux")]
pub mod runtime;
#[cfg(target_os = "linux")]
mod server;
pub mod session;
pub mod terminal;

use std::{env, ffi::OsString, process::ExitCode};

const HELP: &str = "Usage:
  termfold
  termfold PID_PREFIX
  termfold new [NAME]
  termfold attach [NAME]
  termfold list
  termfold kill [NAME]
  termfold diagnose
  termfold --help
  termfold --version";

#[derive(Debug)]
enum Command {
    Select,
    SelectPid(String),
    New(String),
    Attach(String),
    List,
    Kill(String),
    Diagnose,
    Help,
    Version,
    #[cfg(target_os = "linux")]
    Server {
        name: String,
        size: session::Size,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("termfold: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let command = parse_command(env::args_os().skip(1).collect())?;
    match command {
        Command::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Command::Version => {
            println!("termfold {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    let config = config::Config::load()?;

    if matches!(command, Command::Diagnose) {
        return Err("terminal diagnostics are not available in this build".into());
    }

    #[cfg(target_os = "linux")]
    if let Command::Server { name, size } = &command {
        return server::run(
            runtime::RuntimeDir::discover()?,
            name.clone(),
            *size,
            config,
        );
    }
    let _ = (
        config.prefix,
        config.mouse,
        config.scrollback_lines,
        config.date_format,
        config.time_format,
    );

    #[cfg(target_os = "linux")]
    {
        let runtime = runtime::RuntimeDir::discover()?;
        match command {
            Command::Select => select(&runtime),
            Command::SelectPid(prefix) => select_pid(&runtime, &prefix),
            Command::New(name) => client::create_and_attach(&runtime, &name),
            Command::Attach(name) => client::attach(&runtime, &name),
            Command::List => list(&runtime),
            Command::Kill(name) => client::kill(&runtime, &name),
            Command::Help | Command::Version | Command::Diagnose | Command::Server { .. } => {
                unreachable!()
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    Err("termfold requires Linux".into())
}

#[cfg(target_os = "linux")]
fn select(runtime: &runtime::RuntimeDir) -> Result<(), String> {
    let sessions = client::discover(runtime)?;
    let detached = sessions
        .iter()
        .filter(|session| !session.is_attached())
        .collect::<Vec<_>>();
    if sessions.is_empty() {
        client::create_and_attach(runtime, "default")
    } else if detached.len() == 1 {
        client::attach(runtime, &detached[0].name)
    } else {
        print_sessions(&sessions);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn select_pid(runtime: &runtime::RuntimeDir, prefix: &str) -> Result<(), String> {
    let sessions = client::discover(runtime)?;
    let matches = sessions
        .iter()
        .filter(|session| !session.is_attached() && session.pid.to_string().starts_with(prefix))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        client::attach(runtime, &matches[0].name)
    } else {
        print_sessions(&sessions);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn list(runtime: &runtime::RuntimeDir) -> Result<(), String> {
    print_sessions(&client::discover(runtime)?);
    Ok(())
}

#[cfg(target_os = "linux")]
fn print_sessions(sessions: &[client::SessionInfo]) {
    for session in sessions {
        let state = if session.is_attached() {
            "attached"
        } else {
            "detached"
        };
        println!("{} {} {state}", session.pid, session.name);
    }
}

fn parse_command(arguments: Vec<OsString>) -> Result<Command, String> {
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;

    match arguments.as_slice() {
        [] => Ok(Command::Select),
        [value] if value == "--help" => Ok(Command::Help),
        [value] if value == "--version" => Ok(Command::Version),
        [value] if value == "list" => Ok(Command::List),
        [value] if value == "diagnose" => Ok(Command::Diagnose),
        [value] if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            Ok(Command::SelectPid(value.clone()))
        }
        [command] if command == "new" => Ok(Command::New("default".into())),
        [command] if command == "attach" => Ok(Command::Attach("default".into())),
        [command] if command == "kill" => Ok(Command::Kill("default".into())),
        [command, name] if command == "new" => Ok(Command::New(valid_name(name)?)),
        [command, name] if command == "attach" => Ok(Command::Attach(valid_name(name)?)),
        [command, name] if command == "kill" => Ok(Command::Kill(valid_name(name)?)),
        #[cfg(target_os = "linux")]
        [command, name, columns, rows] if command == "--server" => Ok(Command::Server {
            name: valid_name(name)?,
            size: session::Size {
                columns: valid_dimension(columns)?,
                rows: valid_dimension(rows)?,
            },
        }),
        _ => Err(format!("invalid command\n{HELP}")),
    }
}

#[cfg(target_os = "linux")]
fn valid_dimension(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "server terminal dimensions must be non-zero u16 values".into())
}

fn valid_name(name: &str) -> Result<String, String> {
    if (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(name.to_owned())
    } else {
        Err("session name must match [A-Za-z0-9_-]{1,64}".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command, valid_name};

    #[test]
    fn parses_public_commands_and_validates_names() {
        assert!(matches!(
            parse_command(vec!["diagnose".into()]),
            Ok(Command::Diagnose)
        ));
        assert!(matches!(
            parse_command(vec!["123".into()]),
            Ok(Command::SelectPid(value)) if value == "123"
        ));
        assert_eq!(valid_name("logs_1").unwrap(), "logs_1");
        assert!(valid_name("../logs").is_err());
        assert!(valid_name(&"a".repeat(65)).is_err());
    }
}
