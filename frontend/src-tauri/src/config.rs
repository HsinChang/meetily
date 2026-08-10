/// Application configuration constants
///
/// Centralized definitions for default models and settings.
/// Used across database initialization, import, and retranscription.

/// Default Whisper model for transcription when no preference is configured.
/// This is the recommended balance of accuracy and speed.
pub const DEFAULT_WHISPER_MODEL: &str = "large-v3-turbo";

/// Default Parakeet model for transcription when no preference is configured.
/// This is the quantized version optimized for speed.
pub const DEFAULT_PARAKEET_MODEL: &str = "parakeet-tdt-0.6b-v3-int8";

/// Default Fun-ASR model. Chinese-first ASR (Fun-ASR-Nano-2512, Apache-2.0), used where
/// Whisper degrades on noisy Mandarin. q8_0 is the highest-fidelity quant published for
/// the Qwen3 decoder half; there is ample speed headroom (~0.04 RTF) to afford it.
pub const DEFAULT_FUNASR_MODEL: &str = "fun-asr-nano-2512-q8";

/// The files that make up one Fun-ASR model.
///
/// Unlike Whisper (a single .bin), Fun-ASR is split into a SAN-M audio encoder and a Qwen3
/// decoder, loaded together by the funasr-helper sidecar. `*_min_bytes` are used both to
/// detect truncated downloads and to weight download progress.
pub struct FunAsrModelFiles {
    pub encoder: &'static str,
    pub encoder_url: &'static str,
    pub encoder_min_bytes: u64,
    pub llm: &'static str,
    pub llm_url: &'static str,
    pub llm_min_bytes: u64,
}

pub struct FunAsrCatalogEntry {
    pub name: &'static str,
    pub size_mb: u32,
    pub accuracy: &'static str,
    pub speed: &'static str,
    pub description: &'static str,
}

pub const FUNASR_MODEL_CATALOG: &[FunAsrCatalogEntry] = &[FunAsrCatalogEntry {
    name: "fun-asr-nano-2512-q8",
    size_mb: 1274,
    accuracy: "High (Chinese)",
    speed: "Fast",
    description: "Fun-ASR-Nano 800M — Chinese, dialects and accents. Best choice for noisy Mandarin audio.",
}];

/// Per-model file manifest. Kept next to the catalog so adding a quant means editing one place.
pub fn funasr_model_files(model_name: &str) -> Option<FunAsrModelFiles> {
    const ENCODER_URL: &str =
        "https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/resolve/main/funasr-encoder-f16.gguf";

    match model_name {
        "fun-asr-nano-2512-q8" => Some(FunAsrModelFiles {
            encoder: "funasr-encoder-f16.gguf",
            encoder_url: ENCODER_URL,
            encoder_min_bytes: 460_000_000, // ~469 MB
            llm: "qwen3-0.6b-q8_0.gguf",
            llm_url: "https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/resolve/main/qwen3-0.6b-q8_0.gguf",
            llm_min_bytes: 790_000_000, // ~805 MB
        }),
        _ => None,
    }
}

/// Whisper model catalog with metadata for all supported models.
/// Used by both WhisperEngine::discover_models() and discover_models_standalone().
///
/// Format: (name, filename, size_mb, accuracy, speed, description)
pub const WHISPER_MODEL_CATALOG: &[(&str, &str, u32, &str, &str, &str)] = &[
    // Standard f16 models (full precision)
    ("tiny", "ggml-tiny.bin", 74, "Decent", "Very Fast", "Fastest processing, good for real-time use"),
    ("base", "ggml-base.bin", 142, "Good", "Fast", "Good balance of speed and accuracy"),
    ("small", "ggml-small.bin", 466, "Good", "Medium", "Better accuracy, moderate speed"),
    ("medium", "ggml-medium.bin", 1463, "High", "Slow", "High accuracy for professional use"),
    ("large-v3-turbo", "ggml-large-v3-turbo.bin", 1549, "High", "Medium", "Best accuracy with improved speed"),
    ("large-v3", "ggml-large-v3.bin", 2951, "High", "Slow", "Most Accurate, latest large model"),

    // Q5_1 quantized models (balanced speed/accuracy, slightly better quality than Q5_0)
    ("tiny-q5_1", "ggml-tiny-q5_1.bin", 31, "Decent", "Very Fast", "Quantized tiny model, ~50% faster processing"),
    ("base-q5_1", "ggml-base-q5_1.bin", 57, "Good", "Fast", "Quantized base model, good speed/accuracy balance"),
    ("small-q5_1", "ggml-small-q5_1.bin", 181, "Good", "Fast", "Quantized small model, faster than f16 version"),

    // Q5_0 quantized models (balanced speed/accuracy)
    ("medium-q5_0", "ggml-medium-q5_0.bin", 514, "High", "Medium", "Quantized medium model, professional quality"),
    ("large-v3-turbo-q5_0", "ggml-large-v3-turbo-q5_0.bin", 547, "High", "Medium", "Quantized large model, best balance"),
    ("large-v3-q5_0", "ggml-large-v3-q5_0.bin", 1031, "High", "Slow", "Quantized large model, high accuracy"),
];
