//! Translate command - create translations

use colored::Colorize;

pub fn run(id: &str, to: &str) {
    println!("{}", "Creating translation stub...".bold());
    println!("  Definition: {}", id.cyan());
    println!("  Target:     {}", to.green());

    // TODO: Create translation entry
    println!("\n{}", "Translation not yet implemented".yellow());
}
