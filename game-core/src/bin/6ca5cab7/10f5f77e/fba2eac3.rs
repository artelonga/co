use game_core::storage::database::Storage;
use game_core::storage::schema;
use game_core::storage::telemetry;
use game_core::storage::SessionManager;
use game_core::Result;
use game_core::{
    cleanup_terminal, create_poker_universe,
    games::poker::{render::poll_input_with_resize, GameConfig, PokerGame},
    setup_terminal, GameAction,
};
use obfstr::obfstr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Run the poker game
#[allow(clippy::too_many_arguments)]
pub fn run_poker(
    storage: Arc<Storage>,
    user: &schema::UserProfile,
    session: &mut SessionManager<'_>,
    room: Option<String>,
    _create_room: bool,
    name: String,
    _redis: Option<String>,
    chips: u32,
    small_blind: u32,
    big_blind: u32,
) -> Result<()> {
    // Record entering poker universe
    session.enter_universe("poker");
    let _ = telemetry::record_game_enter(&storage, &user.user_id, session.session_id(), "poker");

    // Override display name if provided via CLI (not default)
    let player_name = if name != obfstr!("Player") {
        let mut updated_user = user.clone();
        updated_user.display_name = name.clone();
        let _ = storage.save_user(&updated_user);
        name
    } else {
        user.display_name.clone()
    };

    // Load config from encrypted database
    let mut config = GameConfig::from_storage(&storage);

    // CLI arguments override database values
    if small_blind != 10 {
        config.small_blind = small_blind;
    }
    if big_blind != 20 {
        config.big_blind = big_blind;
    }

    // Determine buy-in from wallet
    let wallet_mgr = game_core::storage::WalletManager::new(&storage);
    let wallet_balance = wallet_mgr.get_balance()?;

    let buy_in = if chips != 1000 {
        chips // Explicit CLI override
    } else {
        config.starting_chips.min(wallet_balance as u32)
    };

    if buy_in == 0 {
        eprintln!("Insufficient balance. Your wallet is empty.");
        session.exit_universe();
        let _ = telemetry::record_game_exit(&storage, &user.user_id, session.session_id(), "poker");
        return Ok(());
    }

    // Debit buy-in from wallet
    let actual_buy_in = wallet_mgr.buy_in(&user.user_id, buy_in)?;
    config.starting_chips = actual_buy_in;

    // Create game with storage backend
    let universe = create_poker_universe();
    let mut game = PokerGame::with_config_and_storage(
        universe,
        config,
        player_name,
        storage.clone(),
        user.user_id.clone(),
        actual_buy_in,
    );

    // If room ID provided, show it (for future multiplayer)
    if let Some(room_id) = room {
        game.room_id = room_id;
    }

    // Add AI players for solo testing
    game.add_player(obfstr!("Bot Alice").to_string(), actual_buy_in);
    game.add_player(obfstr!("Bot Bob").to_string(), actual_buy_in);

    // Start first hand
    game.start_hand();

    // Setup terminal
    setup_terminal()?;

    // Game loop
    let result = poker_game_loop(&mut game);

    // Cleanup terminal
    cleanup_terminal()?;

    // Cash out remaining chips
    let remaining = game
        .players
        .get(game.local_player_index)
        .map(|p| p.chips)
        .unwrap_or(0);
    let cash_out_storage = game_core::storage::open()?;
    let cash_out_wallet = game_core::storage::WalletManager::new(&cash_out_storage);
    let _ = cash_out_wallet.cash_out(&user.user_id, remaining, actual_buy_in);

    // Show session summary
    let net = remaining as i64 - actual_buy_in as i64;
    let final_balance = cash_out_wallet.get_balance().unwrap_or(0);
    if net >= 0 {
        eprintln!("\nSession: +${} | Balance: ${}", net, final_balance);
    } else {
        eprintln!("\nSession: -${} | Balance: ${}", -net, final_balance);
    }

    // Record exiting poker universe
    session.exit_universe();
    let _ = telemetry::record_game_exit(&storage, &user.user_id, session.session_id(), "poker");

    result
}

fn poker_game_loop(game: &mut PokerGame) -> Result<()> {
    use game_core::games::poker::render::{render_poker_game_sized, TerminalSize};

    let mut term_size = TerminalSize::current();
    let mut needs_render = true;

    loop {
        if needs_render {
            render_poker_game_sized(game, term_size);
            needs_render = false;
        }

        let (input, new_size) = poll_input_with_resize()?;

        if let Some(size) = new_size {
            term_size = size;
            needs_render = true;
        }

        if !matches!(input, game_core::Input::None) {
            let action = game.tick(input);

            match action {
                GameAction::Quit => break,
                GameAction::Teleport(_) => {
                    break;
                }
                GameAction::Continue => {
                    needs_render = true;

                    if !game.is_local_turn() && !game.game_over {
                        render_poker_game_sized(game, term_size);
                        simulate_ai_turn(game);
                        needs_render = true;
                    }
                }
            }
        }

        thread::sleep(Duration::from_millis(16));
    }

    Ok(())
}

/// Simple AI that makes random decisions
fn simulate_ai_turn(game: &mut PokerGame) {
    use game_core::games::poker::PokerAction;
    use rand::Rng;

    thread::sleep(Duration::from_millis(300));

    let mut rng = rand::thread_rng();
    let action_roll: f32 = rng.gen();

    let current_player = match game.players.get(game.table.action_position) {
        Some(p) => p,
        None => return,
    };
    let needs_to_call = current_player.current_bet < game.table.current_bet;

    let action = if action_roll < 0.1 {
        if needs_to_call && action_roll < 0.15 {
            PokerAction::Fold
        } else {
            PokerAction::Check
        }
    } else if action_roll < 0.5 {
        if needs_to_call {
            PokerAction::Call
        } else {
            PokerAction::Check
        }
    } else if action_roll < 0.85 {
        let raise_amount = game.table.big_blind * rng.gen_range(1..=2);
        PokerAction::Raise(raise_amount)
    } else if action_roll < 0.95 {
        let raise_amount = game.table.big_blind * rng.gen_range(3..=5);
        PokerAction::Raise(raise_amount)
    } else {
        PokerAction::AllIn
    };

    game.execute_action(action);
}
