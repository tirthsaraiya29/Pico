# Audio & Voice Pipeline

## Overview

Pico's voice pipeline is built to avoid sample-rate distortions ("chipmunk playback"), RIFF WAV header noise injection, wall-clock VAD drift, and silent TTS/STT failures.

## 1. Unified Internal Format & Hardware Resampling (`pico-audio`)

- Every TTS backend outputs `SynthesizedAudio { samples: Vec<f32>, sample_rate: u32, channels: u16, voice, engine }`.
- `decode_wav_bytes_to_f32` explicitly parses RIFF WAV headers via `hound` rather than treating WAV containers as raw 16 kHz PCM bytes.
- `AudioPlayback::start` negotiates the output endpoint's native `sample_rate`, `channels`, and sample format (`F32`, `I16`, `I32`, `U8`, `U16`).
- `AudioResampler` applies stateful Lanczos-3 windowed-sinc resampling cleanly across chunk boundaries from any source rate to the exact hardware endpoint rate, eliminating pitch/speed mismatches (`16 kHz` -> `48 kHz`, `22.05 kHz` -> `48 kHz`, etc.).
- Real-time audio callbacks read/write lock-free, cache-aligned SPSC ring buffers (`ringbuf.rs`) with zero allocations or mutexes in audio callbacks and O(1) barge-in buffer invalidation (`interrupt()`).
- `AudioPlayback::headless()` spawns a lightweight `Condvar`/paced drainer so headless CI tests and servers accurately simulate real-time `is_playing()` completion without hanging.

## 2. Deterministic VAD (`pico-voice/src/vad.rs`)

- Counts processed samples (`sample_rate` default `16,000 Hz`) instead of wall-clock `Instant::now()` so pauses and utterance bounds behave deterministically on hardware capture, WAV replay, and CI tests.
- Maintains a continuous 160ms circular pre-roll buffer (`2,560` samples @ 16 kHz) so initial consonants are preserved when speech triggers after 2 active frames.
- Dynamically tracks noise floor during silence (`2.8x` noise floor floor-adjusted threshold) and ignores single click/pop spikes.

## 3. Text-to-Speech (`pico-voice/src/tts.rs`)

- **`SpeechPreprocessor`**: Cleans markdown links, expands `localhost:PORT`, `ms`, `%`, `KB/MB/GB/TB/MHz/GHz`, normalizes file paths, and splits sentences cleanly without breaking decimals (`3.14`) or domains (`example.com`).
- **Backends**:
  - `piper`: Neural local ONNX CLI (`--model <onnx> --length_scale <scale> --output-raw`) reading text via `stdin` and parsing `<model>.json` `audio.sample_rate`.
  - `winrt` / `native_sapi`: Persistent Windows PowerShell worker using WinRT OneCore voices (`Windows.Media.SpeechSynthesis`) with `System.Speech` `SetOutputToWaveStream` fallback, JSON-framed over `stdin` (zero command injection) and returning base64 RIFF WAV.
  - `espeak` / `speech_dispatcher`: Linux `espeak-ng` / `espeak` `--stdout --stdin` RIFF WAV decoding.
  - `mock`: Deterministic sinusoidal waveform at `22,050 Hz` for headless tests.
- All non-mock synthesis failures propagate explicitly as `Err(PicoError::SpeechSynthesis(...))`.

## 4. Speech-to-Text (`pico-voice/src/stt.rs` & `scripts/asr_service.py`)

- Captures audio into RAII `TempWavGuard` files (always unlinked on scope exit).
- `AsrDaemon` discovers `python3`/`python` (or honors `VoiceConfig::python_binary`), enforces bounded startup & inference timeouts, and supports `Parakeet-TDT-0.6B`, `Moonshine-Tiny-ONNX`, and `Faster-Whisper` via `scripts/asr_service.py` (supporting 8/16/24/32-bit mono/multi-channel WAV normalization to 16 kHz float32) plus `whisper-cli` `.bin` discovery.
- `SpeechNormalizer` removes utterance-initial hesitation fillers (`like um`, `uh`, `er`, `err`, `ah`, `hmm`), hyphenated stutters (`c-can`), and consecutive duplicates while preserving mid-sentence words (`looks like notepad`), math expressions (`3.14 * 2 + 5`), IPs (`8.8.8.8`), and Unix paths (`/etc/hosts`).
