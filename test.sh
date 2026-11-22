#!/usr/bin/env bash
set -euo pipefail

echo "====================================================================="
echo "          PICO: AUTOMATED TEST, BUILD & NATIVE LAUNCH (LINUX)"
echo "====================================================================="
echo ""

# 1. Check Cargo
if ! command -v cargo &> /dev/null; then
    echo "[ERROR] Cargo was not found in PATH. Please install Rust from https://rustup.rs/"
    exit 1
fi

echo "[1/3] Running full workspace unit, integration, and security tests..."
cargo test --workspace
echo "[OK] All workspace tests passed cleanly!"
echo ""

echo "[2/3] Building native desktop assistant and CLI..."
cargo build --workspace
echo "[OK] Binaries built successfully."
echo ""

echo "[3/3] Launching Pico Native Desktop Application..."
echo "Target: target/debug/pico-desktop"
echo ""

if [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    ./target/debug/pico-desktop &
    echo "Pico Desktop launched in background."
else
    echo "No graphical display detected (headless server). Launching CLI voice mode instead:"
    ./target/debug/pico-cli doctor
fi

echo "To test CLI voice mode anytime, run: ./target/debug/pico-cli"
