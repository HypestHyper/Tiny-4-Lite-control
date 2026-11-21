//! What the app and the service need installed, and installing and running the service.
//!
//! Used by the command line (`requirements`, `service setup|start|stop|restart|remove`) and by the
//! window, which opens a terminal for anything that needs a password (installing packages).

use crate::service;
use std::path::Path;
use std::process::{Command, Stdio};

// ── Requirements ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Manager { Apt, Dnf, Pacman, Zypper, Apk, Xbps, Portage }

pub struct Requirement {
    pub name: &'static str,
    /// Any one of these programs will do.
    pub programs: &'static [&'static str],
    pub why: &'static str,
    /// Without it the app cannot control the camera at all.
    pub required: bool,
}

pub const REQUIREMENTS: &[Requirement] = &[
    Requirement { name: "v4l2-ctl", programs: &["v4l2-ctl"], why: "talks to the camera's controls and picture", required: true },
    Requirement { name: "dbus-monitor", programs: &["dbus-monitor"], why: "notices the computer suspending, to put the camera to sleep first", required: false },
    Requirement { name: "systemd-inhibit or elogind-inhibit", programs: &["systemd-inhibit", "elogind-inhibit"], why: "holds the suspend back for a moment while the camera is put to sleep", required: false },
];

/// The package that provides a requirement on a package manager, where one can be named.
pub fn package(requirement: &Requirement, manager: Manager) -> Option<&'static str> {
    Some(match (requirement.name, manager) {
        ("v4l2-ctl", Manager::Portage) => "media-tv/v4l-utils",
        ("v4l2-ctl", _) => "v4l-utils",
        ("dbus-monitor", Manager::Dnf) => "dbus-tools",
        ("dbus-monitor", Manager::Zypper) => "dbus-1",
        ("dbus-monitor", Manager::Portage) => "sys-apps/dbus",
        ("dbus-monitor", _) => "dbus",
        // These come with the init system, not as a package of their own.
        _ => return None,
    })
}

pub fn program_found(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

pub fn is_present(requirement: &Requirement) -> bool {
    // The synthetic camera needs no real v4l2-ctl.
    if crate::backend::ui_test_mode() && requirement.name == "v4l2-ctl" { return true; }
    requirement.programs.iter().any(|p| program_found(p))
}

/// The requirements that are missing.
pub fn missing() -> Vec<&'static Requirement> { REQUIREMENTS.iter().filter(|r| !is_present(r)).collect() }

/// Which package manager this system uses, by what is installed.
pub fn manager() -> Option<Manager> {
    [("apt-get", Manager::Apt), ("dnf", Manager::Dnf), ("pacman", Manager::Pacman), ("zypper", Manager::Zypper),
     ("apk", Manager::Apk), ("xbps-install", Manager::Xbps), ("emerge", Manager::Portage)]
        .into_iter().find(|(program, _)| program_found(program)).map(|(_, manager)| manager)
}

/// The command that installs `packages`, with whatever gives it permission in front.
pub fn install_command(manager: Manager, packages: &[&str]) -> Vec<String> {
    let base: &[&str] = match manager {
        Manager::Apt => &["apt-get", "install", "-y"],
        Manager::Dnf => &["dnf", "install", "-y"],
        Manager::Pacman => &["pacman", "-S", "--needed", "--noconfirm"],
        Manager::Zypper => &["zypper", "--non-interactive", "install"],
        Manager::Apk => &["apk", "add"],
        Manager::Xbps => &["xbps-install", "-y"],
        Manager::Portage => &["emerge", "--ask=n"],
    };
    let mut command: Vec<String> = Vec::new();
    // Root already needs nothing in front.
    if !is_root() {
        if let Some(gain) = ["sudo", "doas", "pkexec"].into_iter().find(|p| program_found(p)) { command.push(gain.to_string()); }
    }
    command.extend(base.iter().map(|s| s.to_string()));
    command.extend(packages.iter().map(|s| s.to_string()));
    command
}

fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false)
}

/// Which of the missing requirements can be installed here, as (requirement, package).
fn installable(manager: Manager) -> Vec<(&'static Requirement, &'static str)> {
    missing().into_iter().filter_map(|r| package(r, manager).map(|p| (r, p))).collect()
}

