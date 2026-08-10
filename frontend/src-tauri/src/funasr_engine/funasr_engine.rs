// funasr_engine/funasr_engine.rs
//
// Fun-ASR-Nano engine: model discovery/download/load plus the transcription facade.
// Inference itself lives in the funasr-helper sidecar (see sidecar.rs); this type owns
// the model files on disk and decides which window sizes reach the helper.
//
// Layout mirrors Parakeet's — one subdirectory per model holding several files:
//   <models_dir>/funasr/<model-name>/{funasr-encoder-f16.gguf, qwen3-0.6b-q8_0.gguf}

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::RwLock;

use super::sidecar::{FunAsrModelPaths, FunAsrSidecar};
use crate::config::{funasr_model_files, FUNASR_MODEL_CATALOG};

/// Longest span sent to the helper in one request.
///
/// Benchmarking on a 611 s Mandarin recording: fixed ~15 s windows beat FSMN-VAD's ~2.5 s
/// segments outright (8/8 vs 6/8 known error sites corrected, 3031 vs 2275 chars), because
/// short windows starve the LLM decoder of context. Callers normally pass VAD segments
/// well under this; the split is a guard for unexpectedly long spans, not the main path.
const MAX_WINDOW_SAMPLES: usize = 25 * 16_000;

/// Below this the helper returns empty anyway (one fbank frame needs 400 samples).
const MIN_WINDOW_SAMPLES: usize = 1_600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelStatus {
    Available,
    Missing,
    Downloading { progress: u8 },
    Error(String),
    Corrupted { file_size: u64, expected_min_size: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub downloaded_mb: f64,
    pub total_mb: f64,
    pub speed_mbps: f64,
    pub percent: u8,
}

impl DownloadProgress {
    pub fn new(downloaded: u64, total: u64, speed_mbps: f64) -> Self {
        let percent = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0).min(100.0) as u8
        } else {
            0
        };
        Self {
            downloaded_bytes: downloaded,
            total_bytes: total,
            downloaded_mb: downloaded as f64 / (1024.0 * 1024.0),
            total_mb: total as f64 / (1024.0 * 1024.0),
            speed_mbps,
            percent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub path: PathBuf,
    pub size_mb: u32,
    pub accuracy: String,
    pub speed: String,
    pub status: ModelStatus,
    pub description: String,
}

pub struct FunAsrEngine {
    models_dir: PathBuf,
    sidecar: Arc<FunAsrSidecar>,
    current_model_name: Arc<RwLock<Option<String>>>,
    available_models: Arc<RwLock<HashMap<String, ModelInfo>>>,
    cancel_download_flag: Arc<RwLock<Option<String>>>,
    active_downloads: Arc<RwLock<HashSet<String>>>,
}

impl FunAsrEngine {
    pub fn new_with_models_dir(models_dir: Option<PathBuf>) -> Result<Self> {
        let base = match models_dir {
            Some(dir) => dir,
            None => std::env::current_dir()?.join("models"),
        };
        let models_dir = base.join("funasr");
        std::fs::create_dir_all(&models_dir)?;

        Ok(Self {
            models_dir,
            sidecar: Arc::new(FunAsrSidecar::new()?),
            current_model_name: Arc::new(RwLock::new(None)),
            available_models: Arc::new(RwLock::new(HashMap::new())),
            cancel_download_flag: Arc::new(RwLock::new(None)),
            active_downloads: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    pub async fn get_models_directory(&self) -> PathBuf {
        self.models_dir.clone()
    }

    fn model_dir(&self, model_name: &str) -> PathBuf {
        self.models_dir.join(model_name)
    }

    /// Resolve the on-disk paths the sidecar needs, verifying both files exist.
    fn resolve_paths(&self, model_name: &str) -> Result<FunAsrModelPaths> {
        let files = funasr_model_files(model_name)
            .ok_or_else(|| anyhow!("Unknown Fun-ASR model '{}'", model_name))?;
        let dir = self.model_dir(model_name);
        let encoder = dir.join(files.encoder);
        let llm = dir.join(files.llm);

        for p in [&encoder, &llm] {
            if !p.exists() {
                return Err(anyhow!("Missing model file: {}", p.display()));
            }
        }
        Ok(FunAsrModelPaths { encoder, llm })
    }

    /// Catalog joined with what is actually on disk.
    pub async fn discover_models(&self) -> Result<Vec<ModelInfo>> {
        let mut models = Vec::new();
        let mut map = HashMap::new();

        for entry in FUNASR_MODEL_CATALOG {
            let dir = self.model_dir(entry.name);
            let status = match self.check_model_files(entry.name).await {
                Ok(true) => ModelStatus::Available,
                Ok(false) => ModelStatus::Missing,
                Err(e) => ModelStatus::Error(e.to_string()),
            };

            let info = ModelInfo {
                name: entry.name.to_string(),
                path: dir,
                size_mb: entry.size_mb,
                accuracy: entry.accuracy.to_string(),
                speed: entry.speed.to_string(),
                status,
                description: entry.description.to_string(),
            };
            map.insert(entry.name.to_string(), info.clone());
            models.push(info);
        }

        *self.available_models.write().await = map;
        Ok(models)
    }

    /// True when every file for the model is present and plausibly complete.
    /// Truncated downloads are the common failure, so size is checked, not just existence.
    async fn check_model_files(&self, model_name: &str) -> Result<bool> {
        let files = match funasr_model_files(model_name) {
            Some(f) => f,
            None => return Ok(false),
        };
        let dir = self.model_dir(model_name);

        for (name, min_bytes) in [
            (files.encoder, files.encoder_min_bytes),
            (files.llm, files.llm_min_bytes),
        ] {
            let path = dir.join(name);
            match fs::metadata(&path).await {
                Ok(md) => {
                    if md.len() < min_bytes {
                        return Err(anyhow!(
                            "{} is truncated ({} bytes, expected at least {})",
                            name,
                            md.len(),
                            min_bytes
                        ));
                    }
                }
                Err(_) => return Ok(false),
            }
        }
        Ok(true)
    }

    pub async fn has_available_models(&self) -> bool {
        for entry in FUNASR_MODEL_CATALOG {
            if matches!(self.check_model_files(entry.name).await, Ok(true)) {
                return true;
            }
        }
        false
    }

    /// Start the sidecar against `model_name`. Cheap when already loaded.
    pub async fn load_model(&self, model_name: &str) -> Result<()> {
        let paths = self.resolve_paths(model_name)?;
        self.sidecar.ensure_running(paths).await?;
        *self.current_model_name.write().await = Some(model_name.to_string());
        log::info!("Fun-ASR model '{}' loaded", model_name);
        Ok(())
    }

    pub async fn unload_model(&self) -> bool {
        let was_loaded = self.sidecar.is_healthy();
        if let Err(e) = self.sidecar.shutdown().await {
            log::warn!("Error shutting down funasr-helper: {}", e);
        }
        *self.current_model_name.write().await = None;
        was_loaded
    }

    pub async fn is_model_loaded(&self) -> bool {
        self.sidecar.is_healthy() && self.sidecar.current_paths().await.is_some()
    }

    pub async fn get_current_model(&self) -> Option<String> {
        if self.sidecar.is_healthy() {
            self.current_model_name.read().await.clone()
        } else {
            None
        }
    }

    /// Transcribe 16 kHz mono f32 samples, splitting anything over MAX_WINDOW_SAMPLES.
    pub async fn transcribe_samples(&self, samples: Vec<f32>) -> Result<String> {
        if !self.sidecar.is_healthy() {
            return Err(anyhow!("No Fun-ASR model loaded"));
        }
        if samples.len() < MIN_WINDOW_SAMPLES {
            return Ok(String::new());
        }

        if samples.len() <= MAX_WINDOW_SAMPLES {
            return self.sidecar.transcribe(&samples).await;
        }

        let mut out = String::new();
        for window in samples.chunks(MAX_WINDOW_SAMPLES) {
            if window.len() < MIN_WINDOW_SAMPLES {
                continue;
            }
            let text = self.sidecar.transcribe(window).await?;
            out.push_str(text.trim());
        }
        Ok(out)
    }

    pub async fn delete_model(&self, model_name: &str) -> Result<String> {
        let dir = self.model_dir(model_name);
        if !dir.exists() {
            return Err(anyhow!("Model '{}' is not downloaded", model_name));
        }

        // The sidecar holds the files open; stop it before unlinking.
        if self.get_current_model().await.as_deref() == Some(model_name) {
            self.unload_model().await;
        }

        fs::remove_dir_all(&dir).await?;
        self.discover_models().await.ok();
        Ok(format!("Deleted Fun-ASR model '{}'", model_name))
    }

    pub async fn cancel_download(&self, model_name: &str) -> Result<()> {
        *self.cancel_download_flag.write().await = Some(model_name.to_string());
        Ok(())
    }

    /// Download every file for a model, resuming partial files via HTTP Range.
    /// Progress is weighted by each file's share of the total so the bar is monotonic.
    pub async fn download_model<F>(&self, model_name: &str, mut progress: Option<F>) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let files = funasr_model_files(model_name)
            .ok_or_else(|| anyhow!("Unknown Fun-ASR model '{}'", model_name))?;

        {
            let mut active = self.active_downloads.write().await;
            if active.contains(model_name) {
                return Err(anyhow!("Download already in progress for '{}'", model_name));
            }
            active.insert(model_name.to_string());
        }
        // Guard so an early return below cannot strand the model in "downloading".
        let _cleanup = DownloadGuard {
            active: self.active_downloads.clone(),
            name: model_name.to_string(),
        };

        let dir = self.model_dir(model_name);
        fs::create_dir_all(&dir).await?;

        let targets = [
            (files.encoder, files.encoder_url, files.encoder_min_bytes),
            (files.llm, files.llm_url, files.llm_min_bytes),
        ];
        let grand_total: u64 = targets.iter().map(|(_, _, min)| *min).sum();
        let mut completed_bytes: u64 = 0;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3600))
            .build()?;

        for (name, url, min_bytes) in targets {
            let dest = dir.join(name);

            if let Ok(md) = fs::metadata(&dest).await {
                if md.len() >= min_bytes {
                    log::info!("Fun-ASR file already present, skipping: {}", name);
                    completed_bytes += md.len();
                    continue;
                }
            }

            let resume_from = fs::metadata(&dest).await.map(|m| m.len()).unwrap_or(0);
            let mut request = client.get(url);
            if resume_from > 0 {
                log::info!("Resuming {} from {} bytes", name, resume_from);
                request = request.header("Range", format!("bytes={}-", resume_from));
            }

            let response = request.send().await?;
            if !response.status().is_success() && response.status().as_u16() != 206 {
                return Err(anyhow!("Download of {} failed: HTTP {}", name, response.status()));
            }

            let remaining = response.content_length().unwrap_or(0);
            let file_total = if resume_from > 0 { resume_from + remaining } else { remaining };

            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resume_from > 0)
                .truncate(resume_from == 0)
                .open(&dest)
                .await?;
            let mut writer = BufWriter::new(file);

            let mut downloaded = resume_from;
            let started = Instant::now();
            let mut stream = response.bytes_stream();
            use futures_util::StreamExt;

            while let Some(chunk) = stream.next().await {
                // Honour cancellation between chunks; the partial file is left on disk so
                // a retry resumes rather than restarting.
                if self.cancel_download_flag.read().await.as_deref() == Some(model_name) {
                    *self.cancel_download_flag.write().await = None;
                    writer.flush().await.ok();
                    return Err(anyhow!("Download cancelled"));
                }

                let chunk = chunk?;
                writer.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;

                if let Some(cb) = progress.as_mut() {
                    let elapsed = started.elapsed().as_secs_f64().max(0.001);
                    let speed = (downloaded - resume_from) as f64 / (1024.0 * 1024.0) / elapsed;
                    cb(DownloadProgress::new(
                        completed_bytes + downloaded,
                        grand_total.max(completed_bytes + file_total),
                        speed,
                    ));
                }
            }

            writer.flush().await?;
            completed_bytes += downloaded;
            log::info!("Downloaded Fun-ASR file {} ({} bytes)", name, downloaded);
        }

        self.discover_models().await.ok();
        log::info!("Fun-ASR model '{}' download complete", model_name);
        Ok(())
    }
}

