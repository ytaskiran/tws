use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::core::model::{AgentSession, AgentStatus, AgentType};

/// Pane info parsed from tmux list-panes output.
struct PaneInfo {
    session_name: String,
    window_index: u32,
    pane_id: String,
    pane_pid: u32,
    pane_title: String,
}

/// Scan all tmux panes for known AI agents (Claude Code, Codex, Pi).
/// Only scans panes belonging to the given tws-managed session names.
pub fn scan_agents(tws_sessions: &[String]) -> Vec<AgentSession> {
    if tws_sessions.is_empty() {
        return Vec::new();
    }

    let session_set: HashSet<&str> = tws_sessions.iter().map(|s| s.as_str()).collect();

    let panes = match list_all_panes() {
        Some(raw) => parse_panes(&raw),
        None => return Vec::new(),
    };

    let panes: Vec<PaneInfo> = panes
        .into_iter()
        .filter(|p| session_set.contains(p.session_name.as_str()))
        .collect();

    if panes.is_empty() {
        return Vec::new();
    }

    let children = match list_all_processes() {
        Some(raw) => parse_processes(&raw),
        None => return Vec::new(),
    };

    match_agents(&panes, &children)
}

fn list_all_panes() -> Option<String> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{window_index}\t#{pane_id}\t#{pane_pid}\t#{pane_title}",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

fn list_all_processes() -> Option<String> {
    let output = Command::new("ps")
        .args(["-e", "-ww", "-o", "pid,ppid,command"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

fn parse_panes(raw: &str) -> Vec<PaneInfo> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\t');
            let session_name = parts.next()?.to_string();
            let window_index = parts.next()?.parse::<u32>().ok()?;
            let pane_id = parts.next()?.to_string();
            let pane_pid = parts.next()?.parse::<u32>().ok()?;
            let pane_title = parts.next().unwrap_or("").to_string();
            Some(PaneInfo {
                session_name,
                window_index,
                pane_id,
                pane_pid,
                pane_title,
            })
        })
        .collect()
}

/// Build a map of parent_pid → Vec<(child_pid, command_name)>.
fn parse_processes(raw: &str) -> HashMap<u32, Vec<(u32, String)>> {
    let mut map: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        // Format: "  PID  PPID COMM" — use split_whitespace to collapse multiple spaces
        let mut parts = trimmed.split_whitespace();
        let pid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
            Some(p) => p,
            None => continue, // skips header line too (PID is not a u32)
        };
        let ppid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
            Some(p) => p,
            None => continue,
        };
        // Remaining tokens are the command (may contain spaces on macOS).
        // `ps -ww` keeps long Nix/Deno wrapper command lines from being truncated
        // before the actual agent script path appears.
        let comm: String = parts.collect::<Vec<&str>>().join(" ");
        if comm.is_empty() {
            continue;
        }
        map.entry(ppid).or_default().push((pid, comm));
    }
    map
}

/// How many lines from the bottom of the pane to inspect. The live status line, the
/// active permission prompt, and the input box all render in the bottom region; the
/// scrollback above it is conversation text that must NOT be matched (it can contain
/// the very phrases we look for — e.g. an agent discussing "Do you want" or "tokens").
const LIVE_REGION_LINES: usize = 20;

/// Infer what an agent is doing from a snapshot of its tmux pane content.
///
/// Pure and side-effect-free (the caller supplies captured pane text), so it can be
/// unit-tested without tmux. Detection priority is **Waiting → Running → Idle**: a
/// permission prompt means the task is paused for the user, so it wins over any
/// lingering spinner text.
///
/// Two properties keep this from misfiring on the agent's own transcript:
///   * only the bottom `LIVE_REGION_LINES` are examined (the live UI region), and
///   * matches are anchored to UI *structure* (a `❯ 1.` selection cursor, a spinner
///     glyph at line-start) rather than to bare phrases that also occur in prose.
///
/// Claude Code is matched precisely. Codex/Pi are best-effort on the shared "interrupt"
/// running hint and otherwise fall back to `Idle` — we default to the least-alarming
/// state rather than guess wrong.
pub fn detect_status(content: &str, _agent_type: AgentType) -> AgentStatus {
    let mut lines: Vec<&str> = content.lines().collect();
    // Drop trailing blank lines first: a pane isn't always full (e.g. a fresh Claude
    // sitting on the trust prompt renders near the top with the rest of the pane blank),
    // so a fixed tail of the raw capture can land entirely in empty space and miss the
    // active prompt.
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let start = lines.len().saturating_sub(LIVE_REGION_LINES);
    let live = &lines[start..];

    if is_waiting(live) {
        AgentStatus::Waiting
    } else if is_running(live) {
        AgentStatus::Running
    } else {
        AgentStatus::Idle
    }
}

