use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ansi_to_tui::IntoText;
use ratatui::style::Color;
use ratatui::text::{Line, Span, Text};

use super::persistence;
use crate::theme::NoteStyleSheet;

/// Cached markdown renderer that delegates to `glow` when available,
/// falling back to `tui-markdown` otherwise.
pub struct MarkdownRenderer {
    glow_available: bool,
    theme_path: PathBuf,
    cache: Option<RenderCache>,
    stylesheet: NoteStyleSheet,
}

struct RenderCache {
    source: String,
    width: u16,
    text: Text<'static>,
}

impl MarkdownRenderer {
    /// Create a new renderer, probing for `glow` on `$PATH`.
    pub fn new(stylesheet: NoteStyleSheet) -> Self {
        let glow_available = Command::new("glow")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());

        // Write the embedded theme to disk once if it doesn't exist.
        // Users can edit the file at ~/.config/tws/glow-theme.json to customize.
        let theme_path = persistence::config_dir().join("glow-theme.json");
        if glow_available && !theme_path.exists() {
            let _ = std::fs::write(&theme_path, include_str!("../../styles/tws-glow.json"));
        }

        Self {
            glow_available,
            theme_path,
            cache: None,
            stylesheet,
        }
    }

    /// Render markdown to styled `Text`, using cache when possible.
    ///
    /// Re-renders only when the source text or target width changes.
    pub fn render(&mut self, markdown: &str, width: u16) -> &Text<'static> {
        let cache_valid = self
            .cache
            .as_ref()
            .is_some_and(|c| c.source == markdown && c.width == width);

        if !cache_valid {
            let text = if self.glow_available {
                render_with_glow(markdown, width, &self.theme_path)
                    .unwrap_or_else(|| self.render_fallback(markdown))
            } else {
                self.render_fallback(markdown)
            };

            self.cache = Some(RenderCache {
                source: markdown.to_string(),
                width,
                text,
            });
        }

        &self.cache.as_ref().unwrap().text
    }

    /// Number of lines in the last rendered output. Returns 0 if nothing cached.
    pub fn line_count(&self) -> usize {
        self.cache.as_ref().map_or(0, |c| c.text.lines.len())
    }

    /// Clear the cache, forcing a re-render on the next `render()` call.
    pub fn invalidate(&mut self) {
        self.cache = None;
    }

    /// Fallback renderer using `tui-markdown` with the tws theme.
    ///
    /// Converts the borrowed `Text<'a>` to owned `Text<'static>` so it can be cached.
    fn render_fallback(&self, markdown: &str) -> Text<'static> {
        let options = tui_markdown::Options::new(self.stylesheet.clone());
        let text = tui_markdown::from_str_with_options(markdown, &options);
        to_owned_text(text)
    }
}

fn render_with_glow(markdown: &str, width: u16, theme_path: &Path) -> Option<Text<'static>> {
    let mut child = Command::new("glow")
        .args([
            "-w",
            &width.to_string(),
            "-s",
            &theme_path.to_string_lossy(),
            "-",
        ])
        .env("CLICOLOR_FORCE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    child.stdin.take()?.write_all(markdown.as_bytes()).ok()?;

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    let mut text = output.stdout.into_text().ok()?;
    clear_reset_backgrounds(&mut text);
    Some(text)
}

/// `glow` emits ANSI resets (`ESC[0m`) on every span, which `ansi_to_tui` maps
/// to `bg: Some(Color::Reset)`. At render time `Color::Reset` repaints the cell
/// with the *terminal's* default background — punching through the app's themed
/// background and making the text area look darker than the rest of the UI.
///
/// Remap those to `bg: None` so the cell keeps whatever background was painted
/// underneath (the app theme), matching how the tree renders. Backgrounds the
/// theme sets explicitly (any non-`Reset` color) are left untouched.
///
/// Also used for captured tmux pane content in the agent preview, which goes
/// through the same `into_text()` path and has the same `Reset` punch-through.
pub(crate) fn clear_reset_backgrounds(text: &mut Text<'static>) {
    for line in &mut text.lines {
        if line.style.bg == Some(Color::Reset) {
            line.style.bg = None;
        }
        for span in &mut line.spans {
            if span.style.bg == Some(Color::Reset) {
                span.style.bg = None;
            }
        }
    }
}

/// Convert `Text<'a>` (with borrowed `Cow::Borrowed` spans) to `Text<'static>`.
fn to_owned_text(text: Text<'_>) -> Text<'static> {
    let lines: Vec<Line<'static>> = text
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style))
                .collect();
            Line::from(spans).style(line.style)
        })
        .collect();
    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn clear_reset_backgrounds_remaps_reset_to_none() {
        let mut text = Text::from(vec![
            Line::from(vec![
                Span::styled("hello", Style::new().fg(Color::Gray).bg(Color::Reset)),
                Span::styled(" world", Style::new().bg(Color::Reset)),
            ])
            .style(Style::new().bg(Color::Reset)),
        ]);

        clear_reset_backgrounds(&mut text);

        assert_eq!(text.lines[0].style.bg, None);
        assert_eq!(text.lines[0].spans[0].style.bg, None);
        assert_eq!(text.lines[0].spans[1].style.bg, None);
        // Foreground is untouched.
        assert_eq!(text.lines[0].spans[0].style.fg, Some(Color::Gray));
    }

    #[test]
    fn clear_reset_backgrounds_preserves_explicit_bg() {
        let explicit = Color::Rgb(42, 42, 42);
        let mut text = Text::from(vec![Line::from(vec![Span::styled(
            "code",
            Style::new().bg(explicit),
        )])]);

        clear_reset_backgrounds(&mut text);

        assert_eq!(text.lines[0].spans[0].style.bg, Some(explicit));
    }
}
