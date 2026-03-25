#[path = "63e3a0b8.rs"]
pub mod invaders;
#[path = "4b5dc076.rs"]
pub mod lobby;
#[path = "4c375e18.rs"]
pub mod pointset;
#[path = "fba2eac3/mod.rs"]
pub mod poker;
#[path = "538d7d9f.rs"]
pub mod snake;
#[path = "a10335a2.rs"]
pub mod tetris;

use crate::engine::{Input, Map, Session, Universe};

pub use invaders::InvadersGame;
pub use lobby::Lobby;
pub use pointset::PointSetGame;
pub use poker::{create_poker_universe, PokerGame};
pub use snake::SnakeGame;
pub use tetris::TetrisGame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameAction {
    Continue,
    Teleport(String),
    Quit,
}

pub trait Game {
    fn new(universe: Universe) -> Self
    where
        Self: Sized;

    fn tick(&mut self, input: Input) -> GameAction;

    fn render(&self) -> Map;

    fn get_session(&self) -> &Session;

    fn get_universe(&self) -> &Universe;
}
