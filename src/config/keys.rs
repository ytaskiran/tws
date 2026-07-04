use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    MoveDown,
    MoveUp,
    MoveLeft,
    MoveRight,
    Quit,
    Enter,
    Add,
    AddCollection,
    Rename,
    Delete,
    KillSession,
    Finder,
    ExpandAll,
    ToggleSelect,
    Deselect,
    RecentSession1,
    RecentSession2,
    RecentSession3,
    RecentSession4,
    RecentSession5,
    ToggleView,
    OpenEditor,
    ScrollUp,
    ScrollDown,
    Confirm,
    Cancel,
    Backspace,
    Move,
    PinAgent,
    PinAgentSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyMode {
    Normal,
    Agents,
    Notes,
    Finder,
    Input,
    ConfirmModal,
}

/// Parse a key string like "q", "A", "ctrl+j", "enter", "space", etc.
/// into a crossterm `KeyEvent`.
pub fn parse_key(s: &str) -> Result<KeyEvent, String> {
    if s.is_empty() {
        return Err("empty key string".to_string());
    }

    // Handle ctrl+ prefix
    if let Some(rest) = s.strip_prefix("ctrl+") {
        if rest.is_empty() {
            return Err("ctrl+ requires a key".to_string());
        }
        let code = parse_keycode(rest)?;
        return Ok(KeyEvent::new(code, KeyModifiers::CONTROL));
    }

    // Handle alt+ prefix
    if let Some(rest) = s.strip_prefix("alt+") {
        if rest.is_empty() {
            return Err("alt+ requires a key".to_string());
        }
        let code = parse_keycode(rest)?;
        return Ok(KeyEvent::new(code, KeyModifiers::ALT));
    }

    // Named special keys (case-insensitive)
    let code = parse_keycode(s)?;
    let modifiers = match &code {
        KeyCode::Char(c) if c.is_uppercase() => KeyModifiers::SHIFT,
        _ => KeyModifiers::NONE,
    };
    Ok(KeyEvent::new(code, modifiers))
}

fn parse_keycode(s: &str) -> Result<KeyCode, String> {
    match s {
        "enter" => Ok(KeyCode::Enter),
        "esc" => Ok(KeyCode::Esc),
        "space" => Ok(KeyCode::Char(' ')),
        "tab" => Ok(KeyCode::Tab),
        "backspace" => Ok(KeyCode::Backspace),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "delete" => Ok(KeyCode::Delete),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" => Ok(KeyCode::PageUp),
        "pagedown" => Ok(KeyCode::PageDown),
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(KeyCode::Char(c)),
                _ => Err(format!("unknown key: {:?}", s)),
            }
        }
    }
}

/// Parse an action name string (snake_case) into an `Action`.
pub fn parse_action(s: &str) -> Result<Action, String> {
    match s {
        "move_down" => Ok(Action::MoveDown),
        "move_up" => Ok(Action::MoveUp),
        "move_left" => Ok(Action::MoveLeft),
        "move_right" => Ok(Action::MoveRight),
        "quit" => Ok(Action::Quit),
        "enter" => Ok(Action::Enter),
        "add" => Ok(Action::Add),
        "add_collection" => Ok(Action::AddCollection),
        "rename" => Ok(Action::Rename),
        "delete" => Ok(Action::Delete),
        "kill_session" => Ok(Action::KillSession),
        "finder" => Ok(Action::Finder),
        "expand_all" => Ok(Action::ExpandAll),
        "toggle_select" => Ok(Action::ToggleSelect),
        "deselect" => Ok(Action::Deselect),
        "recent_session_1" => Ok(Action::RecentSession1),
        "recent_session_2" => Ok(Action::RecentSession2),
        "recent_session_3" => Ok(Action::RecentSession3),
        "recent_session_4" => Ok(Action::RecentSession4),
        "recent_session_5" => Ok(Action::RecentSession5),
        "toggle_view" => Ok(Action::ToggleView),
        "open_editor" => Ok(Action::OpenEditor),
        "scroll_up" => Ok(Action::ScrollUp),
        "scroll_down" => Ok(Action::ScrollDown),
        "confirm" => Ok(Action::Confirm),
        "cancel" => Ok(Action::Cancel),
        "backspace" => Ok(Action::Backspace),
        "move" => Ok(Action::Move),
        "pin_agent" => Ok(Action::PinAgent),
        "pin_agent_slot" => Ok(Action::PinAgentSlot),
        _ => Err(format!("unknown action: {:?}", s)),
    }
}

