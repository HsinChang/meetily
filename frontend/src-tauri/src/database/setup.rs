use log::info;
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use crate::state::AppState;

/// Initialize database on app startup
/// Handles first launch detection and conditional initialization
pub async fn initialize_database_on_startup(app: &AppHandle) -> Result<(), String> {
    // Check if this is the first launch (no database exists yet)
    let is_first_launch = DatabaseManager::is_first_launch(app)
        .await
        .map_err(|e| format!("Failed to check first launch status: {}", e))?;

    // The database is initialized here on every launch, including the first.
    //
    // First launch used to be deferred: this function emitted `first-launch-detected` and
    // left AppState unmanaged, relying on the onboarding wizard to call
    // `initialize_fresh_database` once the user finished setup. Removing the wizard removed
    // that call, so a fresh install came up with no managed state and every state-backed
    // command failed with "state not managed for field `state` on command ...". Existing
    // installs were unaffected, which is why it only showed up on a clean machine.
    let db_manager = DatabaseManager::new_from_app_handle(app)
        .await
        .map_err(|e| format!("Failed to initialize database manager: {}", e))?;

    let pool = db_manager.pool().clone();
    app.manage(AppState { db_manager });

    if is_first_launch {
        info!("First launch detected - seeding default configuration");
        super::commands::apply_first_launch_defaults(&pool).await;

        // Still emitted for any listener; delayed so the window and its React listeners
        // are up. Nothing depends on it for initialization any more.
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if let Err(e) = app_handle.emit("first-launch-detected", ()) {
                log::warn!("Failed to emit first-launch-detected event: {}", e);
            }
        });
    }

    info!("Database initialized successfully");
    Ok(())
}
