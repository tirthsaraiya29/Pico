use pico_audio::resampler::AudioConverter;
use pico_core::config::PicoConfig;
use pico_core::traits::{CandidateAction, MemoryStore, PermissionTier};
use pico_decision::engine::DecisionResult;
use pico_inference::calibration::{Calibrator, PlattParameters};
use pico_inference::ranking::{CrossEncoderRanker, RankingWeights};
use pico_orchestrator::PicoRuntime;
use pico_voice::vad::VadProcessor;
use std::sync::Arc;
use std::time::Instant;

#[cfg(target_os = "windows")]
fn get_current_process_memory_mb() -> f64 {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, pmc: *mut ProcessMemoryCounters, cb: u32)
            -> i32;
    }
    unsafe {
        let mut pmc: ProcessMemoryCounters = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            pmc.working_set_size as f64 / (1024.0 * 1024.0)
        } else {
            0.0
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn get_current_process_memory_mb() -> f64 {
    0.0
}

fn create_bench_runtime() -> Arc<PicoRuntime> {
    let mut config = PicoConfig::default();
    config.security.allowed_directories = vec![std::path::PathBuf::from(".")];
    config.security.audit_log_path = std::path::PathBuf::new();
    config.memory.database_path = std::env::temp_dir().join(format!(
        "pico_bench_mem_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    config.learning.database_path = std::env::temp_dir().join(format!(
        "pico_bench_learn_{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));

    Arc::new(PicoRuntime::new(config).expect("Runtime initialization failed"))
}

#[test]
fn test_benchmark_latency_profile() {
    println!("\n=================== BENCHMARK: LATENCY PROFILE ===================");

    // 1. Resampling Latency (48kHz stereo to 16kHz mono)
    let converter = AudioConverter::new(48000, 2);
    let stereo_chunk = vec![0.3f32; 960]; // 20ms of audio at 48kHz
    let mut resample_durations = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let _ = converter.convert_f32_to_16k_i16(&stereo_chunk);
        resample_durations.push(start.elapsed());
    }
    let avg_resample_us = resample_durations
        .iter()
        .map(|d| d.as_micros())
        .sum::<u128>() as f64
        / 100.0;
    let avg_resample_ms = avg_resample_us / 1000.0;
    println!(
        "Audio Resampling (48kHz->16kHz, 20ms frame): {:.3} ms (Budget: < 1.0 ms)",
        avg_resample_ms
    );
    assert!(
        avg_resample_ms < 1.0,
        "Audio resampling exceeded budget: {:.3} ms",
        avg_resample_ms
    );

    // 2. VAD Chunk Processing Latency (512 samples @ 16kHz)
    let vad_samples = vec![1200i16; 512];
    let mut vad = VadProcessor::new(0.015, 600);
    let mut vad_durations = Vec::with_capacity(500);
    for _ in 0..500 {
        let start = Instant::now();
        let _ = vad.process_chunk(&vad_samples);
        vad_durations.push(start.elapsed());
    }
    let avg_vad_us = vad_durations.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / 500.0;
    let avg_vad_ms = avg_vad_us / 1000.0;
    println!(
        "VAD Processing (512 samples chunk):           {:.3} ms (Budget: < 0.5 ms)",
        avg_vad_ms
    );
    assert!(
        avg_vad_ms < 0.5,
        "VAD processing exceeded budget: {:.3} ms",
        avg_vad_ms
    );

    // 3. BM25 Retrieval across all 2,112 tools
    let runtime = create_bench_runtime();
    let decision = runtime.decision();
    let tool_index = decision.tool_index();
    let idx_read = tool_index.read().unwrap();
    let total_indexed_tools = idx_read.len();
    assert!(
        total_indexed_tools >= 2112,
        "Expected at least 2112 tools indexed, got {}",
        total_indexed_tools
    );

    let test_queries = [
        "check system processor and ram memory",
        "calculate arithmetic formula 45 * 2 + 10",
        "terminate unresponsive process chrome",
        "adjust master speaker sound volume",
        "list running windows background services",
        "ping domain gateway network connection",
        "copy selected text snippet to clipboard",
        "show security event log firewall status",
        "lock user desktop screen session",
        "search files on disk storage drive",
    ];

    let mut bm25_durations = Vec::with_capacity(100);
    for i in 0..100 {
        let q = test_queries[i % test_queries.len()];
        let start = Instant::now();
        let _ = idx_read.retrieve_candidates(q, "windows", None, 10);
        bm25_durations.push(start.elapsed());
    }
    let avg_bm25_us = bm25_durations.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / 100.0;
    let avg_bm25_ms = avg_bm25_us / 1000.0;
    println!(
        "BM25 Candidate Retrieval ({} tools):       {:.3} ms (Budget: < 1.0 ms)",
        total_indexed_tools, avg_bm25_ms
    );
    assert!(
        avg_bm25_ms < 1.0,
        "BM25 retrieval exceeded budget: {:.3} ms",
        avg_bm25_ms
    );
    drop(idx_read);

    // 4. CrossEncoder Ranking & Calibration Latency
    let ranker = CrossEncoderRanker::new(RankingWeights::trained());
    let candidates = vec![
        CandidateAction {
            name: "calculator".into(),
            description: "Evaluates mathematical expressions".into(),
            category: "math".into(),
            permission_tier: PermissionTier::ReadOnly,
            parameters_schema: serde_json::json!({}),
        },
        CandidateAction {
            name: "get_system_info".into(),
            description: "Gets hardware specs and cpu usage".into(),
            category: "system".into(),
            permission_tier: PermissionTier::ReadOnly,
            parameters_schema: serde_json::json!({}),
        },
        CandidateAction {
            name: "windows.filesystem.search_files".into(),
            description: "Searches disk filesystem for files".into(),
            category: "filesystem".into(),
            permission_tier: PermissionTier::ReadOnly,
            parameters_schema: serde_json::json!({}),
        },
    ];

    let intent_affinities = vec![0.9, 0.2, 0.1];
    let history_scores = vec![0.95, 0.80, 0.70];

    let mut rank_durations = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let _ = ranker.rank_candidates(
            "calculate 45 * 2 + 10",
            &candidates,
            &intent_affinities,
            &history_scores,
            1.0,
        );
        rank_durations.push(start.elapsed());
    }
    let avg_rank_us = rank_durations.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / 100.0;
    let avg_rank_ms = avg_rank_us / 1000.0;
    println!(
        "CrossEncoder Ranking & Calibration:           {:.3} ms (Budget: < 2.0 ms)",
        avg_rank_ms
    );
    assert!(
        avg_rank_ms < 2.0,
        "Ranking exceeded budget: {:.3} ms",
        avg_rank_ms
    );
    println!("==================================================================\n");
}

#[tokio::test]
async fn test_benchmark_e2e_decision_and_tool_latency() {
    println!("\n============= BENCHMARK: E2E DECISION & TOOL LATENCY =============");
    let runtime = create_bench_runtime();
    let decision = runtime.decision();
    let registry = runtime.registry();
    let memory = runtime.memory();

    // 1. End-to-end DecisionEngine Latency (Classification + BM25 + Ranking + Calibration + Arguments)
    let decision_queries = [
        "calculate 50 * 2 + 10",
        "show my system information",
        "set volume to 60",
        "list all running processes",
        "hello how are you today",
    ];

    let mut decision_durations = Vec::with_capacity(100);
    for i in 0..100 {
        let q = decision_queries[i % decision_queries.len()];
        let start = Instant::now();
        let _ = decision.decide(q).await;
        decision_durations.push(start.elapsed());
    }
    let avg_dec_us = decision_durations
        .iter()
        .map(|d| d.as_micros())
        .sum::<u128>() as f64
        / 100.0;
    let avg_dec_ms = avg_dec_us / 1000.0;
    println!(
        "End-to-End Decision Latency:                  {:.3} ms (Budget: < 5.0 ms)",
        avg_dec_ms
    );
    assert!(
        avg_dec_ms < 5.0,
        "Decision engine latency exceeded budget: {:.3} ms",
        avg_dec_ms
    );

    // 2. Tool Execution Latency: Calculator (AST parser)
    let mut calc_durations = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let res = registry
            .execute_call(
                "bench_call",
                "calculator",
                serde_json::json!({ "expression": "128 * 4 + 512 / 8" }),
            )
            .await;
        calc_durations.push(start.elapsed());
        assert!(res.success);
    }
    let avg_calc_us = calc_durations.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / 100.0;
    let avg_calc_ms = avg_calc_us / 1000.0;
    println!(
        "Tool Execution: calculator (AST parser):      {:.3} ms (Budget: < 1.0 ms)",
        avg_calc_ms
    );
    assert!(
        avg_calc_ms < 1.0,
        "Calculator execution exceeded budget: {:.3} ms",
        avg_calc_ms
    );

    // 3. Tool Execution Latency: get_system_info
    let mut sys_durations = Vec::with_capacity(20);
    for _ in 0..20 {
        let start = Instant::now();
        let res = registry
            .execute_call("bench_call", "get_system_info", serde_json::json!({}))
            .await;
        sys_durations.push(start.elapsed());
        assert!(res.success);
    }
    let avg_sys_us = sys_durations.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / 20.0;
    let avg_sys_ms = avg_sys_us / 1000.0;
    println!(
        "Tool Execution: get_system_info:              {:.3} ms (Budget: < 50.0 ms)",
        avg_sys_ms
    );
    assert!(
        avg_sys_ms < 50.0,
        "get_system_info exceeded budget: {:.3} ms",
        avg_sys_ms
    );

    // 4. SQLite Memory Store Read/Write Latency
    let mut mem_write_durations = Vec::with_capacity(50);
    let mut mem_read_durations = Vec::with_capacity(50);
    for i in 0..50 {
        let start_w = Instant::now();
        let _ = memory
            .remember_fact("pref", &format!("user preferred theme is dark_{}", i))
            .await;
        mem_write_durations.push(start_w.elapsed());

        let start_r = Instant::now();
        let _ = memory.search_relevant_context("preferred theme", 3).await;
        mem_read_durations.push(start_r.elapsed());
    }
    let avg_write_ms = mem_write_durations
        .iter()
        .map(|d| d.as_micros())
        .sum::<u128>() as f64
        / 50.0
        / 1000.0;
    let avg_read_ms = mem_read_durations
        .iter()
        .map(|d| d.as_micros())
        .sum::<u128>() as f64
        / 50.0
        / 1000.0;
    println!(
        "SQLite Memory Store (Write / Read):           {:.3} ms / {:.3} ms (Budget: < 5.0 ms)",
        avg_write_ms, avg_read_ms
    );
    assert!(
        avg_write_ms < 5.0 && avg_read_ms < 5.0,
        "SQLite memory latency exceeded budget"
    );
    println!("==================================================================\n");
}