/// Clears the in-progress marker however `download_model` returns.
struct DownloadGuard {
    active: Arc<RwLock<HashSet<String>>>,
    name: String,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        let active = self.active.clone();
        let name = self.name.clone();
        tokio::spawn(async move {
            active.write().await.remove(&name);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end check of the Rust -> funasr-helper chain against real model files.
    ///
    /// Ignored by default: needs the ~1.2 GB model download and a built funasr-helper.
    ///   cargo test --lib funasr_engine::tests::transcribes_real_audio -- --ignored --nocapture
    ///
    /// Point MEETILY_TEST_FUNASR_WAV at a 16 kHz mono 16-bit WAV, and optionally
    /// MEETILY_TEST_MODELS_DIR at the directory containing `funasr/<model>/`.
    #[tokio::test]
    #[ignore]
    async fn transcribes_real_audio() {
        let models_dir = std::env::var("MEETILY_TEST_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").expect("HOME not set");
                PathBuf::from(home)
                    .join("Library/Application Support/com.meetily.ai/models")
            });

        let wav_path = std::env::var("MEETILY_TEST_FUNASR_WAV")
            .expect("set MEETILY_TEST_FUNASR_WAV to a 16kHz mono s16 wav");

        let samples = read_wav_16k_mono(&wav_path);
        assert!(!samples.is_empty(), "test wav decoded to zero samples");

        let engine = FunAsrEngine::new_with_models_dir(Some(models_dir))
            .expect("failed to construct engine");

        let models = engine.discover_models().await.expect("discover failed");
        assert!(!models.is_empty(), "catalog is empty");

        engine
            .load_model(crate::config::DEFAULT_FUNASR_MODEL)
            .await
            .expect("load_model failed (are the model files downloaded?)");
        assert!(engine.is_model_loaded().await);
        assert_eq!(
            engine.get_current_model().await.as_deref(),
            Some(crate::config::DEFAULT_FUNASR_MODEL)
        );

        // A window longer than MAX_WINDOW_SAMPLES also exercises the internal split.
        let text = engine
            .transcribe_samples(samples)
            .await
            .expect("transcription failed");
        println!("transcript ({} chars): {}", text.chars().count(), text);
        assert!(!text.trim().is_empty(), "transcription was empty");

        assert!(engine.unload_model().await, "engine reported nothing to unload");
        assert!(!engine.is_model_loaded().await);
    }

    /// Minimal 16-bit PCM WAV reader — avoids pulling the decoder into this test.
    fn read_wav_16k_mono(path: &str) -> Vec<f32> {
        let bytes = std::fs::read(path).expect("failed to read test wav");
        // Walk RIFF chunks to find `data`; the header is not always exactly 44 bytes.
        let mut pos = 12;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let body = pos + 8;
            if id == b"data" {
                let end = (body + size).min(bytes.len());
                return bytes[body..end]
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect();
            }
            pos = body + size + (size & 1);
        }
        panic!("no data chunk found in {}", path);
    }
}