/// Ask a yes/no question on the terminal; the default is yes.
fn confirm(question: &str, yes: bool) -> bool {
    if yes { return true; }
    use std::io::{BufRead, Write};
    print!("{question} [Y/n] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() { return false; }
    !matches!(answer.trim().to_lowercase().as_str(), "n" | "no")
}

pub fn requirements_command(install: bool, yes: bool) -> i32 {
    let mut code = crate::cli::OK;
    for requirement in REQUIREMENTS {
        let present = is_present(requirement);
        println!("{} {:<36} {}{}", if present { "ok     " } else if requirement.required { "MISSING" } else { "missing" }, requirement.name, requirement.why,
            if requirement.required { "" } else { " (optional)" });
        if !present && requirement.required { code = crate::cli::FAILED; }
    }
    if !install { return code; }
    let Some(manager) = manager() else {
        println!("\nNo package manager was recognised, so nothing can be installed from here. Install the missing programs with your system's tools.");
        return code;
    };
    let wanted = installable(manager);
    if wanted.is_empty() { println!("\nNothing to install."); return crate::cli::OK; }
    let packages: Vec<&str> = wanted.iter().map(|(_, p)| *p).collect();
    let command = install_command(manager, &packages);
    println!("\nThis installs: {}\n  {}\n", packages.join(", "), command.join(" "));
    if !confirm("Go ahead?", yes) { return code; }
    match Command::new(&command[0]).args(&command[1..]).status() {
        Ok(status) if status.success() => { println!("\nInstalled."); if missing().iter().any(|r| r.required) { crate::cli::FAILED } else { crate::cli::OK } }
        Ok(_) => { println!("\nThe installation did not finish."); crate::cli::FAILED }
        Err(error) => { println!("\nCould not run {}: {error}", command[0]); crate::cli::FAILED }
    }
}

// ── The service: installing and running it ────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstalledAs { Systemd, Autostart }

pub fn installed() -> Option<InstalledAs> {
    if service::install_path("systemd").is_some_and(|p| p.exists()) { return Some(InstalledAs::Systemd); }
    if service::install_path("xdg").is_some_and(|p| p.exists()) { return Some(InstalledAs::Autostart); }
    None
}

/// Is there a systemd user manager to talk to?
pub fn systemd_user_available() -> bool {
    program_found("systemctl")
        && Command::new("systemctl").args(["--user", "show-environment"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let output = Command::new("systemctl").arg("--user").args(args).output().map_err(|e| format!("Could not run systemctl: {e}"))?;
    if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_string()) }
}

/// Start the service: through systemd if it is installed that way, otherwise as a detached program.
pub fn start() -> Result<String, String> {
    if service::running_status().is_some() { return Ok("The service is already running.".into()); }
    if installed() == Some(InstalledAs::Systemd) && systemd_user_available() {
        systemctl(&["start", "tiny4lite-control.service"])?;
        return Ok("Started.".into());
    }
    use std::os::unix::process::CommandExt;
    Command::new(service::program_path()).args(["service", "run"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .process_group(0)
        .spawn().map_err(|e| format!("Could not start the service: {e}"))?;
    Ok("Started.".into())
}

/// Stop the service. It sends sleep on its way out if that is turned on.
pub fn stop() -> Result<String, String> {
    let Some(status) = service::running_status() else { return Ok("The service is not running.".into()) };
    if installed() == Some(InstalledAs::Systemd) && systemd_user_available() {
        systemctl(&["stop", "tiny4lite-control.service"])?;
    } else {
        Command::new("kill").args(["-TERM", &status.pid.to_string()]).status().map_err(|e| format!("Could not stop the service: {e}"))?;
    }
    // It answers within a moment; wait for it, so the next look at its status is true.
    for _ in 0..40 {
        if service::running_status().is_none() { return Ok("Stopped.".into()); }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Err("The service did not stop within a few seconds.".into())
}

pub fn restart() -> Result<String, String> { stop()?; start() }

/// Stop it and take it out of the session's start-up.
pub fn remove() -> Result<String, String> {
    stop()?;
    match installed() {
        Some(InstalledAs::Systemd) => {
            if systemd_user_available() { let _ = systemctl(&["disable", "tiny4lite-control.service"]); }
            if let Some(path) = service::install_path("systemd") { let _ = std::fs::remove_file(path); }
            if systemd_user_available() { let _ = systemctl(&["daemon-reload"]); }
        }
        Some(InstalledAs::Autostart) => { if let Some(path) = service::install_path("xdg") { let _ = std::fs::remove_file(path); } }
        None => return Ok("The service was not installed.".into()),
    }
    Ok("Removed. It no longer starts with your session.".into())
}

/// Write the service where this system starts user services from, and start it. No password needed.
pub fn install_service() -> Result<String, String> {
    let program = service::program_path();
    if systemd_user_available() {
        let path = service::install_path("systemd").ok_or("No place for the service file.")?;
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        std::fs::write(&path, service::systemd_unit(&program)).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", "tiny4lite-control.service"])?;
        Ok("Installed as a systemd user service, and started.".into())
    } else {
        let path = service::install_path("xdg").ok_or("No place for the autostart entry.")?;
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        std::fs::write(&path, service::desktop_entry(&program)).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        start()?;
        Ok("Installed as a desktop autostart entry (this system has no systemd user manager), and started. Window managers that do not read these entries need one line in their own config: run `tiny4lite-control service targets`.".into())
    }
}

/// What the terminal runs: check what is needed, install the missing parts, install the service, start it.
pub fn setup_command(yes: bool) -> i32 {
    println!("Tiny 2 Lite Control: background service setup\n");
    println!("1. What it needs\n");
    let code = requirements_command(false, true);
    let wanted = manager().map(installable).unwrap_or_default();
    if !wanted.is_empty() {
        println!();
        if confirm("Install the missing programs now? (asks for your password)", yes) {
            requirements_command(true, true);
        }
    }
    if missing().iter().any(|r| r.required) {
        println!("\n{} is still missing, so the service cannot do anything yet. Install it, then run this again.", "v4l2-ctl");
        wait_to_close(yes);
        return crate::cli::FAILED;
    }
    let _ = code;
    println!("\n2. The service\n");
    let result = install_service();
    match &result { Ok(message) => println!("{message}"), Err(error) => println!("Could not install it: {error}") }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    match service::running_status() {
        Some(status) => println!("\nRunning, process {}.", status.pid),
        None => println!("\nIt is not running yet. Check with: tiny4lite-control service status"),
    }
    wait_to_close(yes);
    if result.is_ok() { crate::cli::OK } else { crate::cli::FAILED }
}

fn wait_to_close(yes: bool) {
    if yes { return; }
    use std::io::{BufRead, Write};
    print!("\nPress Enter to close this window.");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
}

// ── Terminals ─────────────────────────────────────────────────────────────

/// The terminal programs to try, each with the words that make it run a command: the program, and the
/// arguments that come before the command.
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("kgx", &["-e"]),
    ("ptyxis", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("lxterminal", &["-e"]),
    ("qterminal", &["-e"]),
    ("tilix", &["-e"]),
    ("terminator", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("ghostty", &["-e"]),
    ("urxvt", &["-e"]),
    ("st", &["-e"]),
    ("xterm", &["-e"]),
];

/// The command that opens a terminal running `args`, or None if no terminal is installed.
pub fn terminal_command(args: &[String]) -> Option<Command> {
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    // The user's own choice first.
    if let Some(own) = std::env::var_os("TERMINAL").map(|t| t.to_string_lossy().to_string()).filter(|t| !t.is_empty()) {
        let flag = TERMINALS.iter().find(|(name, _)| own.rsplit('/').next() == Some(*name)).map(|(_, f)| f.iter().map(|s| s.to_string()).collect()).unwrap_or_else(|| vec!["-e".to_string()]);
        candidates.push((own, flag));
    }
    candidates.extend(TERMINALS.iter().map(|(name, flags)| (name.to_string(), flags.iter().map(|s| s.to_string()).collect())));
    for (program, flags) in candidates {
        let usable = if program.contains('/') { Path::new(&program).is_file() } else { program_found(&program) };
        if usable {
            let mut command = Command::new(&program);
            command.args(&flags).args(args);
            return Some(command);
        }
    }
    None
}

/// Open a terminal that runs this program with `arguments`.
pub fn run_in_terminal(arguments: &[&str]) -> Result<(), String> {
    let mut args = vec![service::program_path()];
    args.extend(arguments.iter().map(|s| s.to_string()));
    let mut command = terminal_command(&args).ok_or_else(|| format!(
        "No terminal program was found. Run this yourself in a terminal: {}", args.join(" ")))?;
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map(|_| ()).map_err(|e| format!("Could not open a terminal: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_tool_has_a_package_on_every_manager() {
        for manager in [Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Zypper, Manager::Apk, Manager::Xbps, Manager::Portage] {
            assert!(package(&REQUIREMENTS[0], manager).is_some(), "{manager:?}");
        }
        // The init system's own tools are not offered as packages.
        assert!(package(&REQUIREMENTS[2], Manager::Apt).is_none());
    }

    #[test]
    fn install_commands_are_written_for_each_manager() {
        let command = install_command(Manager::Apt, &["v4l-utils"]);
        assert!(command.windows(3).any(|w| w == ["apt-get", "install", "-y"]), "{command:?}");
        assert_eq!(command.last().map(String::as_str), Some("v4l-utils"));
        assert!(install_command(Manager::Pacman, &["v4l-utils"]).contains(&"--noconfirm".to_string()));
        assert!(install_command(Manager::Zypper, &["v4l-utils"]).contains(&"--non-interactive".to_string()));
    }

    #[test]
    fn a_terminal_runs_the_command_after_its_own_flags() {
        // Whatever terminal is found, the command is the last words, in order.
        let args = vec!["/opt/t4".to_string(), "service".to_string(), "setup".to_string()];
        if let Some(command) = terminal_command(&args) {
            let words: Vec<String> = command.get_args().map(|a| a.to_string_lossy().to_string()).collect();
            assert!(words.ends_with(&args), "{words:?}");
        }
    }

    #[test]
    fn missing_things_are_found_by_name() {
        assert!(program_found("sh"));
        assert!(!program_found("no-such-program-t4"));
        assert!(!is_present(&Requirement { name: "x", programs: &["no-such-program-t4"], why: "", required: false }));
    }
}
