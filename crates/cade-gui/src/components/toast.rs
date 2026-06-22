use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, ToastMessage};

/// Renders a stack of toast notifications in the top-right corner.
#[component]
pub fn ToastContainer() -> Element {
    let state = use_context::<AppState>();
    let toasts = state.toasts;

    // Auto-dismiss after 4 seconds
    let toast_count = toasts().len();
    use_effect(move || {
        if toast_count > 0 {
            let mut t = toasts;
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(4000).await;
                let mut list = t();
                if !list.is_empty() {
                    list.remove(0);
                    t.set(list);
                }
            });
        }
    });

    rsx! {
        div { class: "fixed top-4 right-4 z-50 flex flex-col space-y-2 pointer-events-none",
            for msg in toasts().iter() {
                toast_bubble { msg: msg.clone() }
            }
        }
    }
}

#[component]
fn toast_bubble(msg: ToastMessage) -> Element {
    let (bg, icon) = match msg.level {
        ToastLevel::Info => ("bg-blue-500/20 border-blue-500/40", "\u{2139}"),
        ToastLevel::Success => ("bg-emerald-500/20 border-emerald-500/40", "\u{2714}"),
        ToastLevel::Warning => ("bg-yellow-500/20 border-yellow-500/40", "\u{26a0}"),
        ToastLevel::Error => ("bg-red-500/20 border-red-500/40", "\u{2716}"),
    };

    rsx! {
        div { class: "pointer-events-auto animate-slide-in backdrop-blur-sm {bg} border rounded-lg px-4 py-3 max-w-sm shadow-lg",
            div { class: "flex items-start space-x-2",
                span { class: "text-sm", "{icon}" }
                div { class: "flex flex-col",
                    span { class: "text-white text-xs font-semibold", "{msg.title}" }
                    if !msg.detail.is_empty() {
                        span { class: "text-gray-300 text-[11px] mt-0.5", "{msg.detail}" }
                    }
                }
            }
        }
    }
}
