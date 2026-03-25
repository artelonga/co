#[path = "ed7002b4.rs"]
mod content;
#[path = "cdb59355.rs"]
mod player;
#[path = "8d846022.rs"]
#[allow(dead_code)]
pub mod types;

use player::CinemaPlayer;

/// Entry point for the "game" subcommand — plays the cinema teaser.
pub fn run_cinema() -> game_core::Result<()> {
    let movie = content::teaser();
    let mut player = CinemaPlayer::new();
    player.play(&movie)
}
