@echo off
setlocal
cd /d "%~dp0"

if not exist "target\release\pico-desktop.exe" (
    echo Building Pico release binaries...
    cargo build --release -p pico-cli -p pico-desktop
    if errorlevel 1 exit /b 1
)

if not exist "target\release\pico-cli.exe" (
    echo Building pico-cli release binary...
    cargo build --release -p pico-cli
    if errorlevel 1 exit /b 1
)

echo Starting Pico local assistant daemon...
start "Pico Assistant Daemon" /min "%~dp0target\release\pico-cli.exe" --config "%~dp0configs\pico.toml" daemon

timeout /t 1 /nobreak >nul
echo Starting Pico Desktop Companion...
start "" "%~dp0target\release\pico-desktop.exe"
