use crate::engine::error::Result;
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;
use std::collections::HashMap;

/// Record a telemetry event to encrypted storage
pub fn record_event(
    storage: &Storage,
    user_id: &str,
    session_id: &str,
    event_type: i32,
    target: &str,
    metadata: HashMap<String, String>,
) -> Result<()> {
    let event = schema::TelemetryEvent {
        event_id: nanoid::nanoid!(12),
        user_id: user_id.to_string(),
        session_id: session_id.to_string(),
        event_type,
        timestamp: Utc::now().timestamp(),
        target: target.to_string(),
        metadata,
    };
    storage.save_telemetry_event(&event)
}

/// Record a login event
pub fn record_login(storage: &Storage, user_id: &str, session_id: &str) -> Result<()> {
    record_event(
        storage,
        user_id,
        session_id,
        schema::TelemetryEventType::EventLogin as i32,
        "",
        HashMap::new(),
    )
}

/// Record a first-run event
pub fn record_first_run(storage: &Storage, user_id: &str, session_id: &str) -> Result<()> {
    record_event(
        storage,
        user_id,
        session_id,
        schema::TelemetryEventType::EventFirstRun as i32,
        "",
        HashMap::new(),
    )
}

/// Record entering a game/universe
pub fn record_game_enter(
    storage: &Storage,
    user_id: &str,
    session_id: &str,
    game_name: &str,
) -> Result<()> {
    record_event(
        storage,
        user_id,
        session_id,
        schema::TelemetryEventType::EventGameEnter as i32,
        game_name,
        HashMap::new(),
    )
}

/// Record exiting a game/universe
pub fn record_game_exit(
    storage: &Storage,
    user_id: &str,
    session_id: &str,
    game_name: &str,
) -> Result<()> {
    record_event(
        storage,
        user_id,
        session_id,
        schema::TelemetryEventType::EventGameExit as i32,
        game_name,
        HashMap::new(),
    )
}

/// Record session end with duration
pub fn record_session_end(
    storage: &Storage,
    user_id: &str,
    session_id: &str,
    duration_secs: u64,
) -> Result<()> {
    let mut meta = HashMap::new();
    meta.insert("duration_secs".to_string(), duration_secs.to_string());
    record_event(
        storage,
        user_id,
        session_id,
        schema::TelemetryEventType::EventSessionEnd as i32,
        "",
        meta,
    )
}
