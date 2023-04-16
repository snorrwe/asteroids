use yew::prelude::*;

#[function_component(App)]
pub fn app() -> Html {
    html! {
        <main>
        </main>
    }
}

fn main() {
    tracing_wasm::set_as_global_default();

    yew::Renderer::<App>::new().render();
    wasm_bindgen_futures::spawn_local(asteroids_core::game());
}
