@echo off
REM Dev: build frontend + run Rust backend
setlocal

set "SCRIPT_DIR=%~dp0"
pushd "%SCRIPT_DIR%.."

REM Build frontend if dist not exists
if not exist "web\dist" (
    echo Building frontend...
    pushd web
    call npm ci && call npm run build
    popd
)

REM Run
cargo run %*
popd