fn fmt_key(code: KeyCode, mods: KeyModifiers) -> String {
    let base = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "BS".to_string(),
        KeyCode::Up => "\u{2191}".to_string(),
        KeyCode::Down => "\u{2193}".to_string(),
        KeyCode::Left => "\u{2190}".to_string(),
        KeyCode::Right => "\u{2192}".to_string(),
        _ => format!("{:?}", code),
    };
    if mods.contains(KeyModifiers::CONTROL) {
        format!("ctrl+{}", base)
    } else {
        base
    }
}

/// For `Char` keycodes, SHIFT is redundant — the case of the char already encodes
/// shift. Stripping it means bindings work regardless of whether the terminal
/// includes the SHIFT flag (behavior varies across terminals and shifted symbols).
fn normalized_mods(code: KeyCode, mods: KeyModifiers) -> KeyModifiers {
    match code {
        KeyCode::Char(_) => mods.difference(KeyModifiers::SHIFT),
        _ => mods,
    }
}

pub struct Keymap {
    map: HashMap<(KeyMode, KeyCode, KeyModifiers), Action>,
}

impl Keymap {
    pub fn default_bindings() -> Self {
        let mut map = HashMap::new();

        macro_rules! bind {
            ($mode:expr, $code:expr, $mods:expr, $action:expr) => {
                map.insert(($mode, $code, normalized_mods($code, $mods)), $action);
            };
        }

        use Action as A;
        use KeyMode as M;

        // ── Normal mode ─────────────────────────────────────────────────────
        bind!(M::Normal, KeyCode::Char('q'), KeyModifiers::NONE, A::Quit);
        bind!(
            M::Normal,
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            A::MoveDown
        );
        bind!(M::Normal, KeyCode::Down, KeyModifiers::NONE, A::MoveDown);
        bind!(M::Normal, KeyCode::Char('k'), KeyModifiers::NONE, A::MoveUp);
        bind!(M::Normal, KeyCode::Up, KeyModifiers::NONE, A::MoveUp);
        bind!(
            M::Normal,
            KeyCode::Char('h'),
            KeyModifiers::NONE,
            A::MoveLeft
        );
        bind!(M::Normal, KeyCode::Left, KeyModifiers::NONE, A::MoveLeft);
        bind!(
            M::Normal,
            KeyCode::Char('l'),
            KeyModifiers::NONE,
            A::MoveRight
        );
        bind!(M::Normal, KeyCode::Right, KeyModifiers::NONE, A::MoveRight);
        bind!(
            M::Normal,
            KeyCode::Char(' '),
            KeyModifiers::NONE,
            A::ToggleSelect
        );
        bind!(M::Normal, KeyCode::Enter, KeyModifiers::NONE, A::Enter);
        bind!(M::Normal, KeyCode::Esc, KeyModifiers::NONE, A::Deselect);
        bind!(M::Normal, KeyCode::Char('a'), KeyModifiers::NONE, A::Add);
        bind!(
            M::Normal,
            KeyCode::Char('A'),
            KeyModifiers::SHIFT,
            A::AddCollection
        );
        bind!(M::Normal, KeyCode::Char('r'), KeyModifiers::NONE, A::Rename);
        bind!(M::Normal, KeyCode::Char('d'), KeyModifiers::NONE, A::Delete);
        bind!(
            M::Normal,
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            A::KillSession
        );
        bind!(M::Normal, KeyCode::Char('m'), KeyModifiers::NONE, A::Move);
        bind!(M::Normal, KeyCode::Char('/'), KeyModifiers::NONE, A::Finder);
        bind!(
            M::Normal,
            KeyCode::Char('e'),
            KeyModifiers::NONE,
            A::ExpandAll
        );
        bind!(
            M::Normal,
            KeyCode::Char('1'),
            KeyModifiers::NONE,
            A::RecentSession1
        );
        bind!(
            M::Normal,
            KeyCode::Char('2'),
            KeyModifiers::NONE,
            A::RecentSession2
        );
        bind!(
            M::Normal,
            KeyCode::Char('3'),
            KeyModifiers::NONE,
            A::RecentSession3
        );
        bind!(
            M::Normal,
            KeyCode::Char('4'),
            KeyModifiers::NONE,
            A::RecentSession4
        );
        bind!(
            M::Normal,
            KeyCode::Char('5'),
            KeyModifiers::NONE,
            A::RecentSession5
        );
        bind!(
            M::Normal,
            KeyCode::Char('v'),
            KeyModifiers::NONE,
            A::ToggleView
        );

        // ── Agents mode ──────────────────────────────────────────────────────
        bind!(
            M::Agents,
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            A::MoveDown
        );
        bind!(M::Agents, KeyCode::Down, KeyModifiers::NONE, A::MoveDown);
        bind!(M::Agents, KeyCode::Char('k'), KeyModifiers::NONE, A::MoveUp);
        bind!(M::Agents, KeyCode::Up, KeyModifiers::NONE, A::MoveUp);
        bind!(M::Agents, KeyCode::Enter, KeyModifiers::NONE, A::Enter);
        bind!(M::Agents, KeyCode::Esc, KeyModifiers::NONE, A::Cancel);
        bind!(M::Agents, KeyCode::Char('q'), KeyModifiers::NONE, A::Quit);
        bind!(
            M::Agents,
            KeyCode::Char('p'),
            KeyModifiers::NONE,
            A::PinAgent
        );
        bind!(
            M::Agents,
            KeyCode::Char('P'),
            KeyModifiers::SHIFT,
            A::PinAgentSlot
        );

        // ── Notes mode ───────────────────────────────────────────────────────
        bind!(M::Notes, KeyCode::Enter, KeyModifiers::NONE, A::OpenEditor);
        bind!(M::Notes, KeyCode::Esc, KeyModifiers::NONE, A::Cancel);
        bind!(
            M::Notes,
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            A::ScrollUp
        );
        bind!(M::Notes, KeyCode::Up, KeyModifiers::NONE, A::ScrollUp);
        bind!(
            M::Notes,
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            A::ScrollDown
        );
        bind!(M::Notes, KeyCode::Down, KeyModifiers::NONE, A::ScrollDown);

        // ── Input modal ──────────────────────────────────────────────────────
        bind!(M::Input, KeyCode::Esc, KeyModifiers::NONE, A::Cancel);
        bind!(M::Input, KeyCode::Enter, KeyModifiers::NONE, A::Confirm);
        bind!(
            M::Input,
            KeyCode::Backspace,
            KeyModifiers::NONE,
            A::Backspace
        );

        // ── Confirm modal ────────────────────────────────────────────────────
        bind!(
            M::ConfirmModal,
            KeyCode::Char('y'),
            KeyModifiers::NONE,
            A::Confirm
        );
        bind!(
            M::ConfirmModal,
            KeyCode::Enter,
            KeyModifiers::NONE,
            A::Confirm
        );
        bind!(
            M::ConfirmModal,
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            A::Cancel
        );
        bind!(M::ConfirmModal, KeyCode::Esc, KeyModifiers::NONE, A::Cancel);

        // ── Finder ───────────────────────────────────────────────────────────
        bind!(M::Finder, KeyCode::Esc, KeyModifiers::NONE, A::Cancel);
        bind!(M::Finder, KeyCode::Enter, KeyModifiers::NONE, A::Confirm);
        bind!(M::Finder, KeyCode::Down, KeyModifiers::NONE, A::MoveDown);
        bind!(
            M::Finder,
            KeyCode::Char('j'),
            KeyModifiers::CONTROL,
            A::MoveDown
        );
        bind!(M::Finder, KeyCode::Up, KeyModifiers::NONE, A::MoveUp);
        bind!(
            M::Finder,
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
            A::MoveUp
        );
        bind!(
            M::Finder,
            KeyCode::Backspace,
            KeyModifiers::NONE,
            A::Backspace
        );

        Self { map }
    }

