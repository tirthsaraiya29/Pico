pub mod budget;
pub mod manager;
pub mod sqlite;

pub use budget::ContextBudgeter;
pub use manager::MemoryManager;
pub use sqlite::SqliteStore;

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::MemoryConfig;
    use pico_core::session::{Message, SessionId};
    use pico_core::traits::MemoryStore;

    #[tokio::test]
    async fn test_in_memory_store_messages() {
        let manager = MemoryManager::in_memory(MemoryConfig::default()).unwrap();
        let sid = SessionId::new();

        let msg1 = Message::new_user(sid, "Hello from test");
        manager.save_message(&msg1).await.unwrap();

        let msg2 = Message::new_assistant(sid, "Hi! I am Pico.", None);
        manager.save_message(&msg2).await.unwrap();

        let history = manager.get_session_history(&sid, 10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Hello from test");
        assert_eq!(history[1].content, "Hi! I am Pico.");
    }

    #[tokio::test]
    async fn test_remember_and_search_fact() {
        let manager = MemoryManager::in_memory(MemoryConfig::default()).unwrap();
        manager
            .remember_fact("user_pref", "User prefers concise answers")
            .await
            .unwrap();

        let facts = manager.search_relevant_context("concise", 5).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("User prefers concise"));
    }
}
