// models_seed.rs
//
// Copies the models that ship built-in with the installer (Whisper large-v3-turbo,
// Parakeet v3 int8, and the Qwen 3.5 4B GGUF) from the read-only Tauri resource
// bundle into the writable app-data `models/` directory on first launch.
//
// Once the files exist under app-data `models/`, the existing engine discovery
// logic (WhisperEngine / ParakeetEngine / summary ModelManager) reports them as
// `Available`, so the "download a model" onboarding step is skipped automatically.
//
// The bundled tree is produced by scripts/fetch-bundled-models.mjs and registered
// via `bundle.resources` in tauri.conf.json. In dev builds the resource `models/`
// dir usually does not exist (dev reads frontend/models/ directly), so seeding is
// a no-op there.

use std::fs;
use std::path::Path;

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, Runtime};

/// Marker file written into the app-data models dir after a successful seed so we
/// skip the (potentially large) directory walk on subsequent launches.
///
/// Bumped to v2 so installs seeded by the earlier version re-run once and pick up
/// `link_summary_models`; their bundled GGUF was copied to a directory the model
/// lookup never reads. The re-walk is cheap because `copy_dir_missing` skips files
/// that already exist.
///
/// Bumped to v3 for the bundled Fun-ASR-Nano models (`models/funasr/<name>/*.gguf`).
/// Without a bump, every existing install short-circuits on the v2 marker and never
/// copies them, leaving Fun-ASR looking un-downloaded on upgrade.
const SEED_MARKER: &str = ".bundled-seeded-v3";

/// Seed bundled models into the app-data models directory. Best-effort: any error
/// is logged and swallowed so startup is never blocked by a copy failure.
pub fn seed_bundled_models<R: Runtime>(app: &AppHandle<R>) {
    let dest_models = match app.path().app_data_dir() {
        Ok(dir) => dir.join("models"),
        Err(e) => {
            log::error!("[seed] Could not resolve app_data_dir: {}", e);
            return;
        }
    };

    // Already seeded once? Nothing to do.
    if dest_models.join(SEED_MARKER).exists() {
        log::debug!("[seed] Bundled models already seeded, skipping");
        return;
    }

    // Locate the bundled `models/` resource directory.
    let src_models = match app.path().resolve("models", BaseDirectory::Resource) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[seed] Could not resolve bundled models resource dir: {}", e);
            return;
        }
    };

    if !src_models.exists() {
        // Expected in dev builds where models are not bundled as resources.
        log::info!(
            "[seed] No bundled models resource dir at {} (dev build?), skipping seed",
            src_models.display()
        );
        return;
    }

    log::info!(
        "[seed] Seeding bundled models: {} -> {}",
        src_models.display(),
        dest_models.display()
    );

    if let Err(e) = fs::create_dir_all(&dest_models) {
        log::error!("[seed] Failed to create dest models dir: {}", e);
        return;
    }

    match copy_dir_missing(&src_models, &dest_models) {
        Ok(copied) => {
            log::info!("[seed] Seed complete: {} file(s) copied", copied);
            link_summary_models(&dest_models);
            // Write marker so we don't re-walk on next launch.
            if let Err(e) = fs::write(dest_models.join(SEED_MARKER), b"1") {
                log::warn!("[seed] Failed to write seed marker: {}", e);
            }
        }
        Err(e) => {
            // Leave the marker absent so a future launch can retry the seed.
            log::error!("[seed] Seed failed: {}", e);
        }
    }
}

/// Make bundled LLM GGUFs visible to the built-in AI model lookup.
///
/// The bundle lays the GGUF out at `models/<file>.gguf`, but the summary engine
/// resolves models from `models/summary/` (see `summary_engine::models::
/// get_models_directory`). Without this step a bundled model is seeded to a
/// directory nothing reads, so summaries and translation both report "model not
/// found" on a fresh install.
///
/// Hard-links where possible to avoid a second multi-GB copy, falling back to a
/// real copy across filesystems. Never overwrites an existing file, so models the
/// user downloaded through the UI are left alone.
fn link_summary_models(dest_models: &Path) {
    let summary_dir = dest_models.join("summary");

    if let Err(e) = fs::create_dir_all(&summary_dir) {
        log::warn!("[seed] Failed to create summary models dir: {}", e);
        return;
    }

    let entries = match fs::read_dir(dest_models) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("[seed] Failed to read seeded models dir: {}", e);
            return;
        }
    };

    for entry in entries.flatten() {
        let src = entry.path();
        if src.extension().and_then(|ext| ext.to_str()) != Some("gguf") {
            continue;
        }

        let dst = summary_dir.join(entry.file_name());
        if dst.exists() {
            log::debug!("[seed] Summary model already present: {}", dst.display());
            continue;
        }

        match fs::hard_link(&src, &dst).or_else(|_| fs::copy(&src, &dst).map(|_| ())) {
            Ok(()) => log::info!("[seed] Linked summary model {}", dst.display()),
            Err(e) => log::warn!("[seed] Failed to link {}: {}", dst.display(), e),
        }
    }
}

/// Recursively copy `src` into `dst`, copying only files that do not already
/// exist at the destination (never overwrites user-managed files). Returns the
/// number of files copied.
fn copy_dir_missing(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let mut copied = 0u64;
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copied += copy_dir_missing(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            if dst_path.exists() {
                log::debug!(
                    "[seed] Skipping existing file: {}",
                    dst_path.display()
                );
                continue;
            }
            fs::copy(&src_path, &dst_path)?;
            copied += 1;
            log::info!("[seed] Copied {}", dst_path.display());
        }
    }

    Ok(copied)
}
