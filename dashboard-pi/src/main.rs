use dashboard_core::board_config::load_board_config;
use std::path::Path;

#[macroquad::main("pi-dashboard")]
async fn main() {
    if let Err(error) = load_board_config(Path::new("board.toml")) {
        eprintln!("{error}");
        std::process::exit(1);
    }

    loop {
        macroquad::window::clear_background(macroquad::color::BLACK);
        macroquad::window::next_frame().await;
    }
}
