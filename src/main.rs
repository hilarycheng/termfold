mod config;
pub mod ipc;
#[cfg(target_os = "linux")]
pub mod pty;
#[cfg(target_os = "linux")]
pub mod runtime;
pub mod session;

use std::{env, ffi::OsString, process::ExitCode};

const HELP: &str = "Usage:
  termfold
  termfold PID_PREFIX
  termfold new [NAME]
  termfold attach [NAME]
  termfold list
  termfold kill [NAME]
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
    Help,
    Version,
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
    let _ = (
        config.prefix,
        config.mouse,
        config.scrollback_lines,
        config.date_format,
        config.time_format,
    );

    Err(match command {
        Command::Select => "session discovery is not implemented".into(),
        Command::SelectPid(pid) => {
            format!("cannot attach process {pid}: session discovery is not implemented")
        }
        Command::New(name) => {
            format!("cannot create session '{name}': session runtime is not implemented")
        }
        Command::Attach(name) => {
            format!("cannot attach session '{name}': session runtime is not implemented")
        }
        Command::List => "cannot list sessions: session discovery is not implemented".into(),
        Command::Kill(name) => {
            format!("cannot kill session '{name}': session runtime is not implemented")
        }
        Command::Help | Command::Version => unreachable!(),
    })
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
        [value] if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            Ok(Command::SelectPid(value.clone()))
        }
        [command] if command == "new" => Ok(Command::New("default".into())),
        [command] if command == "attach" => Ok(Command::Attach("default".into())),
        [command] if command == "kill" => Ok(Command::Kill("default".into())),
        [command, name] if command == "new" => Ok(Command::New(valid_name(name)?)),
        [command, name] if command == "attach" => Ok(Command::Attach(valid_name(name)?)),
        [command, name] if command == "kill" => Ok(Command::Kill(valid_name(name)?)),
        _ => Err(format!("invalid command\n{HELP}")),
    }
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
