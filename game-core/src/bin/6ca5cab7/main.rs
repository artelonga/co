#[path = "99bb8840.rs"]
mod cli;
#[path = "10f5f77e/mod.rs"]
mod commands;
#[path = "689f6a62.rs"]
mod identity;
#[path = "74c95604.rs"]
mod router;

use game_core::Result;

fn main() -> Result<()> {
    let matches = cli::build_cli().get_matches();
    let command = cli::parse_commands(matches);
    router::route(command)
}
