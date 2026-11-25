# Pico Architecture Review & Target Engineering Design

**Audit Baseline**: `faf1a5a8da856a2b88eceb188e92aa1d32e401d9`
**Target Platforms**: Windows 11 (`x86_64-pc-windows-msvc`) and Linux (`x86_64-unknown-linux-gnu`)

---

## 1. Critique of Current Decision & Calibration Architecture

### 1.1 Misrepresented "Cross-Encoder Neural Scoring"
Code comments, `DecisionConfig` (`enable_cross_encoder`), `CrossEncoderRanker`, `README.md`, and `site/index.html` describe candidate scoring as a "Neural Cross-Encoder". In reality (`crates/pico-inference/src/ranking.rs:78-135`, `embeddings.rs:47-90`):
1. `SemanticEmbedder::embed` computes a deterministic 128-dimensional signed FNV-1a bag-of-words / character-3-gram hash projection (zero neural network weights, zero transformer attention, zero learned semantic parameters).
2. `CrossEncoderRanker::score_candidate` is a linear 4-feature weighted dot product plus additive lexical/core bonuses (`raw = clamp(w_sem * sem + w_lex * lex + w_int * int + w_hist * hist + core_bonus, 0, 1)`).
3. Calling this a neural cross-encoder violates technical accuracy and creates unrealistic expectations of semantic generalization. It is properly an **ensemble linear multi-feature ranker over lexical, hashed-ngram, intent-prior, and historical-success features** (and should be named and documented honestly as such).

### 1.2 Flawed Probability Model (`Platt` Independent Saturation)
In `crates/pico-inference/src/ranking.rs:162-170`:
```text
raw_i = score_candidate(request, candidate_i)   // typically in [0.55, 0.85]
p_i = sigmoid(10.0 * raw_i - 5.0)               // evaluated INDEPENDENTLY per candidate!
```
Mathematical defects observed directly in production (`data/pico_memory.sqlite`):
1. At `a = 10.0, b = -5.0`:
   - `raw = 0.78 -> p = 0.943`
   - `raw = 0.72 -> p = 0.900`
   - Sum of reported "probabilities" over the top 3 competing actions frequently exceeds **2.50 (250%)**. Presenting `p = 96%` for `audio.mute` alongside `p = 95%` for `audio.set_volume` makes mutually exclusive actions appear nearly certain simultaneously.
2. `Calibrator::calibrated_probabilities` (`crates/pico-inference/src/calibration.rs:92-120`) computes a blend of temperature-scaled softmax and Platt confidence, yet `CrossEncoderRanker::rank_candidates` never calls it!
3. Required mathematical separation:
   - **Raw ranking score** $s_i \in \mathbb{R}$: monotonic score used solely for ordering candidates $i = 1..K$.
   - **Posterior selection probability** $P(c_i \mid x, \mathcal{C}) = \text{softmax}(s / T)_i$, strictly summing to $1.0$ over competing candidates $\mathcal{C}$, capturing **relative choice ambiguity** (margin $P_{(1)} - P_{(2)}$).
   - **Independent executability plausibility** $P(\text{valid} \mid x, c_i) = \sigma(\alpha \cdot s_i + \beta)$, empirically calibrated via NLL / Platt fitting on real held-out labeled utterances so `score < threshold` routes cleanly to out-of-scope / chat / `"I'm not sure"` instead of forced tool execution at `26%` (`windows.power.enable_plan`).

### 1.3 Evaluator Ground-Truth Leakage & Broken Trainer Gradients
- `crates/pico-learning/src/evaluator.rs:129-130` constructs benchmark inputs with oracle vectors `intent_affinities = [1.0, 0.1, 0.05]` and `historical_scores = [0.9, 0.5, 0.5]`, giving the first candidate an artificial advantage regardless of the actual `IntentClassifier` or `ToolIndex`.
- `crates/pico-learning/src/trainer.rs:58-60` uses fixed constants (`+0.2, +0.1, +0.05`) instead of real observed feature differences $f(x, c_{\text{target}}) - f(x, c_{\text{chosen}})$.
- Target repair: compute real features via `IntentClassifier` and lexical/semantic functions during both evaluation and pairwise margin gradient updates, evaluate on independent held-out examples not seen in rule/training development, and measure ECE + top-1 accuracy genuinely.

