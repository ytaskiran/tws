pub mod keys;
pub mod palette;

use std::collections::HashMap;
use std::fs;

use serde::Deserialize;

use crate::core::persistence;

use keys::{KeyMode, Keymap};
use palette::{Palette, PaletteOverride};

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub theme: Option<String>,
    pub palette: Option<PaletteOverride>,
    pub keys: Option<KeysConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct KeysConfig {
    pub normal: Option<HashMap<String, String>>,
    pub agents: Option<HashMap<String, String>>,
    pub notes: Option<HashMap<String, String>>,
    pub finder: Option<HashMap<String, String>>,
    pub input: Option<HashMap<String, String>>,
    pub confirm: Option<HashMap<String, String>>,
}

/// Load `~/.config/tws/config.toml`. Missing file → default config.
/// Malformed TOML → print error and exit(1).
pub fn load_config() -> Config {
    let path = persistence::config_dir().join("config.toml");
    if !path.exists() {
        return Config::default();
    }
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tws: could not read config.toml: {}", e);
            std::process::exit(1);
        }
    };
    match toml::from_str::<Config>(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("tws: malformed config.toml: {}", e);
            std::process::exit(1);
        }
    }
}

/// Resolve the effective palette:
/// 1. Start with theme preset (default → "default").
/// 2. Check `~/.config/tws/themes/<name>.toml` for user custom themes.
/// 3. Fall back to built-in presets via `palette::load_preset`.
/// 4. Fall back to `Palette::default()` with a warning.
/// 5. Apply any inline `[palette]` overrides from the config.
pub fn resolve_palette(config: &Config) -> Palette {
    let theme_name = config.theme.as_deref().unwrap_or("default");

    // Try user custom theme file first
    let base = try_load_user_theme(theme_name)
        .or_else(|| palette::load_preset(theme_name))
        .unwrap_or_else(|| {
            if theme_name != "default" {
                eprintln!(
                    "tws: unknown theme '{}', falling back to default",
                    theme_name
                );
            }
            Palette::default()
        });

    // Apply inline palette overrides on top
    match &config.palette {
        Some(overrides) => base.with_overrides(overrides),
        None => base,
    }
}

#[derive(Deserialize)]
struct ThemeFile {
    palette: Palette,
}

fn try_load_user_theme(name: &str) -> Option<Palette> {
    let path = persistence::config_dir()
        .join("themes")
        .join(format!("{}.toml", name));
    let text = fs::read_to_string(&path).ok()?;
    match toml::from_str::<ThemeFile>(&text) {
        Ok(tf) => Some(tf.palette),
        Err(e) => {
            eprintln!("tws: malformed theme file {}: {}", path.display(), e);
            None
        }
    }
}

