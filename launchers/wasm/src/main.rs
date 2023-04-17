fn main() {
    tracing_wasm::set_as_global_default();

    wasm_bindgen_futures::spawn_local(asteroids_core::game());
}
