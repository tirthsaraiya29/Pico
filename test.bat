@echo off
setlocal enabledelayedexpansion

echo =====================================================================
echo           PICO: AUTOMATED TEST, BUILD ^& NATIVE LAUNCH
echo =====================================================================
echo.

:: 1. Check Cargo
where cargo >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo [ERROR] Cargo was not found in PATH. Please install Rust from https://rustup.rs/
    pause
    exit /b 1
)

echo [1/3] Running full workspace unit, integration, and security tests...
cargo test --workspace
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo [FATAL] Tests failed! Aborting build and desktop launch.
    pause
    exit /b %ERRORLEVEL%
)
echo [OK] All workspace tests passed cleanly!
echo.

echo [2/3] Building native desktop assistant and CLI...
cargo build --workspace
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo [FATAL] Build failed!
    pause
    exit /b %ERRORLEVEL%
)
echo [OK] Binaries built successfully.
echo.

echo [3/3] Launching Pico Native Desktop Application...
echo Target: target\debug\pico-desktop.exe
echo.
start "" target\debug\pico-desktop.exe

echo Pico Desktop launched in background.
echo You can also test CLI voice mode anytime by running: target\debug\pico-cli.exe
