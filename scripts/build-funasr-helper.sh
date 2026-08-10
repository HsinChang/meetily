#!/usr/bin/env bash
# Build the funasr-helper sidecar and stage it in frontend/src-tauri/binaries/ with the
# target-triple suffix Tauri's externalBin expects. Bundling fails without it:
#   "resource path 'binaries/funasr-helper-...' doesn't exist"
#
# Unlike llama-helper this is a CMake/C++ build, not cargo — the Fun-ASR SAN-M encoder
# is a hand-built ggml graph with no Rust binding.
#
#   ./scripts/build-funasr-helper.sh              # auto GPU backend for the platform
#   MEETILY_GPU=cuda ./scripts/build-funasr-helper.sh
#   MEETILY_GPU=vulkan|cpu|metal ./scripts/build-funasr-helper.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/funasr-helper"
BUILD="$SRC/build"
OUT="$ROOT/frontend/src-tauri/binaries"

TARGET_TRIPLE="$(rustc -vV | awk '/host:/ {print $2}')"
EXE=""
case "$TARGET_TRIPLE" in *windows*) EXE=".exe" ;; esac

GPU="${MEETILY_GPU:-auto}"
if [ "$GPU" = "auto" ]; then
    case "$(uname -s)" in
        Darwin) GPU=metal ;;
        *)      GPU=cpu ;;   # Linux/Windows: opt in explicitly, CUDA/Vulkan SDKs may be absent
    esac
fi

CMAKE_FLAGS=(-DCMAKE_BUILD_TYPE=Release)
case "$GPU" in
    metal)  CMAKE_FLAGS+=(-DGGML_METAL=ON) ;;
    cuda)   CMAKE_FLAGS+=(-DGGML_CUDA=ON) ;;
    vulkan) CMAKE_FLAGS+=(-DGGML_VULKAN=ON) ;;
    cpu)    ;;
    *) echo "unknown MEETILY_GPU='$GPU' (want metal|cuda|vulkan|cpu)" >&2; exit 1 ;;
esac

# Reuse an existing llama.cpp checkout when one is available — the fetch dominates a
# cold build. Honours the caller's own FETCHCONTENT_SOURCE_DIR_LLAMA if already set.
if [ -n "${FETCHCONTENT_SOURCE_DIR_LLAMA:-}" ]; then
    CMAKE_FLAGS+=("-DFETCHCONTENT_SOURCE_DIR_LLAMA=$FETCHCONTENT_SOURCE_DIR_LLAMA")
fi

echo "Building funasr-helper sidecar (gpu=$GPU, target=$TARGET_TRIPLE)..."
cmake -B "$BUILD" -S "$SRC" "${CMAKE_FLAGS[@]}"
cmake --build "$BUILD" -j

mkdir -p "$OUT"
cp "$BUILD/bin/funasr-helper$EXE" "$OUT/funasr-helper-${TARGET_TRIPLE}${EXE}"
echo "-> $OUT/funasr-helper-${TARGET_TRIPLE}${EXE}"
