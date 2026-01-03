//! Lead command - interactive exploration mode
//!
//! Provides an interactive shell with:
//! - Scope indicator in prompt: `co [private]>`
//! - Command history with up/down arrows
//! - Scope switching with `use <scope>`
//! - Direct command execution

use crate::commands;
use colored::Colorize;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Run the interactive lead session
pub fn run() {
    println!("{}", "CO Interactive Mode".bold().green());
    println!("Type 'help' for commands, 'exit' to quit\n");

    // Detect initial scope from current directory
    let mut current_scope = detect_default_scope();

    // Check if stdin is a tty (interactive) or piped (non-interactive)
    if atty::isnt(atty::Stream::Stdin) {
        // Non-interactive mode: use basic stdin for tests/scripts
        run_basic(&mut current_scope);
    } else {
        // Interactive mode: use rustyline with history
        run_interactive(&mut current_scope);
    }

    println!("{}", "Goodbye!".green());
}

/// Run in basic mode (for piped stdin, tests, scripts)
fn run_basic(current_scope: &mut String) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("co [{}]> ", current_scope.cyan());
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match parse_and_execute(trimmed, current_scope) {
                    CommandResult::Continue => continue,
                    CommandResult::Exit => break,
                }
            }
            Err(_) => break,
        }
    }
}

/// Run in interactive mode (with rustyline and history)
fn run_interactive(current_scope: &mut String) {
    // Initialize rustyline editor with history
    let mut rl = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(e) => {
            eprintln!(
                "{} Failed to initialize readline: {}",
                "error:".red().bold(),
                e
            );
            return;
        }
    };

    // Load history from file
    let history_path = get_history_path();
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let prompt = format!("co [{}]> ", current_scope.cyan());

        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(trimmed);

                // Parse and execute command
                match parse_and_execute(trimmed, current_scope) {
                    CommandResult::Continue => continue,
                    CommandResult::Exit => break,
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Use 'exit' or 'quit' to exit");
                continue;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                eprintln!("{} {:?}", "error:".red().bold(), err);
                break;
            }
        }
    }

    // Save history
    if let Some(path) = history_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.save_history(&path);
    }
}

/// Get the history file path
fn get_history_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|p| p.join("co").join("history.txt"))
}

/// Detect default scope from current directory or existing scopes
fn detect_default_scope() -> String {
    // Check for existing scopes in current directory
    let scopes = ["en", "private", "pt"];
    for scope in scopes {
        if Path::new(scope).exists() {
            return scope.to_string();
        }
    }

    // Default to "en"
    "en".to_string()
}

/// Result of command execution
enum CommandResult {
    Continue,
    Exit,
}

/// Parse and execute a command line
fn parse_and_execute(line: &str, current_scope: &mut String) -> CommandResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return CommandResult::Continue;
    }

    let command = parts[0].to_lowercase();
    let args = &parts[1..];

    match command.as_str() {
        // Exit commands
        "exit" | "quit" | "q" => CommandResult::Exit,

        // Help command
        "help" | "?" => {
            print_help();
            CommandResult::Continue
        }

        // Scope switching
        "use" => {
            if args.is_empty() {
                println!("{} Usage: use <scope>", "error:".red().bold());
            } else {
                let new_scope = args[0];
                if Path::new(new_scope).exists() {
                    *current_scope = new_scope.to_string();
                    println!("Switched to scope: {}", new_scope.cyan());
                } else {
                    println!("{} Scope '{}' not found", "error:".red().bold(), new_scope);
                }
            }
            CommandResult::Continue
        }

        // Status command
        "status" => {
            commands::status::run();
            CommandResult::Continue
        }

        // List command
        "list" => {
            let stats = args.contains(&"--stats") || args.contains(&"-s");
            commands::list::run(stats);
            CommandResult::Continue
        }

        // Locate command
        "locate" => {
            if args.is_empty() {
                println!("{} Usage: locate <query>", "error:".red().bold());
            } else {
                // Check for subcommands
                match args[0] {
                    "build" => commands::locate::build::run(),
                    "update" => commands::locate::update::run(),
                    "stats" => commands::locate::stats::run(),
                    _ => {
                        let query: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                        commands::locate::run(&query, Some(current_scope.as_str()));
                    }
                }
            }
            CommandResult::Continue
        }

        // Show command
        "show" => {
            if args.is_empty() {
                println!("{} Usage: show <name>", "error:".red().bold());
            } else {
                let meta = args.contains(&"--meta") || args.contains(&"-m");
                let name = args
                    .iter()
                    .find(|a| !a.starts_with('-'))
                    .unwrap_or(&args[0]);
                commands::show::run(name, meta);
            }
            CommandResult::Continue
        }

        // Validate command
        "validate" => {
            if args.is_empty() {
                commands::validate::all::run();
            } else {
                match args[0] {
                    "all" => commands::validate::all::run(),
                    "item" if args.len() > 1 => commands::validate::item::run(args[1]),
                    _ => println!(
                        "{} Usage: validate [all | item <name>]",
                        "error:".red().bold()
                    ),
                }
            }
            CommandResult::Continue
        }

        // Config command
        "config" => {
            if args.is_empty() || args[0] == "show" {
                commands::config::run(None);
            } else {
                println!(
                    "{} Use 'config show' or 'config set key value'",
                    "info:".cyan().bold()
                );
            }
            CommandResult::Continue
        }

        // Unknown command
        _ => {
            println!("{} Unknown command: {}", "error:".yellow().bold(), command);
            println!("Type 'help' for available commands");
            CommandResult::Continue
        }
    }
}

/// Print help information
fn print_help() {
    println!("{}", "Available Commands:".bold());
    println!();
    println!("  {:<18} Switch to a different scope", "use <scope>".cyan());
    println!("  {:<18} Show graph status", "status".cyan());
    println!(
        "  {:<18} List scopes and languages",
        "list [--stats]".cyan()
    );
    println!("  {:<18} Search for content", "locate <query>".cyan());
    println!("  {:<18} Display content file", "show <name>".cyan());
    println!("  {:<18} Validate content files", "validate [all]".cyan());
    println!("  {:<18} Show configuration", "config show".cyan());
    println!("  {:<18} Show this help", "help".cyan());
    println!("  {:<18} Exit interactive mode", "exit".cyan());
    println!();
    println!("{}", "Index Commands:".bold());
    println!("  {:<18} Build search index", "locate build".cyan());
    println!("  {:<18} Update search index", "locate update".cyan());
    println!("  {:<18} Show index statistics", "locate stats".cyan());
}
