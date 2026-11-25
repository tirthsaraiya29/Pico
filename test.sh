#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

echo "[1/5] Checking Rust formatting..."
cargo fmt --all -- --check

echo "[2/5] Running strict workspace Clippy checks..."
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "[3/5] Running full workspace test suite..."
cargo test --workspace --all-features

echo "[4/5] Checking site/index.html and docs/index.html sync..."
cmp -s site/index.html docs/index.html

echo "[5/5] Validating Python ASR helper scripts..."
python3 -m py_compile scripts/asr_service.py scripts/whisper_daemon.py scripts/whisper_transcribe.py

echo "All checks passed cleanly."