### 1.4 Retrieval & Intent Defects
- `ToolIndex::tokenize` keeps `.` and `_` inside words (`windows.filesystem.delete_file`), preventing BM25 token matches on `"delete"` or `"file"` when querying `"delete file"`. Splitting on separators + adding lightweight domain synonyms and $O(1)$ incremental `total_doc_len` resolves both scale and recall defects.
- `IntentClassifier::is_direct_conversation` matching substring `"second"` hijacked `"Set a timer for 30 seconds"`. Removing clarification words from chat regexes and routing clarification explicitly through session state eliminates the collision.

### 1.5 Target Session-Scoped Clarification Architecture
```text
User Input (session_id, text)
   |
   +--> Check active ClarificationState for session_id (not expired)
   |      |-- Match ordinal ("first", "the second one", "option two", "execute the first", "choose the second")
   |      |-- Match explicit candidate tool/description name
   |      |-- Match confirmation ("yes", "go ahead" / "no", "cancel")
   |      |-- Match missing-arg provision (for MissingArguments state)
   |      |-- Match cancellation / interruption ("never mind", or clearly new unrelated high-confidence request)
   |      `-- Unclear followup -> preserve state & re-prompt (decrement retry budget)
   |
   +--> If resolved to concrete (tool_name, merged_args, orig_request):
   |      -> Re-validate JSON schema & required args (transition to MissingArguments if still incomplete!)
   |      -> Verify SecurityPolicy & Confirmation
   |      -> Execute tool -> Run ObjectiveVerifier
   |      -> Gate ResponseSynthesizer on (result.success && verification.verified)
   |      -> Persist memory, learning observation, and events
```

---

## 2. Critique of Current Audio & Voice Engineering Architecture

### 2.1 Root Cause of Alien / Fast / Broken Voice Output
Two compounding signal-processing failures exist today:
1. **3.0x Sample-Rate Mismatch on Playback**:
   - Windows SAPI (`tts.rs:40-58`) produces `16,000 Hz` mono 16-bit PCM.
   - `AudioPlayback::start` (`playback.rs:92-170`) opens the system default output stream at native hardware rate (almost universally `48,000 Hz` on modern Windows WASAPI / Linux PipeWire-PulseAudio, or `44,100 Hz`), ignores `default_config.sample_rate()`, and reads 1 sample per hardware tick.
   - Resulting playback speed factor: $48000 / 16000 = 3.0\times$ (pitch shifted up $+19.02$ semitones, 3 seconds compressed into 1 second).
2. **2.18x Sample-Rate Mismatch + RIFF Header Corruption on Linux**:
   - `espeak-ng --stdout` outputs `22,050 Hz` WAV containers starting with the 44-byte RIFF header (`"RIFF....WAVEfmt ....data...."`).
   - `VoicePipeline` passes those raw bytes to `play_pcm_bytes`, interpreting ASCII metadata as audio waveform impulses and playing at $48000 / 22050 \approx 2.18\times$ speed.

### 2.2 Real-Time Audio Safety Violations
- `cpal` audio callbacks execute on elevated real-time OS threads (`MMCSS` Pro Audio on Windows, `SCHED_FIFO`/`RTKit` on Linux).
- `playback.rs` calls `buffer.lock()` (`Mutex<VecDeque<f32>>`) on every callback, risking priority inversion dropouts if the UI or pipeline thread locks while allocating.
- `capture.rs` allocates `Vec<f32>` + `Vec<i16>` inside the callback and bridges channels synchronously.
- Target fix: bounded lock-free SPSC ring buffers (`rtrb` crate or atomic read/write indices over pre-allocated `Vec<f32>`/`Vec<i16>`) with zero heap allocations and zero mutexes inside audio callbacks, plus atomic generation/epoch counters for instant `< 1 frame` playback clear on barge-in.

### 2.3 Target Unified Internal Audio Pipeline
```text
[Synthesizer: WinRT/OneCore | Piper | eSpeak-NG | SpeechDispatcher]
   | returns SynthesizedAudio { samples: Vec<f32> (normalized -1..1), sample_rate, channels }
   | (WAV container payloads explicitly decoded via hound -> f32 + declared format)
   v
[Speech Preprocessor & Sentence Chunker] (numbers, paths, abbreviations, rate scaling)
   v
[Stateful Windowed-Sinc / Polyphase Resampler + Channel Mapper]
   | converts source format -> hardware output_config (sample_rate, channels, F32/I16/I32/U8)
   v
[Lock-Free SPSC Audio Ring Buffer] ---> [cpal Output Callback] (0 locks, 0 allocs)

