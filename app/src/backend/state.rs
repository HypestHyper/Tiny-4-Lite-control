//! Process-wide settings state. Camera I/O runs on a worker thread, so the
//! persisted configuration has to be shared across threads.

use crate::config::{self, AppConfig};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

fn store() -> &'static Mutex<AppConfig> {
    static STORE: OnceLock<Mutex<AppConfig>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(AppConfig::default()))
}

/// Replace the in-memory configuration, for example after loading it from disk.
pub fn install_config(config: AppConfig) {
    super::angles::set_tilt_limit(config.tilt_limit);
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

/// Change a setting without waiting for the disk. The change is visible at once
/// and written shortly after, so a slider dragged through fifty values causes
/// one write, not fifty. Use `update_config_checked` when the caller needs to
/// know the write succeeded.
pub fn update_config(update: impl FnOnce(&mut AppConfig)) {
    {
        let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
        update(&mut guard);
        super::angles::set_tilt_limit(guard.tilt_limit);
    }
    schedule_save();
}

struct Saver {
    dirty: Mutex<bool>,
    wake: Condvar,
}

fn saver() -> &'static Saver {
    static SAVER: OnceLock<&'static Saver> = OnceLock::new();
    SAVER.get_or_init(|| {
        let saver: &'static Saver = Box::leak(Box::new(Saver { dirty: Mutex::new(false), wake: Condvar::new() }));
        std::thread::Builder::new().name("config-save".into()).spawn(move || loop {
            {
                let mut dirty = saver.dirty.lock().unwrap_or_else(|e| e.into_inner());
                while !*dirty { dirty = saver.wake.wait(dirty).unwrap_or_else(|e| e.into_inner()); }
            }
            // Let a burst of changes finish, then write the latest state once.
            std::thread::sleep(Duration::from_millis(400));
            *saver.dirty.lock().unwrap_or_else(|e| e.into_inner()) = false;
            if let Err(error) = config::save(&current_config()) { eprintln!("Could not save settings: {error}"); }
        }).expect("config saver thread can start");
        saver
    })
}

fn schedule_save() {
    let saver = saver();
    *saver.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
    saver.wake.notify_one();
}

/// Write any pending change now. Called when the app quits.
pub fn flush_config() {
    let pending = std::mem::replace(&mut *saver().dirty.lock().unwrap_or_else(|e| e.into_inner()), false);
    if pending {
        if let Err(error) = config::save(&current_config()) { eprintln!("Could not save settings: {error}"); }
    }
}
