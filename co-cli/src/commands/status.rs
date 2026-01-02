//! Status command - show graph statistics

use co::LANGUAGES;
use colored::Colorize;

pub fn run() {
    println!("{}", "CO Graph Status".bold().green());
    println!("{}", "═".repeat(40));

    println!("\n{}", "Languages:".bold());
    for lang in LANGUAGES {
        println!("  • {}", lang);
    }

    println!("\n{}", "Index:".bold());
    println!("  Entries: {}", "0 (not built)".yellow());

    println!("\n{}", "Storage:".bold());
    println!("  Root: {}", "./graph/".cyan());
}
