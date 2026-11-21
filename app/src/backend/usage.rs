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
