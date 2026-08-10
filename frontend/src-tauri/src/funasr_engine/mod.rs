// Fun-ASR-Nano transcription engine.
//
// Chinese-first ASR used where Whisper degrades on noisy Mandarin. Inference runs in the
// funasr-helper sidecar (see funasr-helper/README.md) rather than in-process: the Fun-ASR
// SAN-M audio encoder is a hand-built ggml graph with no Rust binding.

pub mod commands;
pub mod funasr_engine;
pub mod sidecar;

pub use funasr_engine::{DownloadProgress, FunAsrEngine, ModelInfo, ModelStatus};
pub use sidecar::{FunAsrModelPaths, FunAsrSidecar};
