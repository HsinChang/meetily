// funasr_engine/commands.rs
//
// Tauri command surface for Fun-ASR, mirroring the parakeet_* commands so the frontend
// model-manager component can be a near-copy of ParakeetModelManager.

use crate::funasr_engine::{DownloadProgress, FunAsrEngine, ModelInfo};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{command, AppHandle, Emitter, Manager, Runtime};

/// Global Fun-ASR engine, initialized during app setup.
pub static FUNASR_ENGINE: Mutex<Option<Arc<FunAsrEngine>>> = Mutex::new(None);

static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the models directory. Call during app setup, before `funasr_init`.
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get app data dir for Fun-ASR: {}", e);
            return;
        }
    };

    let models_dir = app_data_dir.join("models");
    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            log::error!("Failed to create models directory: {}", e);
            return;
        }
    }

    log::info!("Fun-ASR models directory set to: {}", models_dir.display());
    *MODELS_DIR.lock().unwrap() = Some(models_dir);
}

fn get_models_directory() -> Option<PathBuf> {
    MODELS_DIR.lock().unwrap().clone()
}

fn engine() -> Option<Arc<FunAsrEngine>> {
    FUNASR_ENGINE.lock().unwrap().as_ref().cloned()
}

#[command]
pub async fn funasr_init() -> Result<(), String> {
    let mut guard = FUNASR_ENGINE.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    let models_dir = get_models_directory();
    let engine = FunAsrEngine::new_with_models_dir(models_dir)
        .map_err(|e| format!("Failed to initialize Fun-ASR engine: {}", e))?;
    *guard = Some(Arc::new(engine));
    Ok(())
}

#[command]
pub async fn funasr_get_available_models() -> Result<Vec<ModelInfo>, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    engine
        .discover_models()
        .await
        .map_err(|e| format!("Failed to discover Fun-ASR models: {}", e))
}

#[command]
pub async fn funasr_load_model(model_name: String) -> Result<(), String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    engine
        .load_model(&model_name)
        .await
        .map_err(|e| format!("Failed to load Fun-ASR model: {}", e))
}

#[command]
pub async fn funasr_get_current_model() -> Result<Option<String>, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    Ok(engine.get_current_model().await)
}

#[command]
pub async fn funasr_is_model_loaded() -> Result<bool, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    Ok(engine.is_model_loaded().await)
}

#[command]
pub async fn funasr_has_available_models() -> Result<bool, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    Ok(engine.has_available_models().await)
}

/// Ensure a usable model is loaded, picking the configured default when none is named.
/// Called before recording so failures surface up front rather than mid-meeting.
#[command]
pub async fn funasr_validate_model_ready(model_name: Option<String>) -> Result<(), String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    let name = model_name.unwrap_or_else(|| crate::config::DEFAULT_FUNASR_MODEL.to_string());

    if engine.get_current_model().await.as_deref() == Some(name.as_str())
        && engine.is_model_loaded().await
    {
        return Ok(());
    }

    engine
        .load_model(&name)
        .await
        .map_err(|e| format!("Fun-ASR model '{}' is not ready: {}", name, e))
}

#[command]
pub async fn funasr_transcribe_audio(audio_data: Vec<f32>) -> Result<String, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    engine
        .transcribe_samples(audio_data)
        .await
        .map_err(|e| format!("Fun-ASR transcription failed: {}", e))
}

#[command]
pub async fn funasr_get_models_directory() -> Result<String, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    Ok(engine.get_models_directory().await.to_string_lossy().to_string())
}

#[command]
pub async fn funasr_download_model<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;

    let app_for_progress = app_handle.clone();
    let name_for_progress = model_name.clone();
    let progress_callback = move |progress: DownloadProgress| {
        if let Err(e) = app_for_progress.emit(
            "funasr-model-download-progress",
            serde_json::json!({
                "modelName": name_for_progress,
                "progress": progress.percent,
                "downloaded_bytes": progress.downloaded_bytes,
                "total_bytes": progress.total_bytes,
                "downloaded_mb": progress.downloaded_mb,
                "total_mb": progress.total_mb,
                "speed_mbps": progress.speed_mbps,
                "status": if progress.percent == 100 { "completed" } else { "downloading" }
            }),
        ) {
            log::error!("Failed to emit Fun-ASR download progress event: {}", e);
        }
    };

    match engine.download_model(&model_name, Some(progress_callback)).await {
        Ok(()) => {
            if let Err(e) = app_handle.emit(
                "funasr-model-download-complete",
                serde_json::json!({ "modelName": model_name }),
            ) {
                log::error!("Failed to emit Fun-ASR download complete event: {}", e);
            }
            crate::tray::update_tray_menu(&app_handle);
            Ok(())
        }
        Err(e) => {
            if let Err(emit_e) = app_handle.emit(
                "funasr-model-download-error",
                serde_json::json!({ "modelName": model_name, "error": e.to_string() }),
            ) {
                log::error!("Failed to emit Fun-ASR download error event: {}", emit_e);
            }
            Err(format!("Failed to download Fun-ASR model: {}", e))
        }
    }
}

#[command]
pub async fn funasr_cancel_download(model_name: String) -> Result<(), String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    engine
        .cancel_download(&model_name)
        .await
        .map_err(|e| format!("Failed to cancel download: {}", e))
}

#[command]
pub async fn funasr_delete_model(model_name: String) -> Result<String, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    engine
        .delete_model(&model_name)
        .await
        .map_err(|e| format!("Failed to delete Fun-ASR model: {}", e))
}

/// Stop the sidecar and free its ~1.2 GB of resident memory.
#[command]
pub async fn funasr_unload_model() -> Result<bool, String> {
    let engine = engine().ok_or("Fun-ASR engine not initialized")?;
    Ok(engine.unload_model().await)
}