    pub fn resolve(&self, mode: KeyMode, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        self.map
            .get(&(mode, code, normalized_mods(code, modifiers)))
            .copied()
    }

    /// Returns a human-readable label for the primary key bound to `action` in
    /// `mode`. Prefers named/char bindings over arrow keys. Returns `"?"` if
    /// the action is unbound.
    pub fn key_hint(&self, mode: KeyMode, action: Action) -> String {
        let mut char_binding: Option<String> = None;
        let mut fallback: Option<String> = None;
        for (&(m, code, mods), &a) in &self.map {
            if m == mode && a == action {
                let s = fmt_key(code, mods);
                if matches!(
                    code,
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
                ) {
                    fallback.get_or_insert(s);
                } else {
                    char_binding.get_or_insert(s);
                }
            }
        }
        char_binding.or(fallback).unwrap_or_else(|| "?".to_string())
    }

    /// Returns `"<a>/<b>"` combining hints for two actions — useful for paired
    /// nav hints like `k/j · scroll`.
    pub fn key_hint_pair(&self, mode: KeyMode, a: Action, b: Action) -> String {
        format!("{}/{}", self.key_hint(mode, a), self.key_hint(mode, b))
    }

    pub fn apply_overrides(&mut self, mode: KeyMode, overrides: &HashMap<String, String>) {
        for (action_str, key_str) in overrides {
            let action = match parse_action(action_str) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("tws: {}", e);
                    continue;
                }
            };
            let key = match parse_key(key_str) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("tws: invalid key '{}': {}", key_str, e);
                    continue;
                }
            };
            // Remove old binding(s) for this action in this mode
            self.map
                .retain(|&(m, _, _), &mut a| !(m == mode && a == action));
            // Normalize and insert; warn if the key slot is already taken
            let norm_mods = normalized_mods(key.code, key.modifiers);
            if let Some(&existing) = self.map.get(&(mode, key.code, norm_mods)) {
                eprintln!(
                    "tws: [keys] '{}' is already bound to {:?}; overriding with {:?}",
                    key_str, existing, action
                );
            }
            self.map.insert((mode, key.code, norm_mods), action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn parse_simple_char() {
        let key = parse_key("q").unwrap();
        assert_eq!(key.code, KeyCode::Char('q'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_uppercase_char() {
        let key = parse_key("A").unwrap();
        assert_eq!(key.code, KeyCode::Char('A'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_ctrl_combo() {
        let key = parse_key("ctrl+j").unwrap();
        assert_eq!(key.code, KeyCode::Char('j'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(parse_key("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_key("tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key("backspace").unwrap().code, KeyCode::Backspace);
        assert_eq!(parse_key("up").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key("down").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key("left").unwrap().code, KeyCode::Left);
        assert_eq!(parse_key("right").unwrap().code, KeyCode::Right);
    }

    #[test]
    fn parse_special_char() {
        let key = parse_key("/").unwrap();
        assert_eq!(key.code, KeyCode::Char('/'));
    }

    #[test]
    fn parse_action_names() {
        assert_eq!(parse_action("quit").unwrap(), Action::Quit);
        assert_eq!(parse_action("move_down").unwrap(), Action::MoveDown);
        assert_eq!(
            parse_action("add_collection").unwrap(),
            Action::AddCollection
        );
        assert_eq!(parse_action("kill_session").unwrap(), Action::KillSession);
        assert_eq!(parse_action("toggle_view").unwrap(), Action::ToggleView);
        assert_eq!(parse_action("open_editor").unwrap(), Action::OpenEditor);
        assert_eq!(
            parse_action("recent_session_1").unwrap(),
            Action::RecentSession1
        );
    }

    #[test]
    fn parse_unknown_action() {
        assert!(parse_action("nonexistent").is_err());
    }

    #[test]
    fn parse_invalid_key() {
        assert!(parse_key("").is_err());
        assert!(parse_key("ctrl+").is_err());
    }

    #[test]
    fn default_keymap_normal_quit() {
        let km = Keymap::default_bindings();
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('q'), KeyModifiers::NONE),
            Some(Action::Quit)
        );
    }

    #[test]
    fn default_keymap_normal_movement() {
        let km = Keymap::default_bindings();
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Action::MoveDown)
        );
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Down, KeyModifiers::NONE),
            Some(Action::MoveDown)
        );
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('k'), KeyModifiers::NONE),
            Some(Action::MoveUp)
        );
    }

    #[test]
    fn default_keymap_confirm_mode() {
        let km = Keymap::default_bindings();
        assert_eq!(
            km.resolve(
                KeyMode::ConfirmModal,
                KeyCode::Char('y'),
                KeyModifiers::NONE
            ),
            Some(Action::Confirm)
        );
        assert_eq!(
            km.resolve(KeyMode::ConfirmModal, KeyCode::Enter, KeyModifiers::NONE),
            Some(Action::Confirm)
        );
        assert_eq!(
            km.resolve(KeyMode::ConfirmModal, KeyCode::Esc, KeyModifiers::NONE),
            Some(Action::Cancel)
        );
    }

    #[test]
    fn default_keymap_agents_mode() {
        let km = Keymap::default_bindings();
        assert_eq!(
            km.resolve(KeyMode::Agents, KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Action::MoveDown)
        );
        assert_eq!(
            km.resolve(KeyMode::Agents, KeyCode::Enter, KeyModifiers::NONE),
            Some(Action::Enter)
        );
        assert_eq!(
            km.resolve(KeyMode::Agents, KeyCode::Char('q'), KeyModifiers::NONE),
            Some(Action::Quit)
        );
    }

    #[test]
    fn keymap_unbound_returns_none() {
        let km = Keymap::default_bindings();
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('z'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn resolve_uppercase_without_shift_flag() {
        // Terminals that omit SHIFT for uppercase chars must still match.
        let km = Keymap::default_bindings();
        // 'A' is bound as AddCollection — should resolve with or without SHIFT
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('A'), KeyModifiers::SHIFT),
            Some(Action::AddCollection)
        );
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('A'), KeyModifiers::NONE),
            Some(Action::AddCollection)
        );
        // 'P' (agents) bound as PinAgentSlot
        assert_eq!(
            km.resolve(KeyMode::Agents, KeyCode::Char('P'), KeyModifiers::SHIFT),
            Some(Action::PinAgentSlot)
        );
        assert_eq!(
            km.resolve(KeyMode::Agents, KeyCode::Char('P'), KeyModifiers::NONE),
            Some(Action::PinAgentSlot)
        );
    }

    #[test]
    fn resolve_shifted_symbol_works() {
        // Symbols like '?' may arrive with SHIFT from crossterm; binding via config uses NONE.
        // After normalization both should resolve the same.
        let mut km = Keymap::default_bindings();
        let mut overrides = HashMap::new();
        overrides.insert("finder".to_string(), "?".to_string());
        km.apply_overrides(KeyMode::Normal, &overrides);
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('?'), KeyModifiers::NONE),
            Some(Action::Finder)
        );
        assert_eq!(
            km.resolve(KeyMode::Normal, KeyCode::Char('?'), KeyModifiers::SHIFT),
            Some(Action::Finder)
        );
    }
}
