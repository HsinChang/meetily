#!/bin/bash

# Exit on error
set -e

# Add log level selector with default to INFO
LOG_LEVEL=${1:-info}

case $LOG_LEVEL in
    info|debug|trace)
        export RUST_LOG=$LOG_LEVEL
        ;;
    *)
        echo "Invalid log level: $LOG_LEVEL. Valid options: info, debug, trace"
        exit 1
        ;;
esac

# Check and install CMake if needed
echo "Checking CMake version..."
if ! command -v cmake &> /dev/null; then
    echo "CMake not found. Installing via Homebrew..."
    brew install cmake
else
    CMAKE_VERSION=$(cmake --version | head -n1 | cut -d" " -f3)
    if [[ "$CMAKE_VERSION" < "3.5" ]]; then
        echo "CMake version $CMAKE_VERSION is too old. Updating via Homebrew..."
        brew upgrade cmake
    fi
fi

# Clean up previous builds
echo "Cleaning up previous builds..."
rm -rf target/
rm -rf src-tauri/target
rm -rf src-tauri/gen

# Clean up npm, pnp and next
echo "Cleaning up npm, pnp and next..."
rm -rf node_modules
rm -rf .next
rm -rf .pnp.cjs
rm -rf out

echo "Installing dependencies..."
pnpm install

# Fetch the models that ship built-in (whisper turbo, parakeet, qwen).
# Existing files are skipped, so re-runs are cheap. See scripts/fetch-bundled-models.mjs.
echo "Fetching bundled models..."
node src-tauri/scripts/fetch-bundled-models.mjs

# MEETILY_UNIVERSAL=1 builds a universal (arm64 + x86_64) macOS app. Sidecars must then
# exist for both architectures, since Tauri only lipos the main binary itself.
UNIVERSAL="${MEETILY_UNIVERSAL:-0}"
TARGET_TRIPLE=$(rustc -vV | awk '/host:/ {print $2}')
mkdir -p src-tauri/binaries

# Build the llama-helper sidecar (local LLM inference for summaries + translation)
# and place it in binaries/ with the target-triple suffix Tauri's externalBin expects.
# (CI does this too; the local build needs it or bundling fails with
#  "resource path 'binaries/llama-helper-...' doesn't exist".)
echo "Building llama-helper sidecar..."
if [ "$UNIVERSAL" = "1" ]; then
    # Metal is an Apple Silicon concern here; the Intel slice builds CPU-only.
    cargo build --release -p llama-helper --features metal --target aarch64-apple-darwin
    cargo build --release -p llama-helper --target x86_64-apple-darwin
    cp ../target/aarch64-apple-darwin/release/llama-helper "src-tauri/binaries/llama-helper-aarch64-apple-darwin"
    cp ../target/x86_64-apple-darwin/release/llama-helper "src-tauri/binaries/llama-helper-x86_64-apple-darwin"
else
    cargo build --release -p llama-helper --features metal
    cp ../target/release/llama-helper "src-tauri/binaries/llama-helper-${TARGET_TRIPLE}"
fi

# Build the funasr-helper sidecar (Fun-ASR-Nano Chinese transcription). CMake/C++ rather
# than cargo — the SAN-M audio encoder is a hand-built ggml graph with no Rust binding.
echo "Building funasr-helper sidecar..."
if [ "$UNIVERSAL" = "1" ]; then
    MEETILY_ARCHS="arm64 x86_64" ../scripts/build-funasr-helper.sh
else
    ../scripts/build-funasr-helper.sh
fi

if [ "$UNIVERSAL" = "1" ]; then
    # src-tauri's build script downloads ffmpeg for whichever target it is invoked with,
    # so the Intel copy only appears once something has been built for x86_64. Run a check
    # to trigger it now; the lipo step below needs both copies present.
    echo "Fetching x86_64 ffmpeg via build script..."
    (cd src-tauri && cargo check --target x86_64-apple-darwin --quiet) || true

    echo "Combining sidecars into universal binaries..."
    ../scripts/lipo-sidecars.sh
fi

# Build the Next.js application first
echo "Building Next.js application..."
pnpm run build

# Set environment variables for the build

echo "Building Tauri app..."
if [ "$UNIVERSAL" = "1" ]; then
    # `tauri build -- <args>` forwards <args> to cargo (see the tauri:build:* scripts), and
    # cargo has no "universal-apple-darwin" target spec — only the Tauri CLI understands it.
    # So it must be passed to tauri itself, before any `--`.
    pnpm exec tauri build --target universal-apple-darwin
else
    pnpm run tauri build
fi
sleep

