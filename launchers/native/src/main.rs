fn main() {
    tracing_subscriber::fmt::init();

    pollster::block_on(asteroids_core::game());
}
