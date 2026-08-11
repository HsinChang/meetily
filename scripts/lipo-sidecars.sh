#!/usr/bin/env bash
# Combine per-architecture sidecars into universal binaries, for macOS universal builds.
#
# Tauri lipos its own main binary for --target universal-apple-darwin, but external
# binaries are left to us: it looks each one up as "<name>-<target-triple>", which for a
# universal build means "<name>-universal-apple-darwin".
#
# This runs as Tauri's beforeBundleCommand — after cargo has built both architectures
# (which is when src-tauri's build script has downloaded ffmpeg for both) and before the
# bundler copies external binaries in.
#
# A no-op when a sidecar has no x86_64/arm64 pair, so plain single-arch builds are
# unaffected.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/frontend/src-tauri/binaries"
[ -d "$BIN" ] || exit 0

MIN_OS="14.2"
made=0

for arm in "$BIN"/*-aarch64-apple-darwin; do
    [ -e "$arm" ] || continue
    name="$(basename "$arm" -aarch64-apple-darwin)"
    intel="$BIN/${name}-x86_64-apple-darwin"
    universal="$BIN/${name}-universal-apple-darwin"

    if [ ! -e "$intel" ]; then
        echo "lipo-sidecars: no x86_64 slice for '$name', skipping"
        continue
    fi

    lipo -create "$arm" "$intel" -output "$universal"
    echo "lipo-sidecars: $name -> $(lipo -archs "$universal")"
    made=$((made + 1))

    # A slice whose minimum OS is newer than the app's would be rejected by dyld at
    # spawn time on supported systems — the exact failure that shipped in 0.6.1, where a
    # sidecar built against the host SDK demanded macOS 26.
    for slice_arch in $(lipo -archs "$universal"); do
        tmp="$(mktemp)"
        lipo -thin "$slice_arch" "$universal" -output "$tmp"
        # Modern binaries carry LC_BUILD_VERSION; ones built with an older deployment
        # target use LC_VERSION_MIN_MACOSX instead. Read both, or the check silently
        # passes over exactly the binaries it exists to police.
        minos="$(otool -l "$tmp" | awk '/LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; exit}')"
        if [ -z "$minos" ]; then
            minos="$(otool -l "$tmp" | awk '/LC_VERSION_MIN_MACOSX/{f=1} f&&/version/{print $2; exit}')"
        fi
        rm -f "$tmp"
        if [ -n "$minos" ] && [ "$(printf '%s\n%s\n' "$MIN_OS" "$minos" | sort -V | tail -1)" != "$MIN_OS" ]; then
            echo "lipo-sidecars: ERROR $name ($slice_arch) requires macOS $minos, above the app's $MIN_OS" >&2
            exit 1
        fi
    done
done

echo "lipo-sidecars: combined $made sidecar(s)"
