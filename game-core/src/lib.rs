#[path = "ed9f6f25/mod.rs"]
pub mod engine;
#[path = "cd7ac4c6/mod.rs"]
pub mod games;
#[path = "49a25f9f/mod.rs"]
pub mod storage;

pub use engine::{
    at, at_mut, cleanup_terminal, input, poll_input, poll_poker_input, renderer, setup_terminal,
    CellColor, Direction, Entity, EntityType, GameError, Input, InputMode, Interpreter, Map,
    Objective, Portal, Renderer, Result, Rules, Session, SessionConfig, TextBuffer, Tile, Universe,
    Value,
};

pub mod mail;
pub mod plugin;
pub mod universo;

pub use mail::{LogMailProvider, MailProvider};
pub use universo::{Universo, UniversoLocal};

pub use games::{
    create_poker_universe, Game, GameAction, InvadersGame, Lobby, PointSetGame, PokerGame,
    SnakeGame, TetrisGame,
};

pub use plugin::{
    EntityConfig, Plugin, PluginManifest, PluginRegistry, PortalConfig, UniverseConfig,
};
