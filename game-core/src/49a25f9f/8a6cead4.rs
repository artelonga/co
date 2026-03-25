use crate::engine::error::Result;
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;
use obfstr::obfstr;
use std::collections::HashMap;

const SCHEMA_VERSION: u32 = 3;
const INITIAL_BALANCE: u64 = 10_000;

/// Initialize the database on first run or upgrade schema
pub fn initialize(storage: &Storage) -> Result<()> {
    let meta = storage.get_meta()?;

    match meta {
        None => first_run(storage),
        Some(existing) if existing.schema_version < SCHEMA_VERSION => {
            upgrade(storage, existing.schema_version)
        }
        Some(mut existing) => {
            // Just update last_opened_at
            existing.last_opened_at = Utc::now().timestamp();
            storage.save_meta(&existing)
        }
    }
}

/// First-run: seed all default data
fn first_run(storage: &Storage) -> Result<()> {
    let now = Utc::now().timestamp();

    // Database metadata
    let meta = schema::DbMeta {
        schema_version: SCHEMA_VERSION,
        created_at: now,
        last_opened_at: now,
        app_version: obfstr!("0.1.0").to_string(),
    };
    storage.save_meta(&meta)?;

    // Default user profile
    let user = schema::UserProfile {
        user_id: nanoid::nanoid!(12),
        display_name: String::new(),
        created_at: now,
        last_login_at: now,
        stats: Some(schema::UserStats::default()),
        username: String::new(),
        password_hash: String::new(),
        email: String::new(),
    };
    storage.save_user(&user)?;

    // Initial wallet with seed chips
    let wallet = schema::Wallet {
        user_id: user.user_id.clone(),
        balance: INITIAL_BALANCE,
        last_updated: now,
    };
    storage.save_wallet(&wallet)?;

    // Record initial grant transaction
    let tx = schema::Transaction {
        tx_id: nanoid::nanoid!(10),
        tx_type: schema::TransactionType::TxInitialGrant as i32,
        amount: INITIAL_BALANCE as i64,
        balance_after: INITIAL_BALANCE,
        timestamp: now,
        hand_id: String::new(),
        description: obfstr!("Welcome bonus").to_string(),
    };
    storage.save_transaction(&tx)?;

    // Default poker game config (matches existing GameConfig::default())
    let config = schema::GameConfig {
        width: 60,
        height: 24,
        min_width: 50,
        min_height: 20,
        small_blind: 10,
        big_blind: 20,
        starting_chips: 1000,
        max_players: 6,
        min_players: 1,
        action_timeout: 0,
        ante: 0,
    };
    storage.save_game_config(obfstr!("poker"), &config)?;

    // Default theme
    let mut colors = HashMap::new();
    colors.insert("table_border".to_string(), "bright_blue".to_string());
    colors.insert("table_background".to_string(), "default".to_string());
    colors.insert("pot_color".to_string(), "bright_yellow".to_string());
    colors.insert("player_name".to_string(), "white".to_string());
    colors.insert("local_player_name".to_string(), "bright_green".to_string());
    colors.insert("dealer_marker".to_string(), "bright_yellow".to_string());
    colors.insert("card_red".to_string(), "31".to_string());
    colors.insert("card_black".to_string(), "37".to_string());
    colors.insert("card_bracket".to_string(), "white".to_string());
    colors.insert("card_hidden".to_string(), "bright_black".to_string());
    colors.insert("action_selected_bg".to_string(), "bright_green".to_string());
    colors.insert("action_selected_fg".to_string(), "black".to_string());
    colors.insert("action_available".to_string(), "white".to_string());
    colors.insert("action_unavailable".to_string(), "bright_black".to_string());
    colors.insert("chat_sender".to_string(), "bright_cyan".to_string());
    colors.insert("chat_message".to_string(), "white".to_string());
    colors.insert("chat_system".to_string(), "bright_yellow".to_string());
    colors.insert("status_folded".to_string(), "bright_black".to_string());
    colors.insert("status_allin".to_string(), "bright_red".to_string());
    colors.insert("status_waiting".to_string(), "bright_black".to_string());
    colors.insert("turn_indicator".to_string(), "bright_green".to_string());

    let theme = schema::Theme {
        name: obfstr!("Default").to_string(),
        description: obfstr!("Classic poker table theme").to_string(),
        colors,
    };
    storage.save_theme(obfstr!("default"), &theme)?;

    // Default room config
    let room = schema::RoomConfig {
        name: obfstr!("Default Room").to_string(),
        max_players: 6,
        min_players: 1,
        small_blind: 10,
        big_blind: 20,
        starting_chips: 1000,
        auto_start: true,
        allow_rebuy: false,
        allow_spectators: true,
        chat: Some(schema::ChatSettings {
            enabled: true,
            max_message_length: 280,
        }),
    };
    storage.save_room_config(obfstr!("default"), &room)?;

    Ok(())
}

/// Upgrade schema from old_version to current
fn upgrade(storage: &Storage, old_version: u32) -> Result<()> {
    // v1 -> v2: added sessions and telemetry tables (created by Storage::open)
    if old_version < 2 {
        // Tables already created by Storage::open(), just note the upgrade
    }

    // v2 -> v3: migrate game_stats from single-key (game_name) to compound key (game_name:user_id)
    if old_version < 3 {
        migrate_game_stats_to_compound_keys(storage)?;
    }

    // Update meta with new version
    let now = Utc::now().timestamp();
    let mut meta = match storage.get_meta()? {
        Some(m) => m,
        None => schema::DbMeta {
            schema_version: SCHEMA_VERSION,
            created_at: now,
            last_opened_at: now,
            app_version: obfstr!("0.1.0").to_string(),
        },
    };
    meta.schema_version = SCHEMA_VERSION;
    meta.last_opened_at = now;
    meta.app_version = obfstr!("0.1.0").to_string();
    storage.save_meta(&meta)?;
    Ok(())
}

/// Migrate game_stats entries from single-key (game_name) to compound key (game_name:user_id).
/// Old entries that don't contain ':' are re-keyed under the current default user's ID.
fn migrate_game_stats_to_compound_keys(storage: &Storage) -> Result<()> {
    // Find the current user's ID to tag legacy entries
    let user_id = match storage.get_user()? {
        Some(u) => u.user_id,
        None => return Ok(()), // No user yet, nothing to migrate
    };

    let keys = storage.list_game_stat_keys()?;
    for key in &keys {
        // Skip keys that already have compound format
        if key.contains(':') {
            continue;
        }
        // Read the old entry, re-save with compound key, delete old key
        if let Some(stats) = storage.get_game_stats_raw(key)? {
            storage.save_game_stats(&user_id, &stats)?;
            storage.delete_game_stats_key(key)?;
        }
    }
    Ok(())
}
