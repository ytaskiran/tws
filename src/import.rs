use std::io::{self, Write};

use crate::core::model::{Collection, Thread, tmux_session_name_labeled};
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

    let mut collections = persistence::load()?;
    let mut modified = false;

    for session_name in &unmanaged {
        println!("── Session: \"{}\" ──", session_name);

        let col_idx = match pick_collection(&collections)? {
            Some(idx) => idx,
            None => {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
        };

        // If pick_collection returned an index beyond current length, a new one was created
        if col_idx >= collections.len() {
            let name = prompt("  New collection name: ")?;
            if name.is_empty() {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
            collections.push(Collection::new(&name));
            modified = true;
        }

        let thread_idx = match pick_thread(&collections[col_idx])? {
            Some(idx) => idx,
            None => {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
        };

        if thread_idx >= collections[col_idx].threads.len() {
            let name = prompt("  New thread name: ")?;
            if name.is_empty() {
                println!("Skipping \"{}\".\n", session_name);
                continue;
            }
            collections[col_idx].threads.push(Thread::new(&name));
            modified = true;
        }

        let label = prompt_label()?;
        if label.is_empty() {
            println!("Skipping \"{}\".\n", session_name);
            continue;
        }

        let col_name = &collections[col_idx].name;
        let thread_name = &collections[col_idx].threads[thread_idx].name;
        let new_name = tmux_session_name_labeled(col_name, thread_name, &label);

        println!(
            "\n  Rename: \"{}\" → \"{}\"\n",
            session_name, new_name
        );

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
        persistence::save(&collections)?;
        println!("State saved.");
    }

    println!("Import complete.");
    Ok(())
}

fn pick_collection(collections: &[Collection]) -> io::Result<Option<usize>> {
    println!("  Select a collection:");
    for (i, col) in collections.iter().enumerate() {
        println!("    [{}] {}", i + 1, col.name);
    }
    let new_idx = collections.len() + 1;
    println!("    [{}] Create new collection", new_idx);
    println!("    [s] Skip this session");

    loop {
        let input = prompt("  Choice: ")?;
        if input == "s" {
            return Ok(None);
        }
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= collections.len() {
                return Ok(Some(n - 1));
            }
            if n == new_idx {
                return Ok(Some(collections.len()));
            }
        }
        println!("  Invalid choice, try again.");
    }
}

fn pick_thread(collection: &Collection) -> io::Result<Option<usize>> {
    println!("  Select a thread in \"{}\":", collection.name);
    for (i, thread) in collection.threads.iter().enumerate() {
        println!("    [{}] {}", i + 1, thread.name);
    }
    let new_idx = collection.threads.len() + 1;
    println!("    [{}] Create new thread", new_idx);
    println!("    [s] Skip this session");

    loop {
        let input = prompt("  Choice: ")?;
        if input == "s" {
            return Ok(None);
        }
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= collection.threads.len() {
                return Ok(Some(n - 1));
            }
            if n == new_idx {
                return Ok(Some(collection.threads.len()));
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
