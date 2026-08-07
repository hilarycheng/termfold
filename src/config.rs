use std::{collections::BTreeMap, env, fs, io::ErrorKind, path::PathBuf};

use crate::terminal::Color;
use toml::{Table, Value};

#[derive(Debug)]
pub struct Config {
    pub prefix: u8,
    pub mouse: bool,
    pub scrollback_lines: u16,
    pub viewer_tab_width: u8,
    pub date_format: String,
    pub time_format: String,
    pub status_format: String,
    pub status_label: String,
    pub status_theme: String,
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
    pub windows_shell: Vec<String>,
    pub(crate) profiles: BTreeMap<String, crate::profile::Profile>,
    pub(crate) document: Table,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefix: 2,
            mouse: false,
            scrollback_lines: 2_000,
            viewer_tab_width: 8,
            date_format: "%Y-%m-%d".into(),
            time_format: "%H:%M".into(),
            status_format: "[{session}]  {tabs}{fill}|  {date} {time}".into(),
            status_label: String::new(),
            status_theme: "default".into(),
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
            windows_shell: Vec::new(),
            profiles: BTreeMap::new(),
            document: Table::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };

        match fs::read(&path) {
            Ok(source) => Self::parse_bytes(&source),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!(
                "cannot read configuration {}: {error}",
                path.display()
            )),
        }
    }

    fn parse_bytes(source: &[u8]) -> Result<Self, String> {
        let source = std::str::from_utf8(source)
            .map_err(|_| String::from("configuration: invalid UTF-8"))?;
        Self::parse(source)
    }

    fn parse(source: &str) -> Result<Self, String> {
        let document = source
            .parse::<Table>()
            .map_err(|error| format!("configuration: invalid TOML: {error}"))?;
        let profiles = crate::profile::parse(&document)?;
        let mut config = Self::default();
        for (field, value) in &document {
            match field.as_str() {
                "profiles" => {
                    if !value.is_table() {
                        return Err(field_error(field, "expected a table"));
                    }
                }
                "prefix" => config.prefix = parse_prefix(field, value)?,
                "mouse" => config.mouse = parse_bool(field, value)?,
                "scrollback_lines" => config.scrollback_lines = parse_scrollback(field, value)?,
                "viewer_tab_width" => config.viewer_tab_width = parse_tab_width(field, value)?,
                "date_format" => config.date_format = parse_format(field, value)?,
                "time_format" => config.time_format = parse_format(field, value)?,
                "terminal_profile" => config.terminal_profile = parse_profile(field, value)?,
                "inner_term" => config.inner_term = parse_inner_term(field, value)?,
                "windows_shell" => config.windows_shell = parse_string_array(field, value)?,
                "status_format" => config.status_format = parse_status_format(field, value)?,
                "status_label" => config.status_label = parse_text(field, value, 64)?,
                "status_theme" => config.status_theme = parse_status_theme(field, value)?,
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
                _ => return Err(field_error(field, "unknown field")),
            }
        }

        let explicit = [
            config.status_foreground,
            config.status_background,
            config.label_foreground,
            config.label_background,
            config.active_tab_foreground,
            config.active_tab_background,
        ];
        if let Some(colors) = theme_colors(&config.status_theme) {
            config.status_foreground = colors[0];
            config.status_background = colors[1];
            config.label_foreground = colors[2];
            config.label_background = colors[3];
            config.active_tab_foreground = colors[4];
            config.active_tab_background = colors[5];
            for (field, configured, value) in [
                (
                    "status_foreground",
                    &mut config.status_foreground,
                    explicit[0],
                ),
                (
                    "status_background",
                    &mut config.status_background,
                    explicit[1],
                ),
                (
                    "label_foreground",
                    &mut config.label_foreground,
                    explicit[2],
                ),
                (
                    "label_background",
                    &mut config.label_background,
                    explicit[3],
                ),
                (
                    "active_tab_foreground",
                    &mut config.active_tab_foreground,
                    explicit[4],
                ),
                (
                    "active_tab_background",
                    &mut config.active_tab_background,
                    explicit[5],
                ),
            ] {
                if document.contains_key(field) {
                    *configured = value;
                }
            }
        }

        config.profiles = profiles;
        config.document = document;
        Ok(config)
    }
}

fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(|root| PathBuf::from(root).join("Termfold").join("config.toml"))
    }

    #[cfg(not(target_os = "windows"))]
    if let Some(root) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root).join("termfold/config.toml"));
    }

    #[cfg(not(target_os = "windows"))]
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|root| PathBuf::from(root).join(".config/termfold/config.toml"))
}

