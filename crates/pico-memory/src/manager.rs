use crate::budget::ContextBudgeter;
use crate::sqlite::SqliteStore;
use async_trait::async_trait;
use pico_core::config::MemoryConfig;
use pico_core::error::Result;
use pico_core::session::{Message, SessionId};
use pico_core::traits::MemoryStore;
use std::sync::Arc;

pub struct MemoryManager {
    store: Arc<SqliteStore>,
    config: MemoryConfig,
}

impl MemoryManager {
    pub fn new(config: MemoryConfig) -> Result<Self> {
        let store = Arc::new(SqliteStore::open(&config.database_path)?);
        Ok(Self { store, config })
    }

    pub fn in_memory(config: MemoryConfig) -> Result<Self> {
        let store = Arc::new(SqliteStore::in_memory()?);
        Ok(Self { store, config })
    }

    /// Retrieves budgeted messages for a session.
    pub async fn get_budgeted_history(&self, session_id: &SessionId) -> Result<Vec<Message>> {
        let all_msgs = self.get_session_history(session_id, 100).await?;
        let budgeted = ContextBudgeter::budget_messages(&all_msgs, self.config.max_history_tokens);
        Ok(budgeted)
    }
}

#[async_trait]
impl MemoryStore for MemoryManager {
    async fn save_message(&self, message: &Message) -> Result<()> {
        let store = self.store.clone();
        let msg = message.clone();
        tokio::task::spawn_blocking(move || store.insert_message(&msg))
            .await
            .map_err(|e| {
                pico_core::error::PicoError::Memory(format!("Save message panicked: {}", e))
            })?
    }

    async fn get_session_history(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let store = self.store.clone();
        let sid = *session_id;
        tokio::task::spawn_blocking(move || store.get_session_messages(&sid, limit))
            .await
            .map_err(|e| {
                pico_core::error::PicoError::Memory(format!("Get history panicked: {}", e))
            })?
    }

    async fn search_relevant_context(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let store = self.store.clone();
        let q = query.to_string();
        tokio::task::spawn_blocking(move || store.search_facts(&q, limit))
            .await
            .map_err(|e| {
                pico_core::error::PicoError::Memory(format!("Search facts panicked: {}", e))
            })?
    }

    async fn remember_fact(&self, category: &str, fact: &str) -> Result<()> {
        let store = self.store.clone();
        let cat = category.to_string();
        let f = fact.to_string();
        tokio::task::spawn_blocking(move || store.insert_fact(&cat, &f))
            .await
            .map_err(|e| {
                pico_core::error::PicoError::Memory(format!("Remember fact panicked: {}", e))
            })?
    }
}
