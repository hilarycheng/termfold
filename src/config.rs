use std::{env, fs, io::ErrorKind, path::PathBuf};

use crate::terminal::Color;

#[derive(Debug)]
pub struct Config {
    pub prefix: u8,
    pub mouse: bool,
    pub scrollback_lines: u16,
    pub date_format: String,
    pub time_format: String,
    pub status_format: String,
    pub status_label: String,
    pub status_refresh_seconds: u16,
    pub cpu_temperature_path: Option<PathBuf>,
    pub status_foreground: Color,
    pub status_background: Color,
    pub label_foreground: Color,
    pub label_background: Color,
    pub active_tab_foreground: Color,
    pub active_tab_background: Color,
    pub terminal_profile: String,
    pub inner_term: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefix: 2,
            mouse: false,
            scrollback_lines: 2_000,
            date_format: "%Y-%m-%d".into(),
            time_format: "%H:%M".into(),
            status_format: "[{session}]  {tabs}{fill}|  {date} {time}".into(),
            status_label: String::new(),
            status_refresh_seconds: 2,
            cpu_temperature_path: None,
            status_foreground: Color::Indexed(0),
            status_background: Color::Indexed(6),
            label_foreground: Color::Indexed(15),
            label_background: Color::Indexed(1),
            active_tab_foreground: Color::Indexed(0),
            active_tab_background: Color::Indexed(11),
            terminal_profile: "auto".into(),
            inner_term: "termfold-256color".into(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };

        match fs::read_to_string(&path) {
            Ok(source) => Self::parse(&source),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!(
                "cannot read configuration {}: {error}",
                path.display()
            )),
        }
    }

    fn parse(source: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut seen = 0_u32;

        for (index, line) in source.lines().enumerate() {
            let line = strip_comment(line).trim();
            if line.is_empty() {
                continue;
            }

            let Some((field, value)) = line.split_once('=') else {
                return Err(format!(
                    "configuration line {}: expected FIELD = VALUE",
                    index + 1
                ));
            };
            let field = field.trim();
            let value = value.trim();
            let bit = match field {
                "prefix" => 1,
                "mouse" => 2,
                "scrollback_lines" => 4,
                "date_format" => 8,
                "time_format" => 16,
                "terminal_profile" => 32,
                "inner_term" => 64,
                "status_format" => 128,
                "status_label" => 256,
                "status_refresh_seconds" => 512,
                "cpu_temperature_path" => 1024,
                "status_foreground" => 2048,
                "status_background" => 4096,
                "label_foreground" => 8192,
                "label_background" => 16384,
                "active_tab_foreground" => 32768,
                "active_tab_background" => 65536,
                _ => return Err(field_error(field, "unknown field")),
            };
            if seen & bit != 0 {
                return Err(field_error(field, "duplicate field"));
            }
            seen |= bit;

            match field {
                "prefix" => config.prefix = parse_prefix(field, value)?,
                "mouse" => config.mouse = parse_bool(field, value)?,
                "scrollback_lines" => config.scrollback_lines = parse_scrollback(field, value)?,
                "date_format" => config.date_format = parse_format(field, value)?,
                "time_format" => config.time_format = parse_format(field, value)?,
                "terminal_profile" => config.terminal_profile = parse_profile(field, value)?,
                "inner_term" => config.inner_term = parse_inner_term(field, value)?,
                "status_format" => config.status_format = parse_status_format(field, value)?,
                "status_label" => config.status_label = parse_text(field, value, 64)?,
                "status_refresh_seconds" => {
                    config.status_refresh_seconds = parse_refresh(field, value)?
                }
                "cpu_temperature_path" => {
                    config.cpu_temperature_path = Some(parse_temperature_path(field, value)?)
                }
                "status_foreground" => config.status_foreground = parse_color(field, value)?,
                "status_background" => config.status_background = parse_color(field, value)?,
                "label_foreground" => config.label_foreground = parse_color(field, value)?,
                "label_background" => config.label_background = parse_color(field, value)?,
                "active_tab_foreground" => {
                    config.active_tab_foreground = parse_color(field, value)?
                }
                "active_tab_background" => {
                    config.active_tab_background = parse_color(field, value)?
                }
                _ => unreachable!(),
            }
        }

        Ok(config)
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root).join("termfold/config.toml"));
    }

    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|root| PathBuf::from(root).join(".config/termfold/config.toml"))
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return &line[..index];
        }
    }

    line
}

