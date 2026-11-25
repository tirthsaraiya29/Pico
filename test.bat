@echo off
setlocal
cd /d "%~dp0"

echo [1/4] Checking Rust formatting...
cargo fmt --all -- --check
if errorlevel 1 exit /b 1

echo [2/4] Running strict workspace Clippy checks...
cargo clippy --workspace --all-targets --all-features -- -D warnings
if errorlevel 1 exit /b 1

echo [3/4] Running full workspace test suite...
cargo test --workspace --all-features
if errorlevel 1 exit /b 1

echo [4/4] Validating Python ASR helper scripts...
python -m py_compile scripts\asr_service.py scripts\whisper_daemon.py scripts\whisper_transcribe.py
if errorlevel 1 exit /b 1

echo All checks passed cleanly.
