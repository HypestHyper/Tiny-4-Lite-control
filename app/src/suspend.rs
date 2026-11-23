//! Put the camera to sleep when the computer suspends or shuts down.
//!
//! The way to run something before a suspend on any Linux with logind (systemd) or elogind (Void,
//! Alpine, Artix, Gentoo without systemd) is to hold a *delay inhibitor lock* and watch the logind
//! signal `PrepareForSleep`: the lock keeps the suspend waiting while we send the command, and we let
//! go of it when we are done. Without the lock, watching the signal is racy, because the computer may
//! already be asleep when we react. Shutdown and reboot work the same way, with `PrepareForShutdown`:
//! that comes while the computer is still fully up, before services are stopped and before the USB
//! ports are reset, which is too late for a camera command on some machines.
//!
//! No libraries are added: `dbus-monitor` prints the signal, and `systemd-inhibit` (or
//! `elogind-inhibit`) holds the lock for as long as the program it runs lives. That program is `cat`
//! reading a pipe from this process, so it ends, and the lock with it, when the service ends in any way,
//! even if it is killed: nothing is left behind.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What logind announced: a suspend (true just before, false just after waking) or a shutdown or reboot
/// (true just before; false if it was called off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal { Sleep(bool), Shutdown(bool) }

/// Reads `dbus-monitor` output and says when a `PrepareForSleep` or `PrepareForShutdown` signal arrives,
/// and what it carried.
#[derive(Default)]
pub struct Parser { pending: Option<fn(bool) -> Signal> }

impl Parser {
    pub fn feed(&mut self, line: &str) -> Option<Signal> {
        if line.contains("member=PrepareForSleep") { self.pending = Some(Signal::Sleep); return None; }
        if line.contains("member=PrepareForShutdown") { self.pending = Some(Signal::Shutdown); return None; }
        if let Some(make) = self.pending {
            let text = line.trim();
            if let Some(value) = text.strip_prefix("boolean ") {
                self.pending = None;
                return Some(make(value.trim() == "true"));
            }
            // The next signal began before this one gave its value: forget it.
            if text.starts_with("signal ") || text.starts_with("method ") { self.pending = None; }
        }
        None
    }
}

/// What to watch for.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Watch { pub suspend: bool, pub shutdown: bool }

impl Watch {
    pub fn any(self) -> bool { self.suspend || self.shutdown }
    /// What the inhibitor lock holds back.
    fn what(self) -> &'static str {
        match (self.suspend, self.shutdown) { (true, true) => "sleep:shutdown", (false, true) => "shutdown", _ => "sleep" }
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
    command.args(["--system",
        "type='signal',sender='org.freedesktop.login1',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'",
        "type='signal',sender='org.freedesktop.login1',interface='org.freedesktop.login1.Manager',member='PrepareForShutdown'"]);
    command
}

