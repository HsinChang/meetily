// funasr_engine/sidecar.rs
//
// Process lifecycle for the funasr-helper sidecar: spawn, request/response over stdio,
// health, idle unload, graceful shutdown.
//
// This mirrors summary::summary_engine::sidecar (llama-helper) — same binary-resolution
// strategy, same keep-alive shape — with two differences forced by the protocol:
//
//   * Model paths are passed as argv at spawn rather than per request. Switching Fun-ASR
//     models means respawning; there is no in-process model swap.
//   * Requests carry a binary payload (raw f32 PCM) after the header line, so the write
//     path is not just "write a JSON line".

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, RwLock};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Idle seconds before the sidecar is unloaded to reclaim ~1.2 GB of RSS.
/// Override with MEETILY_FUNASR_IDLE_TIMEOUT.
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;

/// Ceiling for a single window. Generation is capped at 512 tokens and a 25 s window
/// takes well under a second, so anything beyond this means the helper is wedged.
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Responses from funasr-helper. Requests are plain lines (see funasr-helper/README.md);
/// only the response direction is JSON.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Ready,
    Response { text: String },
    Pong,
    Goodbye,
    Error { message: String },
}

/// Paths a sidecar instance is bound to for its lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunAsrModelPaths {
    pub encoder: PathBuf,
    pub llm: PathBuf,
}

pub struct FunAsrSidecar {
    child_process: Arc<Mutex<Option<Child>>>,
    stdin_writer: Arc<Mutex<Option<ChildStdin>>>,
    stdout_reader: Arc<Mutex<Option<BufReader<ChildStdout>>>>,
    last_activity: Arc<RwLock<Instant>>,
    is_healthy: Arc<AtomicBool>,
    active_request_count: Arc<AtomicUsize>,
    helper_binary_path: PathBuf,
    current_paths: Arc<RwLock<Option<FunAsrModelPaths>>>,
    idle_timeout_secs: u64,
    n_gpu_layers: i32,
}

/// Decrements the active-request count on drop so a panic mid-request cannot wedge
/// graceful shutdown.
struct RequestGuard {
    counter: Arc<AtomicUsize>,
}

impl RequestGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self { counter }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

