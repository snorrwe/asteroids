fn main() {
    pollster::block_on(asteroids_core::game());
}