fn parse_string(field: &str, value: &Value) -> Result<String, String> {
    value
        .as_str()
        .map(String::from)
        .ok_or_else(|| field_error(field, "expected a quoted string"))
}

fn parse_string_array(field: &str, value: &Value) -> Result<Vec<String>, String> {
    let Some(values) = value.as_array() else {
        return Err(field_error(
            field,
            "expected a non-empty array of quoted strings",
        ));
    };
    if values.is_empty() || values.iter().any(|value| value.as_str().is_none()) {
        Err(field_error(
            field,
            "expected a non-empty array of quoted strings",
        ))
    } else {
        let values = values
            .iter()
            .map(|value| value.as_str().expect("validated string array"))
            .map(String::from)
            .collect::<Vec<_>>();
        if values.iter().any(|value| value.contains('\0')) {
            Err(field_error(
                field,
                "expected a non-empty array of quoted strings",
            ))
        } else {
            Ok(values)
        }
    }
}

fn parse_prefix(field: &str, value: &Value) -> Result<u8, String> {
    let value = parse_string(field, value)?;
    let bytes = value.as_bytes();
    if bytes.len() == 3 && bytes[0] == b'C' && bytes[1] == b'-' && bytes[2].is_ascii_lowercase() {
        Ok(bytes[2] - b'a' + 1)
    } else {
        Err(field_error(field, "expected one key from C-a through C-z"))
    }
}

fn parse_bool(field: &str, value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| field_error(field, "expected true or false"))
}

fn parse_scrollback(field: &str, value: &Value) -> Result<u16, String> {
    let value = value
        .as_integer()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| field_error(field, "expected an integer from 0 through 10000"))?;
    if value <= 10_000 {
        Ok(value)
    } else {
        Err(field_error(
            field,
            "expected an integer from 0 through 10000",
        ))
    }
}

fn parse_tab_width(field: &str, value: &Value) -> Result<u8, String> {
    let value = value
        .as_integer()
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| field_error(field, "expected an integer from 1 through 16"))?;
    if (1..=16).contains(&value) {
        Ok(value)
    } else {
        Err(field_error(field, "expected an integer from 1 through 16"))
    }
}