#[tokio::test]
async fn test_benchmark_accuracy_profile() {
    println!("\n================== BENCHMARK: ACCURACY PROFILE ===================");
    let runtime = create_bench_runtime();
    let decision = runtime.decision();

    // Multi-domain benchmark test dataset mapping user query to expected tool (or conversational)
    struct TestCase {
        query: &'static str,
        expected_tool: Option<&'static str>,
        is_conversational: bool,
    }

    let dataset = [
        // Mathematics / Calculation
        TestCase {
            query: "what is 144 / 12 + 8",
            expected_tool: Some("calculator"),
            is_conversational: false,
        },
        TestCase {
            query: "calculate 30 * 50 - 25",
            expected_tool: Some("calculator"),
            is_conversational: false,
        },
        TestCase {
            query: "compute 1024 / 8",
            expected_tool: Some("calculator"),
            is_conversational: false,
        },
        TestCase {
            query: "solve 99 + 101",
            expected_tool: Some("calculator"),
            is_conversational: false,
        },
        // System Information
        TestCase {
            query: "show system information and hardware specs",
            expected_tool: Some("get_system_info"),
            is_conversational: false,
        },
        TestCase {
            query: "how much ram and cpu cores do I have",
            expected_tool: Some("get_system_info"),
            is_conversational: false,
        },
        TestCase {
            query: "check my computer specs",
            expected_tool: Some("get_system_info"),
            is_conversational: false,
        },
        // Audio Controls
        TestCase {
            query: "set speaker volume to 50 percent",
            expected_tool: Some("audio.set_volume"),
            is_conversational: false,
        },
        TestCase {
            query: "turn down the sound volume",
            expected_tool: Some("audio.set_volume"),
            is_conversational: false,
        },
        TestCase {
            query: "get current audio volume",
            expected_tool: Some("audio"),
            is_conversational: false,
        },
        // Process Management
        TestCase {
            query: "list all running processes and tasks",
            expected_tool: Some("list_processes"),
            is_conversational: false,
        },
        TestCase {
            query: "kill unresponsive process task",
            expected_tool: Some("kill_process"),
            is_conversational: false,
        },
        TestCase {
            query: "terminate process with id 1234",
            expected_tool: Some("kill_process"),
            is_conversational: false,
        },
        // Clipboard
        TestCase {
            query: "copy text to clipboard buffer",
            expected_tool: Some("clipboard"),
            is_conversational: false,
        },
        TestCase {
            query: "paste text from clipboard",
            expected_tool: Some("clipboard"),
            is_conversational: false,
        },
        // Network
        TestCase {
            query: "ping network host 8.8.8.8 to test connectivity",
            expected_tool: Some("network.ping"),
            is_conversational: false,
        },
        // Conversational (No Tool)
        TestCase {
            query: "hello there pico",
            expected_tool: None,
            is_conversational: true,
        },
        TestCase {
            query: "good morning, how are you?",
            expected_tool: None,
            is_conversational: true,
        },
        TestCase {
            query: "thank you for your help",
            expected_tool: None,
            is_conversational: true,
        },
        TestCase {
            query: "who created you?",
            expected_tool: None,
            is_conversational: true,
        },
        TestCase {
            query: "tell me a joke",
            expected_tool: None,
            is_conversational: true,
        },
    ];

    let mut top1_correct = 0usize;
    let mut top3_correct = 0usize;
    let total = dataset.len();

    for tc in &dataset {
        let dec = decision
            .decide(tc.query)
            .await
            .expect("Decision evaluation failed");
        println!("QUERY: '{}' -> DECISION: {:?}", tc.query, dec);
        match dec {
            DecisionResult::ActionSelected {
                action_name,
                ranked_scores,
                ..
            } => {
                if let Some(expected) = tc.expected_tool {
                    let matched_top1 = action_name == expected
                        || action_name.contains(expected)
                        || (action_name.starts_with("windows.") && action_name.contains(expected));
                    if matched_top1 {
                        top1_correct += 1;
                        top3_correct += 1;
                    } else {
                        let matched_top3 = ranked_scores.iter().take(3).any(|s| {
                            s.action_name == expected
                                || s.action_name.contains(expected)
                                || (s.action_name.starts_with("windows.")
                                    && s.action_name.contains(expected))
                        });
                        if matched_top3 {
                            top3_correct += 1;
                        }
                    }
                }
            }
            DecisionResult::DirectConversation { .. } => {
                if tc.is_conversational {
                    top1_correct += 1;
                    top3_correct += 1;
                }
            }
            DecisionResult::ClarificationRequired { top_candidates, .. } => {
                if let Some(expected) = tc.expected_tool {
                    if let Some(top1) = top_candidates.first() {
                        let matched_top1 = top1.action_name == expected
                            || top1.action_name.contains(expected)
                            || (top1.action_name.starts_with("windows.")
                                && top1.action_name.contains(expected));
                        if matched_top1 {
                            top1_correct += 1;
                        }
                    }
                    let matched_top3 = top_candidates.iter().take(3).any(|s| {
                        s.action_name == expected
                            || s.action_name.contains(expected)
                            || (s.action_name.starts_with("windows.")
                                && s.action_name.contains(expected))
                    });
                    if matched_top3 {
                        top3_correct += 1;
                    }
                }
            }
        }
    }

    let top1_acc = (top1_correct as f64 / total as f64) * 100.0;
    let top3_acc = (top3_correct as f64 / total as f64) * 100.0;

    println!("Evaluated {} benchmark test cases across domains.", total);
    println!(
        "Top-1 Accuracy: {:.1}% ({}/{} correct) [Target: >= 85.0%]",
        top1_acc, top1_correct, total
    );
    println!(
        "Top-3 Accuracy: {:.1}% ({}/{} correct) [Target: >= 95.0%]",
        top3_acc, top3_correct, total
    );

    assert!(
        top1_acc >= 85.0,
        "Top-1 accuracy below target: {:.1}%",
        top1_acc
    );
    assert!(
        top3_acc >= 95.0,
        "Top-3 accuracy below target: {:.1}%",
        top3_acc
    );
    println!("==================================================================\n");
}

