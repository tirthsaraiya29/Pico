use pico_core::config::PicoConfig;
use pico_core::events::PicoEvent;
use pico_core::traits::MemoryStore;
use pico_orchestrator::PicoRuntime;
use std::sync::Arc;

#[tokio::test]
async fn test_e2e_runtime_initialization_and_events() {
    let mut config = PicoConfig::default();
    config.security.allowed_directories = vec![std::path::PathBuf::from(".")];
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path =
        std::env::temp_dir().join(format!("pico_e2e_{}.sqlite", uuid::Uuid::new_v4().simple()));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_l1_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    let mut rx = runtime.subscribe_events();

    let sid1 = runtime.current_session_id().await;
    let sid2 = runtime.reset_session().await;
    assert_ne!(sid1, sid2);

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout waiting for event")
        .expect("Failed receiving event");
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
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_tools_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_l2_{}.sqlite",
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
    config.security.audit_log_path = std::path::PathBuf::new();
    let temp_db = std::env::temp_dir().join(format!(
        "pico_e2e_mem_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.memory.database_path = temp_db.clone();
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_l3_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

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
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_cancel_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_l4_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = Arc::new(PicoRuntime::new(config).expect("Runtime initialization failed"));
    let rt_clone = runtime.clone();

    runtime.cancel();
    assert!(!rt_clone.voice().is_playing());
}

#[tokio::test]
async fn test_e2e_probabilistic_turn_and_learning() {
    let mut config = PicoConfig::default();
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_decision_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_learn_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");

    // 1. Test probabilistic calculation decision and execution
    let reply = runtime
        .process_turn("Calculate 25 * 4", false)
        .await
        .expect("Turn processing failed");

    assert!(
        reply.contains("100"),
        "Expected 100 in reply, got: {}",
        reply
    );

    // 2. Test providing feedback
    let fb_res = runtime.provide_feedback(1).await;
    assert!(fb_res.is_ok());

    // 3. Test direct conversation
    let direct_reply = runtime
        .process_turn("Hello, who are you?", false)
        .await
        .expect("Direct turn failed");
    assert!(
        direct_reply.contains("Pico")
            || direct_reply.contains("Hello")
            || direct_reply.contains("assist")
    );
}

#[tokio::test]
async fn test_e2e_multistep_autonomous_execution() {
    let mut config = PicoConfig::default();
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_multi_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_multi_learn_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");

    // Compound multi-step request: evaluate two distinct actions
    let reply = runtime
        .process_turn("Calculate 10 + 20 and then Calculate 50 * 2", false)
        .await
        .expect("Multi-step turn processing failed");

    assert!(reply.contains("30"), "Expected 30 in reply, got: {}", reply);
    assert!(
        reply.contains("100"),
        "Expected 100 in reply, got: {}",
        reply
    );
}

#[tokio::test]
async fn test_e2e_loop_detection() {
    let mut config = PicoConfig::default();
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_loop_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_loop_learn_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");

    // Execute repeating identical command in sequence
    let reply = runtime
        .process_turn("Calculate 40 + 2 and then Calculate 40 + 2", false)
        .await
        .expect("Turn processing failed");

    assert!(
        reply.contains("Execution halted: action 'calculator' was already performed"),
        "Expected loop detection warning, got: {}",
        reply
    );
}

#[tokio::test]
async fn test_e2e_intent_routing_regressions_safe() {
    use async_trait::async_trait;
    use pico_security::ConfirmationProvider;

    struct DenyConfirm;
    #[async_trait]
    impl ConfirmationProvider for DenyConfirm {
        async fn request_confirmation(
            &self,
            _request_id: &str,
            _tool_name: &str,
            _arguments: &serde_json::Value,
        ) -> pico_core::error::Result<bool> {
            Ok(false)
        }
    }

    let mut config = PicoConfig::default();
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_regress_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_regress_l_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");
    runtime.set_confirmation_provider(Arc::new(DenyConfirm)).await;

    // 1. "Set a timer for 30 seconds" MUST NOT be hijacked by substring "second" into DirectConversation
    let reply_timer = runtime
        .process_turn("Set a timer for 30 seconds", false)
        .await
        .expect("Timer turn failed");
    assert!(
        reply_timer.contains("30") && reply_timer.to_lowercase().contains("timer"),
        "Expected active timer reply for 30 seconds, got: {}",
        reply_timer
    );

    // 2. "Close brave" MUST NOT route to open_application or report "Opened Close brave"
    let reply_close = runtime
        .process_turn("Close brave", false)
        .await
        .expect("Close brave turn failed");
    assert!(
        !reply_close.to_lowercase().contains("opened"),
        "Must never report opening when asked to close: {}",
        reply_close
    );

    // 3. "open nonexistent_app_xyz_999" MUST fail honestly, never report "Opened ..."
    let reply_missing_app = runtime
        .process_turn("open nonexistent_app_xyz_999", false)
        .await
        .expect("Missing app turn failed");
    assert!(
        !reply_missing_app.to_lowercase().starts_with("opened nonexistent"),
        "Unknown app must fail rather than claiming success: {}",
        reply_missing_app
    );

    // 4. "increase brightness" MUST route cleanly to display.set_brightness, never windows.display.create_monitor
    let dec_bright = runtime
        .decision()
        .decide("increase brightness")
        .await
        .expect("Brightness decision failed");
    match dec_bright {
        pico_decision::DecisionResult::ActionSelected { action_name, .. } => {
            assert_eq!(action_name, "display.set_brightness");
        }
        other => panic!("Expected ActionSelected display.set_brightness, got {:?}", other),
    }

    // 5. "Mute sounds" should select audio.mute deterministically
    let dec_mute = runtime
        .decision()
        .decide("Mute sounds")
        .await
        .expect("Mute decision failed");
    match dec_mute {
        pico_decision::DecisionResult::ActionSelected { action_name, .. } => {
            assert_eq!(action_name, "audio.mute");
        }
        other => panic!("Expected ActionSelected audio.mute, got {:?}", other),
    }
}

#[tokio::test]
async fn test_e2e_session_scoped_clarification_all_paths() {
    use pico_core::session::{ClarificationKind, ClarificationOption, ClarificationState, SessionId};
    use serde_json::json;

    let mut config = PicoConfig::default();
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_clarify_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_e2e_clarify_learn_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    let runtime = PicoRuntime::new(config).expect("Runtime initialization failed");

    // 1. Standalone "first" with NO pending clarification stays safe
    let reply_alone = runtime.process_turn("first", false).await.expect("Turn failed");
    assert!(!reply_alone.contains("read_text_head"));

    // 2. Inject realistic two-candidate clarification across two isolated sessions (calc vs sysinfo)
    let make_state = |sid: SessionId| {
        ClarificationState::new(
            sid,
            "run quick math or specs check",
            ClarificationKind::CandidateChoice,
            vec![
                ClarificationOption {
                    action_name: "calculator".into(),
                    description: "Evaluate mathematical expression 7 * 9".into(),
                    probability: 0.45,
                    selection_probability: 0.51,
                    parameters_schema: json!({
                        "type": "object",
                        "properties": { "expression": { "type": "string" } },
                        "required": ["expression"]
                    }),
                    pre_extracted_args: json!({ "expression": "7 * 9" }),
                    missing_required: vec![],
                },
                ClarificationOption {
                    action_name: "get_system_info".into(),
                    description: "Read current system hardware specs and RAM".into(),
                    probability: 0.43,
                    selection_probability: 0.49,
                    parameters_schema: json!({ "type": "object", "properties": {} }),
                    pre_extracted_args: json!({}),
                    missing_required: vec![],
                },
            ],
            120,
        )
    };

    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    runtime
        .set_session_clarification(sid_a, Some(make_state(sid_a)))
        .await;
    runtime
        .set_session_clarification(sid_b, Some(make_state(sid_b)))
        .await;

    // Session A replies unclear filler first -> preserves clarification state
    let unclear_a = runtime
        .process_turn_in_session(sid_a, "hmm wait", false)
        .await
        .expect("Unclear reply failed");
    assert!(unclear_a.to_lowercase().contains("first"));
    assert!(runtime.has_pending_clarification(sid_a).await);

    // Session B resolves "execute the first" -> executes calculator (63)
    let reply_b = runtime
        .process_turn_in_session(sid_b, "execute the first", false)
        .await
        .expect("Session B clarification failed");
    assert!(
        reply_b.contains("63"),
        "Session B should execute first candidate calculator (63), got: {}",
        reply_b
    );
    assert!(!runtime.has_pending_clarification(sid_b).await);
    assert!(runtime.has_pending_clarification(sid_a).await);

    // Session A chooses "the second one" -> executes get_system_info cleanly
    let reply_a = runtime
        .process_turn_in_session(sid_a, "the second one", false)
        .await
        .expect("Session A clarification failed");
    assert!(
        reply_a.contains("CPU cores") || reply_a.contains("RAM"),
        "Session A should execute second candidate get_system_info, got: {}",
        reply_a
    );
    assert!(!runtime.has_pending_clarification(sid_a).await);

    // 3. Test "option two"
    let sid_c = SessionId::new();
    runtime
        .set_session_clarification(sid_c, Some(make_state(sid_c)))
        .await;
    let reply_c = runtime
        .process_turn_in_session(sid_c, "option two", false)
        .await
        .expect("Option two failed");
    assert!(reply_c.contains("RAM") || reply_c.contains("CPU cores"));

    // 4. Test MissingArguments multi-turn slot fill
    let sid_d = SessionId::new();
    let missing_calc_state = ClarificationState::new(
        sid_d,
        "calculate",
        ClarificationKind::MissingArguments {
            action_name: "calculator".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": { "expression": { "type": "string" } },
                "required": ["expression"]
            }),
            partial_arguments: json!({}),
            missing_fields: vec!["expression".into()],
        },
        vec![],
        120,
    );
    runtime
        .set_session_clarification(sid_d, Some(missing_calc_state))
        .await;
    let reply_slot = runtime
        .process_turn_in_session(sid_d, "12 * 12", false)
        .await
        .expect("Missing args slot fill failed");
    assert!(
        reply_slot.contains("144"),
        "Expected 144 after providing missing expression, got: {}",
        reply_slot
    );

    // 5. Test Interruption by new unrelated command
    let sid_e = SessionId::new();
    runtime
        .set_session_clarification(sid_e, Some(make_state(sid_e)))
        .await;
    let reply_interrupt = runtime
        .process_turn_in_session(sid_e, "actually calculate 11 * 11", false)
        .await
        .expect("Interruption turn failed");
    assert!(
        reply_interrupt.contains("121"),
        "Expected interrupting command calculation (121), got: {}",
        reply_interrupt
    );
    assert!(!runtime.has_pending_clarification(sid_e).await);
}
