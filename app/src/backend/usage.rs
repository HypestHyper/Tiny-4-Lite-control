//! Who is using the camera: the programs that have its video node open.

use std::path::Path;

/// The programs (process id and name) that have `node` open, apart from this one and the `v4l2-ctl`
/// helpers that this app starts for its own commands and preview.
pub fn holders(node: &str) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return found };
    let me = std::process::id();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me { continue; }
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_string();
        if name == "v4l2-ctl" { continue; }
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        if fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|target| target == Path::new(node))) {
            found.push((pid, if name.is_empty() { format!("process {pid}") } else { name }));
        }
    }
    found
}

/// Every process (other than this one) that has `node` open.
pub fn openers(node: &str) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    let me = std::process::id();
    entries.flatten().filter_map(|entry| {
        let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
        if pid == me { return None; }
        let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
        fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| t == Path::new(node))).then_some(pid)
    }).collect()
}

/// The programs really using the camera, for the background service: any program that has it open,
/// except this one, the control window (which only opens it for a moment per command), and `v4l2-ctl`
/// runs that are not streaming. A preview stream, from this app or any other, counts.
pub fn streaming_holders(node: &str) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return found };
    let me = std::process::id();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me { continue; }
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        if !fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|target| target == Path::new(node))) { continue; }
        let words: Vec<String> = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default()
            .split(|b| *b == 0).filter(|w| !w.is_empty()).map(|w| String::from_utf8_lossy(w).to_string()).collect();
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_string();
        if name == "v4l2-ctl" && !words.iter().any(|w| w.starts_with("--stream")) { continue; }
        if crate::service::is_our_program(pid, &words) { continue; }
        found.push((pid, if name.is_empty() { format!("process {pid}") } else { name }));
    }
    found
}

/// Where the control window notes that someone used a control, for the service's idle timer.
pub fn activity_path() -> std::path::PathBuf { crate::service::runtime_dir().join("activity") }

/// Note that someone just used a control in the window (a setting, the stick, a button). Written at most
/// once every few seconds.
pub fn note_activity() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if now.saturating_sub(LAST.swap(now, Ordering::Relaxed)) < 3 { return; }
    let path = activity_path();
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let _ = std::fs::write(path, now.to_string());
}

/// Seconds since a control was last used in the window, if it has been.
pub fn seconds_since_activity() -> Option<u64> {
    let modified = std::fs::metadata(activity_path()).and_then(|m| m.modified()).ok()?;
    Some(std::time::SystemTime::now().duration_since(modified).map(|d| d.as_secs()).unwrap_or(0))
}

/// The names of the programs using the camera, each once, for a message.
pub fn holder_names(node: &str) -> Vec<String> {
    let mut names: Vec<String> = holders(node).into_iter().map(|(_, name)| name).collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_held_open_shows_its_holder_and_not_this_program() {
        let path = std::env::temp_dir().join(format!("t4-usage-{}", std::process::id()));
        std::fs::write(&path, "x").unwrap();
        let mut child = std::process::Command::new("sleep").arg("5").stdin(std::fs::File::open(&path).unwrap()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let found = holders(path.to_str().unwrap());
        assert!(found.iter().any(|(pid, name)| *pid == child.id() && name == "sleep"), "{found:?}");
        assert!(found.iter().all(|(pid, _)| *pid != std::process::id()));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_nobody_holds_has_no_holders() {
        let path = std::env::temp_dir().join(format!("t4-usage-none-{}", std::process::id()));
        std::fs::write(&path, "x").unwrap();
        assert!(holders(path.to_str().unwrap()).is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