fn parse_string(field: &str, value: &str) -> Result<String, String> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(field_error(field, "expected a quoted string"));
    }

    let mut output = String::new();
    let mut characters = value[1..value.len() - 1].chars();
    while let Some(character) = characters.next() {
        if character == '"' {
            return Err(field_error(field, "unescaped quote in string"));
        }
        if character != '\\' {
            output.push(character);
            continue;
        }

        let escaped = match characters.next() {
            Some('"') => '"',
            Some('\\') => '\\',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            _ => return Err(field_error(field, "invalid string escape")),
        };
        output.push(escaped);
    }

    Ok(output)
}

fn parse_prefix(field: &str, value: &str) -> Result<u8, String> {
    let value = parse_string(field, value)?;
    let bytes = value.as_bytes();
    if bytes.len() == 3 && bytes[0] == b'C' && bytes[1] == b'-' && bytes[2].is_ascii_lowercase() {
        Ok(bytes[2] - b'a' + 1)
    } else {
        Err(field_error(field, "expected one key from C-a through C-z"))
    }
}

fn parse_bool(field: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(field_error(field, "expected true or false")),
    }
}

fn parse_scrollback(field: &str, value: &str) -> Result<u16, String> {
    let value = value
        .parse::<u16>()
        .map_err(|_| field_error(field, "expected an integer from 0 through 10000"))?;
    if value <= 10_000 {
        Ok(value)
    } else {
        Err(field_error(
            field,
            "expected an integer from 0 through 10000",
        ))
    }
}

fn parse_format(field: &str, value: &str) -> Result<String, String> {
    let value = parse_string(field, value)?;
    if value.chars().count() > 64 {
        return Err(field_error(field, "must contain at most 64 characters"));
    }
    if value.chars().any(char::is_control) {
        return Err(field_error(field, "must not contain control characters"));
    }

    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            continue;
        }
        match characters.next() {
            Some('Y' | 'm' | 'd' | 'H' | 'I' | 'M' | 'S' | 'p' | '%') => {}
            _ => return Err(field_error(field, "contains an unsupported time directive")),
        }
    }

    Ok(value)
}

fn parse_text(field: &str, value: &str, maximum: usize) -> Result<String, String> {
    let value = parse_string(field, value)?;
    if value.chars().count() > maximum {
        return Err(field_error(
            field,
            &format!("must contain at most {maximum} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(field_error(field, "must not contain control characters"));
    }
    Ok(value)
}

fn parse_status_format(field: &str, value: &str) -> Result<String, String> {
    let value = parse_text(field, value, 512)?;
    let mut rest = value.as_str();
    let mut required = 0_u8;
    let mut fill = 0;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            return Err(field_error(field, "contains an unterminated placeholder"));
        };
        let placeholder = &rest[..end];
        match placeholder {
            "session" => required |= 1,
            "tabs" => required |= 2,
            "date" => required |= 4,
            "time" => required |= 8,
            "fill" => fill += 1,
            "label" | "cpu_usage" | "memory_usage" | "cpu_temp" => {}
            _ => return Err(field_error(field, "contains an unknown placeholder")),
        }
        rest = &rest[end + 1..];
    }
    if rest.contains('}') {
        return Err(field_error(field, "contains an unmatched closing brace"));
    }
    if required != 15 {
        return Err(field_error(
            field,
            "must contain {session}, {tabs}, {date}, and {time}",
        ));
    }
    if fill != 1 {
        return Err(field_error(field, "must contain exactly one {fill}"));
    }
    Ok(value)
}

fn parse_refresh(field: &str, value: &str) -> Result<u16, String> {
    let value = value
        .parse::<u16>()
        .map_err(|_| field_error(field, "expected an integer from 1 through 3600"))?;
    if (1..=3600).contains(&value) {
        Ok(value)
    } else {
        Err(field_error(
            field,
            "expected an integer from 1 through 3600",
        ))
    }
}

fn parse_temperature_path(field: &str, value: &str) -> Result<PathBuf, String> {
    let value = parse_string(field, value)?;
    let path = PathBuf::from(value);
    if path.starts_with("/sys")
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        Ok(path)
    } else {
        Err(field_error(field, "expected an absolute path below /sys"))
    }
}

