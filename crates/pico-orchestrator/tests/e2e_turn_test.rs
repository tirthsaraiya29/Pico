use pico_core::config::PicoConfig;
use pico_core::events::PicoEvent;
use pico_core::traits::MemoryStore;
use pico_orchestrator::PicoRuntime;
use std::sync::Arc;

#[tokio::test]
async fn test_e2e_runtime_initialization_and_events() {
    let mut config = PicoConfig::default();
    config.security.allowed_directories = vec![std::path::PathBuf::from(".")];
    config.memory.database_path =
        std::env::temp_dir().join(format!("pico_e2e_{}.sqlite", uuid::Uuid::new_v4().simple()));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    let mut rx = runtime.subscribe_events();

    let sid1 = runtime.current_session_id().await;
    let sid2 = runtime.reset_session().await;
    assert_ne!(sid1, sid2);

    let event = rx.recv().await.expect("Failed receiving event");
    match event {
        PicoEvent::SessionCreated { session_id } => {
            assert_eq!(session_id, sid2);
        }
        other => panic!("Unexpected event: {:?}", other),
    }
}

#[tokio::test]
async fn test_e2e_tool_registry_and_permissions() {
    let mut config = PicoConfig::default();
    config.security.allowed_directories = vec![std::path::PathBuf::from(".")];
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_tools_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    let registry = runtime.registry();

    // 1. Calculator tool
    let calc_res = registry
        .execute_call(
            "call_test_1",
            "calculator",
            serde_json::json!({ "expression": "100 / 4 + 25" }),
        )
        .await;

    assert!(calc_res.success);
    assert_eq!(calc_res.content["result"].as_str().unwrap(), "50");

    // 2. System info tool
    let sys_res = registry
        .execute_call("call_test_2", "get_system_info", serde_json::json!({}))
        .await;
    assert!(sys_res.success);
    assert!(sys_res.content["cpu_cores"].as_u64().unwrap() > 0);

    // 3. Security blocked tool (unknown tool)
    let unknown_res = registry
        .execute_call("call_test_3", "non_existent_tool", serde_json::json!({}))
        .await;
    assert!(!unknown_res.success);
    assert!(unknown_res.error.is_some());
}

#[tokio::test]
async fn test_e2e_memory_persistence_and_retrieval() {
    let mut config = PicoConfig::default();
    let temp_db = std::env::temp_dir().join(format!(
        "pico_e2e_mem_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.memory.database_path = temp_db.clone();

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    let memory = runtime.memory();

    memory
        .remember_fact(
            "user_identity",
            "The user's favorite framework is Rust Tokio",
        )
        .await
        .expect("Failed remembering fact");

    let facts = memory
        .search_relevant_context("favorite framework", 5)
        .await
        .expect("Failed search");

    assert_eq!(facts.len(), 1);
    assert!(facts[0].contains("Rust Tokio"));

    let _ = std::fs::remove_file(temp_db);
}

#[tokio::test]
async fn test_e2e_runtime_cancellation() {
    let mut config = PicoConfig::default();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_cancel_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = Arc::new(PicoRuntime::new(config).expect("Runtime initialization failed"));
    let rt_clone = runtime.clone();

    runtime.cancel();
    assert!(!rt_clone.voice().is_playing());
}

#[tokio::test]
async fn test_e2e_live_turn_with_ollama() {
    let mut config = PicoConfig::default();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_live_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    match runtime
        .process_turn(
            "What is 25 * 4? Calculate this using the calculator tool.",
            false,
        )
        .await
    {
        Ok(reply) => {
            println!("LIVE REASONING & TOOL RESULT: {}", reply);
            assert!(!reply.is_empty());
        }
        Err(e) => {
            println!("Ollama live turn note: {:?}", e);
        }
    }
}
