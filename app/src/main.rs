mod backend;
mod capture;
mod cli;
mod config;
mod presets;
mod ui;
mod video;

pub(crate) use backend::*;

fn main() {
    // Commands run without opening a window, and without needing a display.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::is_cli(&args) {
        std::process::exit(cli::run(args));
    }
    ui::app::run();
}