/// Build a `Keymap` starting from defaults and applying any user overrides
/// from the config's `[keys.*]` sections.
pub fn build_keymap(config: &Config) -> Keymap {
    let mut km = Keymap::default_bindings();
    let Some(keys_cfg) = &config.keys else {
        return km;
    };

    if let Some(overrides) = &keys_cfg.normal {
        km.apply_overrides(KeyMode::Normal, overrides);
    }
    if let Some(overrides) = &keys_cfg.agents {
        km.apply_overrides(KeyMode::Agents, overrides);
    }
    if let Some(overrides) = &keys_cfg.notes {
        km.apply_overrides(KeyMode::Notes, overrides);
    }
    if let Some(overrides) = &keys_cfg.finder {
        km.apply_overrides(KeyMode::Finder, overrides);
    }
    if let Some(overrides) = &keys_cfg.input {
        km.apply_overrides(KeyMode::Input, overrides);
    }
    if let Some(overrides) = &keys_cfg.confirm {
        km.apply_overrides(KeyMode::ConfirmModal, overrides);
    }

    km
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_config() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.theme.is_none());
        assert!(config.palette.is_none());
        assert!(config.keys.is_none());
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r##"
            theme = "catppuccin-mocha"
            [palette]
            accent = "#ff0000"
            [keys.normal]
            quit = "Q"
            add = "n"
        "##;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.theme.as_deref(), Some("catppuccin-mocha"));
        assert!(config.palette.is_some());
        let keys = config.keys.unwrap();
        let normal = keys.normal.unwrap();
        assert_eq!(normal.get("quit").map(|s| s.as_str()), Some("Q"));
    }

    #[test]
    fn resolve_palette_default() {
        let config = Config::default();
        let p = resolve_palette(&config);
        assert_eq!(p, palette::Palette::default());
    }

    #[test]
    fn resolve_palette_with_theme() {
        let config: Config = toml::from_str(r##"theme = "nord""##).unwrap();
        let p = resolve_palette(&config);
        assert_eq!(p.accent, ratatui::style::Color::Rgb(136, 192, 208));
    }

    #[test]
    fn resolve_palette_with_theme_and_override() {
        let toml_str = r##"
            theme = "nord"
            [palette]
            accent = "#ff0000"
        "##;
        let config: Config = toml::from_str(toml_str).unwrap();
        let p = resolve_palette(&config);
        assert_eq!(p.accent, ratatui::style::Color::Rgb(255, 0, 0));
        // green not overridden → nord's green, not the default palette's green
        assert_eq!(p.green, ratatui::style::Color::Rgb(163, 190, 140));
    }

    #[test]
    fn override_can_set_color_equal_to_default_palette() {
        // Sentinel-value regression: user explicitly sets green to the same value as
        // the default palette's green (#82b482). With nord as base, green would
        // otherwise be nord's green (163, 190, 140). The override must win.
        let toml_str = r##"
            theme = "nord"
            [palette]
            green = "#82b482"
        "##;
        let config: Config = toml::from_str(toml_str).unwrap();
        let p = resolve_palette(&config);
        assert_eq!(p.green, ratatui::style::Color::Rgb(130, 180, 130));
    }

    #[test]
    fn build_keymap_with_override() {
        let toml_str = r##"
            [keys.normal]
            quit = "Q"
        "##;
        let config: Config = toml::from_str(toml_str).unwrap();
        let km = build_keymap(&config);
        use crossterm::event::{KeyCode, KeyModifiers};
        assert_eq!(
            km.resolve(
                keys::KeyMode::Normal,
                KeyCode::Char('Q'),
                KeyModifiers::SHIFT
            ),
            Some(keys::Action::Quit)
        );
        assert_eq!(
            km.resolve(
                keys::KeyMode::Normal,
                KeyCode::Char('q'),
                KeyModifiers::NONE
            ),
            None
        );
    }

    #[test]
    fn full_config_round_trip() {
        let toml_str = r##"
            theme = "tokyo-night"

            [palette]
            accent = "#ff0000"

            [keys.normal]
            quit = "Q"
            add = "n"

            [keys.confirm]
            confirm = "enter"
        "##;
        let config: Config = toml::from_str(toml_str).unwrap();

        // Palette: tokyo-night base with accent override
        let p = resolve_palette(&config);
        assert_eq!(p.accent, ratatui::style::Color::Rgb(255, 0, 0));
        assert_eq!(p.green, ratatui::style::Color::Rgb(158, 206, 106)); // tokyo-night green

        // Theme builds without panic
        let theme = crate::theme::Theme::build(&p);
        assert_eq!(
            theme.thread,
            ratatui::style::Style::new().fg(ratatui::style::Color::Rgb(255, 0, 0))
        );

        // Keymap: 'Q' should be quit, 'q' unbound, 'n' is add
        let km = build_keymap(&config);
        use crossterm::event::{KeyCode, KeyModifiers};
        assert_eq!(
            km.resolve(
                keys::KeyMode::Normal,
                KeyCode::Char('Q'),
                KeyModifiers::SHIFT
            ),
            Some(keys::Action::Quit)
        );
        assert_eq!(
            km.resolve(
                keys::KeyMode::Normal,
                KeyCode::Char('q'),
                KeyModifiers::NONE
            ),
            None
        );
        assert_eq!(
            km.resolve(
                keys::KeyMode::Normal,
                KeyCode::Char('n'),
                KeyModifiers::NONE
            ),
            Some(keys::Action::Add)
        );
        // Confirm mode still works
        assert_eq!(
            km.resolve(
                keys::KeyMode::ConfirmModal,
                KeyCode::Enter,
                KeyModifiers::NONE
            ),
            Some(keys::Action::Confirm)
        );
    }
}
