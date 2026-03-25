use crossterm::terminal;
use game_core::storage::database::Storage;
use game_core::storage::schema;
use game_core::storage::telemetry;
use game_core::storage::SessionManager;
use game_core::Result;
use game_core::{
    cleanup_terminal, poll_input, setup_terminal, Game, GameAction, Lobby, Renderer, Universe,
};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn run_start(
    storage: Arc<Storage>,
    user: &schema::UserProfile,
    session: &mut SessionManager<'_>,
) -> Result<()> {
    setup_terminal()?;

    let result = game_loop(&storage, user, session);

    cleanup_terminal()?;

    result
}

fn game_loop(
    storage: &Storage,
    user: &schema::UserProfile,
    session: &mut SessionManager<'_>,
) -> Result<()> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width as usize;
    let height = height as usize;

    let mut renderer = Renderer::new(width, height);
    let mut current_universe = "game".to_string();

    // Record entering the lobby
    session.enter_universe(&current_universe);
    let _ = telemetry::record_game_enter(
        storage,
        &user.user_id,
        session.session_id(),
        &current_universe,
    );

    loop {
        let mut current_game: Box<dyn Game> = match current_universe.as_str() {
            "pointset" => {
                let mut u = Universe::pointset();
                u.map = create_map(width, height);
                Box::new(game_core::PointSetGame::new(u))
            }
            "tetris" => {
                let mut u = Universe::tetris();
                u.map = create_map(width, height);
                Box::new(game_core::TetrisGame::new(u))
            }
            "snake" => {
                let mut u = Universe::snake();
                u.map = create_map(width, height);
                Box::new(game_core::SnakeGame::new(u))
            }
            "invaders" => {
                let mut u = Universe::invaders();
                u.map = create_map(width, height);
                Box::new(game_core::InvadersGame::new(u))
            }
            _ => {
                let mut u = Universe::lobby();
                u.map = create_map(width, height);
                Box::new(Lobby::new(u))
            }
        };

        let mut dirty = true;
        loop {
            let input = poll_input()?;

            if !matches!(input, game_core::Input::None) {
                let action = current_game.tick(input);

                match action {
                    GameAction::Continue => {
                        dirty = true;
                    }
                    GameAction::Teleport(next_universe) => {
                        // Record universe transition
                        session.exit_universe();
                        let _ = telemetry::record_game_exit(
                            storage,
                            &user.user_id,
                            session.session_id(),
                            &current_universe,
                        );

                        current_universe = next_universe;

                        session.enter_universe(&current_universe);
                        let _ = telemetry::record_game_enter(
                            storage,
                            &user.user_id,
                            session.session_id(),
                            &current_universe,
                        );
                        break;
                    }
                    GameAction::Quit => {
                        session.exit_universe();
                        let _ = telemetry::record_game_exit(
                            storage,
                            &user.user_id,
                            session.session_id(),
                            &current_universe,
                        );
                        return Ok(());
                    }
                }
            }

            if dirty {
                let map = current_game.render();
                renderer.draw_map(&map);
                renderer.flush()?;
                dirty = false;
            }

            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn create_map(width: usize, height: usize) -> game_core::Map {
    game_core::Map::new(width, height)
}
