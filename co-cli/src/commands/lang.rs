//! Language configuration command
//!
//! Manages the system language setting for UI translations.
//! This command consolidates `lang` and `languages` as aliases.
//!
//! # Command Aliasing in Clap
//!
//! To add aliases to a command in clap, use:
//! - `#[command(visible_alias = "alias")]` - Shows alias in help
//! - `#[command(alias = "alias")]` - Hidden alias (works but not in help)
//!
//! Example:
//! ```ignore
//! #[derive(Subcommand)]
//! enum Commands {
//!     #[command(visible_alias = "languages")]
//!     Lang { ... }
//! }
//! ```
//!
//! Both `co lang` and `co languages` will invoke the same handler.

use co::language::Language;
use colored::Colorize;
use comfy_table::{presets::UTF8_FULL, Table};
use std::fs;
use std::path::Path;

/// Run the lang command
pub fn run(language: Option<String>, list: bool) {
    if list {
        list_languages();
        return;
    }

    match language {
        Some(lang) => set_language(&lang),
        None => show_language(),
    }
}

/// Set the system language
fn set_language(lang: &str) {
    // Create .co directory if it doesn't exist
    let config_dir = Path::new(".co");
    if let Err(e) = fs::create_dir_all(config_dir) {
        eprintln!(
            "{} Failed to create .co directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Write config with system_language
    let config_content = format!("system_language: {}\n", lang);
    let config_path = config_dir.join("config.yaml");

    if let Err(e) = fs::write(&config_path, config_content) {
        eprintln!("{} Failed to write config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} {}",
        "System language set to:".green(),
        lang.cyan().bold()
    );
}

/// Show current system language
fn show_language() {
    let config_path = Path::new(".co/config.yaml");

    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(config_path) {
            for line in content.lines() {
                if line.starts_with("system_language:") {
                    let lang = line.strip_prefix("system_language:").unwrap().trim();
                    let lang_name = get_language_name(lang);
                    println!(
                        "{} {} ({})",
                        "System language:".bold(),
                        lang.cyan(),
                        lang_name
                    );
                    return;
                }
            }
        }
    }

    println!("{} {} (English)", "System language:".bold(), "en".cyan());
}

/// List available languages (content and UI)
fn list_languages() {
    // Show available content languages
    println!("{}\n", "Available Languages".bold().green());

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["ID", "Name", "Type", "ISO Code"]);

    for lang in Language::initial_languages() {
        table.add_row(vec![
            lang.id.cyan().to_string(),
            lang.name,
            format!("{:?}", lang.exegesis_type),
            lang.iso_code.unwrap_or_else(|| "-".to_string()),
        ]);
    }

    println!("{table}");

    println!("\n{}", "Exegesis Types:".bold());
    println!(
        "  {} - Human spoken languages with cultural context",
        "Natural".yellow()
    );
    println!(
        "  {}  - Symbolic systems (mathematics, logic)",
        "Formal".yellow()
    );
    println!(
        "  {} - Non-textual expression (music, visual)",
        "NonVerbal".yellow()
    );
}

/// Get human-readable language name
fn get_language_name(code: &str) -> &str {
    match code {
        "en" | "english" => "English",
        "pt" | "portuguese" => "Portuguese",
        "es" | "spanish" => "Spanish",
        "de" | "german" => "German",
        "fr" | "french" => "French",
        "guarani-mbya" => "Guarani Mbya",
        "math" => "Mathematics",
        "music" => "Music",
        _ => "Unknown",
    }
}
