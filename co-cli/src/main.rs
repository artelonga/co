//! CO CLI - Exegetic graph database interface
//!
//! Commands:
//! - `co init` - Initialize a scope
//! - `co status` - Show graph status
//! - `co query` - Query the graph
//! - `co define` - Create definitions
//! - `co translate` - Manage translations
//! - `co repl` - Interactive mode

use clap::{Parser, Subcommand};

mod commands;
mod i18n;

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
    /// Initialize a new scope
    Init {
        /// Scope name (e.g., "private", "org")
        name: String,
    },

    /// List scopes and languages
    List {
        /// Show file counts per scope
        #[arg(short, long)]
        stats: bool,
    },

    /// Create new content (task, definition, etc.)
    New {
        /// Content type (task, definition, etc.)
        content_type: String,

        /// Content name/id
        name: String,

        /// Target scope (context directory)
        #[arg(short, long, value_name = "SCOPE")]
        r#in: Option<String>,
    },

    /// Show content file
    Show {
        /// Content name/id to display
        name: String,

        /// Show only frontmatter metadata
        #[arg(short, long)]
        meta: bool,
    },

    /// Show graph status and statistics
    Status,

    /// Update content file frontmatter
    Update {
        /// Content name/id to update
        name: String,

        /// New status value
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Delete content file
    Delete {
        /// Content name/id to delete
        name: String,

        /// Confirm deletion (required)
        #[arg(long)]
        confirm: bool,
    },

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

    /// Manage languages and UI translations
    ///
    /// Aliases: `lang`, `languages`
    ///
    /// Examples:
    ///   co lang              # Show current system language
    ///   co lang pt           # Set system language to Portuguese
    ///   co lang --list       # List available languages
    ///   co languages --list  # Same as above (alias)
    #[command(visible_alias = "languages")]
    Lang {
        /// Language code to set (e.g., "pt", "en")
        language: Option<String>,

        /// List available UI translations
        #[arg(short, long)]
        list: bool,
    },

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

    /// Search and filter content, or manage index
    ///
    /// Unified command for locating content by frontmatter or body text.
    ///
    /// Examples:
    ///   co locate status:todo           # Filter by frontmatter
    ///   co locate "important meeting"   # Full-text search
    ///   co locate status:todo meeting   # Combined filter + search
    ///   co locate private status:todo   # Context + filter
    ///   co locate build                 # Build search index
    ///   co locate update                # Update index incrementally
    ///   co locate stats                 # Show index statistics
    Locate {
        /// Query terms, or subcommand (build, update, stats)
        #[arg(required = true)]
        query: Vec<String>,

        /// Context(s) to search in (comma-separated)
        #[arg(short, long, value_name = "CONTEXT")]
        r#in: Option<String>,
    },

    /// Validate content files for errors and warnings
    ///
    /// Checks frontmatter fields, references, and links.
    ///
    /// Examples:
    ///   co validate all           # Validate all content
    ///   co validate item my-task  # Validate specific item
    Validate {
        #[command(subcommand)]
        action: ValidateAction,
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

#[derive(Subcommand)]
enum ValidateAction {
    /// Validate all content files
    All,
    /// Validate a specific item
    Item {
        /// Content name/id to validate
        name: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name } => commands::init::run(&name),
        Commands::List { stats } => commands::list::run(stats),
        Commands::New {
            content_type,
            name,
            r#in,
        } => commands::new::run(&content_type, &name, r#in.as_deref()),
        Commands::Show { name, meta } => commands::show::run(&name, meta),
        Commands::Status => commands::status::run(),
        Commands::Update { name, status } => commands::update::run(&name, status.as_deref()),
        Commands::Delete { name, confirm } => commands::delete::run(&name, confirm),
        Commands::Query { query } => commands::query::run(&query),
        Commands::Define {
            id,
            symbol,
            language,
        } => commands::define::run(&id, &symbol, &language),
        Commands::Translate { id, to } => commands::translate::run(&id, &to),
        Commands::Lang { language, list } => commands::lang::run(language, list),
        Commands::Index { action } => commands::index::run(action),
        Commands::Archive { name } => commands::archive::run(&name),
        Commands::Repl => commands::repl::run(),
        Commands::Config { action } => commands::config::run(action),
        Commands::Locate { query, r#in } => {
            // Check if first arg is a subcommand (build, update, stats)
            if let Some(first) = query.first() {
                match first.as_str() {
                    "build" => return commands::locate::build::run(),
                    "update" => return commands::locate::update::run(),
                    "stats" => return commands::locate::stats::run(),
                    _ => {}
                }
            }
            commands::locate::run(&query, r#in.as_deref())
        }
        Commands::Validate { action } => match action {
            ValidateAction::All => commands::validate::all::run(),
            ValidateAction::Item { name } => commands::validate::item::run(&name),
        },
    }
}
