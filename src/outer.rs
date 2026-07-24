#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Profile {
    Dumb,
    Ansi,
    Vt100,
    Linux,
    Xterm,
    Xterm256,
    Screen,
    Screen256,
    Tmux,
    Tmux256,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ColorLevel {
    Monochrome,
    Ansi16,
    Indexed256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub profile: Profile,
    pub color: ColorLevel,
    pub cursor_addressing: bool,
    pub cursor_visibility: bool,
    pub alternate_screen: bool,
    pub mouse: bool,
    pub faint: bool,
    pub italic: bool,
    pub blink: bool,
    pub strike: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchReason {
    Override,
    Exact,
    Alias,
    Family,
    Fallback,
}

impl MatchReason {
    pub fn name(self) -> &'static str {
        match self {
            Self::Override => "configuration override",
            Self::Exact => "exact built-in name",
            Self::Alias => "built-in alias",
            Self::Family => "audited family match",
            Self::Fallback => "conservative ANSI fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub capabilities: Capabilities,
    pub reason: MatchReason,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dumb => "dumb",
            Self::Ansi => "ansi",
            Self::Vt100 => "vt100",
            Self::Linux => "linux",
            Self::Xterm => "xterm",
            Self::Xterm256 => "xterm-256color",
            Self::Screen => "screen",
            Self::Screen256 => "screen-256color",
            Self::Tmux => "tmux",
            Self::Tmux256 => "tmux-256color",
        }
    }

    fn base(self) -> Capabilities {
        let color = match self {
            Self::Dumb | Self::Vt100 => ColorLevel::Monochrome,
            Self::Ansi | Self::Linux | Self::Xterm | Self::Screen | Self::Tmux => {
                ColorLevel::Ansi16
            }
            Self::Xterm256 | Self::Screen256 | Self::Tmux256 => ColorLevel::Indexed256,
        };
        let multiplexed_or_xterm = matches!(
            self,
            Self::Xterm
                | Self::Xterm256
                | Self::Screen
                | Self::Screen256
                | Self::Tmux
                | Self::Tmux256
        );
        Capabilities {
            profile: self,
            color,
            cursor_addressing: self != Self::Dumb,
            cursor_visibility: matches!(
                self,
                Self::Linux
                    | Self::Xterm
                    | Self::Xterm256
                    | Self::Screen
                    | Self::Screen256
                    | Self::Tmux
                    | Self::Tmux256
            ),
            alternate_screen: multiplexed_or_xterm,
            mouse: multiplexed_or_xterm,
            faint: matches!(
                self,
                Self::Linux
                    | Self::Xterm
                    | Self::Xterm256
                    | Self::Screen
                    | Self::Screen256
                    | Self::Tmux
                    | Self::Tmux256
            ),
            italic: matches!(
                self,
                Self::Xterm | Self::Xterm256 | Self::Tmux | Self::Tmux256
            ),
            blink: self != Self::Dumb,
            strike: matches!(
                self,
                Self::Xterm | Self::Xterm256 | Self::Tmux | Self::Tmux256
            ),
        }
    }

    fn accepts_true_color_hint(self) -> bool {
        matches!(
            self,
            Self::Xterm | Self::Xterm256 | Self::Tmux | Self::Tmux256
        )
    }
}

pub fn built_in(name: &str) -> Option<Profile> {
    Some(match name {
        "dumb" => Profile::Dumb,
        "ansi" => Profile::Ansi,
        "vt100" => Profile::Vt100,
        "linux" => Profile::Linux,
        "xterm" => Profile::Xterm,
        "xterm-256color" => Profile::Xterm256,
        "screen" => Profile::Screen,
        "screen-256color" => Profile::Screen256,
        "tmux" => Profile::Tmux,
        "tmux-256color" => Profile::Tmux256,
        _ => return None,
    })
}

pub fn select(override_name: &str, term: &str, colorterm: &str) -> Selection {
    let (profile, reason) = if override_name != "auto" {
        (
            built_in(override_name).expect("configuration validates terminal profiles"),
            MatchReason::Override,
        )
    } else if let Some(profile) = built_in(term) {
        (profile, MatchReason::Exact)
    } else if matches!(
        term,
        "xterm-kitty" | "foot" | "foot-extra" | "alacritty" | "wezterm"
    ) {
        (Profile::Xterm256, MatchReason::Alias)
    } else if term.starts_with("xterm") || term.starts_with("rxvt") || term.starts_with("st-") {
        (
            if term.contains("256color") {
                Profile::Xterm256
            } else {
                Profile::Xterm
            },
            MatchReason::Family,
        )
    } else if term.starts_with("screen") {
        (
            if term.contains("256color") {
                Profile::Screen256
            } else {
                Profile::Screen
            },
            MatchReason::Family,
        )
    } else if term.starts_with("tmux") {
        (
            if term.contains("256color") {
                Profile::Tmux256
            } else {
                Profile::Tmux
            },
            MatchReason::Family,
        )
    } else {
        (Profile::Ansi, MatchReason::Fallback)
    };
    let mut capabilities = profile.base();
    if profile.accepts_true_color_hint()
        && matches!(
            colorterm.to_ascii_lowercase().as_str(),
            "truecolor" | "24bit"
        )
    {
        capabilities.color = ColorLevel::TrueColor;
    }
    Selection {
        capabilities,
        reason,
    }
}

pub fn from_wire(profile: u8, color: u8) -> Option<Capabilities> {
    let profile = built_in(
        [
            "dumb",
            "ansi",
            "vt100",
            "linux",
            "xterm",
            "xterm-256color",
            "screen",
            "screen-256color",
            "tmux",
            "tmux-256color",
        ]
        .get(usize::from(profile))?,
    )?;
    let color = match color {
        0 => ColorLevel::Monochrome,
        1 => ColorLevel::Ansi16,
        2 => ColorLevel::Indexed256,
        3 => ColorLevel::TrueColor,
        _ => return None,
    };
    let mut capabilities = profile.base();
    if color > capabilities.color
        && !(color == ColorLevel::TrueColor && profile.accepts_true_color_hint())
    {
        return None;
    }
    capabilities.color = color;
    Some(capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_precedence_and_hints_are_conservative() {
        let selected = select("linux", "xterm-256color", "truecolor");
        assert_eq!(selected.capabilities.profile, Profile::Linux);
        assert_eq!(selected.capabilities.color, ColorLevel::Ansi16);
        assert_eq!(selected.reason, MatchReason::Override);

        let selected = select("auto", "xterm-kitty", "24bit");
        assert_eq!(selected.capabilities.profile, Profile::Xterm256);
        assert_eq!(selected.capabilities.color, ColorLevel::TrueColor);
        assert_eq!(selected.reason, MatchReason::Alias);

        let selected = select("auto", "unknown", "truecolor");
        assert_eq!(selected.capabilities.profile, Profile::Ansi);
        assert_eq!(selected.capabilities.color, ColorLevel::Ansi16);
        assert_eq!(selected.reason, MatchReason::Fallback);
    }
}
