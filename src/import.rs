use std::io::{self, Write};

use crate::core::model::{Thread, tmux_session_name_labeled};
use crate::core::persistence;
use crate::tmux::commands as tmux;

pub fn run() -> io::Result<()> {
    let all_sessions = tmux::list_sessions();
    let unmanaged: Vec<&String> = all_sessions
        .iter()
        .filter(|name| !name.starts_with("tws_"))
        .collect();

    if unmanaged.is_empty() {
        println!("No unmanaged tmux sessions found.");
        return Ok(());
    }

    println!(
        "Found {} unmanaged session(s): {}\n",
        unmanaged.len(),
        unmanaged
            .iter()
            .map(|s| format!("\"{}\"", s))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut threads = persistence::load()?;
    let mut modified = false;

    for session_name in &unmanaged {
        println!("── Session: \"{}\" ──", session_name);

        let thread_idx = match pick_thread(&threads)? {
            Some(idx) => idx,
            None => {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
        };

        if thread_idx >= threads.len() {
            let name = prompt("  New thread name: ")?;
            if name.is_empty() {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
            threads.push(Thread::new(&name));
            modified = true;
        }

        let label = prompt_label()?;
        if label.is_empty() {
            println!("Skipping \"{}\".\n", session_name);
            continue;
        }

        let new_name = tmux_session_name_labeled(&threads[thread_idx].name, &label);

        println!("\n  Rename: \"{}\" → \"{}\"\n", session_name, new_name);

        if confirm("  Proceed?")? {
            match tmux::rename_session(session_name, &new_name) {
                Ok(true) => println!("  Renamed successfully.\n"),
                Ok(false) => println!("  tmux rename failed.\n"),
                Err(e) => println!("  Error: {}\n", e),
            }
        } else {
            println!("  Skipped.\n");
        }
    }

    if modified {
        persistence::save(&threads)?;
        println!("State saved.");
    }

    println!("Import complete.");
    Ok(())
}

fn pick_thread(threads: &[Thread]) -> io::Result<Option<usize>> {
    println!("  Select a thread:");
    for (i, thread) in threads.iter().enumerate() {
        println!("    [{}] {}", i + 1, thread.name);
    }
    let new_idx = threads.len() + 1;
    println!("    [{}] Create new thread", new_idx);
    println!("    [s] Skip this session");

    loop {
        let input = prompt("  Choice: ")?;
        if input == "s" {
            return Ok(None);
        }
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= threads.len() {
                return Ok(Some(n - 1));
            }
            if n == new_idx {
                return Ok(Some(threads.len()));
            }
        }
        println!("  Invalid choice, try again.");
    }
}

fn prompt_label() -> io::Result<String> {
    prompt("  Session label (e.g., main, debug): ")
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{}", msg);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn confirm(msg: &str) -> io::Result<bool> {
    let input = prompt(&format!("{} [y/N] ", msg))?;
    Ok(input == "y" || input == "Y")
}
