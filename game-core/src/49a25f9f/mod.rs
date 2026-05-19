#[path = "da2f073e.rs"]
pub mod crypto;
#[path = "3549b002.rs"]
pub mod database;
#[path = "259aa8ef.rs"]
pub mod history;
#[path = "8a6cead4.rs"]
pub mod migration;
#[path = "df0ad6e4.rs"]
pub mod schema;
#[path = "1d98558c.rs"]
pub mod session_mgr;
#[path = "16091175.rs"]
pub mod telemetry;
#[path = "04f8996d.rs"]
pub mod user;
#[path = "e8d44050.rs"]
pub mod wallet;

pub use database::Storage;
pub use history::HandRecorder;
pub use session_mgr::SessionManager;
pub use wallet::WalletManager;

use crate::engine::error::Result;
use obfstr::obfstr;
use std::path::PathBuf;

/// Get the database file path.
/// Respects `GAME_DB_PATH` env var for test environment isolation.
fn db_path() -> PathBuf {
    if let Ok(custom) = std::env::var("GAME_DB_PATH") {
        let expanded = if custom.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                home.join(custom.strip_prefix("~/").unwrap_or(&custom))
            } else {
                PathBuf::from(custom)
            }
        } else {
            PathBuf::from(custom)
        };
        return expanded;
    }
    let base = match dirs::data_dir() {
        Some(d) => d,
        None => PathBuf::from(obfstr!(".")),
    };
    base.join(obfstr!("game")).join(obfstr!("game.db"))
}

/// Open or create the database, running migrations if needed
pub fn open() -> Result<Storage> {
    let path = db_path();
    let storage = Storage::open(&path)?;
    migration::initialize(&storage)?;
    Ok(storage)
}
