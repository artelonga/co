//! `co tool` — git-backed tool registry (CO-331).
//!
//! Manages installable tools as versioned git checkouts:
//!   co tool add <key> --from <url-or-path> [--pin <ref>]
//!   co tool list
//!   co tool update <key> [--pin <ref>] [--follow-main]
//!   co tool update --all
//!   co tool remove <key>
//!   co tool verify [<key>]

use co_web::storage::{Storage, ToolRecord};
use colored::Colorize;
use comfy_table::{Cell, Table};
use std::path::{Path, PathBuf};

use crate::vcs;

// ---------------------------------------------------------------------------
// Action enum (mirrors CLI surface)
// ---------------------------------------------------------------------------

pub enum ToolAction {
    Add {
        key: String,
        from: String,
        pin: Option<String>,
        data_dir: String,
    },
    List {
        data_dir: String,
    },
    Update {
        key: Option<String>,
        pin: Option<String>,
        follow_main: bool,
        all: bool,
        data_dir: String,
    },
    Remove {
        key: String,
        data_dir: String,
    },
    Verify {
        key: Option<String>,
        data_dir: String,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(action: ToolAction) {
    match action {
        ToolAction::Add {
            key,
            from,
            pin,
            data_dir,
        } => add(&key, &from, pin.as_deref(), &data_dir),
        ToolAction::List { data_dir } => list(&data_dir),
        ToolAction::Update {
            key,
            pin,
            follow_main,
            all,
            data_dir,
        } => {
            if all {
                update_all(&data_dir);
            } else if let Some(k) = key {
                update(&k, pin.as_deref(), follow_main, &data_dir);
            } else {
                eprintln!("{} Specify a tool key or --all", "error:".red().bold());
                std::process::exit(1);
            }
        }
        ToolAction::Remove { key, data_dir } => remove(&key, &data_dir),
        ToolAction::Verify { key, data_dir } => verify(key.as_deref(), &data_dir),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the checkout directory for a tool inside the CO data dir.
fn checkout_dir(data_dir: &str, key: &str) -> PathBuf {
    PathBuf::from(data_dir).join("tools").join(key)
}

/// Determine whether `from` is a local path or a remote URL.
fn is_local(from: &str) -> bool {
    from.starts_with('/')
        || from.starts_with("~/")
        || from.starts_with("./")
        || from.starts_with("../")
        || from == "."
        || from == ".."
}

/// Expand a `~`-prefixed path to an absolute path.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    path.to_string()
}

fn open_storage(data_dir: &str) -> Storage {
    Storage::new(data_dir)
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

fn add(key: &str, from: &str, pin: Option<&str>, data_dir: &str) {
    let mut storage = open_storage(data_dir);

    if storage.tool_get(key).is_some() {
        eprintln!(
            "{} Tool '{}' is already registered",
            "error:".red().bold(),
            key
        );
        std::process::exit(1);
    }

    let (remote_url, local_path, effective_path) = if is_local(from) {
        let expanded = expand_tilde(from);
        let abs = PathBuf::from(&expanded);
        if !abs.exists() {
            eprintln!(
                "{} Local path '{}' does not exist",
                "error:".red().bold(),
                expanded
            );
            std::process::exit(1);
        }
        (None, Some(expanded.clone()), abs)
    } else {
        let dest = checkout_dir(data_dir, key);
        if !dest.exists() {
            eprintln!("Cloning {} …", from.cyan());
            if let Err(e) = vcs::clone_repo(from, &dest) {
                eprintln!("{} Clone failed: {}", "error:".red().bold(), e);
                std::process::exit(1);
            }
        }
        if let Some(ref_name) = pin
            && let Err(e) = vcs::checkout(&dest, ref_name)
        {
            eprintln!("{} Checkout failed: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
        (Some(from.to_string()), None, dest)
    };

    // Capture lockfile SHA (best-effort — local paths may not be a git repo).
    let lockfile_sha = vcs::head_sha(&effective_path).ok();

    // Warn about entry_command conflicts (name collisions).
    let entry_cmd = derive_entry_command(key, &effective_path);
    if let Some(ref cmd) = entry_cmd {
        let conflicts = storage.tool_entry_command_conflicts(cmd, key);
        if !conflicts.is_empty() {
            eprintln!(
                "{} entry_command '{}' already used by: {}",
                "warning:".yellow().bold(),
                cmd,
                conflicts.join(", ")
            );
        }
    }

    let record = ToolRecord {
        key: key.to_string(),
        name: key.to_string(),
        description: None,
        remote_url,
        local_path,
        version_pin: pin.map(str::to_string),
        entry_command: entry_cmd,
        installed_at: None,
        last_updated: None,
        follow_main: false,
        lockfile_sha,
    };

    if let Err(e) = storage.tool_add(&record) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Tool '{}' registered (pin: {})",
        "".green(),
        key.cyan(),
        pin.unwrap_or("none")
    );
}

fn list(data_dir: &str) {
    let storage = open_storage(data_dir);
    let tools = storage.tool_list();

    if tools.is_empty() {
        println!("{}", "No tools registered.".yellow());
        println!(
            "{}",
            "Use 'co tool add <key> --from <url-or-path>' to install one.".dimmed()
        );
        return;
    }

    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("Key").fg(comfy_table::Color::Cyan),
        Cell::new("Pin").fg(comfy_table::Color::Cyan),
        Cell::new("Follows main").fg(comfy_table::Color::Cyan),
        Cell::new("Source").fg(comfy_table::Color::Cyan),
        Cell::new("SHA").fg(comfy_table::Color::Cyan),
    ]);

    for t in &tools {
        let source = t
            .local_path
            .as_deref()
            .or(t.remote_url.as_deref())
            .unwrap_or("-");
        let pin = t.version_pin.as_deref().unwrap_or("-");
        let sha_short = t
            .lockfile_sha
            .as_deref()
            .map(|s| &s[..s.len().min(8)])
            .unwrap_or("-");
        let follows = if t.follow_main { "yes" } else { "no" };
        table.add_row(vec![
            Cell::new(&t.key).fg(comfy_table::Color::White),
            Cell::new(pin),
            Cell::new(follows).fg(if t.follow_main {
                comfy_table::Color::Yellow
            } else {
                comfy_table::Color::DarkGrey
            }),
            Cell::new(source),
            Cell::new(sha_short).fg(comfy_table::Color::DarkGrey),
        ]);
    }

    println!("{table}");
    println!(
        "\n{} {}",
        tools.len().to_string().cyan(),
        if tools.len() == 1 { "tool" } else { "tools" }
    );
}

fn update(key: &str, pin: Option<&str>, follow_main: bool, data_dir: &str) {
    let mut storage = open_storage(data_dir);

    let record = match storage.tool_get(key) {
        Some(r) => r,
        None => {
            eprintln!("{} Tool '{}' not found", "error:".red().bold(), key);
            std::process::exit(1);
        }
    };

    let effective_path = resolve_effective_path(&record, data_dir, key);
    do_update(&effective_path, pin, &mut storage, key, follow_main);
}

fn update_all(data_dir: &str) {
    let mut storage = open_storage(data_dir);
    let tools = storage.tool_list_follow_main();

    if tools.is_empty() {
        println!("{}", "No follow-main tools to update.".yellow());
        return;
    }

    for t in tools {
        let effective_path = resolve_effective_path(&t, data_dir, &t.key);
        eprintln!("Updating {} …", t.key.cyan());
        do_update(&effective_path, None, &mut storage, &t.key, true);
    }
}

fn do_update(path: &Path, pin: Option<&str>, storage: &mut Storage, key: &str, follow_main: bool) {
    // Fetch latest from remote (best-effort for local-only tools).
    if let Err(e) = vcs::fetch(path) {
        eprintln!(
            "{} fetch failed for '{}': {}",
            "warning:".yellow().bold(),
            key,
            e
        );
    }

    let ref_to_checkout = if follow_main {
        Some("origin/main")
    } else {
        pin
    };

    if let Some(r) = ref_to_checkout
        && let Err(e) = vcs::checkout(path, r)
    {
        eprintln!("{} checkout '{}' failed: {}", "error:".red().bold(), r, e);
        std::process::exit(1);
    }

    let sha = vcs::head_sha(path).ok();
    let new_follow = if follow_main { Some(true) } else { None };

    if let Err(e) = storage.tool_update_pin(key, pin, new_follow, sha.as_deref()) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} '{}' updated (SHA: {})",
        "".green(),
        key.cyan(),
        sha.as_deref().unwrap_or("unknown")
    );
}

fn remove(key: &str, data_dir: &str) {
    let mut storage = open_storage(data_dir);

    let record = match storage.tool_get(key) {
        Some(r) => r,
        None => {
            eprintln!("{} Tool '{}' not found", "error:".red().bold(), key);
            std::process::exit(1);
        }
    };

    // Delete cloned checkout (only if it's inside the CO data dir, not a user-specified local path).
    if record.local_path.is_none() {
        let dest = checkout_dir(data_dir, key);
        if dest.exists()
            && let Err(e) = std::fs::remove_dir_all(&dest)
        {
            eprintln!(
                "{} Could not remove checkout at {}: {}",
                "warning:".yellow().bold(),
                dest.display(),
                e
            );
        }
    }

    if let Err(e) = storage.tool_remove(key) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!("{} Tool '{}' removed", "".green(), key.cyan());
}

fn verify(key: Option<&str>, data_dir: &str) {
    let storage = open_storage(data_dir);
    let tools = match key {
        Some(k) => match storage.tool_get(k) {
            Some(t) => vec![t],
            None => {
                eprintln!("{} Tool '{}' not found", "error:".red().bold(), k);
                std::process::exit(1);
            }
        },
        None => storage.tool_list(),
    };

    if tools.is_empty() {
        println!("{}", "No tools to verify.".yellow());
        return;
    }

    let mut all_ok = true;
    for t in &tools {
        let effective_path = resolve_effective_path(t, data_dir, &t.key);
        match (vcs::head_sha(&effective_path), &t.lockfile_sha) {
            (Ok(current), Some(locked))
                if current.starts_with(locked.as_str()) || locked.starts_with(current.as_str()) =>
            {
                println!(
                    "{} {} ({})",
                    "".green(),
                    t.key.cyan(),
                    &current[..current.len().min(8)]
                );
            }
            (Ok(current), Some(locked)) => {
                println!(
                    "{} {} — lockfile {} ≠ current {}",
                    "drift:".yellow().bold(),
                    t.key.cyan(),
                    &locked[..locked.len().min(8)],
                    &current[..current.len().min(8)]
                );
                all_ok = false;
            }
            (Ok(current), None) => {
                println!(
                    "{} {} — no lockfile recorded (current: {})",
                    "?".yellow(),
                    t.key.cyan(),
                    &current[..current.len().min(8)]
                );
            }
            (Err(e), _) => {
                println!("{} {} — could not read SHA: {}", "".red(), t.key.cyan(), e);
                all_ok = false;
            }
        }
    }

    if !all_ok {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn resolve_effective_path(record: &ToolRecord, data_dir: &str, key: &str) -> PathBuf {
    if let Some(lp) = &record.local_path {
        PathBuf::from(expand_tilde(lp))
    } else {
        checkout_dir(data_dir, key)
    }
}

/// Derive an entry_command from the tool key or a conventional file in the checkout.
/// Falls back to the key itself (which is a reasonable default for CLI tools).
fn derive_entry_command(key: &str, _checkout: &Path) -> Option<String> {
    Some(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_detects_absolute() {
        assert!(is_local("/usr/local/bin/tool"));
    }

    #[test]
    fn is_local_detects_tilde() {
        assert!(is_local("~/projects/mytool"));
    }

    #[test]
    fn is_local_detects_relative() {
        assert!(is_local("./tool"));
        assert!(is_local("../tool"));
    }

    #[test]
    fn is_remote_for_https() {
        assert!(!is_local("https://github.com/example/tool"));
    }

    #[test]
    fn is_remote_for_ssh() {
        assert!(!is_local("git@github.com:example/tool.git"));
    }

    #[test]
    fn expand_tilde_replaces_home() {
        let expanded = expand_tilde("~/projects/tool");
        assert!(!expanded.starts_with('~'));
    }

    #[test]
    fn expand_tilde_passthrough_absolute() {
        assert_eq!(expand_tilde("/usr/local/bin"), "/usr/local/bin");
    }

    #[test]
    fn checkout_dir_under_data() {
        let p = checkout_dir("/data/co", "claude-code");
        assert_eq!(p, PathBuf::from("/data/co/tools/claude-code"));
    }
}
