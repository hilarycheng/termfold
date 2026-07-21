use std::{env, fs, io::ErrorKind, path::PathBuf};

#[derive(Debug)]
pub struct Config {
    pub prefix: u8,
    pub mouse: bool,
    pub scrollback_lines: u16,
    pub date_format: String,
    pub time_format: String,
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
        let mut seen = 0_u8;

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

fn parse_profile(field: &str, value: &str) -> Result<String, String> {
    let value = parse_string(field, value)?;
    match value.as_str() {
        "auto" | "dumb" | "ansi" | "vt100" | "linux" | "xterm" | "xterm-256color"
        | "screen" | "screen-256color" | "tmux" | "tmux-256color" => Ok(value),
        _ => Err(field_error(field, "unknown built-in terminal profile")),
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
    use super::Config;

    #[test]
    fn parses_terminal_configuration_and_rejects_invalid_values() {
        let config = Config::parse(
            "terminal_profile = \"tmux-256color\"\ninner_term = \"xterm-256color\"",
        )
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
}
