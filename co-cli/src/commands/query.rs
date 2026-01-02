//! Query command - search the graph

use colored::Colorize;

pub fn run(query: &str) {
    println!("{} {}", "Query:".bold(), query.cyan());
    println!("{}", "─".repeat(40));

    // TODO: Implement query parsing and execution
    println!("{}", "Query engine not yet implemented".yellow());
    println!("Expected syntax: field:value [AND|OR] field:value");
}
