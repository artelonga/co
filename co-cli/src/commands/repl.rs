//! REPL command - interactive mode

use colored::Colorize;
use std::io::{self, BufRead, Write};

pub fn run() {
    println!("{}", "CO Interactive Mode".bold().green());
    println!("Type 'help' for commands, 'exit' to quit\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("co> ");
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                match trimmed {
                    "exit" | "quit" | "q" => break,
                    "help" | "?" => print_help(),
                    "" => continue,
                    cmd => {
                        println!("{} {}", "Unknown command:".yellow(), cmd);
                        println!("Type 'help' for available commands");
                    }
                }
            }
            Err(_) => break,
        }
    }

    println!("{}", "Goodbye!".green());
}

fn print_help() {
    println!("{}", "Available Commands:".bold());
    println!("  {}  - Show graph status", "status".cyan());
    println!("  {} <q> - Query the graph", "query".cyan());
    println!("  {}    - List languages", "langs".cyan());
    println!("  {}    - Show this help", "help".cyan());
    println!("  {}    - Exit REPL", "exit".cyan());
}
