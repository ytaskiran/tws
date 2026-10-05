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
    pub dir_picker: Option<HashMap<String, String>>,
}

/// Load `~/.config/tws/config.toml`. Missing file → default config.
/// Malformed TOML → print error and exit(1).
pub fn load_config() -> Config {
    try_load_config().unwrap_or_else(|e| {
        eprintln!("tws: {e}");
        std::process::exit(1);
    })
}

/// Like `load_config`, but returns the error, for a caller that has no place
/// to show it (`tws bar`).
pub fn try_load_config() -> Result<Config, String> {
    let path = persistence::config_dir().join("config.toml");
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("could not read config.toml: {e}"))?;
    toml::from_str::<Config>(&text).map_err(|e| format!("malformed config.toml: {e}"))
}

/// Resolve the effective palette from the config's `theme` and `[palette]`.
pub fn resolve_palette(config: &Config) -> Palette {
    palette_for(
        config.theme.as_deref().unwrap_or("default"),
        config.palette.as_ref(),
    )
}

/// Resolve the palette for one theme name:
/// 1. Check `~/.config/tws/themes/<name>.toml` for user custom themes.
/// 2. Fall back to built-in presets via `palette::load_preset`.
/// 3. Fall back to `Palette::default()` with a warning.
/// 4. Apply any inline `[palette]` overrides.
pub fn palette_for(theme_name: &str, overrides: Option<&PaletteOverride>) -> Palette {
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

    match overrides {
        Some(overrides) => base.with_overrides(overrides),
        None => base,
    }
}

/// Theme names for the picker: the built-in presets, then the user themes in
/// `~/.config/tws/themes/` that parse. A malformed file is left out, so
/// `palette_for` never prints its warning over the TUI.
pub fn theme_names() -> Vec<String> {
    let mut user: Vec<String> = fs::read_dir(persistence::config_dir().join("themes"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter(|p| {
            fs::read_to_string(p).is_ok_and(|text| toml::from_str::<ThemeFile>(&text).is_ok())
        })
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .filter(|n| !palette::PRESETS.contains(&n.as_str()))
        .collect();
    user.sort();
    palette::PRESETS
        .iter()
        .map(|n| n.to_string())
        .chain(user)
        .collect()
}

/// Write `theme = "<name>"` into `~/.config/tws/config.toml`.
pub fn save_theme(name: &str) -> Result<(), String> {
    let dir = persistence::config_dir();
    let path = dir.join("config.toml");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read config.toml: {e}")),
    };
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    // Like state.json: `tws bar` must never read a half-written file, and a
    // symlinked config.toml is written at its target, so the link stays.
    let target = fs::canonicalize(&path).unwrap_or(path);
    crate::core::status::write_atomic(&target, &set_theme(&text, name))
        .map_err(|e| format!("could not write config.toml: {e}"))
}

/// Replace the top-level `theme` line in `text`, or add one as line 1. Only
/// lines before the first `[table]` header are top-level keys, and line 1 is
/// always top-level, so every other line and comment stays as it is.
fn set_theme(text: &str, name: &str) -> String {
    let line = format!("theme = {}", toml::Value::String(name.to_string()));
    let mut lines: Vec<&str> = text.lines().collect();
    let existing = lines
        .iter()
        .take_while(|l| !l.trim_start().starts_with('['))
        .position(|l| {
            let l = l.trim_start();
            ["theme", "\"theme\"", "'theme'"].iter().any(|key| {
                l.strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
            })
        });
    match existing {
        Some(i) => lines[i] = &line,
        None => lines.insert(0, &line),
    }
    lines.join("\n") + "\n"
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
    if let Some(overrides) = &keys_cfg.dir_picker {
        km.apply_overrides(KeyMode::DirPicker, overrides);
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
    fn set_theme_replaces_top_level_line() {
        let text = "# my config\ntheme = \"nord\"\n\n[palette]\naccent = \"#ff0000\"\n";
        assert_eq!(
            set_theme(text, "gruvbox-dark"),
            "# my config\ntheme = \"gruvbox-dark\"\n\n[palette]\naccent = \"#ff0000\"\n"
        );
    }

    #[test]
    fn set_theme_inserts_when_missing() {
        assert_eq!(set_theme("", "nord"), "theme = \"nord\"\n");
        let text = "[keys.normal]\nquit = \"Q\"\n";
        let out = set_theme(text, "nord");
        assert_eq!(out, "theme = \"nord\"\n[keys.normal]\nquit = \"Q\"\n");
        let config: Config = toml::from_str(&out).unwrap();
        assert_eq!(config.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn set_theme_ignores_keys_inside_tables() {
        // `theme` under a table is not the top-level key, and `themes = ` is
        // another key; both stay, and a new top-level line goes first.
        let text = "themes_dir = \"x\"\n[keys.normal]\ntheme = \"t\"\n";
        let out = set_theme(text, "nord");
        assert_eq!(
            out,
            "theme = \"nord\"\nthemes_dir = \"x\"\n[keys.normal]\ntheme = \"t\"\n"
        );
    }

    #[test]
    fn set_theme_replaces_quoted_key() {
        let text = "\"theme\" = \"nord\"\n";
        assert_eq!(set_theme(text, "default"), "theme = \"default\"\n");
    }

    #[test]
    fn set_theme_replaces_whole_line() {
        let text = "  theme   =  \"nord\"  # pick\n";
        assert_eq!(set_theme(text, "default"), "theme = \"default\"\n");
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

        let p = resolve_palette(&config);
        assert_eq!(p.accent, ratatui::style::Color::Rgb(255, 0, 0));
        assert_eq!(p.green, ratatui::style::Color::Rgb(158, 206, 106));

        let theme = crate::theme::Theme::build(&p);
        assert_eq!(
            theme.selection_bar,
            ratatui::style::Style::new().fg(ratatui::style::Color::Rgb(255, 0, 0))
        );

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
