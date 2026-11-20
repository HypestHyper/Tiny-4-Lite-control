//! Process-wide settings state. Camera I/O runs on a worker thread, so the
//! persisted configuration has to be shared across threads.

use crate::config::{self, AppConfig};
use std::sync::{Mutex, OnceLock};

fn store() -> &'static Mutex<AppConfig> {
    static STORE: OnceLock<Mutex<AppConfig>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(AppConfig::default()))
}

/// Replace the in-memory configuration, for example after loading it from disk.
pub fn install_config(config: AppConfig) {
    *store().lock().unwrap_or_else(|e| e.into_inner()) = config;
}

pub fn current_config() -> AppConfig {
    store().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Persist a candidate snapshot first. If disk I/O or validation fails the
/// in-memory configuration stays unchanged as well.
pub fn update_config_checked(update: impl FnOnce(&mut AppConfig)) -> Result<(), String> {
    let mut snapshot = current_config();
    update(&mut snapshot);
    config::save(&snapshot)?;
    install_config(snapshot);
    Ok(())
}

pub fn update_config(update: impl FnOnce(&mut AppConfig)) {
    if let Err(error) = update_config_checked(update) {
        eprintln!("Could not save settings: {error}");
    }
}
