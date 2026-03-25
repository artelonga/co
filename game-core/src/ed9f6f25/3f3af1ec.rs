use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub start_time: DateTime<Utc>,
    pub current_universe: String,
    pub origin_universe: String,
    pub history: Vec<UniverseTransition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UniverseTransition {
    pub from: String,
    pub to: String,
    pub timestamp: DateTime<Utc>,
}

impl Session {
    pub fn new(universe: String) -> Self {
        let now = Utc::now();
        Self {
            id: format!("session-{}", now.timestamp()),
            start_time: now,
            current_universe: universe.clone(),
            origin_universe: universe,
            history: Vec::new(),
        }
    }

    pub fn teleport_to(&mut self, target_universe: String) {
        let transition = UniverseTransition {
            from: self.current_universe.clone(),
            to: target_universe.clone(),
            timestamp: Utc::now(),
        };
        self.history.push(transition);
        self.current_universe = target_universe;
    }

    pub fn get_history(&self) -> &[UniverseTransition] {
        &self.history
    }
}
