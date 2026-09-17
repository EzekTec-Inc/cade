use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, ToastMessage};

/// Renders a stack of toast notifications in the top-right corner.
#[component]
pub fn ToastContainer() -> Element {
    let state = use_context::<AppState>();
    let toasts = state.toasts;

    rsx! {
        div { class: "fixed top-4 right-4 z-50 flex flex-col space-y-2 pointer-events-none max-w-sm",
            for msg in toasts().iter() {
                toast_bubble {
                    key: "{msg.id}",
                    msg: msg.clone(),
                    on_dismiss: move |id| {
                        let mut t = toasts;
                        let mut list = t();
                        list.retain(|m| m.id != id);
                        t.set(list);
                    },
                }
            }
        }
    }
}

#[component]
fn toast_bubble(msg: ToastMessage, on_dismiss: EventHandler<u64>) -> Element {
    let id = msg.id;

    // Each individual toast bubble spawns its own auto-dismiss timeout
    use_effect(move || {
        let dismiss = on_dismiss;
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(4500).await;
            dismiss.call(id);
        });
    });

    let (bg, border, icon, icon_color) = match msg.level {
        ToastLevel::Info => (
            "bg-[#0d1526]/95",
            "border-blue-500/30",
            "\u{2139}",
            "text-blue-400",
        ),
        ToastLevel::Success => (
            "bg-[#091a13]/95",
            "border-emerald-500/30",
            "\u{2714}",
            "text-emerald-400",
        ),
        ToastLevel::Warning => (
            "bg-[#1f1807]/95",
            "border-yellow-500/30",
            "\u{26a0}",
            "text-yellow-400",
        ),
        ToastLevel::Error => (
            "bg-[#1c0c0e]/95",
            "border-red-500/30",
            "\u{2716}",
            "text-red-400",
        ),
    };

    rsx! {
        div { class: "pointer-events-auto backdrop-blur-md {bg} border {border} rounded-xl p-3.5 shadow-2xl transition-all duration-200 animate-slide-in flex items-start justify-between gap-3",
            div { class: "flex items-start space-x-2.5 min-w-0 flex-1",
                span { class: "text-sm {icon_color} shrink-0 mt-0.5", "{icon}" }
                div { class: "flex flex-col min-w-0 flex-1",
                    span { class: "text-slate-100 text-xs font-semibold tracking-tight", "{msg.title}" }
                    if !msg.detail.is_empty() {
                        span { class: "text-slate-400 text-[11px] mt-0.5 leading-relaxed break-words", "{msg.detail}" }
                    }
                }
            }
            button {
                class: "text-slate-500 hover:text-slate-300 text-xs p-1 rounded hover:bg-white/5 transition shrink-0 select-none",
                onclick: move |_| on_dismiss.call(id),
                "✕"
            }
        }
    }
}
