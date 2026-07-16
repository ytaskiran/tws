use ratatui::style::Color;
use serde::{Deserialize, Deserializer};

pub fn parse_hex_color(hex: &str) -> Result<Color, String> {
    let hex = hex.trim();
    if !hex.starts_with('#') {
        return Err(format!("color must start with '#', got: {hex}"));
    }
    let digits = &hex[1..];
    if digits.len() != 6 {
        return Err(format!(
            "color must be 6 hex digits after '#', got {} digits",
            digits.len()
        ));
    }
    let r = u8::from_str_radix(&digits[0..2], 16)
        .map_err(|_| format!("invalid hex digits in red component: {}", &digits[0..2]))?;
    let g = u8::from_str_radix(&digits[2..4], 16)
        .map_err(|_| format!("invalid hex digits in green component: {}", &digits[2..4]))?;
    let b = u8::from_str_radix(&digits[4..6], 16)
        .map_err(|_| format!("invalid hex digits in blue component: {}", &digits[4..6]))?;
    Ok(Color::Rgb(r, g, b))
}

fn deserialize_color<'de, D>(deserializer: D) -> Result<Color, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_hex_color(&s).map_err(serde::de::Error::custom)
}

fn deserialize_option_color<'de, D>(deserializer: D) -> Result<Option<Color>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_hex_color(&s)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Palette {
    #[serde(deserialize_with = "deserialize_color")]
    pub accent: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub green: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub fg: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub dim: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub muted: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub border: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub bg: Color,
    /// Agent status indicators. Running and waiting are saturated enough to
    /// separate at a glance across the tree; idle stays dim so quiet agents
    /// recede.
    #[serde(deserialize_with = "deserialize_color")]
    pub status_running: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub status_waiting: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub status_idle: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            accent: Color::Rgb(204, 120, 50),
            green: Color::Rgb(130, 180, 130),
            fg: Color::Rgb(212, 212, 212),
            dim: Color::Rgb(160, 160, 160),
            muted: Color::Rgb(100, 100, 100),
            border: Color::Rgb(60, 60, 60),
            bg: Color::Rgb(30, 30, 30),
            status_running: Color::Rgb(74, 222, 128), // vivid green
            status_waiting: Color::Rgb(251, 191, 36), // vivid amber
            status_idle: Color::Rgb(100, 100, 100),   // dim gray (== muted)
        }
    }
}

/// Partial palette override: only fields explicitly set by the user are `Some`.
/// Used for `[palette]` overrides in `config.toml` — `None` means "inherit from theme".
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct PaletteOverride {
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub accent: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub green: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub fg: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub dim: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub muted: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub border: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub bg: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub status_running: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub status_waiting: Option<Color>,
    #[serde(default, deserialize_with = "deserialize_option_color")]
    pub status_idle: Option<Color>,
}

impl Palette {
    /// Return a new `Palette` using `self` as the base, applying any `Some` fields from
    /// `overrides` on top. `None` means the field was absent in the config → inherit.
    pub fn with_overrides(&self, overrides: &PaletteOverride) -> Palette {
        Palette {
            accent: overrides.accent.unwrap_or(self.accent),
            green: overrides.green.unwrap_or(self.green),
            fg: overrides.fg.unwrap_or(self.fg),
            dim: overrides.dim.unwrap_or(self.dim),
            muted: overrides.muted.unwrap_or(self.muted),
            border: overrides.border.unwrap_or(self.border),
            bg: overrides.bg.unwrap_or(self.bg),
            status_running: overrides.status_running.unwrap_or(self.status_running),
            status_waiting: overrides.status_waiting.unwrap_or(self.status_waiting),
            status_idle: overrides.status_idle.unwrap_or(self.status_idle),
        }
    }
}

#[derive(Deserialize)]
struct ThemeFile {
    palette: Palette,
}

