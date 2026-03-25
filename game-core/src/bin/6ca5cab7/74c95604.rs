use crate::cli::Commands;
use crate::commands::{balance, cinema, history_cmd, poker, profile, start, update, vim};
use crate::identity;
use game_core::storage::telemetry;
use game_core::storage::SessionManager;
use game_core::Result;
use std::sync::Arc;

pub fn route(command: Commands) -> Result<()> {
    // Update command skips identity/storage — it just pulls and rebuilds
    if matches!(command, Commands::Update) {
        return update::run_update();
    }

    // Cinema command skips identity/storage — plays the movie
    if matches!(command, Commands::Game) {
        return cinema::run_cinema();
    }

    // Open encrypted storage and run migrations
    let storage = Arc::new(game_core::storage::open()?);

    // Ensure user has been onboarded (prompts on first run)
    let user = identity::ensure_identity(&storage)?;

    // Start a persistent session record
    let command_name = match &command {
        Commands::Start => "start",
        Commands::Vim => "vim",
        Commands::Poker { .. } => "poker",
        Commands::Profile => "profile",
        Commands::Balance => "balance",
        Commands::History { .. } => "history",
        Commands::Update | Commands::Game => unreachable!(),
    };

    let mut session = SessionManager::start(&storage, &user.user_id, command_name)?;

    // Record login telemetry
    let _ = telemetry::record_login(&storage, &user.user_id, session.session_id());

    // Dispatch to command handler
    let result = match command {
        Commands::Start => start::run_start(storage.clone(), &user, &mut session),
        Commands::Vim => vim::run_vim(&storage, &user, &mut session),
        Commands::Poker {
            room,
            create_room,
            name,
            redis,
            chips,
            small_blind,
            big_blind,
        } => poker::run_poker(
            storage.clone(),
            &user,
            &mut session,
            room,
            create_room,
            name,
            redis,
            chips,
            small_blind,
            big_blind,
        ),
        Commands::Profile => profile::run_profile(&storage),
        Commands::Balance => balance::run_balance(&storage),
        Commands::History { limit } => history_cmd::run_history(&storage, limit),
        Commands::Update | Commands::Game => unreachable!(),
    };

    // Record session end telemetry
    let _ = telemetry::record_session_end(
        &storage,
        &user.user_id,
        session.session_id(),
        0, // duration computed by session.finish()
    );

    // Finalize and persist session
    let _ = session.finish();

    result
}
