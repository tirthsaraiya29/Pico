use pico_core::error::{PicoError, Result};
use pico_core::session::{Message, Role, SessionId, ToolCall};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::info;
use uuid::Uuid;

pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let conn = Connection::open(p)
            .map_err(|e| PicoError::Memory(format!("Failed to open SQLite database: {}", e)))?;

        // Enable Write-Ahead Logging for concurrency and performance
        conn.execute_batch(
            r#"
            PRAGMA busy_timeout = 5000;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                title TEXT
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls_json TEXT,
                tool_call_id TEXT,
                timestamp TEXT NOT NULL,
                token_count INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                fact TEXT NOT NULL,
                created_at TEXT NOT NULL,
                access_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS preferences (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
            CREATE INDEX IF NOT EXISTS idx_preferences_key ON preferences(key);
            "#,
        )
        .map_err(|e| PicoError::Memory(format!("Failed to initialize SQLite schema: {}", e)))?;

        info!(path = %path.as_ref().display(), "SQLite memory store initialized");

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PicoError::Memory(format!("Failed to open in-memory SQLite: {}", e)))?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                title TEXT
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls_json TEXT,
                tool_call_id TEXT,
                timestamp TEXT NOT NULL,
                token_count INTEGER
            );

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                fact TEXT NOT NULL,
                created_at TEXT NOT NULL,
                access_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS preferences (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| PicoError::Memory(format!("Failed to initialize schema: {}", e)))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert_session(&self, session_id: &SessionId, title: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, title) VALUES (?1, ?2, ?3, ?4)",
            params![session_id.to_string(), now, now, title],
        )
        .map_err(|e| PicoError::Memory(format!("Failed to insert session: {}", e)))?;
        Ok(())
    }

    pub fn insert_message(&self, msg: &Message) -> Result<()> {
        self.insert_session(&msg.session_id, None)?;
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());

        let tool_calls_json = msg
            .tool_calls
            .as_ref()
            .and_then(|tc| serde_json::to_string(tc).ok());

        conn.execute(
            r#"
            INSERT INTO messages (id, session_id, role, content, tool_calls_json, tool_call_id, timestamp, token_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                msg.id.to_string(),
                msg.session_id.to_string(),
                msg.role.to_string(),
                msg.content,
                tool_calls_json,
                msg.tool_call_id,
                msg.timestamp.to_rfc3339(),
                msg.token_count.map(|c| c as i64)
            ],
        )
        .map_err(|e| PicoError::Memory(format!("Failed to insert message: {}", e)))?;

        Ok(())
    }

    pub fn get_session_messages(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, session_id, role, content, tool_calls_json, tool_call_id, timestamp, token_count
            FROM (
                SELECT * FROM messages
                WHERE session_id = ?1
                ORDER BY timestamp DESC
                LIMIT ?2
            )
            ORDER BY timestamp ASC
            "#,
            )
            .map_err(|e| PicoError::Memory(format!("Prepare query failed: {}", e)))?;

        let rows = stmt
            .query_map(params![session_id.to_string(), limit as i64], |row| {
                let id_str: String = row.get(0)?;
                let sid_str: String = row.get(1)?;
                let role_str: String = row.get(2)?;
                let content: String = row.get(3)?;
                let tc_json: Option<String> = row.get(4)?;
                let tool_call_id: Option<String> = row.get(5)?;
                let ts_str: String = row.get(6)?;
                let token_count: Option<i64> = row.get(7)?;

                Ok((
                    id_str,
                    sid_str,
                    role_str,
                    content,
                    tc_json,
                    tool_call_id,
                    ts_str,
                    token_count,
                ))
            })
            .map_err(|e| PicoError::Memory(format!("Query failed: {}", e)))?;

        let mut messages = Vec::new();
        for r in rows.flatten() {
            let id = Uuid::parse_str(&r.0).unwrap_or_else(|_| Uuid::new_v4());
            let sid = SessionId(Uuid::parse_str(&r.1).unwrap_or_else(|_| Uuid::new_v4()));
            let role = match r.2.as_str() {
                "system" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let tool_calls: Option<Vec<ToolCall>> =
                r.4.and_then(|s| serde_json::from_str::<Vec<ToolCall>>(&s).ok());
            let timestamp = chrono::DateTime::parse_from_rfc3339(&r.6)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());

            messages.push(Message {
                id,
                session_id: sid,
                role,
                content: r.3,
                tool_calls,
                tool_call_id: r.5,
                timestamp,
                token_count: r.7.map(|c| c as usize),
            });
        }

        Ok(messages)
    }

    pub fn insert_fact(&self, category: &str, fact: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO memories (id, category, fact, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, category, fact, now],
        )
        .map_err(|e| PicoError::Memory(format!("Failed to insert memory fact: {}", e)))?;

        Ok(())
    }

    pub fn get_all_facts(
        &self,
        category: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cat) = category {
            let mut stmt = conn
                .prepare("SELECT id, category, fact, created_at FROM memories WHERE category = ?1 ORDER BY created_at DESC")
                .map_err(|e| PicoError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map(params![cat], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(|e| PicoError::Memory(e.to_string()))?;
            Ok(rows.flatten().collect())
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, category, fact, created_at FROM memories ORDER BY created_at DESC",
                )
                .map_err(|e| PicoError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(|e| PicoError::Memory(e.to_string()))?;
            Ok(rows.flatten().collect())
        }
    }

    pub fn delete_fact(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let count = conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| PicoError::Memory(format!("Failed to delete fact: {}", e)))?;
        Ok(count > 0)
    }

    pub fn search_facts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        self.search_facts_ranked(query, limit, 0.0)
    }

    /// Searches facts using token-overlap relevance scoring combined with optional
    /// exponential recency decay (`decay_half_life_days > 0.0`).
    pub fn search_facts_ranked(
        &self,
        query: &str,
        limit: usize,
        decay_half_life_days: f32,
    ) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let trimmed = query.trim();
        let lower_q = trimmed.to_lowercase();
        let query_tokens: Vec<String> = lower_q
            .split(|c: char| !c.is_alphanumeric() && c != '%' && c != '_')
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect();

        let escaped = trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{}%", escaped);

        // Fetch candidate rows matching either the full query or all rows when multi-token
        // queries need partial-token relevance ranking.
        let mut stmt = if query_tokens.len() <= 1 {
            conn.prepare(
                "SELECT category, fact, created_at FROM memories \
                 WHERE fact LIKE ?1 ESCAPE '\\' OR category LIKE ?1 ESCAPE '\\'",
            )
            .map_err(|e| PicoError::Memory(format!("Prepare search failed: {}", e)))?
        } else {
            conn.prepare("SELECT category, fact, created_at FROM memories")
                .map_err(|e| PicoError::Memory(format!("Prepare search failed: {}", e)))?
        };

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, String)> {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        };

        let rows = if query_tokens.len() <= 1 {
            stmt.query_map(params![pattern], map_row)
        } else {
            stmt.query_map([], map_row)
        }
        .map_err(|e| PicoError::Memory(format!("Memory search failed: {}", e)))?;

        let now = chrono::Utc::now();
        let mut scored: Vec<(f64, String)> = Vec::new();
        for row in rows {
            let (category, fact, created_at_str) = row.map_err(|e| {
                PicoError::Memory(format!("Row decode failed during search: {}", e))
            })?;
            let fact_lower = fact.to_lowercase();
            let cat_lower = category.to_lowercase();

            let mut relevance = 0.0f64;
            if !lower_q.is_empty() && fact_lower.contains(&lower_q) {
                relevance += 2.0;
            }
            if !lower_q.is_empty() && cat_lower.contains(&lower_q) {
                relevance += 0.75;
            }
            if !query_tokens.is_empty() {
                let matched = query_tokens
                    .iter()
                    .filter(|tok| {
                        fact_lower.contains(tok.as_str()) || cat_lower.contains(tok.as_str())
                    })
                    .count();
                if matched == 0 && relevance == 0.0 {
                    continue;
                }
                relevance += (matched as f64 / query_tokens.len() as f64) * 1.5;
            } else if relevance == 0.0 {
                relevance = 0.1;
            }

            let decay = if decay_half_life_days > 0.0 {
                let age_days = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| {
                        let secs = (now - dt.with_timezone(&chrono::Utc)).num_seconds().max(0);
                        secs as f64 / 86_400.0
                    })
                    .unwrap_or(0.0);
                0.5f64.powf(age_days / decay_half_life_days as f64)
            } else {
                1.0
            };

            scored.push((relevance * decay, fact));
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(_, f)| f).collect())
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![key, value, now],
        )
        .map_err(|e| PicoError::Memory(format!("Failed to set preference: {}", e)))?;
        Ok(())
    }

    pub fn get_preference(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn
            .prepare("SELECT value FROM preferences WHERE key = ?1")
            .map_err(|e| PicoError::Memory(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get(0))
            .map_err(|e| PicoError::Memory(e.to_string()))?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    pub fn list_preferences(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn
            .prepare("SELECT key, value FROM preferences ORDER BY key ASC")
            .map_err(|e| PicoError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| PicoError::Memory(e.to_string()))?;
        Ok(rows.flatten().collect())
    }

    pub fn delete_preference(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let count = conn
            .execute("DELETE FROM preferences WHERE key = ?1", params![key])
            .map_err(|e| PicoError::Memory(format!("Failed to delete preference: {}", e)))?;
        Ok(count > 0)
    }
}