impl FunAsrSidecar {
    pub fn new() -> Result<Self> {
        let helper_binary_path = Self::resolve_helper_binary()?;

        let idle_timeout_secs = std::env::var("MEETILY_FUNASR_IDLE_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS);

        // Offload the Qwen3 decoder to the GPU where one is available. Measured on Apple
        // Silicon: RTF 0.046 -> 0.037, and model load 9.7 s -> 0.3 s. Setting this to 0
        // via env is the escape hatch if a driver misbehaves.
        let n_gpu_layers = std::env::var("MEETILY_FUNASR_NGL")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(99);

        log::info!(
            "FunAsrSidecar initialized: binary={}, idle_timeout={}s, ngl={}",
            helper_binary_path.display(),
            idle_timeout_secs,
            n_gpu_layers
        );

        Ok(Self {
            child_process: Arc::new(Mutex::new(None)),
            stdin_writer: Arc::new(Mutex::new(None)),
            stdout_reader: Arc::new(Mutex::new(None)),
            last_activity: Arc::new(RwLock::new(Instant::now())),
            is_healthy: Arc::new(AtomicBool::new(false)),
            active_request_count: Arc::new(AtomicUsize::new(0)),
            helper_binary_path,
            current_paths: Arc::new(RwLock::new(None)),
            idle_timeout_secs,
            n_gpu_layers,
        })
    }

    fn target_triple() -> String {
        std::env::var("TARGET").unwrap_or_else(|_| {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            { "x86_64-unknown-linux-gnu".to_string() }
            #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
            { "aarch64-unknown-linux-gnu".to_string() }
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            { "x86_64-apple-darwin".to_string() }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            { "aarch64-apple-darwin".to_string() }
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            { "x86_64-pc-windows-msvc".to_string() }
            #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
            { "aarch64-pc-windows-msvc".to_string() }
            #[cfg(not(any(
                all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")),
                all(target_os = "macos", any(target_arch = "x86_64", target_arch = "aarch64")),
                all(target_os = "windows", any(target_arch = "x86_64", target_arch = "aarch64"))
            )))]
            { "unknown".to_string() }
        })
    }

    /// Locate funasr-helper across dev, bundled-app, and AppImage layouts.
    /// Same search order as the llama-helper sidecar, plus the CMake dev output dir.
    fn resolve_helper_binary() -> Result<PathBuf> {
        if let Ok(env_path) = std::env::var("MEETILY_FUNASR_HELPER") {
            if !env_path.is_empty() {
                let path = PathBuf::from(env_path);
                if path.exists() {
                    log::info!("Using funasr-helper from MEETILY_FUNASR_HELPER: {}", path.display());
                    return Ok(path);
                }
            }
        }

        let target_triple = Self::target_triple();
        let binary_name = if cfg!(windows) {
            format!("funasr-helper-{}.exe", target_triple)
        } else {
            format!("funasr-helper-{}", target_triple)
        };

        // Next to the executable (bundled app, AppImage)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let bundled = exe_dir.join(&binary_name);
                if bundled.exists() {
                    log::info!("Found funasr-helper next to executable: {}", bundled.display());
                    return Ok(bundled);
                }
                if let Ok(entries) = std::fs::read_dir(exe_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with("funasr-helper") && !name.ends_with(".d") {
                                log::info!("Found funasr-helper (fuzzy) next to executable: {}", path.display());
                                return Ok(path);
                            }
                        }
                    }
                }
            }
        }

        // Bundled resources
        if let Ok(resource_dir) = std::env::var("RESOURCE_DIR") {
            let resource_path = PathBuf::from(&resource_dir).join(&binary_name);
            if resource_path.exists() {
                log::info!("Found funasr-helper in RESOURCE_DIR: {}", resource_path.display());
                return Ok(resource_path);
            }
        }

        // Dev: the CMake build tree, and the staged binaries/ dir the build script writes
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let src_tauri = PathBuf::from(&manifest_dir);
            let repo_root = src_tauri.parent().and_then(|p| p.parent());

            let mut candidates = vec![
                src_tauri.join("binaries").join(&binary_name),
                src_tauri.join("binaries").join("funasr-helper"),
            ];
            if let Some(root) = repo_root {
                candidates.push(root.join("funasr-helper/build/bin/funasr-helper"));
                candidates.push(root.join("funasr-helper/build/bin/funasr-helper.exe"));
            }

            for candidate in candidates {
                if candidate.exists() {
                    log::info!("Using dev funasr-helper: {}", candidate.display());
                    return Ok(candidate);
                }
            }
        }

        Err(anyhow!(
            "funasr-helper binary not found. Build it with './scripts/build-funasr-helper.sh' \
             or set MEETILY_FUNASR_HELPER."
        ))
    }

    /// Spawn if not already running with these exact model paths.
    pub async fn ensure_running(&self, paths: FunAsrModelPaths) -> Result<()> {
        {
            let current = self.current_paths.read().await;
            if current.as_ref() == Some(&paths) && self.is_healthy() {
                self.update_activity().await;
                return Ok(());
            }
        }
        self.spawn(paths).await
    }

    async fn spawn(&self, paths: FunAsrModelPaths) -> Result<()> {
        self.shutdown().await?;

        log::info!(
            "Spawning funasr-helper (encoder={}, llm={})",
            paths.encoder.display(),
            paths.llm.display()
        );

        // Nice the process on unix so transcription never starves the audio capture threads.
        #[cfg(unix)]
        let mut command = tokio::process::Command::new("nice");
        #[cfg(not(unix))]
        let mut command = tokio::process::Command::new(&self.helper_binary_path);

        #[cfg(unix)]
        command.arg("-n").arg("10").arg(&self.helper_binary_path);

        command
            .arg("--enc").arg(&paths.encoder)
            .arg("-m").arg(&paths.llm)
            .arg("-ngl").arg(self.n_gpu_layers.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;
            command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("Failed to spawn funasr-helper at {:?}", self.helper_binary_path))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to get stdout"))?;

        *self.child_process.lock().await = Some(child);
        *self.stdin_writer.lock().await = Some(stdin);
        *self.stdout_reader.lock().await = Some(BufReader::new(stdout));

        // Wait for {"type":"ready"} — the helper loads ~1.2 GB before serving, which is
        // ~10 s on CPU. Until this arrives a TRANSCRIBE would just queue behind the load.
        match tokio::time::timeout(Duration::from_secs(180), self.read_response()).await {
            Ok(Ok(Response::Ready)) => {}
            Ok(Ok(Response::Error { message })) => {
                self.shutdown().await.ok();
                return Err(anyhow!("funasr-helper failed to start: {}", message));
            }
            Ok(Ok(other)) => {
                self.shutdown().await.ok();
                return Err(anyhow!("Unexpected handshake from funasr-helper: {:?}", other));
            }
            Ok(Err(e)) => {
                self.shutdown().await.ok();
                return Err(e.context("funasr-helper died during startup"));
            }
            Err(_) => {
                self.shutdown().await.ok();
                return Err(anyhow!("funasr-helper did not become ready within 180s"));
            }
        }

        *self.current_paths.write().await = Some(paths);
        self.is_healthy.store(true, Ordering::SeqCst);
        self.update_activity().await;

        log::info!("funasr-helper ready");
        self.start_idle_check_loop();
        Ok(())
    }

    /// Transcribe one window of 16 kHz mono f32 samples.
    pub async fn transcribe(&self, samples: &[f32]) -> Result<String> {
        let _guard = RequestGuard::new(self.active_request_count.clone());

        if !self.is_healthy() {
            return Err(anyhow!("funasr-helper is not running"));
        }

        // Header line, then the payload as little-endian f32. The helper reads exactly
        // n_samples*4 bytes, so a partial write here desynchronises the stream — which is
        // why a write failure below forces a shutdown rather than a retry.
        let mut payload = Vec::with_capacity(samples.len() * 4);
        for s in samples {
            payload.extend_from_slice(&s.to_le_bytes());
        }

        let write_result = async {
            let mut stdin_lock = self.stdin_writer.lock().await;
            let stdin = stdin_lock
                .as_mut()
                .ok_or_else(|| anyhow!("funasr-helper not running"))?;
            stdin
                .write_all(format!("TRANSCRIBE {}\n", samples.len()).as_bytes())
                .await
                .context("Failed to write TRANSCRIBE header")?;
            stdin.write_all(&payload).await.context("Failed to write audio payload")?;
            stdin.flush().await.context("Failed to flush stdin")?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(e) = write_result {
            self.is_healthy.store(false, Ordering::SeqCst);
            let _ = self.shutdown().await;
            return Err(e);
        }

        match tokio::time::timeout(TRANSCRIBE_TIMEOUT, self.read_response()).await {
            Ok(Ok(Response::Response { text })) => {
                self.update_activity().await;
                Ok(text)
            }
            Ok(Ok(Response::Error { message })) => Err(anyhow!("funasr-helper error: {}", message)),
            Ok(Ok(other)) => {
                // Out-of-band message means the stream is no longer aligned with requests.
                self.is_healthy.store(false, Ordering::SeqCst);
                let _ = self.shutdown().await;
                Err(anyhow!("Unexpected response from funasr-helper: {:?}", other))
            }
            Ok(Err(e)) => {
                self.is_healthy.store(false, Ordering::SeqCst);
                let _ = self.shutdown().await;
                Err(e)
            }
            Err(_) => {
                log::error!("funasr-helper transcription timed out, restarting sidecar");
                let _ = self.shutdown().await;
                Err(anyhow!("funasr-helper timed out after {:?}", TRANSCRIBE_TIMEOUT))
            }
        }
    }

    async fn read_response(&self) -> Result<Response> {
        let mut stdout_lock = self.stdout_reader.lock().await;
        let reader = stdout_lock
            .as_mut()
            .ok_or_else(|| anyhow!("funasr-helper not running"))?;

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .context("Failed to read from funasr-helper stdout")?;

        if line.is_empty() {
            return Err(anyhow!("funasr-helper closed stdout (process may have crashed)"));
        }

        serde_json::from_str(line.trim())
            .with_context(|| format!("Malformed response from funasr-helper: {}", line.trim()))
    }

    pub async fn shutdown(&self) -> Result<()> {
        if self.is_healthy() {
            let _ = async {
                let mut stdin_lock = self.stdin_writer.lock().await;
                if let Some(stdin) = stdin_lock.as_mut() {
                    stdin.write_all(b"SHUTDOWN\n").await?;
                    stdin.flush().await?;
                }
                Ok::<(), anyhow::Error>(())
            }
            .await;
        }

        {
            let mut child_lock = self.child_process.lock().await;
            if let Some(mut child) = child_lock.take() {
                match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                    Ok(Ok(status)) => log::info!("funasr-helper exited: {}", status),
                    Ok(Err(e)) => log::error!("Failed to wait for funasr-helper: {}", e),
                    Err(_) => {
                        log::warn!("funasr-helper didn't exit gracefully, killing");
                        let _ = child.kill().await;
                    }
                }
            }
        }

        *self.stdin_writer.lock().await = None;
        *self.stdout_reader.lock().await = None;
        *self.current_paths.write().await = None;
        self.is_healthy.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::SeqCst)
    }

    pub async fn current_paths(&self) -> Option<FunAsrModelPaths> {
        self.current_paths.read().await.clone()
    }

    async fn update_activity(&self) {
        *self.last_activity.write().await = Instant::now();
    }

    /// Unload the sidecar after a period of inactivity — it holds ~1.2 GB resident,
    /// which is not worth keeping around between meetings.
    fn start_idle_check_loop(&self) {
        let child_process = self.child_process.clone();
        let stdin_writer = self.stdin_writer.clone();
        let stdout_reader = self.stdout_reader.clone();
        let last_activity = self.last_activity.clone();
        let is_healthy = self.is_healthy.clone();
        let active_request_count = self.active_request_count.clone();
        let current_paths = self.current_paths.clone();
        let helper_binary_path = self.helper_binary_path.clone();
        let idle_timeout_secs = self.idle_timeout_secs;
        let n_gpu_layers = self.n_gpu_layers;

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;

                if !is_healthy.load(Ordering::SeqCst) {
                    break;
                }
                if active_request_count.load(Ordering::SeqCst) > 0 {
                    continue;
                }
                let idle = last_activity.read().await.elapsed().as_secs();
                if idle < idle_timeout_secs {
                    continue;
                }

                log::info!("funasr-helper idle for {}s, unloading", idle);
                let manager = FunAsrSidecar {
                    child_process: child_process.clone(),
                    stdin_writer: stdin_writer.clone(),
                    stdout_reader: stdout_reader.clone(),
                    last_activity: last_activity.clone(),
                    is_healthy: is_healthy.clone(),
                    active_request_count: active_request_count.clone(),
                    helper_binary_path: helper_binary_path.clone(),
                    current_paths: current_paths.clone(),
                    idle_timeout_secs,
                    n_gpu_layers,
                };
                let _ = manager.shutdown().await;
                break;
            }
        });
    }
}
