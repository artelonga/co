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
mod docs;
mod i18n;
mod vcs;

#[derive(Parser)]
#[command(name = "co")]
#[command(
    author,
    version,
    about = "Exegetic graph database for project development",
    disable_help_subcommand = true
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

    /// Archive content for storage (deindexed from co)
    ///
    /// Move content to archive directory with `indexed: false` tag.
    /// Archived items are preserved but ignored by co locate/validate.
    ///
    /// Examples:
    ///   co archive task-12.1.1           # Archive a task
    ///   co archive task-1 --force        # Replace existing archive
    ///   co archive restore task-12.1.1   # Restore from archive
    ///   co archive list                  # List archived items
    #[command(visible_alias = "ar")]
    Archive {
        #[command(subcommand)]
        action: Option<ArchiveSubcommand>,

        /// Content name/id to archive (when not using subcommand)
        #[arg(required = false)]
        name: Option<String>,

        /// Force replace if already archived
        #[arg(long)]
        force: bool,
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

    /// Manage git-backed installed tools (CO-331)
    ///
    /// Install, version-pin, update, and remove open-source tools as git checkouts.
    ///
    /// Examples:
    ///   co tool add claude-code --from https://github.com/anthropics/claude-code --pin v2.0.0
    ///   co tool add co-auto --from ~/projects/co-auto
    ///   co tool list
    ///   co tool update claude-code --pin v2.1.0
    ///   co tool update --all
    ///   co tool remove claude-code
    ///   co tool verify
    #[command(name = "tool")]
    Tool {
        #[command(subcommand)]
        action: ToolSubcommand,

        /// CO data directory (defaults to platform data dir / co)
        #[arg(long, global = true, value_name = "DIR")]
        data_dir: Option<std::path::PathBuf>,
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

    /// GitHub CLI wrapper for issue operations
    ///
    /// Low-level wrapper around the `gh` CLI tool.
    /// Requires `gh` to be installed and authenticated.
    ///
    /// Examples:
    ///   co gh issue list                # List open issues
    ///   co gh issue list --state all    # List all issues
    ///   co gh issue show 35             # Show issue #35
    Gh {
        #[command(subcommand)]
        action: GhSubcommand,
    },

    /// Collaborate with GitHub for issue synchronization
    ///
    /// High-level commands for syncing GitHub issues with local content.
    ///
    /// Examples:
    ///   co collab pull --all            # Pull all open issues to local files
    ///   co collab pull 35 36            # Pull specific issues
    Collab {
        #[command(subcommand)]
        action: CollabSubcommand,
    },

    /// Plan and execute objectives through git workflow
    ///
    /// Two-phase workflow for structured development:
    /// - Plan: Create structured use-case with acceptance criteria
    /// - Execute: Drive plan through git states (todo → in-progress → review → done)
    ///
    /// Examples:
    ///   co conduct plan "Add user authentication"
    ///   co conduct plan "Feature X" --context context.md
    ///   co conduct execute my-plan-id
    Conduct {
        #[command(subcommand)]
        action: ConductSubcommand,
    },

    /// Write content using an agent
    ///
    /// Invoke a writer agent to generate content based on its backend:
    /// - manual: Interactive prompts for structured input
    /// - claude: Creates skeleton template for Claude Code to fill
    /// - ollama: Local model integration (future)
    ///
    /// Examples:
    ///   co write user-story --agent writer --in private
    ///   co write task --agent writer --context notes.md
    Write {
        /// Content type to generate (user-story, task, etc.)
        content_type: String,

        /// Agent to use for writing
        #[arg(short, long, required = true)]
        agent: String,

        /// Target space (directory)
        #[arg(short, long, value_name = "SPACE")]
        r#in: Option<String>,

        /// Additional context file
        #[arg(long)]
        context: Option<String>,

        /// Content name/id (skips name prompt if provided)
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Analyze content quality and generate suggestions
    ///
    /// Evaluates content against criteria and generates
    /// improvement suggestions and interview questions.
    ///
    /// Examples:
    ///   co analyze my-story         # Analyze content item
    ///   co analyze my-task -v       # Verbose analysis
    Analyze {
        /// Content name/id to analyze
        name: String,

        /// Show verbose analysis details
        #[arg(short, long)]
        verbose: bool,
    },

    /// Get help on concepts, workflows, and commands
    ///
    /// Shows topic-based documentation for CO concepts.
    /// Use `co --help` for command syntax reference.
    ///
    /// Examples:
    ///   co help                    # List all help topics
    ///   co help getting-started    # Quick start guide
    ///   co help spaces             # Understanding spaces
    ///   co help workflows          # Plan & Execute workflow
    ///   co help work-items         # User-stories, tasks, etc.
    #[command(visible_alias = "h")]
    Help {
        /// Topic name (optional)
        topic: Option<String>,
    },

    /// Show CO teaser animation
    ///
    /// Displays a loading animation followed by title slides.
    Teaser,

    /// Start the CO web server locally (localhost-first distribution)
    ///
    /// Binds to 127.0.0.1 by default so the server is only reachable from
    /// this machine. Use --public to expose on the local network (warns).
    /// Data is stored in the platform data dir (~/.local/share/co on Linux,
    /// ~/Library/Application Support/co on macOS) unless --data-dir is set.
    ///
    /// Examples:
    ///   co serve                          # default: 127.0.0.1:54321
    ///   co serve --open                   # also opens the default browser
    ///   co serve --port 8080              # custom port
    ///   co serve --data-dir ~/my-co       # custom data directory
    ///   co serve --public                 # bind 0.0.0.0 (prints a warning)
    Serve {
        /// Server port
        #[arg(short, long, env = "CO_SERVE_PORT", default_value_t = 54321)]
        port: u16,

        /// Data directory for SQLite + universe files (default: platform data dir / co)
        #[arg(short, long, env = "CO_SERVE_DATA")]
        data_dir: Option<std::path::PathBuf>,

        /// Open the default browser after the server starts
        #[arg(long)]
        open: bool,

        /// Bind to 0.0.0.0 instead of 127.0.0.1 (exposes to local network — prints a warning)
        #[arg(long)]
        public: bool,
    },

    /// Bootstrap a universe from the current directory into localhost CO (Fly-style)
    ///
    /// Walks up from the current directory to the git repo root (falls back to
    /// CWD if there is no .git), derives a universe key from the directory name,
    /// and seeds docs/, content/, and work/ into the local CO storage.
    ///
    /// Examples:
    ///   co launch                           # provision universe from current dir
    ///   co launch --key myproject           # override the derived key
    ///   co launch --name "My Project"       # override the display name
    ///   co launch --public                  # mark as public-subscribable
    ///   co launch --now                     # provision and open in browser
    Launch {
        /// Override the derived universe key (default: lowercased + sanitized dir name)
        #[arg(long)]
        key: Option<String>,

        /// Override the display name (default: directory basename)
        #[arg(long)]
        name: Option<String>,

        /// Mark the universe as public-subscribable
        #[arg(long)]
        public: bool,

        /// Also start the server and open the browser after provisioning
        #[arg(long)]
        now: bool,

        /// Server port used when --now is set
        #[arg(short, long, env = "CO_SERVE_PORT", default_value_t = 54321)]
        port: u16,

        /// Data directory for SQLite + universe files (default: platform data dir / co)
        #[arg(short, long, env = "CO_SERVE_DATA")]
        data_dir: Option<std::path::PathBuf>,
    },

    /// Build a universe into a Quartz static garden (public site)
    ///
    /// Feeds the universe's content/ markdown through the redearte Quartz
    /// template and writes a static site to --out (default: public/).
    /// Only content/ is included; _source/ PII dirs are excluded by design.
    ///
    /// Run from within the universe directory:
    ///   co construir                          # build, key from dir name
    ///   co construir grcsamazonia             # same, explicit key
    ///   co construir --out dist/              # custom output dir
    ///   co construir --redearte ~/dev/redearte  # custom template path
    Construir {
        /// Universe key (default: derived from current directory name)
        key: Option<String>,

        /// Output directory for the static site (default: public/)
        #[arg(short, long, default_value = "public")]
        out: String,

        /// Path to the redearte Quartz template (default: CO_REDEARTE_PATH or ~/projects/redearte)
        #[arg(long)]
        redearte: Option<std::path::PathBuf>,
    },

    /// Upload a local universe to a remote CO server over the Vault API
    ///
    /// Resolves the universe from the current directory (same discovery as `co launch`),
    /// reads `_universe.yaml` for key/name, then calls `POST /api/v1/universes` +
    /// `PUT .../vault/{path}` for each `content/**/*.md` file. Re-running converges
    /// (idempotent). Token via --token or CO_TOKEN env; base URL via --remote or CO_REMOTE.
    ///
    /// Examples:
    ///   co push --remote https://co.example.com --token mytoken
    ///   CO_REMOTE=https://co.example.com CO_TOKEN=mytoken co push
    ///   co push --dry-run                        # preview without writing
    ///   co push --delete-missing                 # also remove server entries absent locally
    Push {
        /// Base URL of the remote CO server (or set CO_REMOTE)
        #[arg(long, env = "CO_REMOTE")]
        remote: Option<String>,

        /// API token for authentication (or set CO_TOKEN)
        #[arg(long, env = "CO_TOKEN")]
        token: Option<String>,

        /// Override the derived universe key (default: lowercased + sanitized dir name)
        #[arg(long)]
        key: Option<String>,

        /// Preview create/update/delete plan without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Delete server entries that are not present locally
        #[arg(long)]
        delete_missing: bool,
    },

    /// Release notes — what changed in recent CO versions
    ///
    /// Shows the latest release section from the CHANGELOG embedded in this
    /// binary (always matches the installed version).
    ///
    /// Examples:
    ///   co updates                   # latest release notes
    ///   co updates -n 3              # three most recent releases
    ///   co updates --all             # full release history (headers)
    Updates {
        /// Number of recent releases to show in detail
        #[arg(short = 'n', long, default_value_t = 1)]
        count: usize,

        /// List every release header instead of detailed notes
        #[arg(long)]
        all: bool,
    },

    /// Start the project management board (web UI)
    ///
    /// Launches a local web server with Kanban/Calendar views.
    /// Use subcommands to manage board data.
    ///
    /// Examples:
    ///   co board                     # Start on default port 3000
    ///   co board --port 8080         # Custom port
    ///   co board export              # Export local edits to source tree
    ///   co board reset --confirm     # Reset to embedded baseline
    Board {
        #[command(subcommand)]
        action: Option<BoardSubcommand>,

        /// Server port
        #[arg(short, long, env = "CO_WEB_PORT", default_value_t = 3000)]
        port: u16,

        /// Data directory path
        #[arg(short, long, env = "CO_WEB_DATA", default_value = "./data")]
        data: String,

        /// Static files directory
        #[arg(short, long, env = "CO_WEB_STATIC", default_value = "co-web/static")]
        static_dir: String,

        /// Default experiment variant (a, b, or c)
        #[arg(long, env = "CO_WEB_DEFAULT_VARIANT", default_value = "a")]
        default_variant: String,
    },

    /// Authenticate and manage API tokens
    ///
    /// All password prompts use hidden input (never echoed, never in shell
    /// history). Credentials are stored in ~/.config/co/credentials (mode 600).
    ///
    /// Examples:
    ///   co auth login --email you@example.com --save-token
    ///   co auth reset-password --email you@example.com
    ///   co auth status
    ///   co auth token create --save
    ///   co auth token list --json
    ///   co auth token revoke tok_abc123
    ///   co auth logout --revoke-token
    Auth {
        #[command(subcommand)]
        action: AuthSubcommand,

        /// Profile name in ~/.config/co/credentials (default: "default")
        #[arg(long, global = true)]
        profile: Option<String>,
    },

    /// Deploy a universe to a target platform
    ///
    /// Publishes the built output directory to the specified target.
    /// R2 credentials are read from environment variables:
    ///   R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_BUCKET
    ///
    /// Examples:
    ///   co deploy --universe-id my-universe --dist ./dist
    ///   co deploy --universe-id my-universe --target static-on-r2 --dist ./dist
    ///   co deploy rollback --deploy-id my-universe/20240101T000000Z-abc12345
    Deploy {
        #[command(subcommand)]
        action: Option<DeploySubcommand>,

        /// Target platform (currently only static-on-r2)
        #[arg(long, default_value = "static-on-r2")]
        target: String,

        /// Built output directory to publish
        #[arg(long, default_value = "dist")]
        dist: String,

        /// deploy.yaml path
        #[arg(long, default_value = "deploy.yaml")]
        manifest: String,

        /// Universe identifier (slug or UUID)
        #[arg(long)]
        universe_id: Option<String>,
    },

    /// Pipeline engine: plan, execute, approve via Claude + GitHub
    ///
    /// Modular pipeline for LLM-powered GitHub workflows on any repository.
    ///
    /// Examples:
    ///   co engine plan --repo owner/repo --title "Add feature"
    ///   co engine execute --repo owner/repo --issue 42
    ///   co engine approve --repo owner/repo --pr 15
    ///   co engine auto --repo owner/repo --title "Add feature"
    ///   co engine status
    Engine {
        #[command(subcommand)]
        action: EngineSubcommand,

        /// Path to Obsidian vault for context
        #[arg(long, env = "CO_ENGINE_VAULT", global = true)]
        vault: Option<std::path::PathBuf>,

        /// Auto-approve all pipeline stages (no prompts)
        #[arg(long, global = true)]
        auto_approve: bool,
    },
}

#[derive(Subcommand)]
enum DeploySubcommand {
    /// Roll back to a previous deployment
    Rollback {
        /// Deploy ID to restore (format: {universe_id}/{timestamp}-{suffix})
        #[arg(long)]
        deploy_id: String,
    },
}

#[derive(Subcommand)]
enum BoardSubcommand {
    /// Export local board data to source tree for recompilation
    ///
    /// Copies data from the runtime directory back to co-web/data/
    /// so it gets embedded in the next build.
    ///
    /// Examples:
    ///   co board export                    # Export to default co-web/data/
    ///   co board export --to ./my-data     # Export to custom path
    Export {
        /// Destination directory (default: co-web/data)
        #[arg(long, default_value = "co-web/data")]
        to: String,
    },

    /// Reset local data to embedded baseline
    ///
    /// Discards all local edits and restores the original data.
    ///
    /// Examples:
    ///   co board reset --confirm
    Reset {
        /// Confirm destructive reset
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand)]
enum EngineSubcommand {
    /// Create a GitHub issue from a description (Plan stage)
    Plan {
        /// Target repository (owner/repo)
        #[arg(short, long)]
        repo: String,

        /// Issue title
        #[arg(short, long)]
        title: String,

        /// Issue description / context
        #[arg(short, long, default_value = "")]
        description: String,
    },

    /// Implement an issue as a PR (Execute stage)
    Execute {
        /// Target repository (owner/repo)
        #[arg(short, long)]
        repo: String,

        /// Issue number to implement
        #[arg(short, long)]
        issue: u64,

        /// Local working directory for the repo
        #[arg(short, long)]
        workdir: Option<String>,
    },

    /// Review a pull request (Approve stage)
    Approve {
        /// Target repository (owner/repo)
        #[arg(short, long)]
        repo: String,

        /// PR number to review
        #[arg(short, long)]
        pr: u64,
    },

    /// Run full pipeline: plan → execute → approve
    Auto {
        /// Target repository (owner/repo)
        #[arg(short, long)]
        repo: String,

        /// Feature/fix title
        #[arg(short, long)]
        title: String,

        /// Detailed description
        #[arg(short, long, default_value = "")]
        description: String,

        /// Local working directory for the repo
        #[arg(short, long)]
        workdir: Option<String>,
    },

    /// Search Obsidian vault for context
    Query {
        /// Search query
        query: String,
    },

    /// Show available tools and their status
    Status,

    /// Manage git identities for multi-account workflows
    ///
    /// Examples:
    ///   co engine identity list
    ///   co engine identity add --name work --github-user mywork --ssh-host github-work --email work@co.dev
    ///   co engine identity switch work
    Identity {
        #[command(subcommand)]
        action: IdentitySubcommand,
    },
}

#[derive(Subcommand)]
enum AuthSubcommand {
    /// Interactive password login
    ///
    /// Prompts for password with hidden input. With --save-token, also creates
    /// a 90-day API token and writes it to ~/.config/co/credentials.
    Login {
        /// Email address
        #[arg(long)]
        email: Option<String>,

        /// Create and save a 90-day API token after login
        #[arg(long)]
        save_token: bool,
    },

    /// Full forgot-password flow in one command
    ///
    /// Sends code to verified recovery channels, prompts for code, prompts for
    /// new password (hidden, confirmed twice), then auto-logs in.
    #[command(name = "reset-password")]
    ResetPassword {
        /// Email or username to identify the account
        #[arg(long)]
        email: Option<String>,
    },

    /// Change password (requires active login session)
    ///
    /// Prompts for current and new password with hidden input.
    #[command(name = "change-password")]
    ChangePassword,

    /// Show authentication status and exit 0/1 for scripts
    ///
    /// Prints logged-in user, token ID, expiry, and base URL.
    /// Exits 0 when authenticated; exits 1 otherwise.
    Status,

    /// Manage 90-day API tokens
    Token {
        #[command(subcommand)]
        action: TokenSubcommand,
    },

    /// Clear local credentials (and optionally revoke API token server-side)
    Logout {
        /// Also DELETE the API token from the server
        #[arg(long)]
        revoke_token: bool,
    },
}

#[derive(Subcommand)]
enum TokenSubcommand {
    /// Create a new 90-day API token
    ///
    /// Token value is printed once and never retrievable again.
    /// Use --save to persist it to ~/.config/co/credentials.
    Create {
        /// Token name (default: cli-<hostname>-<YYYY-MM-DD>)
        #[arg(long)]
        name: Option<String>,

        /// Save to ~/.config/co/credentials
        #[arg(long)]
        save: bool,
    },

    /// List all API tokens for the authenticated user
    List {
        /// Print raw JSON instead of a table
        #[arg(long)]
        json: bool,
    },

    /// Revoke (permanently delete) an API token by ID
    Revoke {
        /// Token ID to revoke (from 'co auth token list')
        id: String,
    },
}

#[derive(Subcommand)]
enum IdentitySubcommand {
    /// List all configured identities
    List,

    /// Show the active identity
    Current,

    /// Switch to a different identity
    Switch {
        /// Identity name to switch to
        name: String,
    },

    /// Add a new identity
    Add {
        /// Identity name (e.g., "personal", "work")
        #[arg(long)]
        name: String,

        /// GitHub username
        #[arg(long)]
        github_user: String,

        /// SSH host alias (from ~/.ssh/config)
        #[arg(long, default_value = "github.com")]
        ssh_host: String,

        /// Git commit email
        #[arg(long)]
        email: String,

        /// Git commit name
        #[arg(long)]
        git_name: Option<String>,
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
enum ArchiveSubcommand {
    /// Restore content from archive
    Restore {
        /// Content name/id to restore
        name: String,
    },
    /// List archived items
    List,
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
    /// Validate a deploy.yaml manifest
    ///
    /// Checks the manifest against the deploy.yaml v1 schema and reports
    /// errors with file path, line number, and field path.
    ///
    /// Examples:
    ///   co validate deploy               # validate ./deploy.yaml
    ///   co validate deploy myapp/deploy.yaml
    Deploy {
        /// Path to deploy.yaml (default: deploy.yaml in current directory)
        #[arg(default_value = "deploy.yaml")]
        path: String,
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

// CO-331: git-backed tool registry subcommands
#[derive(Subcommand)]
enum ToolSubcommand {
    /// Register and clone a tool from a git URL or local path
    Add {
        /// Tool key (e.g. "claude-code")
        key: String,
        /// Remote git URL or local path
        #[arg(long, value_name = "URL_OR_PATH")]
        from: String,
        /// Version to pin (tag, branch, or SHA)
        #[arg(long)]
        pin: Option<String>,
    },
    /// List installed tools with their version and state
    List,
    /// Update a tool's version pin or refresh follow-main tools
    Update {
        /// Tool key to update
        key: Option<String>,
        /// New version pin (tag, branch, or SHA)
        #[arg(long)]
        pin: Option<String>,
        /// Switch this tool to always track origin/main
        #[arg(long)]
        follow_main: bool,
        /// Refresh all tools with follow_main=true
        #[arg(long)]
        all: bool,
    },
    /// Remove a tool and delete its checkout
    Remove {
        /// Tool key to remove
        key: String,
    },
    /// Verify each tool's checkout matches its lockfile SHA
    Verify {
        /// Verify only this tool (default: all)
        key: Option<String>,
    },
}

#[derive(Subcommand)]
enum ToolsSubcommand {
    /// Show tool details
    Show {
        /// Tool name/id
        name: String,
    },
    /// Run a tool with arguments
    Run {
        /// Tool name/id
        name: String,
        /// Arguments to pass to the tool
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
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
enum GhSubcommand {
    /// Issue operations
    Issue {
        #[command(subcommand)]
        action: GhIssueAction,
    },
}

#[derive(Subcommand)]
enum GhIssueAction {
    /// List issues from the repository
    List {
        /// Filter by state: open, closed, all
        #[arg(short, long, default_value = "open")]
        state: String,

        /// Filter by label
        #[arg(short, long)]
        label: Option<String>,

        /// Maximum number of issues to show
        #[arg(long, default_value = "30")]
        limit: u32,
    },
    /// Show details of a specific issue
    Show {
        /// Issue number
        number: u64,
    },
}

#[derive(Subcommand)]
enum CollabSubcommand {
    /// Pull GitHub issues to local markdown files
    Pull {
        /// Pull all open issues
        #[arg(long)]
        all: bool,

        /// Specific issue numbers to pull
        #[arg(trailing_var_arg = true)]
        numbers: Vec<u64>,
    },
}

#[derive(Subcommand)]
enum ConductSubcommand {
    /// Create a structured plan from an objective
    Plan {
        /// The objective to plan
        objective: String,

        /// Target space within repo (default: work)
        #[arg(short, long, value_name = "SPACE")]
        r#in: Option<String>,

        /// Target repo alias (default: detect from cwd)
        #[arg(long)]
        repo: Option<String>,

        /// Context file path
        #[arg(long)]
        context: Option<String>,
    },
    /// Execute an approved plan through git workflow
    Execute {
        /// Plan ID to execute
        id: String,

        /// Target repo alias (default: detect from cwd)
        #[arg(long)]
        repo: Option<String>,
    },
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

    /// Switch active workspace context
    Switch {
        /// Repository alias to switch to, or "none" to clear
        alias: String,
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
        Commands::Archive {
            action,
            name,
            force,
        } => {
            let archive_action = match action {
                Some(ArchiveSubcommand::Restore { name }) => {
                    commands::archive::ArchiveAction::Restore { name }
                }
                Some(ArchiveSubcommand::List) => commands::archive::ArchiveAction::List,
                None => {
                    if let Some(name) = name {
                        commands::archive::ArchiveAction::Archive { name, force }
                    } else {
                        eprintln!("error: Missing content name. Usage: co archive <NAME>");
                        std::process::exit(1);
                    }
                }
            };
            commands::archive::run(archive_action)
        }
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
            ValidateAction::Deploy { path } => commands::validate::deploy::run(&path),
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
        Commands::Tool { action, data_dir } => {
            let resolved_data = data_dir
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    dirs::data_local_dir()
                        .map(|d| d.join("co").to_string_lossy().into_owned())
                        .unwrap_or_else(|| "./co-data".to_string())
                });
            let tool_action = match action {
                ToolSubcommand::Add { key, from, pin } => commands::tool::ToolAction::Add {
                    key,
                    from,
                    pin,
                    data_dir: resolved_data,
                },
                ToolSubcommand::List => commands::tool::ToolAction::List {
                    data_dir: resolved_data,
                },
                ToolSubcommand::Update {
                    key,
                    pin,
                    follow_main,
                    all,
                } => commands::tool::ToolAction::Update {
                    key,
                    pin,
                    follow_main,
                    all,
                    data_dir: resolved_data,
                },
                ToolSubcommand::Remove { key } => commands::tool::ToolAction::Remove {
                    key,
                    data_dir: resolved_data,
                },
                ToolSubcommand::Verify { key } => commands::tool::ToolAction::Verify {
                    key,
                    data_dir: resolved_data,
                },
            };
            commands::tool::run(tool_action)
        }
        Commands::Tools { action, query } => {
            let tools_action = match action {
                Some(ToolsSubcommand::Show { name }) => commands::tools::ToolsAction::Show { name },
                Some(ToolsSubcommand::Run { name, args }) => {
                    commands::tools::ToolsAction::Run { name, args }
                }
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
                RepoSubcommand::Switch { alias } => commands::repo::RepoAction::Switch { alias },
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
        Commands::Gh { action } => {
            let gh_action = match action {
                GhSubcommand::Issue { action } => {
                    let issue_action = match action {
                        GhIssueAction::List {
                            state,
                            label,
                            limit,
                        } => commands::gh::issue::IssueAction::List {
                            state,
                            label,
                            limit,
                        },
                        GhIssueAction::Show { number } => {
                            commands::gh::issue::IssueAction::Show { number }
                        }
                    };
                    commands::gh::GhAction::Issue {
                        action: issue_action,
                    }
                }
            };
            commands::gh::run(gh_action)
        }
        Commands::Collab { action } => {
            let collab_action = match action {
                CollabSubcommand::Pull { all, numbers } => {
                    commands::collab::CollabAction::Pull { all, numbers }
                }
            };
            commands::collab::run(collab_action)
        }
        Commands::Conduct { action } => {
            let conduct_action = match action {
                ConductSubcommand::Plan {
                    objective,
                    r#in,
                    repo,
                    context,
                } => commands::conduct::ConductAction::Plan {
                    objective,
                    space: r#in,
                    repo,
                    context,
                },
                ConductSubcommand::Execute { id, repo } => {
                    commands::conduct::ConductAction::Execute { id, repo }
                }
            };
            commands::conduct::run(conduct_action)
        }
        Commands::Write {
            content_type,
            agent,
            r#in,
            context,
            name,
        } => commands::write::run(
            &content_type,
            &agent,
            r#in.as_deref(),
            context.as_deref(),
            name.as_deref(),
        ),
        Commands::Analyze { name, verbose } => commands::analyze::run(&name, verbose),
        Commands::Help { topic } => commands::help::run(topic.as_deref()),
        Commands::Teaser => commands::teaser::run(),
        Commands::Serve {
            port,
            data_dir,
            open,
            public,
        } => {
            let resolved = data_dir
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    dirs::data_local_dir()
                        .map(|d| d.join("co").to_string_lossy().into_owned())
                        .unwrap_or_else(|| "./co-data".to_string())
                });
            commands::serve::run(port, resolved, public, open);
        }
        Commands::Launch {
            key,
            name,
            public,
            now,
            port,
            data_dir,
        } => {
            let resolved = data_dir
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    dirs::data_local_dir()
                        .map(|d| d.join("co").to_string_lossy().into_owned())
                        .unwrap_or_else(|| "./co-data".to_string())
                });
            commands::launch::run(key, name, public, now, port, resolved);
        }
        Commands::Construir { key, out, redearte } => {
            commands::construir::run(key, out, redearte);
        }
        Commands::Push {
            remote,
            token,
            key,
            dry_run,
            delete_missing,
        } => {
            commands::push::run(remote, token, key, dry_run, delete_missing);
        }
        Commands::Updates { count, all } => {
            commands::updates::run(count, all);
        }
        Commands::Board {
            action,
            port,
            data,
            static_dir,
            default_variant,
        } => match action {
            Some(BoardSubcommand::Export { to }) => {
                commands::board::export(&data, &to);
            }
            Some(BoardSubcommand::Reset { confirm }) => {
                commands::board::reset(&data, confirm);
            }
            None => commands::board::run(port, data, static_dir, default_variant),
        },
        Commands::Deploy {
            action,
            target,
            dist,
            manifest,
            universe_id,
        } => match action {
            Some(DeploySubcommand::Rollback { deploy_id }) => {
                commands::deploy::rollback(&deploy_id);
            }
            None => {
                let uid = universe_id.unwrap_or_else(|| {
                    eprintln!("error: --universe-id is required");
                    std::process::exit(1);
                });
                commands::deploy::run(&target, &dist, &manifest, &uid);
            }
        },
        Commands::Auth { action, profile } => {
            let auth_action = match action {
                AuthSubcommand::Login { email, save_token } => {
                    commands::auth::AuthAction::Login { email, save_token }
                }
                AuthSubcommand::ResetPassword { email } => {
                    commands::auth::AuthAction::ResetPassword { email }
                }
                AuthSubcommand::ChangePassword => commands::auth::AuthAction::ChangePassword,
                AuthSubcommand::Status => commands::auth::AuthAction::Status,
                AuthSubcommand::Token {
                    action: token_action,
                } => match token_action {
                    TokenSubcommand::Create { name, save } => {
                        commands::auth::AuthAction::TokenCreate { name, save }
                    }
                    TokenSubcommand::List { json } => {
                        commands::auth::AuthAction::TokenList { json }
                    }
                    TokenSubcommand::Revoke { id } => {
                        commands::auth::AuthAction::TokenRevoke { id }
                    }
                },
                AuthSubcommand::Logout { revoke_token } => {
                    commands::auth::AuthAction::Logout { revoke_token }
                }
            };
            commands::auth::run(auth_action, profile);
        }
        Commands::Engine {
            action,
            vault,
            auto_approve,
        } => {
            let engine_action = match action {
                EngineSubcommand::Plan {
                    repo,
                    title,
                    description,
                } => commands::engine::EngineAction::Plan {
                    repo,
                    title,
                    description,
                },
                EngineSubcommand::Execute {
                    repo,
                    issue,
                    workdir,
                } => commands::engine::EngineAction::Execute {
                    repo,
                    issue,
                    workdir,
                },
                EngineSubcommand::Approve { repo, pr } => {
                    commands::engine::EngineAction::Approve { repo, pr }
                }
                EngineSubcommand::Auto {
                    repo,
                    title,
                    description,
                    workdir,
                } => commands::engine::EngineAction::Auto {
                    repo,
                    title,
                    description,
                    workdir,
                },
                EngineSubcommand::Query { query } => {
                    commands::engine::EngineAction::Query { query }
                }
                EngineSubcommand::Status => commands::engine::EngineAction::Status,
                EngineSubcommand::Identity { action: id_action } => {
                    commands::engine::run_identity(id_action);
                    return;
                }
            };
            commands::engine::run(engine_action, vault, auto_approve)
        }
    }
}
