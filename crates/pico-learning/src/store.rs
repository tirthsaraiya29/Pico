//! Persistent SQLite store for experience datasets and model checkpoints.

use crate::observation::{LabelQuality, ObservationRecord};
use chrono::{DateTime, Utc};
use pico_core::error::{PicoError, Result};
use pico_inference::ranking::RankingWeights;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCheckpoint {
    pub version: String,
    pub weights: RankingWeights,
    pub top1_accuracy: f32,
    pub top3_recall: f32,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}

pub struct ExperienceStore {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl ExperienceStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PicoError::Database(format!("Failed to create DB directory: {}", e))
            })?;
        }

        let conn = Connection::open(path)
            .map_err(|e| PicoError::Database(format!("Failed to open SQLite DB: {}", e)))?;

        // Initialize pragmas for reliability and concurrency
        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| PicoError::Database(format!("Failed to set PRAGMA: {}", e)))?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: path.to_path_buf(),
        };

        store.migrate()?;
        Ok(store)
    }

    /// In-memory database helper for testing.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PicoError::Database(format!("Failed to open in-memory SQLite: {}", e)))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: PathBuf::from(":memory:"),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Returns the filesystem path to the database file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS observations (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                request_text TEXT NOT NULL,
                primary_intent TEXT NOT NULL,
                candidate_names TEXT NOT NULL,
                candidate_scores TEXT NOT NULL,
                selected_action TEXT NOT NULL,
                confidence REAL NOT NULL,
                execution_status TEXT NOT NULL,
                latency_ms INTEGER NOT NULL,
                user_feedback INTEGER,
                corrected_action TEXT,
                quality INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_obs_timestamp ON observations(timestamp);
            CREATE INDEX IF NOT EXISTS idx_obs_quality ON observations(quality);
            CREATE INDEX IF NOT EXISTS idx_obs_action ON observations(selected_action);

            CREATE TABLE IF NOT EXISTS model_checkpoints (
                version TEXT PRIMARY KEY,
                weights_json TEXT NOT NULL,
                top1_accuracy REAL NOT NULL,
                top3_recall REAL NOT NULL,
                created_at TEXT NOT NULL,
                is_active INTEGER NOT NULL
            );",
        )
        .map_err(|e| PicoError::Database(format!("Failed to run migrations: {}", e)))?;

        Ok(())
    }

    /// Inserts a newly observed decision record.
    pub fn insert_observation(&self, record: &ObservationRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let cand_names = serde_json::to_string(&record.candidate_names).unwrap_or_default();
        let cand_scores = serde_json::to_string(&record.candidate_scores).unwrap_or_default();

        conn.execute(
            "INSERT INTO observations (
                id, timestamp, request_text, primary_intent, candidate_names,
                candidate_scores, selected_action, confidence, execution_status,
                latency_ms, user_feedback, corrected_action, quality
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.id,
                record.timestamp.to_rfc3339(),
                record.request_text,
                record.primary_intent,
                cand_names,
                cand_scores,
                record.selected_action,
                record.confidence,
                record.execution_status,
                record.latency_ms as i64,
                record.user_feedback,
                record.corrected_action,
                record.quality as i32,
            ],
        )
        .map_err(|e| PicoError::Database(format!("Failed to insert observation: {}", e)))?;

        Ok(())
    }

    /// Updates user feedback for a specific observation.
    pub fn update_feedback(&self, observation_id: &str, feedback: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE observations SET user_feedback = ?1, quality = MAX(quality, ?2) WHERE id = ?3",
            params![
                feedback,
                LabelQuality::ExplicitFeedback as i32,
                observation_id
            ],
        )
        .map_err(|e| PicoError::Database(format!("Failed to update feedback: {}", e)))?;
        Ok(())
    }

    /// Records an explicit user correction against an observation or the latest decision.
    pub fn record_correction(&self, observation_id: &str, corrected_action: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE observations SET corrected_action = ?1, quality = ?2 WHERE id = ?3",
            params![
                corrected_action,
                LabelQuality::UserCorrection as i32,
                observation_id
            ],
        )
        .map_err(|e| PicoError::Database(format!("Failed to record correction: {}", e)))?;
        Ok(())
    }

    /// Retrieves all observations above or equal to a specified minimum quality tier.
    pub fn get_training_observations(
        &self,
        min_quality: LabelQuality,
        limit: usize,
    ) -> Result<Vec<ObservationRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, request_text, primary_intent, candidate_names,
                        candidate_scores, selected_action, confidence, execution_status,
                        latency_ms, user_feedback, corrected_action, quality
                 FROM observations
                 WHERE quality >= ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| PicoError::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(params![min_quality as i32, limit as i64], |row| {
                let ts_str: String = row.get(1)?;
                let ts = DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let names_json: String = row.get(4)?;
                let scores_json: String = row.get(5)?;
                let q_int: i32 = row.get(12)?;

                let quality = match q_int {
                    3 => LabelQuality::UserCorrection,
                    2 => LabelQuality::ExplicitFeedback,
                    1 => LabelQuality::ExecutionOutcome,
                    _ => LabelQuality::Inferred,
                };

                Ok(ObservationRecord {
                    id: row.get(0)?,
                    timestamp: ts,
                    request_text: row.get(2)?,
                    primary_intent: row.get(3)?,
                    candidate_names: serde_json::from_str(&names_json).unwrap_or_default(),
                    candidate_scores: serde_json::from_str(&scores_json).unwrap_or_default(),
                    selected_action: row.get(6)?,
                    confidence: row.get(7)?,
                    execution_status: row.get(8)?,
                    latency_ms: row.get::<_, i64>(9)? as u64,
                    user_feedback: row.get(10)?,
                    corrected_action: row.get(11)?,
                    quality,
                })
            })
            .map_err(|e| PicoError::Database(format!("Query failed: {}", e)))?;

        let mut results = Vec::new();
        for obs in rows.flatten() {
            results.push(obs);
        }
        Ok(results)
    }

    /// Saves a model checkpoint.
    pub fn save_checkpoint(&self, checkpoint: &ModelCheckpoint) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let weights_json = serde_json::to_string(&checkpoint.weights).unwrap_or_default();

        if checkpoint.is_active {
            conn.execute("UPDATE model_checkpoints SET is_active = 0", [])
                .map_err(|e| {
                    PicoError::Database(format!("Failed to clear active checkpoint: {}", e))
                })?;
        }

        conn.execute(
            "INSERT OR REPLACE INTO model_checkpoints (
                version, weights_json, top1_accuracy, top3_recall, created_at, is_active
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                checkpoint.version,
                weights_json,
                checkpoint.top1_accuracy,
                checkpoint.top3_recall,
                checkpoint.created_at.to_rfc3339(),
                if checkpoint.is_active { 1 } else { 0 },
            ],
        )
        .map_err(|e| PicoError::Database(format!("Failed to save checkpoint: {}", e)))?;

        info!("Persisted model checkpoint '{}'", checkpoint.version);
        Ok(())
    }

    /// Retrieves active checkpoint.
    pub fn get_active_checkpoint(&self) -> Result<Option<ModelCheckpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT version, weights_json, top1_accuracy, top3_recall, created_at, is_active FROM model_checkpoints WHERE is_active = 1 LIMIT 1")
            .map_err(|e| PicoError::Database(format!("Failed to prepare query: {}", e)))?;

        let mut rows = stmt
            .query_map([], |row| {
                let weights_json: String = row.get(1)?;
                let ts_str: String = row.get(4)?;
                let ts = DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(ModelCheckpoint {
                    version: row.get(0)?,
                    weights: serde_json::from_str(&weights_json).unwrap_or_default(),
                    top1_accuracy: row.get(2)?,
                    top3_recall: row.get(3)?,
                    created_at: ts,
                    is_active: row.get::<_, i32>(5)? == 1,
                })
            })
            .map_err(|e| PicoError::Database(format!("Query failed: {}", e)))?;

        if let Some(res) = rows.next() {
            Ok(Some(res.map_err(|e| PicoError::Database(e.to_string()))?))
        } else {
            Ok(None)
        }
    }

    /// Returns all saved checkpoints ordered by creation date descending.
    pub fn list_checkpoints(&self) -> Result<Vec<ModelCheckpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT version, weights_json, top1_accuracy, top3_recall, created_at, is_active FROM model_checkpoints ORDER BY created_at DESC")
            .map_err(|e| PicoError::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let weights_json: String = row.get(1)?;
                let ts_str: String = row.get(4)?;
                let ts = DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(ModelCheckpoint {
                    version: row.get(0)?,
                    weights: serde_json::from_str(&weights_json).unwrap_or_default(),
                    top1_accuracy: row.get(2)?,
                    top3_recall: row.get(3)?,
                    created_at: ts,
                    is_active: row.get::<_, i32>(5)? == 1,
                })
            })
            .map_err(|e| PicoError::Database(format!("Query failed: {}", e)))?;

        let mut list = Vec::new();
        for c in rows.flatten() {
            list.push(c);
        }
        Ok(list)
    }

    /// Roll back to a specific checkpoint or the previous checkpoint.
    pub fn rollback_checkpoint(&self, target_version: Option<&str>) -> Result<ModelCheckpoint> {
        let checkpoints = self.list_checkpoints()?;
        if checkpoints.is_empty() {
            return Err(PicoError::Internal(
                "No model checkpoints available for rollback".into(),
            ));
        }

        let target = if let Some(v) = target_version {
            checkpoints
                .into_iter()
                .find(|c| c.version == v)
                .ok_or_else(|| PicoError::NotFound(format!("Model checkpoint '{}' not found", v)))?
        } else {
            // Find the most recent non-active checkpoint
            checkpoints
                .into_iter()
                .find(|c| !c.is_active)
                .ok_or_else(|| {
                    PicoError::Internal("No prior checkpoint found for rollback".into())
                })?
        };

        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE model_checkpoints SET is_active = 0", [])
            .map_err(|e| PicoError::Database(format!("Failed to clear active: {}", e)))?;
        conn.execute(
            "UPDATE model_checkpoints SET is_active = 1 WHERE version = ?1",
            params![target.version],
        )
        .map_err(|e| PicoError::Database(format!("Failed to activate target checkpoint: {}", e)))?;

        info!("Rolled back model to checkpoint '{}'", target.version);
        Ok(target)
    }

    /// Returns total count of recorded observations.
    pub fn count_observations(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0))
            .map_err(|e| PicoError::Database(format!("Failed to count observations: {}", e)))?;
        Ok(count as usize)
    }

    /// Clears all observations while preserving model checkpoints.
    pub fn clear_observations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM observations", [])
            .map_err(|e| PicoError::Database(format!("Failed to clear observations: {}", e)))?;
        Ok(())
    }
}
