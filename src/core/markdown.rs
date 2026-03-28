use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ansi_to_tui::IntoText;
use ratatui::text::{Line, Span, Text};

use super::persistence;
use crate::theme;

/// Cached markdown renderer that delegates to `glow` when available,
/// falling back to `tui-markdown` otherwise.
pub struct MarkdownRenderer {
    glow_available: bool,
    theme_path: PathBuf,
    cache: Option<RenderCache>,
}

struct RenderCache {
    source: String,
    width: u16,
    text: Text<'static>,
}

impl MarkdownRenderer {
    /// Create a new renderer, probing for `glow` on `$PATH`.
    pub fn new() -> Self {
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
                    .unwrap_or_else(|| render_fallback(markdown))
            } else {
                render_fallback(markdown)
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
}

fn render_with_glow(markdown: &str, width: u16, theme_path: &Path) -> Option<Text<'static>> {
    let mut child = Command::new("glow")
        .args(["-w", &width.to_string(), "-s", &theme_path.to_string_lossy(), "-"])
        .env("CLICOLOR_FORCE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    child
        .stdin
        .take()?
        .write_all(markdown.as_bytes())
        .ok()?;

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    output.stdout.into_text().ok()
}

/// Fallback renderer using `tui-markdown` with the tws theme.
///
/// Converts the borrowed `Text<'a>` to owned `Text<'static>` so it can be cached.
fn render_fallback(markdown: &str) -> Text<'static> {
    let options = tui_markdown::Options::new(theme::NoteStyleSheet);
    let text = tui_markdown::from_str_with_options(markdown, &options);
    to_owned_text(text)
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
