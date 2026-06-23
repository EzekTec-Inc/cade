use dioxus::prelude::*;

macro_rules! stub_page {
    ($name:ident, $title:expr, $desc:expr) => {
        #[component]
        #[allow(dead_code)]
        pub fn $name() -> Element {
            rsx! {
                div { class: "flex-1 bg-[#0f1115] h-full overflow-y-auto select-text",
                    header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#111218]",
                        h1 { class: "text-lg font-semibold text-white", $title }
                    }
                    div { class: "p-10",
                        div { class: "bg-[#16171d] border border-[#272833] rounded-xl p-8 text-center max-w-lg mx-auto",
                            div { class: "text-4xl mb-4", "\u{1f6a7}" }
                            h2 { class: "text-white font-semibold text-lg mb-2", $title }
                            p { class: "text-gray-400 text-sm", $desc }
                        }
                    }
                }
            }
        }
    };
}

stub_page!(PlaceholderView, "Placeholder", "This page is not yet implemented.");