fn parse_color(field: &str, value: &str) -> Result<Color, String> {
    let value = parse_string(field, value)?;
    if value == "default" {
        return Ok(Color::Default);
    }
    const NAMES: [&str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright-black",
        "bright-red",
        "bright-green",
        "bright-yellow",
        "bright-blue",
        "bright-magenta",
        "bright-cyan",
        "bright-white",
    ];
    if let Some(index) = NAMES.iter().position(|name| *name == value) {
        return Ok(Color::Indexed(index as u8));
    }
    if value.len() == 7
        && value.starts_with('#')
        && let (Ok(red), Ok(green), Ok(blue)) = (
            u8::from_str_radix(&value[1..3], 16),
            u8::from_str_radix(&value[3..5], 16),
            u8::from_str_radix(&value[5..7], 16),
        )
    {
        return Ok(Color::Rgb(red, green, blue));
    }
    Err(field_error(
        field,
        "expected default, an ANSI colour name, or #RRGGBB",
    ))
}

fn parse_profile(field: &str, value: &str) -> Result<String, String> {
    let value = parse_string(field, value)?;
    if value == "auto" || crate::outer::built_in(&value).is_some() {
        Ok(value)
    } else {
        Err(field_error(field, "unknown built-in terminal profile"))
    }
}

fn parse_inner_term(field: &str, value: &str) -> Result<String, String> {
    let value = parse_string(field, value)?;
    match value.as_str() {
        "termfold-256color" | "xterm-256color" => Ok(value),
        _ => Err(field_error(field, "unsupported inner terminal value")),
    }
}

fn field_error(field: &str, message: &str) -> String {
    format!("configuration field '{field}': {message}")
}

#[cfg(test)]
mod tests {
    use super::{Color, Config};

    #[test]
    fn parses_terminal_configuration_and_rejects_invalid_values() {
        let config =
            Config::parse("terminal_profile = \"tmux-256color\"\ninner_term = \"xterm-256color\"")
                .unwrap();
        assert_eq!(config.terminal_profile, "tmux-256color");
        assert_eq!(config.inner_term, "xterm-256color");

        assert_eq!(
            Config::parse("terminal_profile = \"unknown\"").unwrap_err(),
            "configuration field 'terminal_profile': unknown built-in terminal profile"
        );
        assert_eq!(
            Config::parse("inner_term = \"screen-256color\"").unwrap_err(),
            "configuration field 'inner_term': unsupported inner terminal value"
        );
    }

    #[test]
    fn parses_status_configuration_and_rejects_unsafe_formats() {
        let config = Config::parse(
            "status_format = \"[{session}] {tabs}{fill}{label} {cpu_usage}% {date} {time}\"\n\
             status_label = \"PROD │ db-02\"\n\
             status_refresh_seconds = 5\n\
             cpu_temperature_path = \"/sys/class/thermal/thermal_zone0/temp\"\n\
             status_foreground = \"#010203\"\n\
             active_tab_background = \"bright-yellow\"",
        )
        .unwrap();
        assert_eq!(config.status_label, "PROD │ db-02");
        assert_eq!(config.status_refresh_seconds, 5);
        assert_eq!(config.status_foreground, Color::Rgb(1, 2, 3));
        assert_eq!(config.active_tab_background, Color::Indexed(11));

        assert_eq!(
            Config::parse("status_format = \"{session}{tabs}{fill}{date}\"").unwrap_err(),
            "configuration field 'status_format': must contain {session}, {tabs}, {date}, and {time}"
        );
        assert_eq!(
            Config::parse("status_background = \"transparent\"").unwrap_err(),
            "configuration field 'status_background': expected default, an ANSI colour name, or #RRGGBB"
        );
        assert_eq!(
            Config::parse("cpu_temperature_path = \"/tmp/sensor\"").unwrap_err(),
            "configuration field 'cpu_temperature_path': expected an absolute path below /sys"
        );
    }
}
