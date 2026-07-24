#[cfg(target_os = "linux")]
mod client;
mod config;
pub mod ipc;
mod outer;
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
        return diagnose(&config);
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
        &config.date_format,
        &config.time_format,
    );

    #[cfg(target_os = "linux")]
    {
        let runtime = runtime::RuntimeDir::discover()?;
        match command {
            Command::Select => select(&runtime, &config),
            Command::SelectPid(prefix) => select_pid(&runtime, &prefix, &config),
            Command::New(name) => client::create_and_attach(&runtime, &name, &config),
            Command::Attach(name) => client::attach(&runtime, &name, &config),
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
fn select(runtime: &runtime::RuntimeDir, config: &config::Config) -> Result<(), String> {
    let sessions = client::discover(runtime)?;
    let detached = sessions
        .iter()
        .filter(|session| !session.is_attached())
        .collect::<Vec<_>>();
    if sessions.is_empty() {
        client::create_and_attach(runtime, "default", config)
    } else if detached.len() == 1 {
        client::attach(runtime, &detached[0].name, config)
    } else {
        print_sessions(&sessions);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn select_pid(
    runtime: &runtime::RuntimeDir,
    prefix: &str,
    config: &config::Config,
) -> Result<(), String> {
    let sessions = client::discover(runtime)?;
    let matches = sessions
        .iter()
        .filter(|session| !session.is_attached() && session.pid.to_string().starts_with(prefix))
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        client::attach(runtime, &matches[0].name, config)
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

#[cfg(target_os = "linux")]
fn diagnose(config: &config::Config) -> Result<(), String> {
    let term = env::var_os("TERM").unwrap_or_default();
    let colorterm = env::var_os("COLORTERM").unwrap_or_default();
    let selected = outer::select(
        &config.terminal_profile,
        &term.to_string_lossy(),
        &colorterm.to_string_lossy(),
    );
    let capabilities = selected.capabilities;
    let runtime = runtime::RuntimeDir::discover()?;
    let expected_terminfo = runtime.path().join("terminfo");
    let (terminfo, validation) = match runtime.materialize_terminfo() {
        Ok(path) => (path, "valid".to_owned()),
        Err(error) => (expected_terminfo, format!("invalid: {error}")),
    };
    let size = client::terminal_size();

    println!("outer TERM: {:?}", term.to_string_lossy());
    println!("outer COLORTERM: {:?}", colorterm.to_string_lossy());
    println!(
        "outer profile: {} ({})",
        capabilities.profile.name(),
        selected.reason.name()
    );
    println!("colour level: {}", color_level_name(capabilities.color));
    println!(
        "mouse support: {} (configured {})",
        yes_no(capabilities.mouse),
        if config.mouse { "on" } else { "off" }
    );
    println!(
        "alternate-screen support: {}",
        yes_no(capabilities.alternate_screen)
    );
    println!("inner TERM: {}", config.inner_term);
    println!("private TERMINFO: {}", terminfo.display());
    println!("private TERMINFO validation: {validation}");
    println!(
        "terminal size: {} columns, {} rows",
        size.columns, size.rows
    );
    println!(
        "Termfold: {} {}",
        env!("CARGO_PKG_VERSION"),
        env::consts::ARCH
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn diagnose(_: &config::Config) -> Result<(), String> {
    Err("termfold requires Linux".into())
}

fn color_level_name(level: outer::ColorLevel) -> &'static str {
    match level {
        outer::ColorLevel::Monochrome => "monochrome",
        outer::ColorLevel::Ansi16 => "16 colours",
        outer::ColorLevel::Indexed256 => "256 colours",
        outer::ColorLevel::TrueColor => "true colour",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
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