fn parse_format(field: &str, value: &Value) -> Result<String, String> {
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

fn parse_text(field: &str, value: &Value, maximum: usize) -> Result<String, String> {
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

fn parse_status_format(field: &str, value: &Value) -> Result<String, String> {
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

fn parse_refresh(field: &str, value: &Value) -> Result<u16, String> {
    let value = value
        .as_integer()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| field_error(field, "expected an integer from 1 through 3600"))?;
    if (1..=3600).contains(&value) {
        Ok(value)
    } else {
        Err(field_error(
            field,
            "expected an integer from 1 through 3600",
        ))
    }
}

fn parse_temperature_path(field: &str, value: &Value) -> Result<PathBuf, String> {
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

fn parse_color(field: &str, value: &Value) -> Result<Color, String> {
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

fn parse_status_theme(field: &str, value: &Value) -> Result<String, String> {
    let value = parse_string(field, value)?;
    if value == "default" || theme_colors(&value).is_some() {
        Ok(value)
    } else {
        Err(field_error(field, "unknown built-in status theme"))
    }
}

fn theme_colors(name: &str) -> Option<[Color; 6]> {
    let colors = match name {
        "catppuccin-latte" => [0x4c4f69, 0xeff1f5, 0xeff1f5, 0xd20f39, 0xeff1f5, 0x1e66f5],
        "catppuccin-mocha" => [0xcdd6f4, 0x1e1e2e, 0x1e1e2e, 0xf38ba8, 0x1e1e2e, 0x89b4fa],
        "solarized-light" => [0x657b83, 0xfdf6e3, 0xfdf6e3, 0xdc322f, 0xfdf6e3, 0x268bd2],
        "solarized-dark" => [0x839496, 0x002b36, 0xfdf6e3, 0xdc322f, 0xfdf6e3, 0x268bd2],
        "gruvbox-light" => [0x3c3836, 0xfbf1c7, 0xfbf1c7, 0xcc241d, 0xfbf1c7, 0x458588],
        "gruvbox-dark" => [0xebdbb2, 0x282828, 0xfbf1c7, 0xcc241d, 0x282828, 0xd79921],
        "tokyo-night-day" => [0x3760bf, 0xe1e2e7, 0xe1e2e7, 0xf52a65, 0xe1e2e7, 0x2e7de9],
        "tokyo-night" => [0xc0caf5, 0x1a1b26, 0x1a1b26, 0xf7768e, 0x1a1b26, 0x7aa2f7],
        "dracula" => [0xf8f8f2, 0x282a36, 0x282a36, 0xff5555, 0x282a36, 0x8be9fd],
        "nord" => [0xd8dee9, 0x2e3440, 0x2e3440, 0xbf616a, 0x2e3440, 0x88c0d0],
        _ => return None,
    };
    Some(colors.map(|value| {
        Color::Rgb(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    }))
}

fn parse_profile(field: &str, value: &Value) -> Result<String, String> {
    let value = parse_string(field, value)?;
    if value == "auto" || crate::outer::built_in(&value).is_some() {
        Ok(value)
    } else {
        Err(field_error(field, "unknown built-in terminal profile"))
    }
}

fn parse_inner_term(field: &str, value: &Value) -> Result<String, String> {
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
    use super::{Color, Config, Value};

    #[test]
    fn parses_terminal_configuration_and_rejects_invalid_values() {
        let config = Config::parse(
            "terminal_profile = \"tmux-256color\"\n\
             inner_term = \"xterm-256color\"\n\
             windows_shell = [\"C:\\\\msys64\\\\usr\\\\bin\\\\bash.exe\", \"--login\"]",
        )
        .unwrap();
        assert_eq!(config.terminal_profile, "tmux-256color");
        assert_eq!(config.inner_term, "xterm-256color");
        assert_eq!(
            config.windows_shell,
            ["C:\\msys64\\usr\\bin\\bash.exe", "--login"]
        );

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
    fn validates_viewer_tab_width() {
        assert_eq!(Config::parse("").unwrap().viewer_tab_width, 8);
        assert_eq!(
            Config::parse("viewer_tab_width = 1")
                .unwrap()
                .viewer_tab_width,
            1
        );
        assert_eq!(
            Config::parse("viewer_tab_width = 16")
                .unwrap()
                .viewer_tab_width,
            16
        );
        for value in ["0", "17"] {
            assert_eq!(
                Config::parse(&format!("viewer_tab_width = {value}")).unwrap_err(),
                "configuration field 'viewer_tab_width': expected an integer from 1 through 16"
            );
        }
        assert_eq!(
            Config::parse("viewer_tab_width = 1\nviewer_tab_width = 2")
                .unwrap_err()
                .contains("duplicate key"),
            true
        );
    }

    #[test]
    fn accepts_toml_syntax_rejects_unknown_types_and_retains_profiles() {
        let config = Config::parse(
            "prefix = 'C-a' # standard TOML literal string\n\
             windows_shell = [\n\
                 'C:\\msys64\\usr\\bin\\bash.exe',\n\
                 '--login',\n\
             ]\n\
             [profiles.default]\n\
             directory = '/tmp'\n\
             tabs = [{ shell = true }]",
        )
        .unwrap();
        assert_eq!(config.prefix, 1);
        assert_eq!(
            config.windows_shell,
            ["C:\\msys64\\usr\\bin\\bash.exe", "--login"]
        );
        assert_eq!(
            config
                .document
                .get("profiles")
                .and_then(Value::as_table)
                .and_then(|profiles| profiles.get("default"))
                .and_then(Value::as_table)
                .and_then(|profile| profile.get("directory"))
                .and_then(Value::as_str),
            Some("/tmp")
        );

        assert_eq!(
            Config::parse("unknown = true").unwrap_err(),
            "configuration field 'unknown': unknown field"
        );
        assert_eq!(
            Config::parse("mouse = \"false\"").unwrap_err(),
            "configuration field 'mouse': expected true or false"
        );
        assert!(
            Config::parse("prefix = \"C-b\" = true")
                .unwrap_err()
                .contains("invalid TOML")
        );
        assert_eq!(
            Config::parse_bytes(b"prefix = \"C-b\"\xff").unwrap_err(),
            "configuration: invalid UTF-8"
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

    #[test]
    fn applies_status_themes_before_individual_colour_overrides() {
        let config =
            Config::parse("status_foreground = \"#010203\"\nstatus_theme = \"catppuccin-mocha\"")
                .unwrap();
        assert_eq!(config.status_foreground, Color::Rgb(1, 2, 3));
        assert_eq!(config.status_background, Color::Rgb(30, 30, 46));
        assert_eq!(config.active_tab_background, Color::Rgb(137, 180, 250));

        assert_eq!(
            Config::parse("status_theme = \"unknown\"").unwrap_err(),
            "configuration field 'status_theme': unknown built-in status theme"
        );
    }
}
