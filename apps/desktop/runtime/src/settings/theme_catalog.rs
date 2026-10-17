/// Shared by persistence, theme cycling, and the desktop appearance picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalThemeGroup {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalThemeOption {
    pub name: &'static str,
    pub group: TerminalThemeGroup,
}

pub const TERMINAL_THEMES: &[TerminalThemeOption] = &[
    TerminalThemeOption {
        name: "Auto",
        group: TerminalThemeGroup::System,
    },
    TerminalThemeOption {
        name: "Codux Dark",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Carbon",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Slate",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Deep Ocean",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Forest Night",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Ember",
        group: TerminalThemeGroup::Dark,
    },
    TerminalThemeOption {
        name: "Codux Light",
        group: TerminalThemeGroup::Light,
    },
    TerminalThemeOption {
        name: "Paper Ink",
        group: TerminalThemeGroup::Light,
    },
    TerminalThemeOption {
        name: "Ivory",
        group: TerminalThemeGroup::Light,
    },
    TerminalThemeOption {
        name: "Matcha",
        group: TerminalThemeGroup::Light,
    },
    TerminalThemeOption {
        name: "Rose Dawn",
        group: TerminalThemeGroup::Light,
    },
];

fn normalized_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Retired presets retain their closest usable tone rather than resetting to Auto.
pub fn canonical_terminal_theme(value: &str) -> &'static str {
    let normalized = normalized_name(value);
    match normalized.as_str() {
        "arctic night" | "amethyst dusk" => "Slate",
        "rose noir" => "Ember",
        "glacier" | "morning mist" => "Paper Ink",
        "lavender" | "rosewater" => "Rose Dawn",
        "sandstone" => "Ivory",
        _ => TERMINAL_THEMES
            .iter()
            .find(|theme| normalized_name(theme.name) == normalized)
            .map(|theme| theme.name)
            .unwrap_or("Auto"),
    }
}
