use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::models::*;

pub struct ExperimentStore {
    base_path: PathBuf,
}

impl ExperimentStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        let base_path = data_dir.as_ref().join("experiment");
        fs::create_dir_all(&base_path).expect("Failed to create experiment directory");
        fs::create_dir_all(base_path.join("feedback"))
            .expect("Failed to create feedback directory");
        Self { base_path }
    }

    pub fn get_or_create_assignment(
        &mut self,
        participant_id: &str,
        default_variant: &str,
    ) -> ExperimentAssignment {
        let assignments = self.load_assignments();
        if let Some(existing) = assignments
            .iter()
            .find(|a| a.participant_id == participant_id)
        {
            return existing.clone();
        }

        let assignment = ExperimentAssignment {
            participant_id: participant_id.to_string(),
            variant: default_variant.to_string(),
            assigned_at: Utc::now(),
        };

        let mut all = assignments;
        all.push(assignment.clone());
        self.save_assignments(&all);
        assignment
    }

    pub fn switch_variant(&mut self, participant_id: &str, variant: &str) -> ExperimentAssignment {
        let mut assignments = self.load_assignments();
        let now = Utc::now();

        if let Some(existing) = assignments
            .iter_mut()
            .find(|a| a.participant_id == participant_id)
        {
            existing.variant = variant.to_string();
            existing.assigned_at = now;
            let result = existing.clone();
            self.save_assignments(&assignments);
            result
        } else {
            let assignment = ExperimentAssignment {
                participant_id: participant_id.to_string(),
                variant: variant.to_string(),
                assigned_at: now,
            };
            assignments.push(assignment.clone());
            self.save_assignments(&assignments);
            assignment
        }
    }

    pub fn submit_feedback(
        &mut self,
        participant_id: &str,
        variant: &str,
        feedback: SubmitFeedback,
    ) -> FeedbackEntry {
        let entry = FeedbackEntry {
            participant_id: participant_id.to_string(),
            variant: variant.to_string(),
            rating: feedback.rating.clamp(1, 5),
            preferred_variant: feedback.preferred_variant,
            comments: feedback.comments,
            custom_palette: feedback.custom_palette,
            timestamp: Utc::now(),
        };

        let id = Uuid::new_v4();
        let path = self.base_path.join("feedback").join(format!("{id}.yaml"));
        let content = serde_yaml::to_string(&entry).unwrap_or_default();
        let _ = fs::write(path, content);

        entry
    }

    pub fn get_summary(&self) -> ExperimentSummary {
        let feedback = self.load_all_feedback();
        let mut by_variant: HashMap<String, Vec<&FeedbackEntry>> = HashMap::new();

        for entry in &feedback {
            by_variant
                .entry(entry.variant.clone())
                .or_default()
                .push(entry);
        }

        let mut variants: Vec<VariantSummary> = by_variant
            .into_iter()
            .map(|(variant, entries)| {
                let feedback_count = entries.len() as u64;
                let avg_rating = if entries.is_empty() {
                    0.0
                } else {
                    entries.iter().map(|e| e.rating as f64).sum::<f64>() / entries.len() as f64
                };
                let preferred_count = entries
                    .iter()
                    .filter(|e| e.preferred_variant == variant)
                    .count() as u64;
                let comments: Vec<String> = entries
                    .iter()
                    .filter(|e| !e.comments.is_empty())
                    .map(|e| e.comments.clone())
                    .collect();

                VariantSummary {
                    variant,
                    feedback_count,
                    avg_rating,
                    preferred_count,
                    comments,
                }
            })
            .collect();

        variants.sort_by(|a, b| a.variant.cmp(&b.variant));

        ExperimentSummary {
            total_feedback: feedback.len() as u64,
            variants,
        }
    }

    fn load_assignments(&self) -> Vec<ExperimentAssignment> {
        let path = self.base_path.join("assignments.yaml");
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        serde_yaml::from_str(&content).unwrap_or_default()
    }

    fn save_assignments(&self, assignments: &[ExperimentAssignment]) {
        let path = self.base_path.join("assignments.yaml");
        let content = serde_yaml::to_string(assignments).unwrap_or_default();
        let _ = fs::write(path, content);
    }

    fn load_all_feedback(&self) -> Vec<FeedbackEntry> {
        let feedback_dir = self.base_path.join("feedback");
        let entries = match fs::read_dir(&feedback_dir) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut feedback = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml")
                && let Ok(content) = fs::read_to_string(&path)
                && let Ok(entry) = serde_yaml::from_str::<FeedbackEntry>(&content)
            {
                feedback.push(entry);
            }
        }
        feedback.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        feedback
    }
}
