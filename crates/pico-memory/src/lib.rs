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

    #[tokio::test]
    async fn test_preferences_lifecycle() {
        let manager = MemoryManager::in_memory(MemoryConfig::default()).unwrap();
        manager
            .set_preference("editor".into(), "vscode".into())
            .await
            .unwrap();

        let val = manager.get_preference("editor".into()).await.unwrap();
        assert_eq!(val, Some("vscode".to_string()));

        let list = manager.list_preferences().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], ("editor".to_string(), "vscode".to_string()));

        let deleted = manager.delete_preference("editor".into()).await.unwrap();
        assert!(deleted);
        let val_after = manager.get_preference("editor".into()).await.unwrap();
        assert_eq!(val_after, None);
    }

    #[tokio::test]
    async fn test_facts_lifecycle() {
        let manager = MemoryManager::in_memory(MemoryConfig::default()).unwrap();
        manager
            .remember_fact("system", "Target OS is Linux")
            .await
            .unwrap();

        let all = manager.list_facts(None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, "system");
        assert_eq!(all[0].2, "Target OS is Linux");

        let id = all[0].0.clone();
        let deleted = manager.delete_fact(id).await.unwrap();
        assert!(deleted);

        let remaining = manager.list_facts(None).await.unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn test_search_facts_escapes_like_wildcards() {
        let manager = MemoryManager::in_memory(MemoryConfig::default()).unwrap();
        manager
            .remember_fact("metric", "CPU usage hit 95%")
            .await
            .unwrap();
        manager
            .remember_fact("secret", "unrelated memory entry")
            .await
            .unwrap();

        let percent_hits = manager.search_relevant_context("%", 10).await.unwrap();
        assert_eq!(percent_hits.len(), 1);
        assert_eq!(percent_hits[0], "CPU usage hit 95%");

        let underscore_hits = manager.search_relevant_context("a_b", 10).await.unwrap();
        assert!(underscore_hits.is_empty());
    }
}
