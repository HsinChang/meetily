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

# Build the llama-helper sidecar (local LLM inference for summaries + translation)
# and place it in binaries/ with the target-triple suffix Tauri's externalBin expects.
# (CI does this too; the local build needs it or bundling fails with
#  "resource path 'binaries/llama-helper-...' doesn't exist".)
echo "Building llama-helper sidecar..."
TARGET_TRIPLE=$(rustc -vV | awk '/host:/ {print $2}')
cargo build --release -p llama-helper --features metal
mkdir -p src-tauri/binaries
cp ../target/release/llama-helper "src-tauri/binaries/llama-helper-${TARGET_TRIPLE}"

# Build the funasr-helper sidecar (Fun-ASR-Nano Chinese transcription). CMake/C++ rather
# than cargo — the SAN-M audio encoder is a hand-built ggml graph with no Rust binding.
echo "Building funasr-helper sidecar..."
../scripts/build-funasr-helper.sh

# Build the Next.js application first
echo "Building Next.js application..."
pnpm run build

# Set environment variables for the build

echo "Building Tauri app..."
pnpm run tauri build
sleep