/// True when a permission / selection prompt is awaiting the user.
///
/// Anchored to the highlighted selection cursor Claude renders for the active choice —
/// a line whose first non-space glyph is `❯` immediately followed by a numbered option,
/// e.g. `❯ 1. Yes`. This is structural: the bare input prompt (`❯ ` with no number)
/// doesn't match, and ordinary prose never contains a `❯ <n>.` cursor.
fn is_waiting(live: &[&str]) -> bool {
    live.iter().any(|line| {
        // Skip leading whitespace and an optional box border (`│`) so both bordered
        // (`│ ❯ 1. Yes │`) and borderless (`❯ 1. Yes`) prompts match.
        let trimmed = line.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{2502}');
        let rest = match trimmed.strip_prefix('\u{276f}') {
            Some(r) => r.trim_start(),
            None => return false,
        };
        let mut chars = rest.chars();
        matches!(
            (chars.next(), chars.next()),
            (Some(d), Some('.')) if d.is_ascii_digit()
        )
    })
}

/// Spinner glyphs Claude Code cycles through on its live working line — the growing
/// "sparkle": `·` `✢` `✳` `✶` `✻` `✽`. Enumerated empirically by sampling live panes; the
/// leading glyph is what distinguishes the real status line from prose that merely quotes
/// it (message lines start with the `⏺` bullet, never a spinner glyph).
const SPINNER_GLYPHS: [char; 6] = [
    '\u{00b7}', '\u{2722}', '\u{2733}', '\u{2736}', '\u{273b}', '\u{273d}',
];

/// True when the agent is actively working.
///
/// The "(esc to interrupt)" hint is absent in bypass-permissions mode and newer Claude
/// builds, where the only running signal is the live spinner line, e.g.
/// `✽ Churning… (2m 41s · ↓ 4.2k tokens)`. So the primary signal is structural: a line
/// whose first glyph is a spinner glyph and which carries the working ellipsis, token
/// counter, or interrupt hint. A completed history line (`✻ Sautéed for 9s`) starts
/// with a glyph but has none of those, so it's excluded. As a cross-agent fallback we
/// also accept the full "esc to interrupt" phrase anywhere in the live region (Codex).
fn is_running(live: &[&str]) -> bool {
    live.iter().any(|line| {
        let trimmed = line.trim_start();
        let starts_with_spinner = trimmed
            .chars()
            .next()
            .is_some_and(|c| SPINNER_GLYPHS.contains(&c));
        if starts_with_spinner
            && (trimmed.contains('\u{2026}')
                || trimmed.contains("tokens)")
                || trimmed.to_ascii_lowercase().contains("interrupt"))
        {
            return true;
        }
        trimmed.to_ascii_lowercase().contains("esc to interrupt")
    })
}

/// Check if a command line matches a known agent.
/// `command` is the full command string from `ps -o command` (exe + args).
fn identify_agent(command: &str) -> Option<AgentType> {
    let mut tokens = command.split_whitespace();
    let exe = tokens.next()?;
    let exe_basename = exe.rsplit('/').next().unwrap_or(exe);

    match exe_basename {
        "claude" => Some(AgentType::ClaudeCode),
        "codex" => Some(AgentType::Codex),
        "pi" | "pi-coding-agent" => Some(AgentType::Pi),
        // npm-installed agents run as: node /path/to/node_modules/<pkg>/cli.js
        // Nix-installed Pi runs as: deno run ... /nix/store/...-pi-coding-agent-.../dist/cli.js
        // Claude Code: @anthropic-ai/claude-code  →  path component "claude-code" or "claude"
        // Codex:       @openai/codex              →  path component "codex"
        // Pi:          @earendil-works/pi-coding-agent → path component containing "pi-coding-agent"
        "node" | "deno" => identify_agent_script(tokens),
        _ => None,
    }
}

fn identify_agent_script<'a>(tokens: impl Iterator<Item = &'a str>) -> Option<AgentType> {
    for token in tokens {
        let components: Vec<&str> = token.split('/').collect();
        if components.contains(&"codex") {
            return Some(AgentType::Codex);
        }
        if components
            .iter()
            .any(|&c| c == "claude" || c == "claude-code")
        {
            return Some(AgentType::ClaudeCode);
        }
        if components
            .iter()
            .any(|&c| c == "pi" || c == "pi-coding-agent")
        {
            return Some(AgentType::Pi);
        }
    }
    None
}

