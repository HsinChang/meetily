use log::{error, info};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use crate::state::AppState;

#[derive(Serialize)]
pub struct DatabaseCheckResult {
    pub exists: bool,
    pub size: u64,
}

/// Check if this is the first launch (no database exists yet)
#[tauri::command]
pub async fn check_first_launch(app: AppHandle) -> Result<bool, String> {
    DatabaseManager::is_first_launch(&app)
        .await
        .map_err(|e| format!("Failed to check first launch: {}", e))
}

/// Open a dialog to select a folder or file for legacy database import
#[tauri::command]
pub async fn select_legacy_database_path(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    info!("Opening dialog to select legacy database location");

    let file_path = app
        .dialog()
        .file()
        .add_filter("Database Files", &["db"])
        .blocking_pick_file();

    if let Some(path) = file_path {
        let path_str = path.to_string();
        info!("User selected path: {}", path_str);
        Ok(Some(path_str))
    } else {
        info!("User cancelled file selection");
        Ok(None)
    }
}

/// Detect legacy database from a selected path (root repo, backend folder, or db file)
#[tauri::command]
pub async fn detect_legacy_database(selected_path: String) -> Result<Option<String>, String> {
    let path = PathBuf::from(&selected_path);

    info!("Detecting legacy database from path: {}", selected_path);

    // Case 1: User selected the .db file directly
    if path.is_file() {
        if let Some(extension) = path.extension() {
            if extension == "db" {
                info!("Direct .db file selected: {}", selected_path);
                return Ok(Some(selected_path));
            }
        }
    }

    // Case 2: User selected directory containing meeting_minutes.db
    if path.is_dir() {
        let direct_db = path.join("meeting_minutes.db");
        if direct_db.exists() && direct_db.is_file() {
            let db_path = direct_db.to_string_lossy().to_string();
            info!("Found database in selected directory: {}", db_path);
            return Ok(Some(db_path));
        }

        // Case 3: User selected root repo (check backend subdirectory)
        let backend_db = path.join("backend").join("meeting_minutes.db");
        if backend_db.exists() && backend_db.is_file() {
            let db_path = backend_db.to_string_lossy().to_string();
            info!("Found database in backend subdirectory: {}", db_path);
            return Ok(Some(db_path));
        }
    }

    info!("No legacy database found at path: {}", selected_path);
    Ok(None)
}

/// Check for legacy database in the default app data directory
#[tauri::command]
pub async fn check_default_legacy_database(app: AppHandle) -> Result<Option<String>, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let legacy_db = app_data_dir.join("meeting_minutes.db");
    info!("Checking for default legacy database at: {:?}", legacy_db);

    if legacy_db.exists() && legacy_db.is_file() {
        let path_str = legacy_db.to_string_lossy().to_string();
        info!("Found default legacy database: {}", path_str);
        Ok(Some(path_str))
    } else {
        info!("No default legacy database found");
        Ok(None)
    }
}

/// Check if the Homebrew database exists and return its size
/// This is specifically for detecting old Python backend installations
#[tauri::command]
pub async fn check_homebrew_database(path: String) -> Result<Option<DatabaseCheckResult>, String> {
    let db_path = PathBuf::from(&path);
    
    info!("Checking for Homebrew database at: {}", path);
    
    // Check if file exists and is a regular file
    if db_path.exists() && db_path.is_file() {
        // Get file metadata to check size
        match std::fs::metadata(&db_path) {
            Ok(metadata) => {
                let size = metadata.len();
                info!("Found Homebrew database: {} ({} bytes)", path, size);
                
                // Only consider it valid if it has content (not empty)
                if size > 0 {
                    Ok(Some(DatabaseCheckResult {
                        exists: true,
                        size,
                    }))
                } else {
                    info!("Database file exists but is empty");
                    Ok(None)
                }
            }
            Err(e) => {
                error!("Failed to read database metadata: {}", e);
                Ok(None)
            }
        }
    } else {
        info!("No database found at Homebrew location");
        Ok(None)
    }
}

/// Known locations of a database left behind by the old Python/Homebrew Meetily.
/// The Homebrew prefix differs between Intel and Apple Silicon.
const LEGACY_DB_SEARCH_PATHS: &[&str] = &[
    "/opt/homebrew/var/meetily/meeting_minutes.db",
    "/usr/local/var/meetily/meeting_minutes.db",
];

/// Look for a database from a previous Meetily install, so the import prompt is only
/// shown to users who actually have one.
///
/// `app_data/meeting_minutes.db` is deliberately not reported: `DatabaseManager::new`
/// already migrates that automatically on first launch, so there is nothing to ask about.
#[tauri::command]
pub async fn find_legacy_database(_app: AppHandle) -> Result<Option<String>, String> {
    for path in LEGACY_DB_SEARCH_PATHS {
        if PathBuf::from(path).is_file() {
            info!("Found legacy database: {}", path);
            return Ok(Some(path.to_string()));
        }
    }
    info!("No legacy database found");
    Ok(None)
}

