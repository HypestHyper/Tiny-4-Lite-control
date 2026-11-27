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
    super::angles::set_limits(&config);
    *store().lock().unwrap_or_else(|e| e.into_inner()) = config;
    remember_file();
}

/// When the settings file was last written or read by this program. The window and the background service
/// share it; a different time means the other one wrote it.
fn seen() -> &'static Mutex<Option<std::time::SystemTime>> {
    static SEEN: OnceLock<Mutex<Option<std::time::SystemTime>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(None))
}

fn file_time() -> Option<std::time::SystemTime> {
    std::fs::metadata(config::config_path()).and_then(|m| m.modified()).ok()
}

fn remember_file() { *seen().lock().unwrap_or_else(|e| e.into_inner()) = file_time(); }

/// Load the settings again if another program (the window, the background service) wrote them since this
/// one last did, unless this one has a change waiting to be written. Returns whether it reloaded.
pub fn reload_if_changed() -> bool {
    if *saver().dirty.lock().unwrap_or_else(|e| e.into_inner()) { return false; }
    let now = file_time();
    if now.is_none() || now == *seen().lock().unwrap_or_else(|e| e.into_inner()) { return false; }
    match config::load() {
        Ok(loaded) => { install_config(loaded); true }
        Err(_) => false,
    }
}

pub fn current_config() -> AppConfig {
    store().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Persist a candidate snapshot first. If disk I/O or validation fails the
/// in-memory configuration stays unchanged as well.
pub fn update_config_checked(update: impl FnOnce(&mut AppConfig)) -> Result<(), String> {
    // Start from what is on disk if the other program changed it, so its change is not written over.
    reload_if_changed();
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
        super::angles::set_limits(&guard);
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
            match config::save(&current_config()) {
                Ok(()) => remember_file(),
                Err(error) => eprintln!("Could not save settings: {error}"),
            }
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
        match config::save(&current_config()) {
            Ok(()) => remember_file(),
            Err(error) => eprintln!("Could not save settings: {error}"),
        }
    }
}
