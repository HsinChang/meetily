@echo off
setlocal

REM Usage: clean_build_windows.bat [cuda^|vulkan^|openblas]
REM   No argument = CPU-only build.
REM See docs/WINDOWS_BUILD.md for prerequisites.

set GPU_FEATURE=%1
if "%GPU_FEATURE%"=="" (
    set FEATURE_ARGS=
    echo Building CPU-only ^(pass cuda, vulkan, or openblas to enable GPU^).
) else (
    set FEATURE_ARGS=--features %GPU_FEATURE%
    echo Building with GPU feature: %GPU_FEATURE%
)

echo Cleaning dependencies...
if exist node_modules rd /s /q node_modules

echo Installing dependencies...
call pnpm install
if errorlevel 1 exit /b 1

REM NOTE: bundled models are NOT fetched here. They total ~4.7GB and get baked
REM into the installer via the models/**/* resource glob. Run this manually only
REM if you want an offline-capable installer:
REM     node src-tauri\scripts\fetch-bundled-models.mjs

echo Building llama-helper sidecar (CPU-only)...
REM Deliberately no GPU features: llama-cpp-sys-2 has a CMake race condition
REM on Windows with vulkan. GPU acceleration applies to whisper-rs, not this sidecar.
cargo build --release -p llama-helper
if errorlevel 1 exit /b 1

if not exist src-tauri\binaries mkdir src-tauri\binaries
copy /y ..\target\release\llama-helper.exe "src-tauri\binaries\llama-helper-x86_64-pc-windows-msvc.exe"
if errorlevel 1 exit /b 1

echo Building the project...
call pnpm tauri build -- --target x86_64-pc-windows-msvc %FEATURE_ARGS%
if errorlevel 1 exit /b 1

echo.
echo Build complete. Installers:
echo   ..\target\x86_64-pc-windows-msvc\release\bundle\msi\
echo   ..\target\x86_64-pc-windows-msvc\release\bundle\nsis\

endlocal
