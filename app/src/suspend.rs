//! Put the camera to sleep when the computer suspends.
//!
//! The way to run something before a suspend on any Linux with logind (systemd) or elogind (Void,
//! Alpine, Artix, Gentoo without systemd) is to hold a *delay inhibitor lock* and watch the logind
//! signal `PrepareForSleep`: the lock keeps the suspend waiting while we send the command, and we let
//! go of it when we are done. Without the lock, watching the signal is racy, because the computer may
//! already be asleep when we react.
//!
//! No libraries are added: `dbus-monitor` prints the signal, and `systemd-inhibit` (or
//! `elogind-inhibit`) holds the lock for as long as the program it runs lives.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Reads `dbus-monitor` output and says when a `PrepareForSleep` signal arrives, and what it carried:
/// true just before the computer sleeps, false just after it wakes.
#[derive(Default)]
pub struct Parser { in_signal: bool }

impl Parser {
    pub fn feed(&mut self, line: &str) -> Option<bool> {
        if line.contains("member=PrepareForSleep") {
            self.in_signal = true;
            return None;
        }
        if self.in_signal {
            let text = line.trim();
            if let Some(value) = text.strip_prefix("boolean ") {
                self.in_signal = false;
                return Some(value.trim() == "true");
            }
            // The next signal began before this one gave its value: forget it.
            if text.starts_with("signal ") || text.starts_with("method ") { self.in_signal = false; }
        }
        None
    }
}

fn found(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The program that holds the lock, or why there is none.
pub fn inhibit_program() -> Option<&'static str> {
    ["systemd-inhibit", "elogind-inhibit"].into_iter().find(|p| found(p))
}

/// Whether suspend can be watched here, or what is missing.
pub fn available() -> Result<(), String> {
    if crate::backend::ui_test_mode() && std::env::var_os("TINY4LITE_TEST_SUSPEND_MONITOR").is_some() { return Ok(()); }
    if !found("dbus-monitor") { return Err("dbus-monitor is not installed (the dbus tools package)".into()); }
    if inhibit_program().is_none() { return Err("neither systemd-inhibit nor elogind-inhibit is installed".into()); }
    Ok(())
}

fn monitor_command() -> Command {
    if crate::backend::ui_test_mode() {
        if let Ok(script) = std::env::var("TINY4LITE_TEST_SUSPEND_MONITOR") {
            let mut command = Command::new("sh");
            command.args(["-c", &script]);
            return command;
        }
    }
    let mut command = Command::new("dbus-monitor");
    command.args(["--system", "type='signal',sender='org.freedesktop.login1',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'"]);
    command
}

fn take_lock() -> Option<Child> {
    if crate::backend::ui_test_mode() && std::env::var_os("TINY4LITE_TEST_SUSPEND_MONITOR").is_some() {
        return Command::new("sh").args(["-c", "while :; do sleep 1; done"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
    }
    let program = inhibit_program()?;
    // `sleep infinity` is not everywhere (busybox), so the lock is held by a loop instead.
    Command::new(program)
        .args(["--what=sleep", "--mode=delay", "--who=tiny4lite-control", "--why=Putting the camera to sleep", "sh", "-c", "while :; do sleep 3600; done"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().ok()
}

fn release(lock: &Mutex<Option<Child>>) {
    if let Some(mut child) = lock.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Watches for suspend until dropped.
pub struct Watcher {
    monitor: Arc<Mutex<Option<Child>>>,
    lock: Arc<Mutex<Option<Child>>>,
}

impl Watcher {
    /// `before_sleep` is run (and waited for) while the lock is held; then the lock is let go.
    pub fn start(before_sleep: impl Fn() + Send + 'static) -> Result<Watcher, String> {
        available()?;
        let mut monitor = monitor_command().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("Could not start the suspend watcher: {e}"))?;
        let stdout = monitor.stdout.take().ok_or("The suspend watcher gave no output.")?;
        let lock: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(take_lock()));
        let monitor = Arc::new(Mutex::new(Some(monitor)));
        let thread_lock = lock.clone();
        std::thread::Builder::new().name("suspend-watch".into()).spawn(move || {
            let mut parser = Parser::default();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match parser.feed(&line) {
                    Some(true) => {
                        before_sleep();
                        release(&thread_lock);
                    }
                    // Awake again: hold the lock once more for the next time.
                    Some(false) => {
                        let mut held = thread_lock.lock().unwrap_or_else(|e| e.into_inner());
                        if held.is_none() { *held = take_lock(); }
                    }
                    None => {}
                }
            }
        }).map_err(|e| format!("Could not start the suspend watcher: {e}"))?;
        Ok(Watcher { monitor, lock })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        release(&self.lock);
        if let Some(mut child) = self.monitor.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNAL: &str = "signal time=1700000000.1 sender=:1.4 -> destination=(null destination) serial=812 path=/org/freedesktop/login1; interface=org.freedesktop.login1.Manager; member=PrepareForSleep";

    #[test]
    fn the_signal_is_read_for_going_to_sleep_and_waking() {
        let mut parser = Parser::default();
        assert_eq!(parser.feed(SIGNAL), None);
        assert_eq!(parser.feed("   boolean true"), Some(true));
        assert_eq!(parser.feed(SIGNAL), None);
        assert_eq!(parser.feed("   boolean false"), Some(false));
    }

    #[test]
    fn other_booleans_and_other_signals_are_ignored() {
        let mut parser = Parser::default();
        assert_eq!(parser.feed("   boolean true"), None);
        assert_eq!(parser.feed("signal time=1 sender=:1.1 -> destination=(null destination) serial=1 path=/x; interface=a.b; member=Other"), None);
        assert_eq!(parser.feed("   boolean true"), None);
        // A signal that never gave its value does not borrow the next one's.
        assert_eq!(parser.feed(SIGNAL), None);
        assert_eq!(parser.feed("signal time=2 sender=:1.1 -> destination=(null destination) serial=2 path=/x; interface=a.b; member=Other"), None);
        assert_eq!(parser.feed("   boolean true"), None);
    }
}
