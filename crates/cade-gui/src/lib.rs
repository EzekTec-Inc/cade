use dioxus::prelude::*;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    // Launch the Dioxus web application
    launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        div {
            style: "display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 100vh; background-color: #0f1115; color: #e5e7eb; font-family: system-ui, -apple-system, sans-serif; padding: 20px;",
            h1 {
                style: "font-size: 2.5rem; font-weight: 800; margin-bottom: 10px; letter-spacing: -0.025em;",
                "CADE Dashboard"
            }
            p {
                style: "font-size: 1.125rem; color: #9ca3af; margin-bottom: 24px; max-width: 28rem; text-align: center;",
                "This GUI has been successfully refactored from scratch with Dioxus to implement the new screen layout."
            }
            div {
                style: "background-color: #1f2937; border: 1px solid #374151; border-radius: 8px; padding: 24px; max-width: 24rem; width: 100%; box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.1);",
                h2 {
                    style: "font-size: 1.25rem; font-weight: 700; margin-bottom: 12px; margin-top: 0;",
                    "Status"
                }
                p {
                    style: "color: #d1d5db; font-size: 0.875rem; margin-bottom: 16px; line-height: 1.25rem;",
                    "The Egui architecture has been removed. A clean, high-performance Dioxus framework is now in place and ready for the pixel-perfect layout implementation."
                }
                div {
                    style: "display: flex; align-items: center; gap: 8px; color: #34d399; font-size: 0.875rem; font-weight: 600;",
                    span {
                        style: "display: inline-block; width: 12px; height: 12px; border-radius: 50%; background-color: #10b981; box-shadow: 0 0 8px #10b981;"
                    }
                    span { "Ready for Layout Implementation" }
                }
            }
        }
    }
}
