use ratatui::style::{Color, Modifier, Style};

use crate::config::palette::Palette;

// ---------------------------------------------------------------------------
// Helper functions for color manipulation
// ---------------------------------------------------------------------------

/// Brighten an RGB color by adding `amount` to each channel, capped at 255.
fn brighten(color: Color, amount: u8) -> Color {
    if let Color::Rgb(r, g, b) = color {
        Color::Rgb(
            r.saturating_add(amount),
            g.saturating_add(amount),
            b.saturating_add(amount),
        )
    } else {
        color
    }
}

/// Darken a color by blending it toward `target` by `fraction` (0.0 = unchanged, 1.0 = target).
fn darken_toward(color: Color, target: Color, fraction: f32) -> Color {
    if let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (color, target) {
        let blend =
            |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * fraction).round() as u8 };
        Color::Rgb(blend(r1, r2), blend(g1, g2), blend(b1, b2))
    } else {
        color
    }
}

/// Midpoint between two colors.
fn midpoint(a: Color, b: Color) -> Color {
    darken_toward(a, b, 0.5)
}

// ---------------------------------------------------------------------------
// Theme — all styles derived from a Palette
// ---------------------------------------------------------------------------

pub struct Theme {
    // Full-screen background fill
    pub background: Style,

    // Raw color (for components that need Color, not Style)
    pub dim_text: Color,

    // Tree hierarchy
    pub collection: Style,
    pub thread: Style,
    pub thread_dim: Style,
    pub session: Style,
    pub highlight: Style,
    pub highlight_unfocused: Style,

    // Pin badge in agents view
    pub pin_badge: Style,

    // Chrome
    pub separator: Style,

    // Status bar
    pub statusbar_key: Style,
    pub statusbar_desc: Style,

    // Cursor
    pub cursor: Style,

    // Modals
    pub modal_border: Style,
    pub modal_title: Style,
    pub modal_muted: Style,

    // Empty state
    pub empty_title: Style,
    pub empty_hint: Style,

    // Agents
    pub agent: Style,
    pub agent_connector: Style,

    // Agent status dots
    pub status_working: Style,
    pub status_waiting: Style,
    pub status_idle: Style,

    // Badges
    pub badge_dot: Style,
    pub badge_count: Style,

    // Flash
    pub flash: Style,

    // Recent bar
    pub recent_number: Style,
    pub recent_name: Style,

    // Scrollbar
    pub scrollbar_thumb: Style,
    pub scrollbar_track: Style,

    // Notes sidebar
    pub notes_border_focused: Style,
    pub notes_border_unfocused: Style,
    pub notes_title_focused: Style,
    pub notes_title_unfocused: Style,
    pub notes_placeholder: Style,

    // Agent preview
    pub preview_border: Style,
    pub preview_title: Style,
    pub preview_placeholder: Style,
}

impl Theme {
    pub fn build(p: &Palette) -> Self {
        let dim_text = p.dim;
        let muted_text = p.muted;
        let subtle_border = p.border;

        // Derived: statusbar key is between dim and muted
        let statusbar_key_color = midpoint(p.dim, p.muted);
        // Derived: statusbar desc is between muted and border
        let statusbar_desc_color = midpoint(p.muted, p.border);
        // Derived: agent color is a light gray (between fg and dim)
        let agent_color = midpoint(p.fg, p.dim);

        Self {
            // Full-screen background
            background: Style::new().bg(p.bg),

            // Raw color
            dim_text,

            // Tree hierarchy
            collection: Style::new()
                .fg(brighten(p.accent, 16))
                .add_modifier(Modifier::BOLD),
            thread: Style::new().fg(p.accent),
            thread_dim: Style::new().fg(darken_toward(p.accent, p.border, 0.5)),
            session: Style::new().fg(p.green),
            highlight: Style::new()
                .fg(p.bg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
            highlight_unfocused: Style::new().fg(Color::White).bg(p.border),
            pin_badge: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),

            // Chrome
            separator: Style::new().fg(subtle_border),

            // Status bar
            statusbar_key: Style::new().fg(statusbar_key_color),
            statusbar_desc: Style::new().fg(statusbar_desc_color),

            // Cursor
            cursor: Style::new().fg(p.accent).add_modifier(Modifier::SLOW_BLINK),

            // Modals
            modal_border: Style::new().fg(p.accent),
            modal_title: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            modal_muted: Style::new().fg(muted_text),

            // Empty state
            empty_title: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            empty_hint: Style::new().fg(muted_text),

            // Agents
            agent: Style::new().fg(agent_color),
            agent_connector: Style::new().fg(muted_text),

            // Agent status dots (derived from palette: green / accent / muted)
            status_working: Style::new().fg(p.green),
            status_waiting: Style::new().fg(p.accent),
            status_idle: Style::new().fg(muted_text),

            // Badges
            badge_dot: Style::new().fg(p.green),
            badge_count: Style::new().fg(muted_text),

            // Flash
            flash: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),

            // Recent bar
            recent_number: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            recent_name: Style::new().fg(dim_text),

            // Scrollbar
            scrollbar_thumb: Style::new().fg(muted_text),
            scrollbar_track: Style::new().fg(subtle_border),

            // Notes sidebar
            notes_border_focused: Style::new().fg(p.accent),
            notes_border_unfocused: Style::new().fg(subtle_border),
            notes_title_focused: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            notes_title_unfocused: Style::new().fg(dim_text),
            notes_placeholder: Style::new().fg(muted_text),

            // Agent preview (shares unfocused styles with notes)
            preview_border: Style::new().fg(subtle_border),
            preview_title: Style::new().fg(dim_text),
            preview_placeholder: Style::new().fg(muted_text),
        }
    }
}

