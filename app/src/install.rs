//! Installing the app itself: an AppImage that is only launched from the file manager gives no command, no
//! app-menu entry, and a background service tied to wherever that file happens to be.
//!
//! Installing keeps the AppImage in `~/Applications` (the usual place for them; one launched from there is
//! used where it is, one launched from elsewhere is copied there), and makes `~/.local/bin/tiny4lite-control`
//! a link to it: that is the `tiny4lite-control` command, and the app-menu entry and the background service
//! use it too. Installing a newer download only moves the link. Nothing needs root.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DESKTOP_NAME: &str = "io.github.hypesthyper.Tiny4LiteControl.desktop";
const ICON: &str = include_str!("../assets/tiny4lite-control.svg");
const ICON_SYMBOLIC: &str = include_str!("../assets/tiny4lite-control-symbolic.svg");

fn home() -> PathBuf { std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")) }

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

/// The command, a link to the installed AppImage: `~/.local/bin/tiny4lite-control`.
pub fn bin_path() -> PathBuf { home().join(".local/bin/tiny4lite-control") }
/// Where AppImages are kept.
pub fn applications_dir() -> PathBuf { home().join("Applications") }
/// The AppImage the command links to, if installed.
pub fn installed_target() -> Option<PathBuf> { bin_path().canonicalize().ok() }
pub fn desktop_path() -> PathBuf { data_home().join("applications").join(DESKTOP_NAME) }
fn icon_paths() -> [PathBuf; 2] {
    let icons = data_home().join("icons/hicolor");
    [icons.join("scalable/apps/tiny4lite-control.svg"), icons.join("symbolic/apps/tiny4lite-control-symbolic.svg")]
}

/// The file this app runs from: the AppImage itself when it is one, otherwise the program.
pub fn running_from() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::current_exe().ok())
}

pub fn installed() -> bool { bin_path().is_file() }


/// The running file is not the installed AppImage (a newer download, most likely): installing it moves the
/// command, the menu entry and the service onto it.
pub fn update_available() -> bool {
    let Some(target) = installed_target() else { return false };
    running_from().and_then(|f| f.canonicalize().ok()).is_some_and(|from| from != target)
}

/// Whether `~/.local/bin` is on the PATH, so `tiny4lite-control` works as a command.
pub fn bin_on_path() -> bool {
    let dir = bin_path().parent().map(Path::to_path_buf).unwrap_or_default();
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| p == dir))
}

/// How to put `~/.local/bin` on the PATH, for the user's shell.
pub fn path_hint() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("fish") { "fish_add_path ~/.local/bin".into() }
    else if shell.ends_with("zsh") { "echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc".into() }
    else { "echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.bashrc".into() }
}

/// The app-menu entry, starting the installed copy.
pub fn desktop_entry(program: &Path) -> String {
    let program = crate::service::quoted_path(&program.to_string_lossy());
    format!("[Desktop Entry]\nType=Application\nName=Tiny 2 Lite Control\nComment=Camera, gimbal and tracking controls for the OBSBOT Tiny 2 Lite\n\
        Exec={program}\nIcon=tiny4lite-control\nTerminal=false\nCategories=AudioVideo;Video;Utility;\nStartupNotify=true\n\
        StartupWMClass=io.github.hypesthyper.Tiny4LiteControl\nActions=Slim;Preview;\n\n\
        [Desktop Action Slim]\nName=Quick controls\nExec={program} --slim\n\n\
        [Desktop Action Preview]\nName=Preview in its own window\nExec={program} --preview\n")
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?; }
    std::fs::write(path, text).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// Copy the running app to `~/.local/bin`, add the app-menu entry and icons, and move the background
