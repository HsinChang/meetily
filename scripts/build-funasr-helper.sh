#!/usr/bin/env bash
# Build the funasr-helper sidecar and stage it in frontend/src-tauri/binaries/ with the
# target-triple suffix Tauri's externalBin expects. Bundling fails without it:
#   "resource path 'binaries/funasr-helper-...' doesn't exist"
#
# Unlike llama-helper this is a CMake/C++ build, not cargo — the Fun-ASR SAN-M encoder
# is a hand-built ggml graph with no Rust binding. That also means it does NOT inherit
# frontend/src-tauri/.cargo/config.toml, so the deployment target is pinned in
# funasr-helper/CMakeLists.txt instead.
#
#   ./scripts/build-funasr-helper.sh                 # host arch only
#   MEETILY_ARCHS="arm64 x86_64" ./scripts/build-funasr-helper.sh   # + universal
#   MEETILY_GPU=cuda ./scripts/build-funasr-helper.sh
#
# Each architecture is configured and built in its own tree, then lipo'd together, rather
# than using a single CMAKE_OSX_ARCHITECTURES="x86_64;arm64" pass: ggml probes CPU features
# with check_cxx_compiler_flag, which runs once per configure and would apply one arch's
# answers to both slices.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/funasr-helper"
OUT="$ROOT/frontend/src-tauri/binaries"

HOST_TRIPLE="$(rustc -vV | awk '/host:/ {print $2}')"
EXE=""
case "$HOST_TRIPLE" in *windows*) EXE=".exe" ;; esac

# Map an Apple arch name to the Rust target triple Tauri names sidecars after.
triple_for_arch() {
    case "$1" in
        arm64)  echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "unknown arch '$1'" >&2; exit 1 ;;
    esac
}

# GPU backend per architecture. Apple Silicon gets Metal; Intel Macs fall back to CPU,
# where the Metal backend buys little for LLM decode and adds driver risk.
gpu_for_arch() {
    case "$1" in
        arm64)  echo "${MEETILY_GPU:-metal}" ;;
        x86_64) echo "${MEETILY_GPU:-cpu}" ;;
    esac
}

cmake_gpu_flags() {
    case "$1" in
        metal)  echo "-DGGML_METAL=ON" ;;
        cuda)   echo "-DGGML_CUDA=ON" ;;
        vulkan) echo "-DGGML_VULKAN=ON" ;;
        cpu)    echo "-DGGML_METAL=OFF" ;;
        *) echo "unknown MEETILY_GPU='$1' (want metal|cuda|vulkan|cpu)" >&2; exit 1 ;;
    esac
}

build_one() {
    local arch="$1" build_dir="$2"
    local gpu; gpu="$(gpu_for_arch "$arch")"
    # GGML_NATIVE=ON (ggml's default) compiles with -mcpu=native, tuning the binary to the
    # *build* machine. That is wrong for anything shipped: a slice built on an M3 can carry
    # instructions an M1 lacks, and when cross-compiling it leaks the host CPU into the
    # other arch's flags outright ("unknown target CPU 'apple-m3'"). Off gives a portable
    # baseline; ggml still enables the features common to all Apple Silicon (dotprod, i8mm).
    local flags=(-DCMAKE_BUILD_TYPE=Release -DGGML_NATIVE=OFF)
    # shellcheck disable=SC2207
    flags+=($(cmake_gpu_flags "$gpu"))

    if [ "$(uname -s)" = "Darwin" ]; then
        flags+=(-DCMAKE_OSX_ARCHITECTURES="$arch")
    fi
    if [ -n "${FETCHCONTENT_SOURCE_DIR_LLAMA:-}" ]; then
        flags+=("-DFETCHCONTENT_SOURCE_DIR_LLAMA=$FETCHCONTENT_SOURCE_DIR_LLAMA")
    fi

    # CMake caches FETCHCONTENT_SOURCE_DIR_LLAMA. If an earlier run pointed it at a
    # checkout that has since been deleted (a temp dir, a cleaned workspace), CMake fails
    # outright rather than falling back to fetching, so drop the stale tree.
    if [ -f "$build_dir/CMakeCache.txt" ]; then
        local cached
        cached="$(sed -n 's/^FETCHCONTENT_SOURCE_DIR_LLAMA:[^=]*=//p' "$build_dir/CMakeCache.txt" | head -1)"
        if [ -n "$cached" ] && [ ! -d "$cached" ]; then
            echo "Cached llama.cpp source dir is gone ($cached); reconfiguring from scratch."
            rm -rf "$build_dir"
        fi
    fi

    echo "Building funasr-helper for $arch (gpu=$gpu)..."
    cmake -B "$build_dir" -S "$SRC" "${flags[@]}"
    cmake --build "$build_dir" -j
}

mkdir -p "$OUT"

# Default to the host architecture unless a list is requested.
if [ -n "${MEETILY_ARCHS:-}" ]; then
    ARCHS=($MEETILY_ARCHS)
elif [ "$(uname -s)" = "Darwin" ]; then
    ARCHS=("$(uname -m)")
else
    ARCHS=("native")
fi

BUILT=()
for arch in "${ARCHS[@]}"; do
    if [ "$arch" = "native" ]; then
        build_one "" "$SRC/build"
        cp "$SRC/build/bin/funasr-helper$EXE" "$OUT/funasr-helper-${HOST_TRIPLE}${EXE}"
        echo "-> $OUT/funasr-helper-${HOST_TRIPLE}${EXE}"
        exit 0
    fi
    build_dir="$SRC/build-$arch"
    build_one "$arch" "$build_dir"
    triple="$(triple_for_arch "$arch")"
    cp "$build_dir/bin/funasr-helper" "$OUT/funasr-helper-${triple}"
    echo "-> $OUT/funasr-helper-${triple}"
    BUILT+=("$OUT/funasr-helper-${triple}")
done

# Tauri names sidecars after the build's target triple, so a universal app looks for
# funasr-helper-universal-apple-darwin. Emit it alongside the per-arch files so either
# resolution path finds a binary.
if [ "${#BUILT[@]}" -gt 1 ]; then
    lipo -create "${BUILT[@]}" -output "$OUT/funasr-helper-universal-apple-darwin"
    echo "-> $OUT/funasr-helper-universal-apple-darwin ($(lipo -archs "$OUT/funasr-helper-universal-apple-darwin"))"
fi