// ---------------------------------------------------------------------------
// NoteStyleSheet — tui-markdown stylesheet derived from Palette
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct NoteStyleSheet {
    accent: Color,
    green: Color,
    dim: Color,
    muted: Color,
}

impl NoteStyleSheet {
    pub fn new(p: &Palette) -> Self {
        Self {
            accent: p.accent,
            green: p.green,
            dim: p.dim,
            muted: p.muted,
        }
    }
}

impl tui_markdown::StyleSheet for NoteStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(self.accent).add_modifier(Modifier::BOLD),
            2 => Style::new().fg(self.accent),
            _ => Style::new().fg(self.dim).add_modifier(Modifier::ITALIC),
        }
    }

    fn code(&self) -> Style {
        Style::new().fg(self.green)
    }

    fn link(&self) -> Style {
        Style::new()
            .fg(self.accent)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(self.muted).add_modifier(Modifier::ITALIC)
    }

    fn heading_meta(&self) -> Style {
        Style::new().fg(self.muted)
    }

    fn metadata_block(&self) -> Style {
        Style::new().fg(self.muted)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tui_markdown::StyleSheet;

    #[test]
    fn default_theme_matches_old_constants() {
        let p = Palette::default();
        let t = Theme::build(&p);

        // Collection: bold, brightened accent
        assert_eq!(
            t.collection,
            Style::new()
                .fg(Color::Rgb(220, 136, 66))
                .add_modifier(Modifier::BOLD)
        );
        // Thread: plain accent
        assert_eq!(t.thread, Style::new().fg(Color::Rgb(204, 120, 50)));
        // Session: green
        assert_eq!(t.session, Style::new().fg(Color::Rgb(130, 180, 130)));
        // Highlight: bg=accent, fg=bg, bold
        assert_eq!(
            t.highlight,
            Style::new()
                .fg(Color::Rgb(30, 30, 30))
                .bg(Color::Rgb(204, 120, 50))
                .add_modifier(Modifier::BOLD)
        );
        // Modal border = accent
        assert_eq!(t.modal_border, Style::new().fg(Color::Rgb(204, 120, 50)));
    }

    #[test]
    fn custom_palette_changes_derived_styles() {
        let p = Palette {
            accent: Color::Rgb(255, 0, 0),
            ..Default::default()
        };
        let t = Theme::build(&p);

        // Thread should use the new accent
        assert_eq!(t.thread, Style::new().fg(Color::Rgb(255, 0, 0)));
        // Collection brightened
        assert_eq!(
            t.collection,
            Style::new()
                .fg(Color::Rgb(255, 16, 16))
                .add_modifier(Modifier::BOLD)
        );
        // Highlight bg should be new accent
        assert_eq!(
            t.highlight,
            Style::new()
                .fg(Color::Rgb(30, 30, 30))
                .bg(Color::Rgb(255, 0, 0))
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn brighten_caps_at_255() {
        assert_eq!(
            brighten(Color::Rgb(250, 250, 250), 16),
            Color::Rgb(255, 255, 255)
        );
    }

    #[test]
    fn darken_toward_fraction_zero_is_unchanged() {
        let c = Color::Rgb(200, 100, 50);
        assert_eq!(darken_toward(c, Color::Rgb(0, 0, 0), 0.0), c);
    }

    #[test]
    fn midpoint_blends_evenly() {
        let a = Color::Rgb(100, 100, 100);
        let b = Color::Rgb(200, 200, 200);
        assert_eq!(midpoint(a, b), Color::Rgb(150, 150, 150));
    }

    #[test]
    fn note_stylesheet_uses_palette() {
        let p = Palette {
            accent: Color::Rgb(255, 0, 0),
            ..Default::default()
        };
        let ss = NoteStyleSheet::new(&p);
        assert_eq!(
            ss.heading(1),
            Style::new()
                .fg(Color::Rgb(255, 0, 0))
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn status_colors_are_derived_from_palette() {
        let p = Palette::default();
        let t = Theme::build(&p);
        assert_eq!(t.status_working.fg, Some(p.green));
        assert_eq!(t.status_waiting.fg, Some(p.accent));
        assert_eq!(t.status_idle.fg, Some(p.muted));
    }
}
