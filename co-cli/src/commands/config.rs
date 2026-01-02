//! Config command - manage configuration

use crate::ConfigAction;
use colored::Colorize;

pub fn run(action: Option<ConfigAction>) {
    match action {
        Some(ConfigAction::Show) | None => show_config(),
        Some(ConfigAction::Set { key, value }) => set_config(&key, &value),
    }
}

fn show_config() {
    println!("{}", "CO Configuration".bold().green());
    println!("{}", "─".repeat(30));
    println!("  graph_root: {}", "./graph/".cyan());
    println!("  index_path: {}", "./.co/index.bin".cyan());
    println!("  default_language: {}", "english".cyan());
}

fn set_config(key: &str, value: &str) {
    println!("Setting {} = {}", key.cyan(), value.green());
    // TODO: Persist configuration
    println!(
        "{}",
        "Configuration persistence not yet implemented".yellow()
    );
}