#[test]
fn test_benchmark_calibration_profile() {
    println!("\n================= BENCHMARK: CALIBRATION PROFILE =================");
    let platt_params = PlattParameters::default();

    // Verify Platt scaling mapping range and monotonicity
    let low_score = 0.1f32;
    let mid_score = 0.5f32;
    let high_score = 0.9f32;

    let p_low = Calibrator::platt_calibrate(low_score, &platt_params);
    let p_mid = Calibrator::platt_calibrate(mid_score, &platt_params);
    let p_high = Calibrator::platt_calibrate(high_score, &platt_params);

    assert!(
        p_low < p_mid && p_mid < p_high,
        "Calibration must be strictly monotonic"
    );
    assert!(
        p_low < 0.05,
        "Low score must have near-zero calibrated probability"
    );
    assert!(
        p_high > 0.95,
        "High score must have near-one calibrated probability"
    );

    // Evaluation on a calibrated validation dataset (predicted probability, actual label)
    let validation_samples: Vec<(f32, bool)> = vec![
        // Bin [0.8, 1.0]: high confidence, predominantly true
        (0.95, true),
        (0.92, true),
        (0.90, true),
        (0.88, true),
        (0.85, true),
        // Bin [0.6, 0.8]: moderate-high confidence, 75% true
        (0.75, true),
        (0.72, true),
        (0.68, true),
        (0.65, false),
        // Bin [0.4, 0.6]: ambiguous, 50% true
        (0.55, true),
        (0.45, false),
        // Bin [0.2, 0.4]: low confidence, 25% true
        (0.35, true),
        (0.30, false),
        (0.25, false),
        (0.22, false),
        // Bin [0.0, 0.2]: very low confidence, 0% true
        (0.15, false),
        (0.10, false),
        (0.05, false),
        (0.02, false),
        (0.01, false),
    ];

    let metrics = Calibrator::evaluate_calibration(&validation_samples, 5);
    println!(
        "Expected Calibration Error (ECE): {:.4} (Target: < 0.15)",
        metrics.expected_calibration_error
    );
    println!(
        "Maximum Calibration Error (MCE):  {:.4}",
        metrics.max_calibration_error
    );
    println!(
        "Brier Calibration Score:          {:.4} (Target: < 0.20)",
        metrics.brier_score
    );

    assert!(
        metrics.expected_calibration_error < 0.15,
        "ECE exceeded target: {:.4}",
        metrics.expected_calibration_error
    );
    assert!(
        metrics.brier_score < 0.20,
        "Brier score exceeded target: {:.4}",
        metrics.brier_score
    );
    println!("==================================================================\n");
}

