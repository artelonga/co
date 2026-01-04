//! CO CLI - Exegetic graph database interface
//!
//! Commands:
//! - `co init` - Initialize a space
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
    /// Initialize a new space
    Init {
        /// Space name (e.g., "private", "work")
        name: Option<String>,

        /// Check for spaces that aren't gitignored (commit guard)
        #[arg(long)]
        check: bool,
    },

    /// List spaces and languages
    List {
        /// Show file counts per space
        #[arg(short, long)]
        stats: bool,
    },

    /// Create new content (task, definition, etc.)
    New {
        /// Content type (task, definition, etc.)
        content_type: String,

        /// Content name/id
        name: String,

        /// Target space (directory)
        #[arg(short, long, value_name = "SPACE")]
        r#in: Option<String>,
    },

    /// Create content interactively with role selection
    ///
    /// Interactive command for collaborative content creation.
    /// Prompts for role (user/agent) and structured input.
    ///
    /// Examples:
    ///   co create user-story my-story --in private
    ///   co create task my-task --in work --story parent-story
    Create {
        /// Content type (user-story, task, etc.)
        content_type: String,

        /// Content name/id
        name: String,

        /// Target space (directory)
        #[arg(short, long, value_name = "SPACE")]
        r#in: Option<String>,

        /// Parent story ID (for tasks)
        #[arg(long)]
        story: Option<String>,
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

    /// Interactive exploration mode with space context
    ///
    /// Enter an interactive shell with command history and space switching.
    ///
    /// Examples:
    ///   co lead              # Start interactive mode
    ///   use private          # Switch to private space
    ///   locate status:todo   # Run commands in current space
    Lead,

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
    ///   co locate private status:todo   # Space + filter
    ///   co locate build                 # Build search index
    ///   co locate update                # Update index incrementally
    ///   co locate stats                 # Show index statistics
    Locate {
        /// Query terms, or subcommand (build, update, stats)
        #[arg(required = true)]
        query: Vec<String>,

        /// Space(s) to search in (comma-separated)
        #[arg(short, long, value_name = "SPACE")]
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

    /// List and manage agents
    ///
    /// Agents are AI-powered extension interfaces.
    ///
    /// Examples:
    ///   co agents                    # List all agents
    ///   co agents type:researcher    # Filter by type
    ///   co agents show researcher    # Show agent details
    Agents {
        #[command(subcommand)]
        action: Option<AgentsSubcommand>,

        /// Query filters (field:value)
        #[arg(trailing_var_arg = true)]
        query: Vec<String>,
    },

    /// List and manage tools
    ///
    /// Tools are deterministic script/utility extensions.
    ///
    /// Examples:
    ///   co tools                   # List all tools
    ///   co tools category:git      # Filter by category
    ///   co tools show gh-wrapper   # Show tool details
    Tools {
        #[command(subcommand)]
        action: Option<ToolsSubcommand>,

        /// Query filters (field:value)
        #[arg(trailing_var_arg = true)]
        query: Vec<String>,
    },

    /// Manage feature type schemas
    ///
    /// View and modify property definitions for feature types.
    ///
    /// Examples:
    ///   co schema list                                    # List all schemas
    ///   co schema show work                               # Show work schema
    ///   co schema add-property work priority:number       # Add property
    ///   co schema remove-property work old_field --force  # Remove property
    ///   co schema modify-property work status --required  # Modify property
    Schema {
        #[command(subcommand)]
        action: SchemaSubcommand,
    },

    /// Manage registered repositories for federated queries
    ///
    /// Register repositories to enable cross-repo queries.
    ///
    /// Examples:
    ///   co repo list                    # List registered repos
    ///   co repo add . --alias work      # Register current dir
    ///   co repo remove work             # Unregister a repo
    ///   co repo tag work client         # Add tags to current repo
    ///   co repo untag work              # Remove tags
    Repo {
        #[command(subcommand)]
        action: RepoSubcommand,
    },

    /// Manage spaces for multi-repo workflows
    ///
    /// A space can be a registered git repo, a private folder,
    /// or any directory with `.co/` configuration.
    ///
    /// Examples:
    ///   co space list                   # List all spaces
    ///   co space current                # Show current space details
    Space {
        #[command(subcommand)]
        action: SpaceSubcommand,
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

#[derive(Subcommand)]
enum AgentsSubcommand {
    /// Show agent details
    Show {
        /// Agent name/id
        name: String,
    },
}

#[derive(Subcommand)]
enum ToolsSubcommand {
    /// Show tool details
    Show {
        /// Tool name/id
        name: String,
    },
}

#[derive(Subcommand)]
enum SpaceSubcommand {
    /// List all registered spaces
    List,
    /// Show current space details
    Current,
}

#[derive(Subcommand)]
enum RepoSubcommand {
    /// List registered repositories
    List,

    /// Add a repository
    Add {
        /// Path to repository (use . for current directory)
        path: String,

        /// Short alias for the repository
        #[arg(long)]
        alias: String,

        /// SSH host for git operations (e.g., "github-work", "github-personal")
        #[arg(long)]
        ssh_host: Option<String>,
    },

    /// Remove a repository
    Remove {
        /// Repository alias or path
        identifier: String,
    },

    /// Add tags to current repository
    Tag {
        /// Tags to add
        #[arg(required = true)]
        tags: Vec<String>,
    },

    /// Remove tags from current repository
    Untag {
        /// Tags to remove
        #[arg(required = true)]
        tags: Vec<String>,
    },
}

#[derive(Subcommand)]
enum SchemaSubcommand {
    /// List all feature schemas
    List,

    /// Show a specific schema
    Show {
        /// Feature name
        name: String,
    },

    /// Add a property to a schema
    #[command(name = "add-property")]
    AddProperty {
        /// Feature name
        feature: String,

        /// Property definition (name:kind, e.g., "priority:number")
        property: String,

        /// Mark property as required
        #[arg(long)]
        required: bool,
    },

    /// Remove a property from a schema
    #[command(name = "remove-property")]
    RemoveProperty {
        /// Feature name
        feature: String,

        /// Property name to remove
        property: String,

        /// Force removal without confirmation
        #[arg(long)]
        force: bool,
    },

    /// Modify a property in a schema
    #[command(name = "modify-property")]
    ModifyProperty {
        /// Feature name
        feature: String,

        /// Property name to modify
        property: String,

        /// New property kind
        #[arg(long)]
        kind: Option<String>,

        /// Set required status
        #[arg(long)]
        required: Option<bool>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, check } => {
            if check {
                commands::init::check_unprotected_spaces();
            } else if let Some(name) = name {
                commands::init::run(&name);
            } else {
                eprintln!("error: Missing space name. Usage: co init <NAME>");
                std::process::exit(1);
            }
        }
        Commands::List { stats } => commands::list::run(stats),
        Commands::New {
            content_type,
            name,
            r#in,
        } => commands::new::run(&content_type, &name, r#in.as_deref()),
        Commands::Create {
            content_type,
            name,
            r#in,
            story,
        } => commands::create::run(&content_type, &name, r#in.as_deref(), story.as_deref()),
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
        Commands::Lead => commands::lead::run(),
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
        Commands::Agents { action, query } => {
            let agents_action = match action {
                Some(AgentsSubcommand::Show { name }) => {
                    commands::agents::AgentsAction::Show { name }
                }
                None => commands::agents::AgentsAction::List { query },
            };
            commands::agents::run(agents_action)
        }
        Commands::Tools { action, query } => {
            let tools_action = match action {
                Some(ToolsSubcommand::Show { name }) => commands::tools::ToolsAction::Show { name },
                None => commands::tools::ToolsAction::List { query },
            };
            commands::tools::run(tools_action)
        }
        Commands::Schema { action } => {
            let schema_action = match action {
                SchemaSubcommand::List => commands::schema::SchemaAction::List,
                SchemaSubcommand::Show { name } => commands::schema::SchemaAction::Show { name },
                SchemaSubcommand::AddProperty {
                    feature,
                    property,
                    required,
                } => {
                    // Parse property:kind format
                    let parts: Vec<&str> = property.split(':').collect();
                    if parts.len() != 2 {
                        eprintln!(
                            "error: Property must be in format 'name:kind' (e.g., 'priority:number')"
                        );
                        std::process::exit(1);
                    }
                    commands::schema::SchemaAction::AddProperty {
                        feature,
                        property: parts[0].to_string(),
                        kind: parts[1].to_string(),
                        required,
                    }
                }
                SchemaSubcommand::RemoveProperty {
                    feature,
                    property,
                    force,
                } => commands::schema::SchemaAction::RemoveProperty {
                    feature,
                    property,
                    force,
                },
                SchemaSubcommand::ModifyProperty {
                    feature,
                    property,
                    kind,
                    required,
                } => commands::schema::SchemaAction::ModifyProperty {
                    feature,
                    property,
                    kind,
                    required,
                },
            };
            commands::schema::run(schema_action)
        }
        Commands::Repo { action } => {
            let repo_action = match action {
                RepoSubcommand::List => commands::repo::RepoAction::List,
                RepoSubcommand::Add {
                    path,
                    alias,
                    ssh_host,
                } => commands::repo::RepoAction::Add {
                    path: std::path::PathBuf::from(path),
                    alias,
                    ssh_host,
                },
                RepoSubcommand::Remove { identifier } => {
                    commands::repo::RepoAction::Remove { identifier }
                }
                RepoSubcommand::Tag { tags } => commands::repo::RepoAction::Tag { tags },
                RepoSubcommand::Untag { tags } => commands::repo::RepoAction::Untag { tags },
            };
            commands::repo::run(repo_action)
        }
        Commands::Space { action } => {
            let space_action = match action {
                SpaceSubcommand::List => commands::space::SpaceAction::List,
                SpaceSubcommand::Current => commands::space::SpaceAction::Current,
            };
            commands::space::run(space_action)
        }
    }
}