/// Import a legacy database, replacing the current one, then restart.
///
/// Since the app now always initializes a database at startup, `meeting_minutes.sqlite`
/// exists by the time this runs — and `DatabaseManager::new` only migrates a legacy `.db`
/// when the `.sqlite` is absent. Rather than duplicating that migration here, this stages
/// the legacy file where the existing (and tested) migration path expects it, removes the
/// freshly created database, and restarts so the migration runs on the next launch.
///
/// Refuses to run once the current database holds meetings, so an import can never
/// destroy real data.
#[tauri::command]
pub async fn import_and_initialize_database(
    app: AppHandle,
    legacy_db_path: String,
) -> Result<(), String> {
    info!("Starting import of legacy database from: {}", legacy_db_path);

    if !PathBuf::from(&legacy_db_path).is_file() {
        return Err(format!("No database at {}", legacy_db_path));
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    // Guard: only import over an empty database.
    if let Some(state) = app.try_state::<AppState>() {
        let meetings: i64 = sqlx::query_scalar("SELECT count(*) FROM meetings")
            .fetch_one(state.db_manager.pool())
            .await
            .unwrap_or(0);
        if meetings > 0 {
            return Err(format!(
                "This install already has {} meeting(s). Importing would replace them, so it has been cancelled.",
                meetings
            ));
        }

        // Checkpoint and close the pool before the file is removed underneath it.
        if let Err(e) = state.db_manager.cleanup().await {
            log::warn!("Database cleanup before import failed: {}", e);
        }
    }

    // Stage the legacy file where DatabaseManager::new looks for it.
    std::fs::copy(&legacy_db_path, app_data_dir.join("meeting_minutes.db"))
        .map_err(|e| format!("Failed to copy legacy database: {}", e))?;

    // Remove the empty database (and its WAL sidecars) so the migration is taken.
    for name in [
        "meeting_minutes.sqlite",
        "meeting_minutes.sqlite-wal",
        "meeting_minutes.sqlite-shm",
    ] {
        let p = app_data_dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p)
                .map_err(|e| format!("Failed to clear {}: {}", p.display(), e))?;
        }
    }

    info!("Legacy database staged; restarting to complete the import");
    app.restart();
}

/// Seed the model configuration a brand-new install starts with.
///
/// Shared by `initialize_fresh_database` and the startup path in `super::setup`, which
/// both have to produce the same defaults for a first launch. Failures are logged rather
/// than propagated: a missing default row is recoverable from the UI, whereas refusing to
/// start is not.
pub(crate) async fn apply_first_launch_defaults(pool: &sqlx::SqlitePool) {
    // "First launch" is decided by the absence of meeting_minutes.sqlite, which is also
    // true while a legacy database is being migrated into place. Writing defaults
    // unconditionally then discards the imported configuration — it reset an imported
    // `funasr` selection back to `parakeet`. Only seed what is missing.
    let has_transcript_config: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transcript_settings WHERE id = '1')")
            .fetch_one(pool)
            .await
            .unwrap_or(false);
    let has_model_config: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM settings WHERE id = '1')")
            .fetch_one(pool)
            .await
            .unwrap_or(false);

    if has_transcript_config && has_model_config {
        info!("Existing configuration found (imported database?), leaving defaults alone");
        return;
    }

    let default_summary_model =
        crate::summary::summary_engine::commands::get_recommended_summary_model_for_current_system()
            .unwrap_or("qwen3.5:2b");

    // Default Summary Model: Built-in AI (Qwen recommendation for this system)
    if !has_model_config {
      if let Err(e) = crate::database::repositories::setting::SettingsRepository::save_model_config(
        pool,
        "builtin-ai",
        default_summary_model,
        "large-v3", // Default whisper model (unused for builtin but required)
        None,
    )
    .await
    {
        error!("Failed to set default summary model config: {}", e);
    }

    // Default Transcription Model: Parakeet
    if let Err(e) =
        crate::database::repositories::setting::SettingsRepository::save_transcript_config(
            pool,
            "parakeet",
            crate::config::DEFAULT_PARAKEET_MODEL,
        )
        .await
    {
        error!("Failed to set default transcription model config: {}", e);
      }
    }
}

/// Initialize a fresh database (for users who don't want to import)
#[tauri::command]
pub async fn initialize_fresh_database(app: AppHandle) -> Result<(), String> {
    info!("Initializing fresh database");

    let db_manager = DatabaseManager::new_from_app_handle(&app)
        .await
        .map_err(|e| {
            error!("Failed to initialize fresh database: {}", e);
            format!("Failed to initialize database: {}", e)
        })?;

    // Update app state with the new manager
    app.manage(AppState { db_manager: db_manager.clone() });

    apply_first_launch_defaults(db_manager.pool()).await;

    info!("Fresh database initialized successfully with default models");

    // Emit event to notify frontend that database is ready
    app.emit("database-initialized", ())
        .map_err(|e| format!("Failed to emit database-initialized event: {}", e))?;

    Ok(())
}

/// Get the database directory path
#[tauri::command]
pub async fn get_database_directory(app: AppHandle) -> Result<String, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    Ok(app_data_dir.to_string_lossy().to_string())
}

/// Open the database folder in the system file explorer
#[tauri::command]
pub async fn open_database_folder(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    // Ensure directory exists before trying to open it
    if !app_data_dir.exists() {
        std::fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let folder_path = app_data_dir.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    info!("Opened database folder: {}", folder_path);
    Ok(())
}
