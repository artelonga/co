//! Languages command - list available languages

use co::language::Language;
use colored::Colorize;
use comfy_table::{presets::UTF8_FULL, Table};

#[allow(dead_code)]
pub fn run() {
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