/// service (if it is set up) onto the installed copy. Installing over an older copy updates it.
pub fn install() -> Result<String, String> {
    let from = running_from().ok_or("Could not tell which file this app runs from.")?;
    let from = from.canonicalize().unwrap_or(from);
    let bin = bin_path();
    // The AppImage itself lives in ~/Applications: used where it is if it is there already, copied there
    // otherwise (next to it first, then swapped in, so a copy that is running keeps running).
    let apps = applications_dir();
    let apps_real = apps.canonicalize().unwrap_or_else(|_| apps.clone());
    let target = if from.parent() == Some(apps_real.as_path()) { from.clone() } else {
        std::fs::create_dir_all(&apps).map_err(|e| format!("Could not make {}: {e}", apps.display()))?;
        let name = from.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "Tiny4LiteControl-x86_64.AppImage".into());
        let target = apps.join(name);
        let fresh = target.with_extension("new");
        std::fs::copy(&from, &fresh).map_err(|e| format!("Could not copy {} to {}: {e}", from.display(), fresh.display()))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("Could not make it executable: {e}"))?;
        std::fs::rename(&fresh, &target).map_err(|e| format!("Could not put it in place: {e}"))?;
        target
    };
    // The command: a link to it (replacing an older link or copy).
    if let Some(dir) = bin.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?; }
    if bin.symlink_metadata().is_ok() { std::fs::remove_file(&bin).map_err(|e| format!("Could not replace {}: {e}", bin.display()))?; }
    std::os::unix::fs::symlink(&target, &bin).map_err(|e| format!("Could not link {} to {}: {e}", bin.display(), target.display()))?;
    write_file(&desktop_path(), &desktop_entry(&bin))?;
    let [icon, symbolic] = icon_paths();
    write_file(&icon, ICON)?;
    write_file(&symbolic, ICON_SYMBOLIC)?;
    // Menus that cache entries are told; where the tools are missing, they find it at next login.
    let quiet = |program: &str, args: &[&str]| { let _ = Command::new(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status(); };
    if let Some(dir) = desktop_path().parent().and_then(Path::to_str) { quiet("update-desktop-database", &[dir]); }
    quiet("gtk-update-icon-cache", &["-q", "-t", &data_home().join("icons/hicolor").to_string_lossy()]);
    let service = crate::setup::move_service_to(&bin.to_string_lossy());
    let mut message = format!("Installed: {} (the AppImage), the tiny4lite-control command and the app menu entry.", target.display());
    if let Some(note) = service { message.push(' '); message.push_str(&note); }
    // Removing the app took the service with it: put it back, on the installed copy.
    if crate::backend::current_config().service_removed_with_app && crate::setup::installed().is_none() {
        match crate::setup::install_service() {
            Ok(_) => message.push_str(" The background service is back, as it was before the app was removed."),
            Err(error) => message.push_str(&format!(" The background service could not be set up again: {error}")),
        }
    }
    let _ = crate::backend::update_config_checked(|c| c.service_removed_with_app = false);
    if !bin_on_path() { message.push_str(&format!(" To use the tiny4lite-control command, add ~/.local/bin to your PATH: {}", path_hint())); }
    Ok(message)
}

