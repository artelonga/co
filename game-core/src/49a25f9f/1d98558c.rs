use crate::engine::error::Result;
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;

/// Manages persistent session lifecycle in encrypted storage
pub struct SessionManager<'a> {
    storage: &'a Storage,
    record: schema::SessionRecord,
}

impl<'a> SessionManager<'a> {
    /// Start a new session and save immediately (survives crashes)
    pub fn start(storage: &'a Storage, user_id: &str, command: &str) -> Result<Self> {
        let now = Utc::now().timestamp();
        let record = schema::SessionRecord {
            session_id: nanoid::nanoid!(12),
            user_id: user_id.to_string(),
            started_at: now,
            ended_at: 0,
            command: command.to_string(),
            visits: Vec::new(),
            duration_secs: 0,
        };
        storage.save_session(&record)?;
        Ok(Self { storage, record })
    }

    /// Record entering a universe/game
    pub fn enter_universe(&mut self, name: &str) {
        let visit = schema::UniverseVisit {
            universe_name: name.to_string(),
            entered_at: Utc::now().timestamp(),
            exited_at: 0,
            duration_secs: 0,
        };
        self.record.visits.push(visit);
    }

    /// Record exiting the current universe/game
    pub fn exit_universe(&mut self) {
        if let Some(visit) = self.record.visits.last_mut() {
            if visit.exited_at == 0 {
                let now = Utc::now().timestamp();
                visit.exited_at = now;
                let elapsed = now.saturating_sub(visit.entered_at);
                visit.duration_secs = elapsed.max(0) as u64;
            }
        }
    }

    /// Finalize the session and persist to encrypted storage
    pub fn finish(mut self) -> Result<()> {
        let now = Utc::now().timestamp();
        self.record.ended_at = now;
        let elapsed = now.saturating_sub(self.record.started_at);
        self.record.duration_secs = elapsed.max(0) as u64;

        // Close any open universe visit
        if let Some(visit) = self.record.visits.last_mut() {
            if visit.exited_at == 0 {
                visit.exited_at = now;
                let visit_elapsed = now.saturating_sub(visit.entered_at);
                visit.duration_secs = visit_elapsed.max(0) as u64;
            }
        }

        self.storage.save_session(&self.record)?;
        Ok(())
    }

    /// Get the session ID for telemetry correlation
    pub fn session_id(&self) -> &str {
        &self.record.session_id
    }
}
