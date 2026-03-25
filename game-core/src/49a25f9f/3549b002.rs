use crate::engine::error::{GameError, Result};
use crate::storage::crypto;
use crate::storage::schema;
use obfstr::obfstr;
use prost::Message;
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

const META_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const USER_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("user");
const CONFIG_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("config");
const WALLET_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("wallet");
const HAND_HISTORY_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("hands");
const TRANSACTION_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("transactions");
const THEME_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("themes");
const ROOM_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("rooms");
const SESSION_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("sessions");
const TELEMETRY_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("telemetry");
const GAME_STATS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("game_stats");
const VERIFY_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("verify");
const RATE_LIMIT_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("rate_limit");
const PLUGIN_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("plugins");

pub struct Storage {
    db: Database,
    key: [u8; 32],
}

impl Storage {
    /// Open or create the database at the given path
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    GameError::Storage(format!("Failed to create db directory: {}", e))
                })?;
            }
        }

        let key = crypto::derive_key(path)?;

        let db = Database::create(path)
            .map_err(|e| GameError::Storage(format!("Failed to open database: {}", e)))?;

        // Ensure all tables exist
        let write_txn = db
            .begin_write()
            .map_err(|e| GameError::Storage(format!("Failed to begin write: {}", e)))?;
        {
            let _ = write_txn
                .open_table(META_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(USER_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(CONFIG_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(WALLET_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(HAND_HISTORY_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(TRANSACTION_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(THEME_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(ROOM_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(SESSION_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(TELEMETRY_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(GAME_STATS_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(VERIFY_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(RATE_LIMIT_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
            let _ = write_txn
                .open_table(PLUGIN_TABLE)
                .map_err(|e| GameError::Storage(format!("Failed to create table: {}", e)))?;
        }
        write_txn
            .commit()
            .map_err(|e| GameError::Storage(format!("Failed to commit: {}", e)))?;

        Ok(Self { db, key })
    }

    /// Encrypt and store a protobuf message
    fn put_proto<M: Message>(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
        msg: &M,
    ) -> Result<()> {
        let proto_bytes = msg.encode_to_vec();
        let encrypted = crypto::encrypt(&self.key, &proto_bytes)?;

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| GameError::Storage(format!("Failed to begin write: {}", e)))?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;
            table
                .insert(key, encrypted.as_slice())
                .map_err(|e| GameError::Storage(format!("Failed to insert: {}", e)))?;
        }
        write_txn
            .commit()
            .map_err(|e| GameError::Storage(format!("Failed to commit: {}", e)))?;
        Ok(())
    }

    /// Read and decrypt a protobuf message
    fn get_proto<M: Message + Default>(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Result<Option<M>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| GameError::Storage(format!("Failed to begin read: {}", e)))?;
        let table = read_txn
            .open_table(table_def)
            .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;

        match table.get(key) {
            Ok(Some(guard)) => {
                let encrypted = guard.value();
                let plaintext = crypto::decrypt(&self.key, encrypted)?;
                let msg = M::decode(plaintext.as_slice())
                    .map_err(|e| GameError::Storage(format!("Failed to decode protobuf: {}", e)))?;
                Ok(Some(msg))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(GameError::Storage(format!("Failed to read: {}", e))),
        }
    }

    /// Delete a key from a table
    fn delete_key(&self, table_def: TableDefinition<&str, &[u8]>, key: &str) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| GameError::Storage(format!("Failed to begin write: {}", e)))?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;
            let _ = table.remove(key);
        }
        write_txn
            .commit()
            .map_err(|e| GameError::Storage(format!("Failed to commit: {}", e)))?;
        Ok(())
    }

    /// List all keys in a table
    fn list_keys(&self, table_def: TableDefinition<&str, &[u8]>) -> Result<Vec<String>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| GameError::Storage(format!("Failed to begin read: {}", e)))?;
        let table = read_txn
            .open_table(table_def)
            .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;

        let mut keys = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| GameError::Storage(format!("Failed to iterate: {}", e)))?;
        for entry in iter {
            let (key, _) =
                entry.map_err(|e| GameError::Storage(format!("Failed to read entry: {}", e)))?;
            keys.push(key.value().to_string());
        }
        Ok(keys)
    }

    // ---- Domain-specific convenience methods ----

    pub fn get_meta(&self) -> Result<Option<schema::DbMeta>> {
        self.get_proto(META_TABLE, obfstr!("meta"))
    }

    pub fn save_meta(&self, meta: &schema::DbMeta) -> Result<()> {
        self.put_proto(META_TABLE, obfstr!("meta"), meta)
    }

    pub fn get_user(&self) -> Result<Option<schema::UserProfile>> {
        self.get_proto(USER_TABLE, obfstr!("user"))
    }

    pub fn save_user(&self, user: &schema::UserProfile) -> Result<()> {
        self.put_proto(USER_TABLE, obfstr!("user"), user)
    }

    pub fn get_wallet(&self) -> Result<Option<schema::Wallet>> {
        self.get_proto(WALLET_TABLE, obfstr!("wallet"))
    }

    pub fn save_wallet(&self, wallet: &schema::Wallet) -> Result<()> {
        self.put_proto(WALLET_TABLE, obfstr!("wallet"), wallet)
    }

    pub fn get_game_config(&self, game_name: &str) -> Result<Option<schema::GameConfig>> {
        self.get_proto(CONFIG_TABLE, game_name)
    }

    pub fn save_game_config(&self, game_name: &str, config: &schema::GameConfig) -> Result<()> {
        self.put_proto(CONFIG_TABLE, game_name, config)
    }

    pub fn save_hand(&self, hand: &schema::HandRecord) -> Result<()> {
        self.put_proto(HAND_HISTORY_TABLE, &hand.hand_id, hand)
    }

    pub fn get_hand(&self, hand_id: &str) -> Result<Option<schema::HandRecord>> {
        self.get_proto(HAND_HISTORY_TABLE, hand_id)
    }

    pub fn list_hand_ids(&self) -> Result<Vec<String>> {
        self.list_keys(HAND_HISTORY_TABLE)
    }

    pub fn save_transaction(&self, tx: &schema::Transaction) -> Result<()> {
        self.put_proto(TRANSACTION_TABLE, &tx.tx_id, tx)
    }

    pub fn get_transaction(&self, tx_id: &str) -> Result<Option<schema::Transaction>> {
        self.get_proto(TRANSACTION_TABLE, tx_id)
    }

    pub fn list_transaction_ids(&self) -> Result<Vec<String>> {
        self.list_keys(TRANSACTION_TABLE)
    }

    pub fn get_theme(&self, name: &str) -> Result<Option<schema::Theme>> {
        self.get_proto(THEME_TABLE, name)
    }

    pub fn save_theme(&self, name: &str, theme: &schema::Theme) -> Result<()> {
        self.put_proto(THEME_TABLE, name, theme)
    }

    pub fn get_room_config(&self, name: &str) -> Result<Option<schema::RoomConfig>> {
        self.get_proto(ROOM_TABLE, name)
    }

    pub fn save_room_config(&self, name: &str, config: &schema::RoomConfig) -> Result<()> {
        self.put_proto(ROOM_TABLE, name, config)
    }

    // ---- Session tracking ----

    pub fn save_session(&self, session: &schema::SessionRecord) -> Result<()> {
        self.put_proto(SESSION_TABLE, &session.session_id, session)
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<schema::SessionRecord>> {
        self.get_proto(SESSION_TABLE, session_id)
    }

    pub fn list_session_ids(&self) -> Result<Vec<String>> {
        self.list_keys(SESSION_TABLE)
    }

    // ---- Telemetry events ----

    pub fn save_telemetry_event(&self, event: &schema::TelemetryEvent) -> Result<()> {
        self.put_proto(TELEMETRY_TABLE, &event.event_id, event)
    }

    pub fn get_telemetry_event(&self, event_id: &str) -> Result<Option<schema::TelemetryEvent>> {
        self.get_proto(TELEMETRY_TABLE, event_id)
    }

    pub fn list_telemetry_event_ids(&self) -> Result<Vec<String>> {
        self.list_keys(TELEMETRY_TABLE)
    }

    // ---- Game stats (multi-user, compound key: game_name:user_id) ----

    pub fn get_game_stats(
        &self,
        game_name: &str,
        user_id: &str,
    ) -> Result<Option<schema::GameStats>> {
        let key = format!("{}:{}", game_name, user_id);
        self.get_proto(GAME_STATS_TABLE, &key)
    }

    pub fn save_game_stats(&self, user_id: &str, stats: &schema::GameStats) -> Result<()> {
        let key = format!("{}:{}", stats.game_name, user_id);
        self.put_proto(GAME_STATS_TABLE, &key, stats)
    }

    /// List all stats entries for a given game (all users)
    pub fn list_game_stats_by_game(&self, game_name: &str) -> Result<Vec<schema::GameStats>> {
        let prefix = format!("{}:", game_name);
        self.scan_game_stats(|key| key.starts_with(&prefix))
    }

    /// List all stats entries for a given game with user_ids extracted from compound keys
    pub fn list_game_stats_with_user_ids(
        &self,
        game_name: &str,
    ) -> Result<Vec<(String, schema::GameStats)>> {
        let prefix = format!("{}:", game_name);
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| GameError::Storage(format!("Failed to begin read: {}", e)))?;
        let table = read_txn
            .open_table(GAME_STATS_TABLE)
            .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;

        let mut results = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| GameError::Storage(format!("Failed to iterate: {}", e)))?;
        for entry in iter {
            let (key, value) =
                entry.map_err(|e| GameError::Storage(format!("Failed to read entry: {}", e)))?;
            let key_str = key.value();
            if key_str.starts_with(&prefix) {
                let user_id = key_str[prefix.len()..].to_string();
                let plaintext = crypto::decrypt(&self.key, value.value())?;
                let stats = schema::GameStats::decode(plaintext.as_slice())
                    .map_err(|e| GameError::Storage(format!("Failed to decode protobuf: {}", e)))?;
                results.push((user_id, stats));
            }
        }
        Ok(results)
    }

    /// List all stats entries for a given user (all games)
    pub fn list_game_stats_by_user(&self, user_id: &str) -> Result<Vec<schema::GameStats>> {
        let suffix = format!(":{}", user_id);
        self.scan_game_stats(|key| key.ends_with(&suffix))
    }

    /// Scan game_stats table, returning entries whose key matches the predicate
    fn scan_game_stats<F: Fn(&str) -> bool>(&self, predicate: F) -> Result<Vec<schema::GameStats>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| GameError::Storage(format!("Failed to begin read: {}", e)))?;
        let table = read_txn
            .open_table(GAME_STATS_TABLE)
            .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;

        let mut results = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| GameError::Storage(format!("Failed to iterate: {}", e)))?;
        for entry in iter {
            let (key, value) =
                entry.map_err(|e| GameError::Storage(format!("Failed to read entry: {}", e)))?;
            if predicate(key.value()) {
                let plaintext = crypto::decrypt(&self.key, value.value())?;
                let stats = schema::GameStats::decode(plaintext.as_slice())
                    .map_err(|e| GameError::Storage(format!("Failed to decode protobuf: {}", e)))?;
                results.push(stats);
            }
        }
        Ok(results)
    }

    pub fn list_game_stat_keys(&self) -> Result<Vec<String>> {
        self.list_keys(GAME_STATS_TABLE)
    }

    /// Read a game stats entry by raw key (used during migration)
    pub fn get_game_stats_raw(&self, key: &str) -> Result<Option<schema::GameStats>> {
        self.get_proto(GAME_STATS_TABLE, key)
    }

    /// Delete a game stats entry by raw key (used during migration)
    pub fn delete_game_stats_key(&self, key: &str) -> Result<()> {
        self.delete_key(GAME_STATS_TABLE, key)
    }

    // ---- Multi-user registration support ----

    /// Store a user profile keyed by user_id
    pub fn save_user_by_id(&self, user: &schema::UserProfile) -> Result<()> {
        let key = format!("id:{}", user.user_id);
        self.put_proto(USER_TABLE, &key, user)
    }

    /// Retrieve a user profile by user_id
    pub fn get_user_by_id(&self, user_id: &str) -> Result<Option<schema::UserProfile>> {
        let key = format!("id:{}", user_id);
        self.get_proto(USER_TABLE, &key)
    }

    /// Check if a username is already taken (case-insensitive)
    pub fn get_user_id_by_username(&self, username: &str) -> Result<Option<String>> {
        let key = format!("uname:{}", username.to_lowercase());
        self.get_encrypted_string(USER_TABLE, &key)
    }

    /// Store a username → user_id index entry (case-insensitive)
    pub fn save_username_index(&self, username: &str, user_id: &str) -> Result<()> {
        let key = format!("uname:{}", username.to_lowercase());
        self.put_encrypted_string(USER_TABLE, &key, user_id)
    }

    /// Store a wallet keyed by user_id
    pub fn save_wallet_for_user(&self, user_id: &str, wallet: &schema::Wallet) -> Result<()> {
        let key = format!("wallet:{}", user_id);
        self.put_proto(WALLET_TABLE, &key, wallet)
    }

    /// Retrieve a wallet by user_id
    pub fn get_wallet_for_user(&self, user_id: &str) -> Result<Option<schema::Wallet>> {
        let key = format!("wallet:{}", user_id);
        self.get_proto(WALLET_TABLE, &key)
    }

    // ---- Email index ----

    /// Store an email → user_id index entry (case-insensitive)
    pub fn save_email_index(&self, email: &str, user_id: &str) -> Result<()> {
        let key = format!("email:{}", email.to_lowercase());
        self.put_encrypted_string(USER_TABLE, &key, user_id)
    }

    /// Look up a user_id by email (case-insensitive)
    pub fn get_user_id_by_email(&self, email: &str) -> Result<Option<String>> {
        let key = format!("email:{}", email.to_lowercase());
        self.get_encrypted_string(USER_TABLE, &key)
    }

    // ---- Verification codes ----

    /// Store a JSON-serialized verification code entry keyed by email
    pub fn save_verify_code(&self, email: &str, json: &str) -> Result<()> {
        let key = format!("verify:{}", email.to_lowercase());
        self.put_encrypted_string(VERIFY_TABLE, &key, json)
    }

    /// Retrieve the verification code JSON for an email
    pub fn get_verify_code(&self, email: &str) -> Result<Option<String>> {
        let key = format!("verify:{}", email.to_lowercase());
        self.get_encrypted_string(VERIFY_TABLE, &key)
    }

    /// Delete the verification code for an email
    pub fn delete_verify_code(&self, email: &str) -> Result<()> {
        let key = format!("verify:{}", email.to_lowercase());
        self.delete_key(VERIFY_TABLE, &key)
    }

    // ---- Rate limiting ----

    /// Store a JSON-serialized rate limit entry keyed by email
    pub fn save_rate_limit(&self, email: &str, json: &str) -> Result<()> {
        let key = format!("ratelimit:{}", email.to_lowercase());
        self.put_encrypted_string(RATE_LIMIT_TABLE, &key, json)
    }

    /// Retrieve the rate limit JSON for an email
    pub fn get_rate_limit(&self, email: &str) -> Result<Option<String>> {
        let key = format!("ratelimit:{}", email.to_lowercase());
        self.get_encrypted_string(RATE_LIMIT_TABLE, &key)
    }

    // ---- Plugin universe config storage ----

    /// Store a plugin universe config as JSON, keyed by plugin name
    pub fn save_plugin_config(&self, plugin_name: &str, json: &str) -> Result<()> {
        let key = format!("plugin:{}", plugin_name);
        self.put_encrypted_string(PLUGIN_TABLE, &key, json)
    }

    /// Retrieve a plugin universe config JSON by plugin name
    pub fn get_plugin_config(&self, plugin_name: &str) -> Result<Option<String>> {
        let key = format!("plugin:{}", plugin_name);
        self.get_encrypted_string(PLUGIN_TABLE, &key)
    }

    /// Check if a plugin config already exists in the database
    pub fn has_plugin_config(&self, plugin_name: &str) -> Result<bool> {
        Ok(self.get_plugin_config(plugin_name)?.is_some())
    }

    // ---- Raw encrypted string helpers ----

    fn put_encrypted_string(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let encrypted = crypto::encrypt(&self.key, value.as_bytes())?;

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| GameError::Storage(format!("Failed to begin write: {}", e)))?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;
            table
                .insert(key, encrypted.as_slice())
                .map_err(|e| GameError::Storage(format!("Failed to insert: {}", e)))?;
        }
        write_txn
            .commit()
            .map_err(|e| GameError::Storage(format!("Failed to commit: {}", e)))?;
        Ok(())
    }

    fn get_encrypted_string(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Result<Option<String>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| GameError::Storage(format!("Failed to begin read: {}", e)))?;
        let table = read_txn
            .open_table(table_def)
            .map_err(|e| GameError::Storage(format!("Failed to open table: {}", e)))?;

        match table.get(key) {
            Ok(Some(guard)) => {
                let encrypted = guard.value();
                let plaintext = crypto::decrypt(&self.key, encrypted)?;
                let s = String::from_utf8(plaintext)
                    .map_err(|e| GameError::Storage(format!("Invalid UTF-8: {}", e)))?;
                Ok(Some(s))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(GameError::Storage(format!("Failed to read: {}", e))),
        }
    }
}