/// Take the installed copy, the app-menu entry and the icons away. The background service is removed first
/// if it runs the installed copy, since it could not start any more.
pub fn uninstall() -> Result<String, String> {
    let bin = bin_path();
    let mut notes = Vec::new();
    if crate::setup::installed().is_some() && crate::setup::service_program().is_some_and(|p| Path::new(&p) == bin) {
        crate::setup::remove()?;
        let _ = crate::backend::update_config_checked(|c| c.service_removed_with_app = true);
        notes.push("The background service was removed too (it ran the installed copy); installing again brings it back.");
    }
    let target = installed_target();
    for path in [bin.clone(), desktop_path()].into_iter().chain(icon_paths()) {
        if path.symlink_metadata().is_ok() { std::fs::remove_file(&path).map_err(|e| format!("Could not remove {}: {e}", path.display()))?; }
    }
    if let Some(target) = target { notes.push("The AppImage itself is left where it is:"); return Ok(format!("Removed the tiny4lite-control command and the app menu entry. {} {}", notes.join(" "), target.display())); }
    if let Some(dir) = desktop_path().parent().and_then(Path::to_str) {
        let _ = Command::new("update-desktop-database").arg(dir).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    Ok(format!("Removed {} and the app menu entry. {}", bin.display(), notes.join(" ")).trim().to_string())
}

/// Run when the window opens, so a new version needs no removing and reinstalling:
/// - a new AppImage opened from ~/Applications (where installed copies live) becomes the installed one: the
///   command, the menu entry and the service move to it;
/// - a background service still running an older version (the installed file was replaced by a newer one
///   of the same name) is restarted on the new one;
/// - a service whose program is gone is pointed at the installed copy.
/// Says what it did, or what the user can do, for each case that applied.
pub fn keep_up_to_date() -> Vec<Result<String, String>> {
    let mut said = Vec::new();
    let from = running_from().and_then(|f| f.canonicalize().ok());
    let apps = applications_dir().canonicalize().ok();
    let in_apps = from.as_ref().and_then(|f| f.parent().map(Path::to_path_buf)).is_some_and(|dir| Some(&dir) == apps.as_ref());
    let Some(target) = installed_target() else {
        // The command links to an AppImage that was deleted (an old version, cleared away): this one takes
        // its place, if it lives in ~/Applications too.
        if bin_path().symlink_metadata().is_ok() && in_apps {
            said.push(install().map(|m| format!("This version is now the installed one. {m}")));
        }
        return said;
    };
    let mut moved = false;
    if update_available() {
        if in_apps {
            said.push(install().map(|m| format!("This version is now the installed one. {m}")));
            moved = true;
        } else if let Some(from) = &from {
            said.push(Ok(format!("This window runs {}, not the installed {}. To use this one from now on: Device tab \u{2192} App \u{2192} Install this version.", from.display(), target.display())));
        }
    }
    if !moved {
        if let Some(program) = crate::setup::service_program() {
            if !Path::new(&program).exists() {
                if let Some(note) = crate::setup::move_service_to(&bin_path().to_string_lossy()) { said.push(Ok(note)); }
            } else if let Some(status) = crate::service::running_status() {
                let same_file = Path::new(&program).canonicalize().ok().is_some_and(|p| Some(&p) == from.as_ref());
                if same_file && status.version != crate::cli::build_label() {
                    said.push(crate::setup::restart().map(|_| "The background service was restarted on this version.".to_string()));
                }
            }
        }
    }
    said
}

/// `tiny4lite-control app install|remove|status`.
pub fn command(word: Option<&str>) -> i32 {
    let report = |r: Result<String, String>| match r { Ok(m) => { println!("{m}"); crate::cli::OK } Err(e) => { eprintln!("{e}"); crate::cli::FAILED } };
    match word {
        Some("install") => report(install()),
        // What the window does when it opens: adopt a new copy in ~/Applications, restart an outdated service.
        Some("update") => {
            let results = keep_up_to_date();
            if results.is_empty() { println!("Nothing to do: the installed copy and the background service are this version, or this copy is not installed."); }
            let mut code = crate::cli::OK;
            for result in results { match result { Ok(m) => println!("{m}"), Err(e) => { eprintln!("{e}"); code = crate::cli::FAILED; } } }
            code
        }
        Some("remove") | Some("uninstall") => report(uninstall()),
        Some("status") | None => {
            println!("Running from:  {}", running_from().map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".into()));
            println!("Installed:     {}", match installed_target() { Some(t) => format!("{} (command: {})", t.display(), bin_path().display()), None => "no".into() });
            if update_available() { println!("The running file differs from the installed copy: install again to update it."); }
            if installed() && !bin_on_path() { println!("~/.local/bin is not on your PATH: {}", path_hint()); }
            crate::cli::OK
        }
        Some(other) => { eprintln!("app takes install, update, remove or status, not `{other}`."); crate::cli::USAGE }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_entry_starts_the_installed_copy_and_matches_the_window_class() {
        let entry = desktop_entry(Path::new("/home/me/.local/bin/tiny4lite-control"));
        assert!(entry.contains("Exec=/home/me/.local/bin/tiny4lite-control\n"));
        assert!(entry.contains("Exec=/home/me/.local/bin/tiny4lite-control --slim"));
        assert!(entry.contains("StartupWMClass=io.github.hypesthyper.Tiny4LiteControl"));
        assert!(entry.contains("Icon=tiny4lite-control"));
        // A path with a space is quoted.
        assert!(desktop_entry(Path::new("/home/me/my apps/t4")).contains("Exec=\"/home/me/my apps/t4\""));
    }

    #[test]
    fn the_icons_are_built_in() {
        assert!(ICON.contains("<svg") && ICON_SYMBOLIC.contains("<svg"));
    }
}
