mod backend;
mod capture;
mod config;
mod presets;
mod ui;
mod video;

pub(crate) use backend::*;

fn main() {
    ui::app::run();
}
