#[cfg(any(target_os = "linux", target_os = "windows"))]
mod client;
mod config;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod input;
pub mod ipc;
mod outer;
mod profile;
#[cfg(target_os = "linux")]
pub mod pty;
#[cfg(target_os = "windows")]
#[path = "pty_windows.rs"]
pub mod pty;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod render;
#[cfg(target_os = "linux")]
pub mod runtime;
#[cfg(target_os = "windows")]
#[path = "runtime_windows.rs"]
pub mod runtime;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod server;
pub mod session;
pub mod terminal;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod viewer;

use std::{env, ffi::OsString, process::ExitCode};

const HELP: &str = "Usage:
  termfold
  termfold PID_PREFIX
  termfold new [NAME] [--profile PROFILE | --no-profile]
  termfold attach [NAME]
  termfold list
  termfold kill [--yes] [NAME]
  termfold view FILE [--session NAME]
  termfold diagnose
  termfold --help
  termfold --version";

#[derive(Debug)]
enum Command {
    Select,
    SelectPid(String),
    New {
        name: String,
        profile: ProfileSelection,
    },
    Attach(String),
    List,
    Kill {
        name: String,
        yes: bool,
    },
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    View {
        path: String,
        session: Option<String>,
    },
    Diagnose,
    Help,
    Version,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    Server {
        name: String,
        size: session::Size,
        profile: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProfileSelection {
    Default,
    Named(String),
    None,
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

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Command::Server {
        name,
        size,
        profile,
    } = &command
    {
        return server::run(
            runtime::RuntimeDir::discover()?,
            name.clone(),
            *size,
            config,
            profile.clone(),
        );
    }
    let _ = (
        config.prefix,
        config.mouse,
        config.scrollback_lines,
        &config.date_format,
        &config.time_format,
    );

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let runtime = runtime::RuntimeDir::discover()?;
        match command {
            Command::Select => select(&runtime, &config),
            Command::SelectPid(prefix) => select_pid(&runtime, &prefix, &config),
            Command::New { name, profile } => client::create_and_attach(
                &runtime,
                &name,
                &config,
                resolve_profile(&config, profile)?,
            ),
            Command::Attach(name) => client::attach(&runtime, &name, &config),
            Command::List => list(&runtime),
            Command::Kill { name, yes } => client::kill(&runtime, &name, yes),
            Command::View { path, session } => {
                let session = session
                    .or_else(|| env::var("TERMFOLD_SESSION").ok())
                    .ok_or_else(|| {
                        "termfold view requires an explicit session outside Termfold".to_owned()
                    })?;
                client::view(&runtime, &valid_name(&session)?, &path)
            }
            Command::Help | Command::Version | Command::Diagnose | Command::Server { .. } => {
                unreachable!()
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    Err("termfold requires Linux or Windows".into())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn select(runtime: &runtime::RuntimeDir, config: &config::Config) -> Result<(), String> {
    let sessions = client::discover(runtime)?;
    let detached = sessions
        .iter()
        .filter(|session| !session.is_attached())
        .collect::<Vec<_>>();
    if sessions.is_empty() {
        client::create_and_attach(
            runtime,
            "default",
            config,
            resolve_profile(config, ProfileSelection::Default)?,
        )
    } else if detached.len() == 1 {
        client::attach(runtime, &detached[0].name, config)
    } else {
        print_sessions(&sessions);
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn list(runtime: &runtime::RuntimeDir) -> Result<(), String> {
    print_sessions(&client::discover(runtime)?);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn diagnose(config: &config::Config) -> Result<(), String> {
    let (term, colorterm) = outer::detected_environment();
    let selected = outer::select(&config.terminal_profile, &term, &colorterm);
    let capabilities = selected.capabilities;
    let runtime = runtime::RuntimeDir::discover()?;
    let expected_terminfo = runtime.path().join("terminfo");
    let (terminfo, validation) = match runtime.materialize_terminfo() {
        Ok(path) => (path, "valid".to_owned()),
        Err(error) => (expected_terminfo, format!("invalid: {error}")),
    };
    let size = client::terminal_size();

    println!("outer TERM: {term:?}");
    println!("outer COLORTERM: {colorterm:?}");
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

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn diagnose(_: &config::Config) -> Result<(), String> {
    Err("termfold requires Linux or Windows".into())
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
        [command] if command == "new" => Ok(Command::New {
            name: "default".into(),
            profile: ProfileSelection::Default,
        }),
        [command] if command == "attach" => Ok(Command::Attach("default".into())),
        [command] if command == "kill" => Ok(Command::Kill {
            name: "default".into(),
            yes: false,
        }),
        [command, value] if command == "kill" && value == "--yes" => Ok(Command::Kill {
            name: "default".into(),
            yes: true,
        }),
        [command, name] if command == "new" => Ok(Command::New {
            name: valid_name(name)?,
            profile: ProfileSelection::Default,
        }),
        [command, flag] if command == "new" && flag == "--no-profile" => Ok(Command::New {
            name: "default".into(),
            profile: ProfileSelection::None,
        }),
        [command, flag, profile] if command == "new" && flag == "--profile" => Ok(Command::New {
            name: "default".into(),
            profile: ProfileSelection::Named(valid_profile_name(profile)?),
        }),
        [command, name, flag] if command == "new" && flag == "--no-profile" => Ok(Command::New {
            name: valid_name(name)?,
            profile: ProfileSelection::None,
        }),
        [command, name, flag, profile] if command == "new" && flag == "--profile" => {
            Ok(Command::New {
                name: valid_name(name)?,
                profile: ProfileSelection::Named(valid_profile_name(profile)?),
            })
        }
        [command, name] if command == "attach" => Ok(Command::Attach(valid_name(name)?)),
        [command, name] if command == "kill" => Ok(Command::Kill {
            name: valid_name(name)?,
            yes: false,
        }),
        [command, flag, name] if command == "kill" && flag == "--yes" => Ok(Command::Kill {
            name: valid_name(name)?,
            yes: true,
        }),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        [command, path] if command == "view" && !path.is_empty() => Ok(Command::View {
            path: path.clone(),
            session: None,
        }),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        [command, flag, name, path]
            if command == "view" && flag == "--session" && !path.is_empty() =>
        {
            Ok(Command::View {
                path: path.clone(),
                session: Some(valid_name(name)?),
            })
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        [command, path, flag, name]
            if command == "view" && flag == "--session" && !path.is_empty() =>
        {
            Ok(Command::View {
                path: path.clone(),
                session: Some(valid_name(name)?),
            })
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        [command, name, columns, rows] if command == "--server" => Ok(Command::Server {
            name: valid_name(name)?,
            size: session::Size {
                columns: valid_dimension(columns)?,
                rows: valid_dimension(rows)?,
            },
            profile: None,
        }),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        [command, name, columns, rows, flag] if command == "--server" && flag == "--no-profile" => {
            Ok(Command::Server {
                name: valid_name(name)?,
                size: session::Size {
                    columns: valid_dimension(columns)?,
                    rows: valid_dimension(rows)?,
                },
                profile: None,
            })
        }
        [command, name, columns, rows, flag, profile]
            if command == "--server" && flag == "--profile" =>
        {
            Ok(Command::Server {
                name: valid_name(name)?,
                size: session::Size {
                    columns: valid_dimension(columns)?,
                    rows: valid_dimension(rows)?,
                },
                profile: Some(valid_profile_name(profile)?),
            })
        }
        _ => Err(format!("invalid command\n{HELP}")),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
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

fn valid_profile_name(name: &str) -> Result<String, String> {
    if (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(name.to_owned())
    } else {
        Err("profile name must match [A-Za-z0-9_-]{1,64}".into())
    }
}

fn resolve_profile(
    config: &config::Config,
    selection: ProfileSelection,
) -> Result<Option<String>, String> {
    match selection {
        ProfileSelection::Default => Ok(config
            .profiles
            .contains_key("default")
            .then(|| "default".to_owned())),
        ProfileSelection::Named(name) => {
            if config.profiles.contains_key(&name) {
                Ok(Some(name))
            } else {
                Err(format!("profile '{name}' does not exist"))
            }
        }
        ProfileSelection::None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, ProfileSelection, config, parse_command, resolve_profile, valid_name};

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
        assert!(matches!(
            parse_command(vec!["new".into()]),
            Ok(Command::New { name, profile: ProfileSelection::Default }) if name == "default"
        ));
        assert!(matches!(
            parse_command(vec!["new".into(), "logs".into(), "--no-profile".into()]),
            Ok(Command::New { name, profile: ProfileSelection::None }) if name == "logs"
        ));
        assert!(matches!(
            parse_command(vec![
                "new".into(),
                "--profile".into(),
                "dev".into(),
            ]),
            Ok(Command::New { name, profile: ProfileSelection::Named(profile) })
                if name == "default" && profile == "dev"
        ));
        assert!(
            parse_command(vec![
                "attach".into(),
                "default".into(),
                "--profile".into(),
                "dev".into(),
            ])
            .is_err()
        );
        assert!(
            parse_command(vec![
                "new".into(),
                "--profile".into(),
                "dev".into(),
                "--no-profile".into(),
            ])
            .is_err()
        );
        assert!(
            resolve_profile(&config::Config::default(), ProfileSelection::Default)
                .is_ok_and(|profile| profile.is_none())
        );
        assert!(
            resolve_profile(
                &config::Config::default(),
                ProfileSelection::Named("missing".into())
            )
            .is_err()
        );
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        assert!(matches!(
            parse_command(vec![
                "--server".into(),
                "default".into(),
                "80".into(),
                "24".into(),
                "--profile".into(),
                "dev".into(),
            ]),
            Ok(Command::Server { profile: Some(profile), .. }) if profile == "dev"
        ));
        assert_eq!(valid_name("logs_1").unwrap(), "logs_1");
        assert!(valid_name("../logs").is_err());
        assert!(valid_name(&"a".repeat(65)).is_err());
    }
}