[cpal Input Callback] ---> [Lock-Free SPSC Ring Buffer]
   |
   v
[Audio Processing Thread: Format->f32 -> Mono -> Anti-Aliased Resampler -> 16 kHz i16]
   |
   +--> [Sample-Counted VAD + 150ms Pre-Roll Ring + Echo Gate]
          |-- Active speech during playback -> Atomic barge-in interrupt
          `-- End-of-utterance -> [STT Engine: Parakeet / Moonshine / Whisper / WinRT-SAPI]
```

---

## 3. Critique of Tool Catalog Architecture

Currently 2,016 out of 2,112 catalog actions (`windows.rs` & `linux.rs` Categories 2-22) are non-functional clones of 42 static templates. Maintaining fake tools corrupts BM25 retrieval (injecting irrelevant noise into top-15 candidates) and produces catastrophic user surprises (`mute` vs `set_volume` vs `delete_endpoint`).

Target catalog policy:
- Remove every non-functional duplicated placeholder entry cleanly. Never pad counts.
- Retain and harden all genuinely working built-ins + replace duplicated catalog categories with concise, verified, platform-accurate tools covering real user needs: filesystem inspection/read/write/search/stat/mkdir/copy/move/delete, process list/info/close-by-name/kill-by-pid, system info/uptime/battery/disk/network interfaces, real OS audio (WASAPI volume/mute/devices on Windows; `wpctl`/`pactl` on Linux), display/brightness (WMI monitor brightness / `brightnessctl`/`xrandr`), power (lock/sleep/display-off, confirmation gated), timers, clipboard, app launch/focus/close, calendar/notes/quick stash integration.
- Enforce automated catalog integrity tests verifying unique command adapters, valid parameter placeholders matching schema properties, and safe sandboxing.

---

## 4. Target Decoupled Desktop Companion & Runtime IPC Architecture

### 4.1 Why Current `apps/pico-desktop` Violates Resource Budgets
Currently `apps/pico-desktop` bundles `PicoRuntime`, `ToolIndex`, two `rusqlite` databases, `cpal` streams, and a `622 MB` Python ONNX `asr_service` subprocess, while running an unconditional `request_repaint_after(16ms)` loop (continuous CPU wakeups) and doing `sysinfo` polling inside UI drawing.

### 4.2 Target Split-Process Architecture
```text
+----------------------------------+         Local Authenticated IPC         +------------------------------------+
|  apps/pico-desktop               |   Windows: \\.\pipe\pico-assistant-<id> |  pico-cli daemon / pico runtime    |
|  Lightweight Companion + Tray    | <=====================================> |  Voice, STT, TTS, Decision Engine  |
|  Target: < 50 MB RSS, < 1% idle  |   Linux: $XDG_RUNTIME_DIR/pico.sock     |  Tool Registry, Memory, Learning   |
+----------------------------------+     (0o600 / Owner-only DACL, max 64KB) +------------------------------------+
  - Native UI + Tray Icon + Global Hotkey
  - Event-driven repaint (Wait until IPC/Media/Timer/Input event)
  - OS Media Session Controller (WinRT GSMTC / Linux MPRIS D-Bus)
  - Dominant Artwork Color Theme + Smooth Contrast-Safe Gradient
  - Exact System Volume Controller (WASAPI IAudioEndpointVolume / pactl-wpctl)
  - Persistent Multi-Timers, Pomodoro, Quick Notes, Drag-Drop Stash (local JSON store)
  - Optional Assistant Trigger / Status Strip via IPC (graceful offline state if daemon stopped)
```

### 4.3 Desktop Companion Visual & Memory Design Strategy
- Keep `apps/pico-desktop` dependencies strictly minimal (no `pico-orchestrator`, `pico-decision`, `pico-inference`, `pico-learning`, `pico-memory`, `rusqlite`, or `reqwest` linked into the companion binary!).
- Use direct lightweight state persistence (`serde_json` atomic write-and-rename under `%APPDATA%\Pico\companion\` / `~/.local/share/pico/companion/`).
- Measure release binary RSS (`GetProcessMemoryInfo` WorkingSetSize on Windows, `/proc/<pid>/status` `VmRSS` on Linux) and average idle CPU (`< 1%`) to verify exact renderer weight (`eframe` minimal glow vs pure native Win32/softbuffer if needed). If OpenGL `glow` driver RSS on Windows exceeds 50 MB, switch companion rendering cleanly or tune window/context allocation before shipping so measured release RSS stays strictly below `< 50 MB`!
