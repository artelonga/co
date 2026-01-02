//! CO CLI - Exegetic graph database interface
//!
//! Commands:
//! - `co status` - Show graph status
//! - `co query` - Query the graph
//! - `co define` - Create definitions
//! - `co translate` - Manage translations
//! - `co repl` - Interactive mode

use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "co")]
#[command(
    author,
    version,
    about = "Exegetic graph database for project development"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show graph status and statistics
    Status,

    /// Query the graph with DSL
    Query {
        /// Query string (e.g., "status:active AND priority:high")
        query: String,
    },

    /// Create a new definition
    Define {
        /// Definition ID
        id: String,

        /// Symbol/term
        #[arg(short, long)]
        symbol: String,

        /// Language for this definition
        #[arg(short, long, default_value = "english")]
        language: String,
    },

    /// Translate a definition to another language
    Translate {
        /// Definition ID to translate
        id: String,

        /// Target language
        #[arg(short, long)]
        to: String,
    },

    /// List available languages
    Languages,

    /// Rebuild the index
    Index {
        #[command(subcommand)]
        action: IndexAction,
    },

    /// Archive old content
    Archive {
        /// Archive name (e.g., "2024-Q4")
        name: String,
    },

    /// Enter interactive REPL mode
    Repl,

    /// Show or edit configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum IndexAction {
    /// Build index from scratch
    Build,
    /// Update index incrementally
    Update,
    /// Show index statistics
    Stats,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set { key: String, value: String },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Status => commands::status::run(),
        Commands::Query { query } => commands::query::run(&query),
        Commands::Define {
            id,
            symbol,
            language,
        } => commands::define::run(&id, &symbol, &language),
        Commands::Translate { id, to } => commands::translate::run(&id, &to),
        Commands::Languages => commands::languages::run(),
        Commands::Index { action } => commands::index::run(action),
        Commands::Archive { name } => commands::archive::run(&name),
        Commands::Repl => commands::repl::run(),
        Commands::Config { action } => commands::config::run(action),
    }
}
