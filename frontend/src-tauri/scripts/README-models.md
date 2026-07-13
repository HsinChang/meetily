# Built-in models

The app ships three models built-in so the first-run "download a model" step is skipped:

| Model | File(s) | Approx. size | Source |
|-------|---------|--------------|--------|
| Whisper Large V3 Turbo | `models/ggml-large-v3-turbo.bin` | ~1.55 GB | HF `ggerganov/whisper.cpp` |
| Qwen 3.5 4B (LLM) | `models/Qwen3.5-4B-Q4_K_M.gguf` | ~2.5 GB | HF `unsloth/Qwen3.5-4B-GGUF` |
| Parakeet TDT 0.6B v3 (int8) | `models/parakeet/parakeet-tdt-0.6b-v3-int8/{encoder-model.int8.onnx, decoder_joint-model.int8.onnx, nemo128.onnx, vocab.txt}` | ~0.7 GB | meetily v3 ONNX mirror |

Total ~5 GB. **These binaries are git-ignored** (`src-tauri/models/` — see `.gitignore`) and must be
fetched before a production build.

## Download the models

From the `frontend/` directory:

```bash
pnpm fetch:models
# or, equivalently:
node src-tauri/scripts/fetch-bundled-models.mjs
```

The script downloads into `src-tauri/models/`, mirroring the on-disk layout the app expects. Existing
files with a plausible size are skipped, so re-runs are cheap and resumable-ish (partial files are
re-downloaded). Requires Node 18+ (uses the built-in `fetch`).

## How they get into the app

1. `src-tauri/models/**/*` is registered as a Tauri resource in `tauri.conf.json` (`bundle.resources`),
   so `pnpm tauri:build` packages the tree into the installer.
2. On first launch, `src-tauri/src/models_seed.rs` copies the bundled tree into the app-data
   `models/` directory. Once the files exist there, engine discovery reports the models as
   `Available` and no download prompt is shown.

`clean_build.sh` / `clean_build_windows.bat` run the fetch step automatically before building. In
**dev** (`pnpm tauri:dev`) the models are not bundled and the seed step is a no-op — dev reads
`frontend/models/` as before, so fetching is only needed for a bundled/production build.

## Changing a model

Update the URL/filename in `fetch-bundled-models.mjs` **and** the matching Rust catalog entry
(`src/config.rs` for Whisper, `src/summary/summary_engine/models.rs` for Qwen, or the Parakeet engine
for Parakeet) so the seeded file name matches what discovery looks for.
