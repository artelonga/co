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

/// Get the database file path
fn db_path() -> PathBuf {
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
