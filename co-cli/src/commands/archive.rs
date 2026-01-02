//! Archive command - compress old content

use colored::Colorize;

pub fn run(name: &str) {
    println!("{} {}", "Creating archive:".bold(), name.cyan());
    println!("{}", "─".repeat(40));

    // TODO: Implement archival
    println!("{}", "Archive creation not yet implemented".yellow());
    println!("Will use zstd compression with queryable manifest");
}
