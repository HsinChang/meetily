@echo off

echo Cleaning npm dependencies...
rd /s /q node_modules
del /f /q package-lock.json

echo Installing npm dependencies...
pnpm install

echo Fetching bundled models...
node src-tauri\scripts\fetch-bundled-models.mjs

echo Building llama-helper sidecar...
for /f "tokens=2" %%i in ('rustc -vV ^| findstr /b "host:"') do set TARGET_TRIPLE=%%i
cargo build --release -p llama-helper --features vulkan
if not exist src-tauri\binaries mkdir src-tauri\binaries
copy /y ..\target\release\llama-helper.exe "src-tauri\binaries\llama-helper-%TARGET_TRIPLE%.exe"

echo Building the project...
pnpm run tauri build
