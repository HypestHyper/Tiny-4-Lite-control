mod backend;
mod capture;
mod cli;
mod config;
mod denoise;
mod install;
mod presets;
mod service;
mod setup;
mod suspend;
mod ui;
mod video;

pub(crate) use backend::*;

/// Where a crash is written down: ~/.local/state/tiny4lite-control/crash.log.
pub(crate) fn crash_log_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("tiny4lite-control").join("crash.log")
}

/// Write every crash (a panic) to the crash log as well as the terminal, with where it happened, so a
/// crash that nobody saw in a terminal can still be looked at.
fn record_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let trace = std::backtrace::Backtrace::force_capture();
        let text = format!("--- crash at {when} (unix time), version {}, thread {thread}\n{info}\n{trace}\n", cli::build_label());
        let path = crash_log_path();
        if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = file.write_all(text.as_bytes());
        }
        eprintln!("tiny4lite-control crashed; the details are in {}", path.display());
        default(info);
    }));
}

fn main() {
    record_crashes();
    // Commands run without opening a window, and without needing a display.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::is_cli(&args) {
        std::process::exit(cli::run(args));
    }
    ui::app::run();
}
