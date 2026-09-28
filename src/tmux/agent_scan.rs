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

    let table = match list_all_processes() {
        Some(raw) => parse_processes(&raw),
        None => return Vec::new(),
    };

    match_agents(&panes, &table)
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

/// Processes from `ps`, indexed for a search down the tree from a pane.
#[derive(Default)]
struct ProcessTable {
    commands: HashMap<u32, String>,
    children: HashMap<u32, Vec<u32>>,
}

fn parse_processes(raw: &str) -> ProcessTable {
    let mut table = ProcessTable::default();
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
        // pid 0 and 1 are the kernel and init. A self-parent is bad data.
        if pid <= 1 || pid == ppid {
            continue;
        }
        table.commands.insert(pid, comm);
        table.children.entry(ppid).or_default().push(pid);
    }
    // `ps` order is not stable. Sorted children make the search deterministic.
    for kids in table.children.values_mut() {
        kids.sort_unstable();
    }
    table
}

/// Check if a command line matches a known agent.
/// `command` is the full command string from `ps -o command` (exe + args).
fn identify_agent(command: &str) -> Option<AgentType> {
    let mut tokens = command.split_whitespace();
    let exe = tokens.next()?;
    let exe_basename = exe.rsplit('/').next().unwrap_or(exe);

    // The native installer runs `.../claude/versions/<version>`, so the
    // basename is a version number and not `claude`.
    if is_native_claude_path(exe) {
        return Some(AgentType::ClaudeCode);
    }

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

fn is_native_claude_path(exe: &str) -> bool {
    let components: Vec<&str> = exe.split('/').collect();
    components
        .windows(3)
        .any(|w| w[0] == "claude" && w[1] == "versions" && !w[2].is_empty())
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

/// How far below `pane_pid` to look. Depth 0 is `pane_pid` itself. Depth 3
/// covers a wrapper chain such as `npx` -> `sh` -> `node` -> agent.
const MAX_AGENT_DEPTH: usize = 3;

/// Breadth-first search from `pane_pid`, so the shallowest agent wins.
/// The `seen` set stops a cycle in bad `ps` data.
fn find_agent(table: &ProcessTable, pane_pid: u32) -> Option<AgentType> {
    let mut seen = HashSet::new();
    let mut level = vec![pane_pid];
    for _ in 0..=MAX_AGENT_DEPTH {
        let mut next = Vec::new();
        for pid in level {
            if !seen.insert(pid) {
                continue;
            }
            if let Some(agent_type) = table.commands.get(&pid).and_then(|c| identify_agent(c)) {
                return Some(agent_type);
            }
            if let Some(kids) = table.children.get(&pid) {
                next.extend(kids);
            }
        }
        level = next;
    }
    None
}

fn match_agents(panes: &[PaneInfo], table: &ProcessTable) -> Vec<AgentSession> {
    panes
        .iter()
        .filter_map(|pane| {
            let agent_type = find_agent(table, pane.pane_pid)?;
            Some(AgentSession {
                agent_type,
                tmux_session_name: pane.session_name.clone(),
                window_index: pane.window_index,
                pane_id: pane.pane_id.clone(),
                display_name: make_display_name(pane, agent_type),
                renamed: false,
                pin_slot: None,
                status: AgentStatus::Unknown,
                status_since: 0,
            })
        })
        .collect()
}

fn find_pane_agent(panes: &[PaneInfo], table: &ProcessTable, pane_id: &str) -> Option<AgentType> {
    let pane = panes.iter().find(|p| p.pane_id == pane_id)?;
    find_agent(table, pane.pane_pid)
}

/// Unlike `scan_agents`, this deliberately skips the tws-session filter:
/// forking must work in any pane, managed by tws or not.
pub fn agent_in_pane(pane_id: &str) -> Option<AgentType> {
    let panes = parse_panes(&list_all_panes()?);
    let table = parse_processes(&list_all_processes()?);
    find_pane_agent(&panes, &table, pane_id)
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

    fn pane(pane_id: &str, pane_pid: u32) -> PaneInfo {
        PaneInfo {
            session_name: "twsr_dev".into(),
            window_index: 0,
            pane_id: pane_id.into(),
            pane_pid,
            pane_title: "".into(),
        }
    }

    fn table(rows: &[(u32, u32, &str)]) -> ProcessTable {
        let mut raw = String::from("  PID  PPID COMMAND\n");
        for (pid, ppid, command) in rows {
            raw.push_str(&format!("{pid} {ppid} {command}\n"));
        }
        parse_processes(&raw)
    }

    #[test]
    fn parse_processes_basic() {
        let raw = "  PID  PPID COMM\n  100     1 /bin/zsh\n  200   100 claude\n  300   100 vim\n";
        let table = parse_processes(raw);
        assert_eq!(table.children.get(&100), Some(&vec![200, 300]));
        assert_eq!(table.commands.get(&200).map(String::as_str), Some("claude"));
        assert_eq!(
            table.commands.get(&100).map(String::as_str),
            Some("/bin/zsh")
        );
    }

    #[test]
    fn parse_processes_skips_bad_data() {
        let raw = "  PID  PPID COMM\n    0     0 kernel_task\n    1     0 /sbin/launchd\n  100   100 claude\n  garbage\n  200   100 vim\n";
        let table = parse_processes(raw);
        assert!(!table.commands.contains_key(&0));
        assert!(!table.commands.contains_key(&1));
        assert!(!table.commands.contains_key(&100));
        assert_eq!(table.children.get(&100), Some(&vec![200]));
    }

    #[test]
    fn find_agent_at_depth_0() {
        let t = table(&[(100, 1, "claude")]);
        assert_eq!(find_agent(&t, 100), Some(AgentType::ClaudeCode));
    }

    #[test]
    fn find_agent_at_depth_1_and_2() {
        let t = table(&[
            (100, 1, "/bin/zsh"),
            (200, 100, "codex"),
            (300, 100, "npx"),
            (
                301,
                300,
                "node /opt/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js",
            ),
        ]);
        assert_eq!(find_agent(&t, 100), Some(AgentType::Codex));
        assert_eq!(find_agent(&t, 300), Some(AgentType::Pi));
    }

    #[test]
    fn find_agent_at_depth_3_but_not_4() {
        let t = table(&[
            (100, 1, "sh"),
            (200, 100, "sh"),
            (300, 200, "sh"),
            (400, 300, "claude"),
            (500, 400, "sh"),
            (600, 500, "codex"),
        ]);
        assert_eq!(find_agent(&t, 100), Some(AgentType::ClaudeCode));
        let deep = table(&[
            (100, 1, "sh"),
            (200, 100, "sh"),
            (300, 200, "sh"),
            (400, 300, "sh"),
            (500, 400, "claude"),
        ]);
        assert_eq!(find_agent(&deep, 100), None);
    }

    #[test]
    fn find_agent_prefers_shallow_agent() {
        let t = table(&[
            (100, 1, "sh"),
            (150, 100, "sh"),
            (160, 150, "codex"),
            (200, 100, "claude"),
        ]);
        // The deeper codex has the lower pid. Depth still decides.
        assert_eq!(find_agent(&t, 100), Some(AgentType::ClaudeCode));
    }

    #[test]
    fn find_agent_survives_cycle() {
        let t = table(&[(100, 200, "sh"), (200, 100, "sh")]);
        assert_eq!(find_agent(&t, 100), None);
    }

    #[test]
    fn find_agent_none_for_missing_pane_pid() {
        let t = table(&[(100, 1, "claude")]);
        assert_eq!(find_agent(&t, 999), None);
    }

    #[test]
    fn match_agents_one_agent_per_pane() {
        let panes = vec![pane("%0", 100)];
        let t = table(&[(100, 1, "sh"), (200, 100, "claude"), (201, 100, "codex")]);
        let agents = match_agents(&panes, &t);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, AgentType::ClaudeCode);
    }

    #[test]
    fn match_agents_finds_pane_process_agent() {
        let panes = vec![pane("%0", 100)];
        let t = table(&[(100, 1, "claude --resume abc")]);
        let agents = match_agents(&panes, &t);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane_id, "%0");
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
    fn identify_agent_native_installer_path() {
        assert_eq!(
            identify_agent("/Users/me/.local/share/claude/versions/2.1.284"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(
            identify_agent("/Users/me/.local/share/claude/versions/2.1.284 --resume abc"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(
            identify_agent("/Users/me/.local/share/other/versions/1.0"),
            None
        );
    }

    #[test]
    fn identify_agent_node_npm() {
        assert_eq!(
            identify_agent("node /opt/homebrew/lib/node_modules/@openai/codex/dist/cli.js"),
            Some(AgentType::Codex)
        );
        assert_eq!(
            identify_agent("node /home/user/.nvm/versions/node/v20/lib/node_modules/codex/cli.js"),
            Some(AgentType::Codex)
        );
        assert_eq!(
            identify_agent("node /opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(
            identify_agent("node /usr/lib/node_modules/claude-code/dist/cli.js"),
            Some(AgentType::ClaudeCode)
        );
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
            pane_title: "\u{2810} fix-bug".into(),
            ..pane("%0", 100)
        }];
        let t = table(&[(100, 1, "zsh"), (200, 100, "claude")]);

        let agents = match_agents(&panes, &t);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, AgentType::ClaudeCode);
        assert_eq!(agents[0].tmux_session_name, "twsr_dev");
        assert_eq!(agents[0].pane_id, "%0");
        assert_eq!(agents[0].display_name, "fix-bug");
        assert!(!agents[0].renamed);
        assert_eq!(agents[0].status, crate::core::model::AgentStatus::Unknown);
        assert_eq!(agents[0].status_since, 0);
    }

    #[test]
    fn match_agents_skips_non_agents() {
        let panes = vec![pane("%0", 100)];
        let t = table(&[(100, 1, "zsh"), (200, 100, "vim"), (201, 100, "node")]);
        assert!(match_agents(&panes, &t).is_empty());
    }

    #[test]
    fn match_agents_multiple_agents_one_session() {
        let panes = vec![
            PaneInfo {
                pane_title: "\u{2810} task-a".into(),
                ..pane("%0", 100)
            },
            PaneInfo {
                window_index: 1,
                ..pane("%1", 101)
            },
        ];
        let t = table(&[
            (100, 1, "zsh"),
            (101, 1, "zsh"),
            (200, 100, "claude"),
            (300, 101, "codex"),
        ]);

        let agents = match_agents(&panes, &t);
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
    fn clean_pane_title_codex_passthrough() {
        assert_eq!(
            clean_pane_title("codex-task", AgentType::Codex),
            "codex-task"
        );
        assert_eq!(clean_pane_title("pi-task", AgentType::Pi), "pi-task");
    }

    #[test]
    fn find_pane_agent_matches_target_pane() {
        let panes = vec![pane("%0", 100), pane("%1", 101)];
        let t = table(&[
            (100, 1, "zsh"),
            (101, 1, "zsh"),
            (200, 100, "claude"),
            (300, 101, "codex"),
        ]);

        assert_eq!(
            find_pane_agent(&panes, &t, "%0"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(find_pane_agent(&panes, &t, "%1"), Some(AgentType::Codex));
    }

    #[test]
    fn find_pane_agent_none_for_unknown_pane_or_no_agent() {
        let panes = vec![pane("%0", 100)];
        let t = table(&[(100, 1, "zsh"), (200, 100, "vim")]);

        assert_eq!(find_pane_agent(&panes, &t, "%0"), None);
        assert_eq!(find_pane_agent(&panes, &t, "%9"), None);
    }

    #[test]
    fn find_pane_agent_ignores_session_membership() {
        let panes = vec![PaneInfo {
            session_name: "not_a_tws_session".into(),
            ..pane("%5", 100)
        }];
        let t = table(&[(100, 1, "zsh"), (200, 100, "claude")]);

        assert_eq!(
            find_pane_agent(&panes, &t, "%5"),
            Some(AgentType::ClaudeCode)
        );
    }

    #[test]
    fn find_pane_agent_finds_fork_pane_process() {
        let panes = vec![pane("%7", 100)];
        let t = table(&[(100, 1, "claude --resume abc --fork-session")]);

        assert_eq!(
            find_pane_agent(&panes, &t, "%7"),
            Some(AgentType::ClaudeCode)
        );
    }
}
