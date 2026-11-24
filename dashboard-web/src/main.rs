#[macroquad::main("pi-dashboard (playground)")]
async fn main() {
    loop {
        macroquad::window::clear_background(macroquad::color::BLACK);
        macroquad::window::next_frame().await;
    }
}
