use ratatui::style::{Color, Modifier, Style};

use crate::config::palette::Palette;

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

pub struct Theme {
    pub background: Style,
    pub selection_bar: Style,
    pub thread_name: Style,
    pub thread_idle: Style,
    pub session_name: Style,
    pub agent_name: Style,
    pub agent_name_loud: Style,
    pub pin_digit: Style,
    pub path_dim: Style,
    pub header_brand: Style,
    pub header_view_active: Style,
    pub header_view_inactive: Style,
    pub guide: Style,
    pub band: Style,
    pub header_rule: Style,
    pub header_rule_active: Style,
    pub meta: Style,
    pub dim_text: Color,
    pub highlight: Style,
    pub highlight_unfocused: Style,

    pub separator: Style,
    pub statusbar_key: Style,
    pub statusbar_desc: Style,

    pub cursor: Style,
    pub modal_border: Style,
    pub modal_title: Style,
    pub modal_muted: Style,

    pub empty_title: Style,
    pub empty_hint: Style,

    pub status_working: Style,
    pub status_waiting: Style,
    pub status_idle: Style,

    pub flash: Style,

    pub recent_number: Style,
    pub recent_name: Style,

    pub scrollbar_thumb: Style,
    pub scrollbar_track: Style,

    pub notes_border_focused: Style,
    pub notes_border_unfocused: Style,
    pub notes_title_focused: Style,
    pub notes_title_unfocused: Style,
    pub notes_placeholder: Style,

    pub preview_border: Style,
    pub preview_title: Style,
    pub preview_placeholder: Style,
}

impl Theme {
    pub fn build(p: &Palette) -> Self {
        let dim_text = p.dim;
        let muted_text = p.muted;
        let subtle_border = p.border;

        let statusbar_key_color = p.dim;
        let statusbar_desc_color = midpoint(p.muted, p.border);

        let selection_tint = darken_toward(p.accent, p.bg, 0.86);

        Self {
            background: Style::new().bg(p.bg),
            selection_bar: Style::new().fg(p.accent),
            thread_name: Style::new().fg(p.fg).add_modifier(Modifier::BOLD),
            thread_idle: Style::new().fg(muted_text),
            session_name: Style::new().fg(p.fg),
            agent_name: Style::new().fg(dim_text),
            agent_name_loud: Style::new().fg(p.fg),
            pin_digit: Style::new().fg(muted_text),
            path_dim: Style::new().fg(muted_text),
            header_brand: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            header_view_active: Style::new().fg(p.fg),
            header_view_inactive: Style::new().fg(muted_text),
            guide: Style::new().fg(subtle_border),
            band: Style::new().bg(darken_toward(p.fg, p.bg, 0.95)),
            header_rule: Style::new().fg(subtle_border),
            header_rule_active: Style::new().fg(p.accent),
            meta: Style::new().fg(midpoint(p.muted, p.border)),
            dim_text,
            highlight: Style::new().bg(selection_tint),
            highlight_unfocused: Style::new().bg(darken_toward(p.border, p.bg, 0.5)),

            separator: Style::new().fg(subtle_border),
            statusbar_key: Style::new().fg(statusbar_key_color),
            statusbar_desc: Style::new().fg(statusbar_desc_color),

            cursor: Style::new().fg(p.accent).add_modifier(Modifier::SLOW_BLINK),

            modal_border: Style::new().fg(p.accent),
            modal_title: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            modal_muted: Style::new().fg(muted_text),

            empty_title: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            empty_hint: Style::new().fg(muted_text),

            status_working: Style::new().fg(p.green),
            status_waiting: Style::new().fg(p.accent),
            status_idle: Style::new().fg(muted_text),

            flash: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),

            recent_number: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            recent_name: Style::new().fg(dim_text),

            scrollbar_thumb: Style::new().fg(muted_text),
            scrollbar_track: Style::new().fg(subtle_border),

            notes_border_focused: Style::new().fg(p.accent),
            notes_border_unfocused: Style::new().fg(subtle_border),
            notes_title_focused: Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            notes_title_unfocused: Style::new().fg(muted_text),
            notes_placeholder: Style::new().fg(muted_text),

            preview_border: Style::new().fg(subtle_border),
            preview_title: Style::new().fg(muted_text),
            preview_placeholder: Style::new().fg(muted_text),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use tui_markdown::StyleSheet;

    #[test]
    fn default_theme_matches_constants() {
        let p = Palette::default();
        let t = Theme::build(&p);

        assert_eq!(
            t.thread_name,
            Style::new()
                .fg(Color::Rgb(212, 212, 212))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(t.session_name, Style::new().fg(Color::Rgb(212, 212, 212)));
        // Selection is a faint accent tint, not a full inverse bar.
        assert_eq!(t.highlight, Style::new().bg(Color::Rgb(54, 43, 33)));
        assert_eq!(t.selection_bar, Style::new().fg(Color::Rgb(204, 120, 50)));
        assert_eq!(t.modal_border, Style::new().fg(Color::Rgb(204, 120, 50)));
    }

    #[test]
    fn custom_palette_changes_derived_styles() {
        let p = Palette {
            accent: Color::Rgb(255, 0, 0),
            ..Default::default()
        };
        let t = Theme::build(&p);

        assert_eq!(t.selection_bar, Style::new().fg(Color::Rgb(255, 0, 0)));
        assert_eq!(t.highlight, Style::new().bg(Color::Rgb(62, 26, 26)));
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
