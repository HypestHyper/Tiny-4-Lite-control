//! Backups of the app's settings and presets: a copy each day the app is opened (the last ten kept), and
//! export to and import from a file the user chooses.

use crate::config::{self, AppConfig};
use std::path::{Path, PathBuf};

/// How many daily copies are kept.
const KEEP: usize = 10;

pub fn backups_dir() -> PathBuf {
    config::config_path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("backups")
}

/// Today's date as YYYY-MM-DD (UTC), without a date library.
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The file name a backup or an export of today gets.
pub fn suggested_name() -> String { format!("tiny4lite-control-settings-{}.toml", today()) }

/// Copy the settings file into the backups folder once a day, keeping the last ten. Quiet on failure.
pub fn daily() {
    let source = config::config_path();
    if !source.exists() { return; }
    let folder = backups_dir();
    if std::fs::create_dir_all(&folder).is_err() { return; }
    let target = folder.join(format!("settings-{}.toml", today()));
    if !target.exists() { let _ = std::fs::copy(&source, &target); }
    let mut copies: Vec<PathBuf> = std::fs::read_dir(&folder).map(|entries| entries.flatten().map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("settings-") && n.ends_with(".toml")))
        .collect()).unwrap_or_default();
    copies.sort();
    while copies.len() > KEEP { let _ = std::fs::remove_file(copies.remove(0)); }
}

/// Write the current settings and presets to a file.
pub fn export(path: &Path) -> Result<(), String> {
    let text = toml::to_string_pretty(&crate::backend::current_config()).map_err(|e| format!("Could not write the settings: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

/// Read settings from a file, checking they are this app's and that it can use them.
pub fn read(path: &Path) -> Result<AppConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    // Every settings file this app writes starts with its version; another program's TOML would otherwise read
    // as all defaults and wipe the settings.
    let table: toml::Table = text.parse().map_err(|_| "This file is not a Tiny 2 Lite Control settings file.".to_string())?;
    if !table.get("version").is_some_and(|v| v.is_integer()) {
        return Err("This file is not a Tiny 2 Lite Control settings file.".into());
    }
    let imported: AppConfig = toml::from_str(&text).map_err(|_| "This file is not a Tiny 2 Lite Control settings file.".to_string())?;
    if imported.version > config::CONFIG_VERSION {
        return Err("These settings come from a newer version of the app. Update the app first.".into());
    }
    if imported.presets.len() > 10 { return Err("The file has more than 10 presets.".into()); }
    Ok(imported)
}

/// Replace the settings and presets with imported ones, keeping a copy of the current ones first.
pub fn import(imported: AppConfig) -> Result<(), String> {
    let folder = backups_dir();
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = export(&folder.join(format!("before-import-{}-{stamp}.toml", today())));
    crate::backend::update_config_checked(move |c| *c = imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_a_date() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20") && &t[4..5] == "-" && &t[7..8] == "-");
    }

    #[test]
    fn exported_settings_read_back_and_strangers_are_refused() {
        let dir = std::env::temp_dir().join(format!("t4-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.toml");
        std::fs::write(&file, toml::to_string_pretty(&AppConfig::default()).unwrap()).unwrap();
        assert!(read(&file).is_ok());
        std::fs::write(&file, "presets = 5\\n").unwrap();
        assert!(read(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
