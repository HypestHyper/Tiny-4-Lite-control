//! Is there a newer version? The latest release on the project's GitHub page, read once at start-up (with
//! `curl`, in the background; nothing is shown if it cannot be reached or there is no release).

pub const REPO: &str = "HypestHyper/Tiny-4-Lite-control";
pub const RELEASES: &str = "https://github.com/HypestHyper/Tiny-4-Lite-control/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub struct Release { pub version: String, pub url: String }

/// The latest published release, if there is one.
pub fn latest() -> Option<Release> {
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "8", "-H", "Accept: application/vnd.github+json", "-A", "tiny4lite-control"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .output().ok()?;
    if !output.status.success() { return None; }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let tag = json.get("tag_name")?.as_str()?.to_string();
    let url = json.get("html_url").and_then(|u| u.as_str()).unwrap_or(RELEASES).to_string();
    Some(Release { version: tag.trim_start_matches(['v', 'V']).to_string(), url })
}

/// Whether `candidate` (like "0.3.1") is newer than `current`, number by number.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect() };
    let (a, b) = (parts(candidate), parts(current));
    if a.is_empty() { return false; }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y { return x > y; }
    }
    false
}

pub fn current() -> &'static str { env!("CARGO_PKG_VERSION") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_number_by_number() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
