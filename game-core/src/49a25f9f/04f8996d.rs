use crate::engine::error::Result;
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;

/// Get existing user or create a new one on first run
pub fn get_or_create_user(storage: &Storage) -> Result<schema::UserProfile> {
    match storage.get_user()? {
        Some(mut user) => {
            user.last_login_at = Utc::now().timestamp();
            if let Some(ref mut stats) = user.stats {
                stats.sessions_played += 1;
            }
            storage.save_user(&user)?;
            Ok(user)
        }
        None => {
            let now = Utc::now().timestamp();
            let user = schema::UserProfile {
                user_id: nanoid::nanoid!(12),
                display_name: String::new(),
                created_at: now,
                last_login_at: now,
                stats: Some(schema::UserStats {
                    sessions_played: 1,
                    ..Default::default()
                }),
                username: String::new(),
                password_hash: String::new(),
                email: String::new(),
            };
            storage.save_user(&user)?;
            Ok(user)
        }
    }
}

/// Update user stats after a completed hand
pub fn update_stats_after_hand(
    storage: &Storage,
    user: &mut schema::UserProfile,
    won: bool,
    pot: u32,
    hand_rank: u32,
) -> Result<()> {
    let stats = match user.stats.as_mut() {
        Some(s) => s,
        None => {
            user.stats = Some(schema::UserStats::default());
            match user.stats.as_mut() {
                Some(s) => s,
                None => return Ok(()),
            }
        }
    };

    stats.hands_played += 1;

    if won {
        stats.hands_won += 1;
        stats.total_chips_won += pot as u64;
        if pot as u64 > stats.biggest_pot_won {
            stats.biggest_pot_won = pot as u64;
        }
        if hand_rank > stats.best_hand_rank {
            stats.best_hand_rank = hand_rank;
        }
    } else {
        stats.total_chips_lost += pot as u64;
    }

    storage.save_user(user)?;
    Ok(())
}