fn take_lock(watch: Watch) -> Option<Child> {
    if crate::backend::ui_test_mode() && std::env::var_os("TINY4LITE_TEST_SUSPEND_MONITOR").is_some() {
        return Command::new("cat").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
    }
    let program = inhibit_program()?;
    // `cat` waits on our pipe: closing it (or this process ending) ends `cat`, then the inhibitor lets go.
    Command::new(program)
        .args([&format!("--what={}", watch.what()), "--mode=delay", "--who=tiny4lite-control", "--why=Putting the camera to sleep", "cat"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().ok()
}

fn release(lock: &Mutex<Option<Child>>) {
    if let Some(mut child) = lock.lock().unwrap_or_else(|e| e.into_inner()).take() {
        // Closing the pipe lets `cat` finish and the inhibitor exit by itself; kill only if it does not.
        drop(child.stdin.take());
        for _ in 0..20 {
            if child.try_wait().is_ok_and(|done| done.is_some()) { return; }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The longest the suspend is held back for the camera.
const SLEEP_WORK_LIMIT: std::time::Duration = std::time::Duration::from_millis(1000);

/// Watches for suspend until dropped.
pub struct Watcher {
    monitor: Arc<Mutex<Option<Child>>>,
    lock: Arc<Mutex<Option<Child>>>,
    alive: Arc<AtomicBool>,
}

impl Watcher {
    /// `before_sleep` is run (and waited for) while the lock is held, before a suspend or, when `watch` asks
    /// for it, before a shutdown or reboot; then the lock is let go.
    pub fn start(watch: Watch, before_sleep: impl Fn(bool) + Send + Sync + 'static, after_wake: impl Fn() + Send + 'static) -> Result<Watcher, String> {
        let before_sleep = std::sync::Arc::new(before_sleep);
        available()?;
        let mut monitor = monitor_command().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("Could not start the suspend watcher: {e}"))?;
        let stdout = monitor.stdout.take().ok_or("The suspend watcher gave no output.")?;
        let lock: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(take_lock(watch)));
        let monitor = Arc::new(Mutex::new(Some(monitor)));
        let thread_lock = lock.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let thread_alive = alive.clone();
        std::thread::Builder::new().name("suspend-watch".into()).spawn(move || {
            let mut parser = Parser::default();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let signal = parser.feed(&line);
                // A suspend or (when watched) a shutdown is coming: put the camera to sleep under the lock.
                let going = match signal {
                    Some(Signal::Sleep(true)) if watch.suspend => Some(false),
                    Some(Signal::Shutdown(true)) if watch.shutdown => Some(true),
                    _ => None,
                };
                if let Some(shutdown) = going {
                    // Never hold it back for long: the work gets a second, then the lock goes whether it has
                    // finished or not (it carries on in the background).
                    let started = std::time::Instant::now();
                    let (done_tx, done_rx) = std::sync::mpsc::channel();
                    let work = before_sleep.clone();
                    std::thread::spawn(move || { work(shutdown); let _ = done_tx.send(()); });
                    let finished = done_rx.recv_timeout(SLEEP_WORK_LIMIT).is_ok();
                    release(&thread_lock);
                    eprintln!("[tiny4lite-control service] {}: {} after {} ms; it was let go.",
                        if shutdown { "Shutdown" } else { "Suspend" },
                        if finished { "camera command done" } else { "camera command still running" },
                        started.elapsed().as_millis());
                    continue;
                }
                // Awake again, or a shutdown called off: hold the lock once more for the next time.
                let back = match signal {
                    Some(Signal::Sleep(false)) if watch.suspend => Some(true),
                    Some(Signal::Shutdown(false)) if watch.shutdown => Some(false),
                    _ => None,
                };
                if let Some(woke) = back {
                    {
                        let mut held = thread_lock.lock().unwrap_or_else(|e| e.into_inner());
                        if held.is_none() { *held = take_lock(watch); }
                    }
                    if woke { after_wake(); }
                }
            }
            // dbus-monitor ended: nothing more will come, so the lock is of no use.
            release(&thread_lock);
            thread_alive.store(false, Ordering::Relaxed);
        }).map_err(|e| format!("Could not start the suspend watcher: {e}"))?;
        Ok(Watcher { monitor, lock, alive })
    }

    /// False once the watcher has stopped seeing signals (its dbus-monitor ended).
    pub fn alive(&self) -> bool { self.alive.load(Ordering::Relaxed) }
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
        assert_eq!(parser.feed("   boolean true"), Some(Signal::Sleep(true)));
        assert_eq!(parser.feed(SIGNAL), None);
        assert_eq!(parser.feed("   boolean false"), Some(Signal::Sleep(false)));
    }

    #[test]
    fn a_shutdown_is_told_apart_from_a_suspend() {
        let mut parser = Parser::default();
        assert_eq!(parser.feed(&SIGNAL.replace("PrepareForSleep", "PrepareForShutdown")), None);
        assert_eq!(parser.feed("   boolean true"), Some(Signal::Shutdown(true)));
        assert_eq!(Watch { suspend: true, shutdown: true }.what(), "sleep:shutdown");
        assert_eq!(Watch { suspend: false, shutdown: true }.what(), "shutdown");
        assert_eq!(Watch { suspend: true, shutdown: false }.what(), "sleep");
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
