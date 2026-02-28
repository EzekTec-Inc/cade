use ksni::menu::StandardItem;
use std::sync::mpsc;

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

/// Run the system tray in a background thread.
/// Returns a sender to control tray state.
pub fn spawn_tray() -> Result<mpsc::Sender<TrayMsg>, String> {
    let (tx, rx) = mpsc::channel::<TrayMsg>();

    std::thread::spawn(move || {
        use ksni::*;

        struct CadeTray {
            status: TrayStatus,
        }

        impl Tray for CadeTray {
            fn icon_name(&self) -> String {
                "utilities-terminal".to_string()
            }

            fn title(&self) -> String {
                match self.status {
                    TrayStatus::Idle => "CADE — Idle".to_string(),
                    TrayStatus::Working => "CADE — Working…".to_string(),
                    TrayStatus::AwaitingApproval => "CADE — Needs Approval!".to_string(),
                }
            }

            fn menu(&self) -> Vec<MenuItem<Self>> {
                vec![
                    MenuItem::Standard(StandardItem {
                        label: "Show CADE".to_string(),
                        activate: Box::new(|_| {}),
                        ..Default::default()
                    }),
                    MenuItem::Separator,
                    MenuItem::Standard(StandardItem {
                        label: "Quit".to_string(),
                        activate: Box::new(|_| std::process::exit(0)),
                        ..Default::default()
                    }),
                ]
            }
        }

        let tray = ksni::TrayService::new(CadeTray { status: TrayStatus::Idle });
        let handle = tray.handle();

        // Spawn message handler
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    TrayMsg::SetStatus(_status) => {
                        // TODO: update tray icon/tooltip via handle
                        let _ = handle.update(|t: &mut CadeTray| {
                            // t.status = _status;
                            let _ = t; // suppress unused warning for now
                        });
                    }
                    TrayMsg::Quit => break,
                }
            }
        });

        let _ = tray.run();
    });

    Ok(tx)
}
