# funasr-helper

Persistent [Fun-ASR-Nano](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512) transcription sidecar for Meetily. Used by the `funasr` transcription provider for Chinese audio, where Whisper degrades badly on noisy microphone recordings.

Derived from Fun-ASR's `runtime/llama.cpp/funasr-cli/funasr-cli.cpp` (Apache-2.0). The signal path is upstream's, unmodified:

```
f32 16k mono -> kaldi fbank -> SAN-M encoder + adaptor (ggml)
             -> low-frame-rate truncation
             -> [prefix | audio embeds | suffix] -> Qwen3-0.6B (llama.cpp)
```

## Why a fork rather than the upstream CLI

Upstream loads ~1.2 GB of GGUF **per invocation**. That's fine for one-shot batch files and hopeless for live chunked transcription. This fork loads once and serves windows over stdio.

Four other differences, all driven by how Meetily feeds it:

- **Audio arrives as raw f32 samples on stdin**, not as a file — Meetily's pipeline already produces 16 kHz mono f32, so `miniaudio.h` / `funasr_audio.h` (4 MB of vendored headers) are dropped.
- **FSMN-VAD is dropped.** Meetily runs Silero VAD upstream. Benchmarking also showed FSMN's ~2.5 s segments *hurt* accuracy versus fixed ~15 s windows — short windows starve the LLM decoder of context.
- **GPU offload via `-ngl`**; upstream hardcodes `n_gpu_layers = 0`.
- **The ggml backend is created once** rather than per window.

## Build

```bash
./scripts/build-funasr-helper.sh          # from the repo root; auto-selects the GPU backend
MEETILY_GPU=cuda ./scripts/build-funasr-helper.sh
```

Stages the binary into `frontend/src-tauri/binaries/funasr-helper-<target-triple>`, where Tauri's `externalBin` expects it. CMake fetches llama.cpp pinned to `8086439a` — the same commit upstream Fun-ASR pins. Point `FETCHCONTENT_SOURCE_DIR_LLAMA` at an existing checkout to skip the fetch.

Bumping the llama.cpp pin must be done in lockstep with re-validating the encoder graph against upstream output (see below).

## Protocol

Requests are plain lines; responses are JSON, one per line on stdout. Requests aren't JSON because parsing it in C++ would mean vendoring a JSON library to read three fixed message shapes; responses are, so the Rust side can use serde — matching the `llama-helper` sidecar contract.

| Request | Response |
|---|---|
| `TRANSCRIBE <n_samples>\n` + `n_samples*4` bytes of little-endian f32 | `{"type":"response","text":"..."}` |
| `PING\n` | `{"type":"pong"}` |
| `SHUTDOWN\n` | `{"type":"goodbye"}`, then exit 0 |

`{"type":"ready"}` is emitted once models are loaded. Errors are `{"type":"error","message":"..."}`.

A short read on an audio payload is unrecoverable — the stream is desynchronised — so the helper exits and the parent must respawn it.

```bash
funasr-helper --enc funasr-encoder-f16.gguf -m qwen3-0.6b-q8_0.gguf -ngl 99
```

`-t` threads, `-c` context (default 2048), `-n` max tokens per window (512), `--rep` repetition penalty, `--prompt` overrides the Chinese transcription prompt.

## Verifying a change

The fork must stay output-equivalent to upstream. Transcribe a 16 kHz mono WAV through both at the same window size and diff:

```bash
llama-funasr-cli --enc enc.gguf -m llm.gguf -a audio.wav --chunk 15   # reference
```

against the helper driven at 15 s windows with `-ngl 0`. On a 611 s Mandarin recording these were byte-identical. Expect ~0.5% divergence with `-ngl 99` — GPU float ordering shifts greedy sampling at near-ties, mostly on filler tokens (呃/嗯) — so compare CPU-to-CPU when checking correctness.

## Measured performance

Apple M-series, 611 s Mandarin meeting recording, 15 s windows:

| | Model load | RTF |
|---|---|---|
| `-ngl 0` (CPU) | 9.7 s | 0.046 |
| `-ngl 99` (Metal) | 0.3 s | 0.037 |

~20–27× faster than real time, so live transcription has ample headroom.
