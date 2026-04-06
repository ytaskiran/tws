use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::core::model::{AgentSession, AgentType};

/// Pane info parsed from tmux list-panes output.
struct PaneInfo {
    session_name: String,
    window_index: u32,
    pane_id: String,
    pane_pid: u32,
    pane_title: String,
}

/// Scan all tmux panes for known AI agents (Claude Code, Codex).
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
        .args(["-e", "-o", "pid,ppid,command"])
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
        // Remaining tokens are the command (may contain spaces on macOS)
        let comm: String = parts.collect::<Vec<&str>>().join(" ");
        if comm.is_empty() {
            continue;
        }
        map.entry(ppid).or_default().push((pid, comm));
    }
    map
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
        // npm-installed agents run as: node /path/to/node_modules/<pkg>/cli.js
        // Claude Code: @anthropic-ai/claude-code  →  path component "claude-code" or "claude"
        // Codex:       @openai/codex              →  path component "codex"
        "node" => tokens
            .find(|t| !t.starts_with('-'))
            .and_then(|script| {
                let components: Vec<&str> = script.split('/').collect();
                if components.iter().any(|&c| c == "codex") {
                    Some(AgentType::Codex)
                } else if components.iter().any(|&c| c == "claude" || c == "claude-code") {
                    Some(AgentType::ClaudeCode)
                } else {
                    None
                }
            }),
        _ => None,
    }
}

/// Strip agent-specific prefixes from pane titles to get a clean display name.
fn clean_pane_title(title: &str, agent_type: AgentType) -> String {
    let trimmed = title.trim();
    match agent_type {
        AgentType::ClaudeCode => {
            // Claude Code uses braille dots (U+2800..U+28FF) as spinner indicators,
            // and prefixes titles with ✳ (U+2733, eight spoked asterisk) as its logo.
            let s = trimmed
                .trim_start_matches(|c: char| c.is_whitespace() || ('\u{2800}'..='\u{28ff}').contains(&c));
            let s = s.strip_prefix('\u{2733}').unwrap_or(s).trim_start();
            s.to_string()
        }
        AgentType::Codex => trimmed.to_string(),
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
                        pane_title: pane.pane_title.clone(),
                        display_name,
                        renamed: false,
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
        assert!(kids.iter().any(|(pid, comm)| *pid == 200 && comm == "claude"));
    }

    #[test]
    fn identify_agent_basename() {
        assert_eq!(identify_agent("claude"), Some(AgentType::ClaudeCode));
        assert_eq!(identify_agent("/usr/local/bin/claude"), Some(AgentType::ClaudeCode));
        assert_eq!(identify_agent("codex"), Some(AgentType::Codex));
        assert_eq!(identify_agent("/opt/homebrew/bin/codex"), Some(AgentType::Codex));
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
        // node running something unrelated — should not match
        assert_eq!(identify_agent("node /path/to/my-app/index.js"), None);
        assert_eq!(identify_agent("node /path/to/codex-tutorial/index.js"), None);
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
            PaneInfo { session_name: "tws_work_proj".into(), window_index: 0, pane_id: "%0".into(), pane_pid: 100, pane_title: "\u{2810} task-a".into() },
            PaneInfo { session_name: "tws_work_proj".into(), window_index: 1, pane_id: "%1".into(), pane_pid: 101, pane_title: "".into() },
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
        assert_eq!(clean_pane_title("\u{2810} fix-bug", AgentType::ClaudeCode), "fix-bug");
        assert_eq!(clean_pane_title("\u{2812}\u{2812} task", AgentType::ClaudeCode), "task");
        assert_eq!(clean_pane_title("plain title", AgentType::ClaudeCode), "plain title");
        assert_eq!(clean_pane_title("\u{2733} fix-bug", AgentType::ClaudeCode), "fix-bug");
        assert_eq!(clean_pane_title("\u{2733} task with spaces", AgentType::ClaudeCode), "task with spaces");
        assert_eq!(clean_pane_title("", AgentType::ClaudeCode), "");
    }

    #[test]
    fn clean_pane_title_codex_passthrough() {
        assert_eq!(clean_pane_title("codex-task", AgentType::Codex), "codex-task");
    }
}