/// Strip agent-specific prefixes from pane titles to get a clean display name.
fn clean_pane_title(title: &str, agent_type: AgentType) -> String {
    let trimmed = title.trim();
    match agent_type {
        AgentType::ClaudeCode => {
            // Claude Code uses braille dots (U+2800..U+28FF) as spinner indicators,
            // and prefixes titles with ✳ (U+2733, eight spoked asterisk) as its logo.
            let s = trimmed.trim_start_matches(|c: char| {
                c.is_whitespace() || ('\u{2800}'..='\u{28ff}').contains(&c)
            });
            let s = s.strip_prefix('\u{2733}').unwrap_or(s).trim_start();
            s.to_string()
        }
        AgentType::Codex | AgentType::Pi => trimmed.to_string(),
    }
}

fn make_display_name(pane: &PaneInfo, agent_type: AgentType) -> String {
    let cleaned = clean_pane_title(&pane.pane_title, agent_type);
    if cleaned.is_empty() {
        format!("{} (w:{})", agent_type.display_name(), pane.window_index)
    } else {
        cleaned
    }
}

fn match_agents(
    panes: &[PaneInfo],
    children: &HashMap<u32, Vec<(u32, String)>>,
) -> Vec<AgentSession> {
    let mut agents = Vec::new();
    for pane in panes {
        if let Some(kids) = children.get(&pane.pane_pid) {
            for (_pid, comm) in kids {
                if let Some(agent_type) = identify_agent(comm) {
                    let display_name = make_display_name(pane, agent_type);
                    agents.push(AgentSession {
                        agent_type,
                        tmux_session_name: pane.session_name.clone(),
                        window_index: pane.window_index,
                        pane_id: pane.pane_id.clone(),
                        display_name,
                        renamed: false,
                        pin_slot: None,
                        // Filled in by the caller after capturing pane content;
                        // Idle is the safe default until then.
                        status: AgentStatus::Idle,
                    });
                }
            }
        }
    }
    agents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_panes_basic() {
        let raw = "twsr_dev\t0\t%0\t12345\tsome title\ntwsr_dev\t1\t%1\t12346\t\n";
        let panes = parse_panes(raw);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].session_name, "twsr_dev");
        assert_eq!(panes[0].window_index, 0);
        assert_eq!(panes[0].pane_id, "%0");
        assert_eq!(panes[0].pane_pid, 12345);
        assert_eq!(panes[0].pane_title, "some title");
        assert_eq!(panes[1].window_index, 1);
        assert_eq!(panes[1].pane_title, "");
    }

    #[test]
    fn parse_processes_basic() {
        let raw = "  PID  PPID COMM\n  100     1 /bin/zsh\n  200   100 claude\n  300   100 vim\n";
        let map = parse_processes(raw);
        let kids = map.get(&100).unwrap();
        assert_eq!(kids.len(), 2);
        assert!(
            kids.iter()
                .any(|(pid, comm)| *pid == 200 && comm == "claude")
        );
    }

    #[test]
    fn identify_agent_basename() {
        assert_eq!(identify_agent("claude"), Some(AgentType::ClaudeCode));
        assert_eq!(
            identify_agent("/usr/local/bin/claude"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(identify_agent("codex"), Some(AgentType::Codex));
        assert_eq!(
            identify_agent("/opt/homebrew/bin/codex"),
            Some(AgentType::Codex)
        );
        assert_eq!(identify_agent("pi"), Some(AgentType::Pi));
        assert_eq!(identify_agent("pi-coding-agent"), Some(AgentType::Pi));
        assert_eq!(
            identify_agent("/nix/store/hash-pi-coding-agent-0.78.0/bin/pi"),
            Some(AgentType::Pi)
        );
        assert_eq!(identify_agent("vim"), None);
        assert_eq!(identify_agent("node"), None);
    }

    #[test]
    fn identify_agent_node_npm() {
        // npm-installed codex
        assert_eq!(
            identify_agent("node /opt/homebrew/lib/node_modules/@openai/codex/dist/cli.js"),
            Some(AgentType::Codex)
        );
        assert_eq!(
            identify_agent("node /home/user/.nvm/versions/node/v20/lib/node_modules/codex/cli.js"),
            Some(AgentType::Codex)
        );
        // npm-installed Claude Code (@anthropic-ai/claude-code)
        assert_eq!(
            identify_agent("node /opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(
            identify_agent("node /usr/lib/node_modules/claude-code/dist/cli.js"),
            Some(AgentType::ClaudeCode)
        );
        // npm-installed Pi coding agent
        assert_eq!(
            identify_agent(
                "node /usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js"
            ),
            Some(AgentType::Pi)
        );
        assert_eq!(
            identify_agent(
                "node /home/user/.nvm/versions/node/v20/lib/node_modules/pi-coding-agent/dist/index.js"
            ),
            Some(AgentType::Pi)
        );
        assert_eq!(
            identify_agent(
                "deno run --allow-all /nix/store/hash-pi-coding-agent-0.78.0/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js"
            ),
            Some(AgentType::Pi)
        );
        // node running something unrelated — should not match
        assert_eq!(identify_agent("node /path/to/my-app/index.js"), None);
        assert_eq!(
            identify_agent("node /path/to/codex-tutorial/index.js"),
            None
        );
        assert_eq!(identify_agent("node"), None);
    }

    #[test]
    fn match_agents_finds_claude() {
        let panes = vec![PaneInfo {
            session_name: "twsr_dev".into(),
            window_index: 0,
            pane_id: "%0".into(),
            pane_pid: 100,
            pane_title: "\u{2810} fix-bug".into(),
        }];
        let mut children = HashMap::new();
        children.insert(100, vec![(200, "claude".into())]);

        let agents = match_agents(&panes, &children);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, AgentType::ClaudeCode);
        assert_eq!(agents[0].tmux_session_name, "twsr_dev");
        assert_eq!(agents[0].pane_id, "%0");
        assert_eq!(agents[0].display_name, "fix-bug");
        assert!(!agents[0].renamed);
    }

    #[test]
    fn match_agents_skips_non_agents() {
        let panes = vec![PaneInfo {
            session_name: "twsr_dev".into(),
            window_index: 0,
            pane_id: "%0".into(),
            pane_pid: 100,
            pane_title: "".into(),
        }];
        let mut children = HashMap::new();
        children.insert(100, vec![(200, "vim".into()), (201, "node".into())]);

        let agents = match_agents(&panes, &children);
        assert!(agents.is_empty());
    }

    #[test]
    fn match_agents_multiple_agents_one_session() {
        let panes = vec![
            PaneInfo {
                session_name: "tws_work_proj".into(),
                window_index: 0,
                pane_id: "%0".into(),
                pane_pid: 100,
                pane_title: "\u{2810} task-a".into(),
            },
            PaneInfo {
                session_name: "tws_work_proj".into(),
                window_index: 1,
                pane_id: "%1".into(),
                pane_pid: 101,
                pane_title: "".into(),
            },
        ];
        let mut children = HashMap::new();
        children.insert(100, vec![(200, "claude".into())]);
        children.insert(101, vec![(300, "codex".into())]);

        let agents = match_agents(&panes, &children);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].agent_type, AgentType::ClaudeCode);
        assert_eq!(agents[0].display_name, "task-a");
        assert_eq!(agents[1].agent_type, AgentType::Codex);
        assert_eq!(agents[1].display_name, "Codex (w:1)"); // fallback: empty title
    }

    #[test]
    fn clean_pane_title_strips_braille() {
        assert_eq!(
            clean_pane_title("\u{2810} fix-bug", AgentType::ClaudeCode),
            "fix-bug"
        );
        assert_eq!(
            clean_pane_title("\u{2812}\u{2812} task", AgentType::ClaudeCode),
            "task"
        );
        assert_eq!(
            clean_pane_title("plain title", AgentType::ClaudeCode),
            "plain title"
        );
        assert_eq!(
            clean_pane_title("\u{2733} fix-bug", AgentType::ClaudeCode),
            "fix-bug"
        );
        assert_eq!(
            clean_pane_title("\u{2733} task with spaces", AgentType::ClaudeCode),
            "task with spaces"
        );
        assert_eq!(clean_pane_title("", AgentType::ClaudeCode), "");
    }

    #[test]
    fn detect_status_running_from_interrupt() {
        let content = "✻ Herding… (12s · ↑ 1.2k tokens · esc to interrupt)";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Running
        );
        // Codex capitalizes it differently.
        assert_eq!(
            detect_status("Working  Esc to interrupt", AgentType::Codex),
            AgentStatus::Running
        );
    }

    #[test]
    fn detect_status_running_bypass_mode_no_interrupt_hint() {
        // Real captured line from a Claude Code pane in bypass-permissions mode:
        // there is NO "esc to interrupt" text — the live spinner + token counter is
        // the only running signal.
        let content = "\
⏺ Running 1 shell command…

✽ Churning… (2m 41s · ↓ 4.2k tokens)

  Opus 4.8 | tws ⎇main | ctx: 13% | tokens: 132k
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Running
        );
    }

    #[test]
    fn detect_status_running_across_spinner_cycle() {
        // The spinner animates through a cycle of glyphs; every frame must read Running.
        // The middot frame (`·`) is easy to miss and was a real detection gap.
        for glyph in [
            '\u{00b7}', '\u{2722}', '\u{2733}', '\u{2736}', '\u{273b}', '\u{273d}',
        ] {
            let content = format!("{glyph} Churning… (10m 28s · ↓ 33.0k tokens)");
            assert_eq!(
                detect_status(&content, AgentType::ClaudeCode),
                AgentStatus::Running,
                "glyph U+{:04X} should read Running",
                glyph as u32
            );
        }
    }

    #[test]
    fn detect_status_running_spinner_before_token_counter() {
        // First moment of a turn: spinner + ellipsis, counter not yet shown.
        assert_eq!(
            detect_status("✻ Herding…", AgentType::ClaudeCode),
            AgentStatus::Running
        );
    }

    #[test]
    fn detect_status_footer_tokens_is_not_running() {
        // The context footer shows "tokens:" (colon), which must NOT read as running,
        // and a completed history line starts with a spinner glyph but has no ellipsis.
        let content = "\
⏺ All done.
✻ Sautéed for 9s

  Opus 4.8 | tws ⎇main | ctx: 13% | tokens: 132k
❯ ";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Idle
        );
    }

    #[test]
    fn detect_status_waiting_from_permission_prompt() {
        let content = "\
╭──────────────────────────────────────╮
│ Do you want to make this edit?       │
│ ❯ 1. Yes                             │
│   2. No, tell Claude what to do      │
╰──────────────────────────────────────╯";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Waiting
        );
    }

    #[test]
    fn detect_status_waiting_wins_over_running() {
        // A stale spinner line above a permission prompt must not mask the prompt.
        let content = "\
✽ Churning… (2m 41s · ↓ 4.2k tokens)
╭──────────────────────────────────────╮
│ Do you want to proceed?              │
│ ❯ 1. Yes                             │
│   2. No                              │
╰──────────────────────────────────────╯";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Waiting
        );
    }

    #[test]
    fn detect_status_waiting_trust_prompt_with_trailing_blanks() {
        // Real first-launch trust prompt: rendered near the top with the rest of the
        // pane blank. The `❯ 1.` cursor must be found despite ~dozens of trailing empty
        // lines below it (regression: a fixed tail landed entirely in the blank region).
        let mut content = String::from(
            "\
 Quick safety check: Is this a project you created or one you trust?

 ❯ 1. Yes, I trust this folder
   2. No, exit

 Enter to confirm · Esc to cancel",
        );
        for _ in 0..60 {
            content.push('\n');
        }
        assert_eq!(
            detect_status(&content, AgentType::ClaudeCode),
            AgentStatus::Waiting
        );
    }

    #[test]
    fn detect_status_ignores_transcript_prose() {
        // Regression: an IDLE agent whose visible scrollback happens to contain the
        // trigger phrases (an agent literally discussing this detection code) must not
        // read as Waiting or Running. Only the live region at the bottom counts, and
        // matches are anchored to UI structure — prose has no `❯ 1.` cursor and no
        // spinner-glyph status line.
        let content = "\
⏺ The naive version matched \"Do you want\" and \"tokens)\" anywhere, e.g.:
     1. it flagged Waiting from my own message text
     2. it flagged Running from a stray (… tokens) in prose
  So we anchor structurally instead.

────────────────────────────────────────────────────────── agent-status ──
❯
────────────────────────────────────────────────────────────────────────────
  Opus 4.8 | tws ⎇main | ctx: 13% | tokens: 132k
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Idle
        );
    }

    #[test]
    fn detect_status_idle_at_prompt() {
        let content = "\
╭──────────────────────────────────────╮
│ > │
╰──────────────────────────────────────╯
  ? for shortcuts";
        assert_eq!(
            detect_status(content, AgentType::ClaudeCode),
            AgentStatus::Idle
        );
        assert_eq!(detect_status("", AgentType::ClaudeCode), AgentStatus::Idle);
    }

    #[test]
    fn clean_pane_title_codex_passthrough() {
        assert_eq!(
            clean_pane_title("codex-task", AgentType::Codex),
            "codex-task"
        );
        assert_eq!(clean_pane_title("pi-task", AgentType::Pi), "pi-task");
    }
}