pub fn load_preset(name: &str) -> Option<Palette> {
    let toml_str = match name {
        "default" => include_str!("../themes/default.toml"),
        "catppuccin-mocha" => include_str!("../themes/catppuccin-mocha.toml"),
        "catppuccin-latte" => include_str!("../themes/catppuccin-latte.toml"),
        "catppuccin-macchiato" => include_str!("../themes/catppuccin-macchiato.toml"),
        "catppuccin-frappe" => include_str!("../themes/catppuccin-frappe.toml"),
        "gruvbox-dark" => include_str!("../themes/gruvbox-dark.toml"),
        "gruvbox-light" => include_str!("../themes/gruvbox-light.toml"),
        "tokyo-night" => include_str!("../themes/tokyo-night.toml"),
        "nord" => include_str!("../themes/nord.toml"),
        _ => return None,
    };
    let theme_file: ThemeFile = toml::from_str(toml_str).expect("built-in theme TOML is valid");
    Some(theme_file.palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex_color("#cc7832"), Ok(Color::Rgb(204, 120, 50)));
        assert_eq!(parse_hex_color("#000000"), Ok(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_hex_color("#ffffff"), Ok(Color::Rgb(255, 255, 255)));
        // case insensitive
        assert_eq!(parse_hex_color("#FF9E64"), Ok(Color::Rgb(255, 158, 100)));
    }

    #[test]
    fn parse_hex_invalid() {
        // missing '#'
        assert!(parse_hex_color("cc7832").is_err());
        // too short
        assert!(parse_hex_color("#cc78").is_err());
        // invalid hex chars
        assert!(parse_hex_color("#gggggg").is_err());
        // empty after '#'
        assert!(parse_hex_color("#").is_err());
    }

    #[test]
    fn default_palette_matches_current_theme() {
        let p = Palette::default();
        assert_eq!(p.accent, Color::Rgb(204, 120, 50));
        assert_eq!(p.green, Color::Rgb(130, 180, 130));
        assert_eq!(p.fg, Color::Rgb(212, 212, 212));
        assert_eq!(p.dim, Color::Rgb(160, 160, 160));
        assert_eq!(p.muted, Color::Rgb(100, 100, 100));
        assert_eq!(p.border, Color::Rgb(60, 60, 60));
        assert_eq!(p.bg, Color::Rgb(30, 30, 30));
        assert_eq!(p.status_running, Color::Rgb(74, 222, 128));
        assert_eq!(p.status_waiting, Color::Rgb(251, 191, 36));
        assert_eq!(p.status_idle, Color::Rgb(100, 100, 100));
    }

    #[test]
    fn palette_deserialize_from_toml() {
        let toml_str = r##"
            accent = "#ff0000"
            green  = "#00ff00"
        "##;
        let p: Palette = toml::from_str(toml_str).expect("should deserialize");
        assert_eq!(p.accent, Color::Rgb(255, 0, 0));
        assert_eq!(p.green, Color::Rgb(0, 255, 0));
        // unspecified fields fall back to defaults
        let defaults = Palette::default();
        assert_eq!(p.fg, defaults.fg);
        assert_eq!(p.dim, defaults.dim);
        assert_eq!(p.muted, defaults.muted);
        assert_eq!(p.border, defaults.border);
        assert_eq!(p.bg, defaults.bg);
    }

    #[test]
    fn palette_deserialize_empty() {
        let p: Palette = toml::from_str("").expect("empty string should deserialize to default");
        assert_eq!(p, Palette::default());
    }

    #[test]
    fn load_embedded_preset_catppuccin_mocha() {
        let p = load_preset("catppuccin-mocha").expect("catppuccin-mocha should exist");
        assert_eq!(p.accent, Color::Rgb(203, 166, 247));
    }

    #[test]
    fn load_embedded_preset_default() {
        let p = load_preset("default").expect("default should exist");
        assert_eq!(p, Palette::default());
    }

    #[test]
    fn load_unknown_preset_returns_none() {
        assert!(load_preset("nonexistent").is_none());
    }
}