#[tokio::test]
async fn test_benchmark_resource_profile() {
    println!("\n================== BENCHMARK: RESOURCE PROFILE ===================");
    let initial_mem = get_current_process_memory_mb();
    println!("Initial Process Working Set: {:.2} MB", initial_mem);

    let runtime = create_bench_runtime();
    let decision = runtime.decision();

    let queries = [
        "calculate 12 * 12",
        "show cpu usage",
        "set volume to 80",
        "what is 100 / 25",
        "hello pico",
    ];

    // Stress test: execute 500 decisions
    let start_all = Instant::now();
    for i in 0..500 {
        let q = queries[i % queries.len()];
        let _ = decision.decide(q).await;
    }
    let total_elapsed = start_all.elapsed();
    let qps = 500.0 / total_elapsed.as_secs_f64();

    let final_mem = get_current_process_memory_mb();
    let mem_diff = final_mem - initial_mem;
    println!(
        "Processed 500 requests in {:.2}s ({:.1} QPS)",
        total_elapsed.as_secs_f64(),
        qps
    );
    println!(
        "Final Process Working Set:   {:.2} MB (Growth: {:+.2} MB)",
        final_mem, mem_diff
    );

    // Assert high throughput and modest memory bounds
    assert!(qps > 100.0, "Throughput below target: {:.1} QPS", qps);
    if final_mem > 0.0 {
        assert!(
            final_mem < 250.0,
            "Working set exceeds budget: {:.2} MB",
            final_mem
        );
    }
    println!("==================================================================\n");
}
