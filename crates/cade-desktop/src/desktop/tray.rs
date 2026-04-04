use std::sync::mpsc;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

/// System tray status indicator
#[derive(Debug, Clone, Copy)]
pub enum TrayStatus {
    Idle,
    Working,
    AwaitingApproval,
}

/// Message sent from main thread to tray
pub enum TrayMsg {
    SetStatus(TrayStatus),
    Quit,
}

/// Tray menu action returned to main thread
pub enum TrayAction {
    ShowWindow,
    NewSession,
    Quit,
}

/// Create a 1x1 transparent icon for the tray
fn create_dummy_icon() -> Icon {
    let rgba = vec![0, 0, 0, 0];
    Icon::from_rgba(rgba, 1, 1).unwrap()
}

/// Run the system tray in a background thread.
/// Returns a sender to control tray state.
pub fn spawn_tray() -> Result<mpsc::Sender<TrayMsg>, String> {
    let (tx, rx) = mpsc::channel::<TrayMsg>();

    std::thread::spawn(move || {
        let tray_menu = Menu::new();
        let show_i = MenuItem::new("Show CADE", true, None);
        let quit_i = MenuItem::new("Quit", true, None);

        tray_menu
            .append_items(&[&show_i, &PredefinedMenuItem::separator(), &quit_i])
            .unwrap();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("CADE — Idle")
            .with_icon(create_dummy_icon())
            .build()
            .unwrap();

        let menu_channel = MenuEvent::receiver();

        loop {
            // Check for commands from the main thread
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    TrayMsg::SetStatus(status) => {
                        let title = match status {
                            TrayStatus::Idle => "CADE — Idle",
                            TrayStatus::Working => "CADE — Working…",
                            TrayStatus::AwaitingApproval => "CADE — Needs Approval!",
                        };
                        let _ = tray_icon.set_tooltip(Some(title));
                    }
                    TrayMsg::Quit => break,
                }
            }

            // Check for menu clicks
            if let Ok(event) = menu_channel.try_recv() {
                if event.id == quit_i.id() {
                    std::process::exit(0);
                } else if event.id == show_i.id() {
                    // Do nothing for now
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(tx)
}
