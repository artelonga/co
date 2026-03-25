use chrono::Utc;
use game_core::engine::error::{GameError, Result};
use game_core::storage::database::Storage;
use game_core::storage::schema;
use obfstr::obfstr;
use std::io::{self, Write};

/// Ensure the user has a display name. Prompts interactively on first run.
/// Must be called BEFORE setup_terminal() (raw mode disables line-buffered stdin).
pub fn ensure_identity(storage: &Storage) -> Result<schema::UserProfile> {
    let mut user = match storage.get_user()? {
        Some(u) => u,
        None => {
            return Err(GameError::Storage(
                obfstr!("User record missing after migration").into(),
            ));
        }
    };

    if user.display_name.is_empty() {
        // First run — prompt for username
        println!();
        println!("{}", obfstr!("Welcome to the Game Engine!"));
        println!();
        print!("{}", obfstr!("Enter your player name: "));
        match io::stdout().flush() {
            Ok(()) => {}
            Err(_) => {
                return Err(GameError::Terminal(
                    obfstr!("Failed to flush stdout").into(),
                ));
            }
        }

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {}
            Err(_) => {
                return Err(GameError::Terminal(obfstr!("Failed to read input").into()));
            }
        }

        let name = input.trim();

        // Validate: non-empty, max 20 chars, no control characters
        let name = if name.is_empty() || name.len() > 20 || name.chars().any(|c| c.is_control()) {
            obfstr!("Player").to_string()
        } else {
            name.to_string()
        };

        user.display_name = name;
        user.last_login_at = Utc::now().timestamp();
        if let Some(ref mut stats) = user.stats {
            stats.sessions_played = 1;
        }
        storage.save_user(&user)?;

        println!();
        println!("{} {}", obfstr!("Welcome,"), &user.display_name);
        println!();
    } else {
        // Returning user — update login timestamp
        user.last_login_at = Utc::now().timestamp();
        if let Some(ref mut stats) = user.stats {
            stats.sessions_played += 1;
        }
        storage.save_user(&user)?;
    }

    Ok(user)
}
