# Building Meetily for Windows x64

Meetily 0.5.0, target `x86_64-pc-windows-msvc`, bundles `.msi` + `.exe` (NSIS).

## Cross-compiling from macOS/Linux is not supported

Do not try. The blockers are structural:

- `whisper-rs` compiles whisper.cpp (C++/cmake) against the **MSVC** toolchain.
- `Cargo.toml` patches `esaxx-rs` to a `feat/dynamic-msvc-link` branch — MSVC-specific.
- MSI bundling requires **WiX**, which is Windows-only.
- `ort` (ONNX Runtime, for Parakeet) resolves platform-specific binaries.
- The `vulkan` feature links `vulkan-1.lib` from the Windows Vulkan SDK.

Use one of the two paths below.

---

## Path A — GitHub Actions (recommended)

The existing workflow already does everything correctly, on a clean `windows-latest` runner.

### First: the updater signing key

`tauri.conf.json` sets `createUpdaterArtifacts: true` and ships a `pubkey`, so Tauri
**requires** the matching private key. The fork `HsinChang/meetily` currently has **no
repository secrets**, so a run would fail at bundle time with a missing-private-key
error. Fix it once, either way:

```bash
# Option 1 — generate your own key pair and register it
cd frontend && pnpm tauri signer generate -w ~/.tauri/meetily.key
gh secret set TAURI_SIGNING_PRIVATE_KEY -R HsinChang/meetily < ~/.tauri/meetily.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -R HsinChang/meetily
```

Note: a self-generated key won't match the `pubkey` in `tauri.conf.json`, so
auto-updates from upstream releases won't verify. For a private build that's fine —
but if you want it coherent, replace `plugins.updater.pubkey` with your new public key.

```
# Option 2 — disable updater artifacts entirely
# in frontend/src-tauri/tauri.conf.json: "createUpdaterArtifacts": false
```

### Then run it

```bash
gh workflow run build-windows.yml -R HsinChang/meetily \
  -f build-type=release \
  -f sign-build=false \
  -f upload-artifacts=true

gh run watch                       # follow progress
gh run download <run-id>           # pull the .msi / .exe
```

`sign-build=false` is mandatory here — the DigiCert KeyLocker steps need org secrets
(`SM_API_KEY`, `SM_CLIENT_CERT_FILE_B64`, …) that the fork doesn't have. The result is
an unsigned installer: fine for testing, but Windows SmartScreen will warn on first run.

Artifacts land in `target/x86_64-pc-windows-msvc/release/bundle/{msi,nsis}/`.

---

## Path B — a real Windows x64 machine

### 1. Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Visual Studio Build Tools | 2022 | **"Desktop development with C++"** workload — supplies MSVC + Windows SDK |
| Rust | stable, via [rustup](https://rustup.rs) | must be the `x86_64-pc-windows-msvc` host |
| Node.js | 20 | |
| pnpm | 8 | `npm i -g pnpm@8` |
| CMake | 3.5+ | usually included with the VS C++ workload |
| Vulkan SDK | 1.4.309.0 | only for `--features vulkan` |
| LLVM/Clang | latest | needed by `bindgen` for whisper-rs |

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustup target add x86_64-pc-windows-msvc
```

> A copy of `vs_buildtools.exe` is already checked into `frontend/`.

### 2. GPU feature selection

Pick **one**. This only affects whisper transcription in the Tauri app.

| Hardware | Flag | Extra setup |
|---|---|---|
| NVIDIA | `--features cuda` | CUDA Toolkit 12.x |
| AMD / Intel | `--features vulkan` | Vulkan SDK 1.4.309.0 (CI's choice) |
| Any (safe default) | *(none)* | CPU-only |

For Vulkan, confirm `$env:VULKAN_SDK` is set and `$env:VULKAN_SDK\Lib\vulkan-1.lib` exists.
Also copy `vulkan-1.dll` into `frontend/src-tauri/vulkan-runtime/` so it gets bundled —
CI does this explicitly, and the app fails at runtime without it.

### 3. Updater signing key (required)

`tauri.conf.json` sets `createUpdaterArtifacts: true`, and a `pubkey` is present. **The
build will fail** unless the matching private key is in the environment:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "<base64 minisign private key>"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<password>"
```

See `frontend/.env.example`. If you don't have the project key, either generate a
throwaway pair (`pnpm tauri signer generate`) or set `createUpdaterArtifacts: false`
in `tauri.conf.json` — the resulting installer just won't self-update.

### 4. Authenticode signing (optional)

`scripts/sign-windows.ps1` no-ops cleanly when `DIGICERT_KEYPAIR_ALIAS` is unset, so
leaving it unset produces an unsigned build without erroring.

### 5. Build

```powershell
cd frontend
.\clean_build_windows.bat            # CPU-only
.\clean_build_windows.bat vulkan     # or: cuda / openblas
```

Or manually:

```powershell
cd frontend
pnpm install
cargo build --release -p llama-helper
mkdir src-tauri\binaries -Force
copy ..\target\release\llama-helper.exe src-tauri\binaries\llama-helper-x86_64-pc-windows-msvc.exe
pnpm run build
pnpm tauri build -- --target x86_64-pc-windows-msvc --features vulkan
```

`ffmpeg.exe` needs no manual step — `src-tauri/build/ffmpeg.rs` downloads and places
the Windows build automatically.

### 6. Output

```
target/x86_64-pc-windows-msvc/release/bundle/msi/*.msi
target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe
```

---

## Gotchas

**Bundled models inflate the installer.** `tauri.conf.json` lists `models/**/*` as a
resource. CI never populates it, so CI installers are ~lean and models download at
first run. If you run `node src-tauri/scripts/fetch-bundled-models.mjs` locally you
pull **~4.7 GB** (Qwen3.5-4B 2.6G, whisper large-v3-turbo 1.5G, parakeet 640M), all of
which gets baked into the installer. Skip that step unless you specifically want an
offline-capable installer.

**Never build `llama-helper` with `--features vulkan` on Windows.** `llama-cpp-sys-2`
has a CMake race condition there. CI builds it CPU-only on purpose; the GPU feature
belongs on the Tauri app (whisper-rs), not the sidecar.

**First build is slow** — 30–60 min, since whisper.cpp and llama.cpp compile from
source. Subsequent builds are incremental.

**Long path errors:** `git config --system core.longpaths true`, and keep the repo
somewhere shallow like `C:\meetily`.
