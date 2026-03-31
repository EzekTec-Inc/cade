# Cross-Platform Desktop Refactor Plan

## Objective
Migrate the `cade-desktop` crate from Linux-specific tools and APIs (X11/Wayland/D-Bus) to native cross-platform implementations supporting Windows, macOS, and Linux, without changing the public API exposed to the rest of the CADE workspace.

## Phase 1: Input & Window Control (`desktop/control.rs`)
**Current:** Spawns `xdotool` and `ydotool` via `tokio::process::Command`.
**New:** Pure Rust native OS APIs.

1. **Dependencies:**
   - Add `enigo = "0.2"` to `crates/cade-desktop/Cargo.toml` for cross-platform keyboard/mouse simulation.
   - Add `active-win-pos-rs = "0.8"` (or `active-win-pos`) for cross-platform window management (focusing/listing).

2. **Implementation:**
   - Remove all `Command::new("xdotool")` and `Command::new("ydotool")` wrappers.
   - Refactor `DesktopControl::focus_window(title)` to iterate over open windows using `active-win-pos-rs` and bring the matching window to the foreground.
   - Refactor `type_text`, `key_press`, `move_mouse`, and `click` to instantiate `enigo::Enigo` and call its native simulation methods (`enigo.key(...)`, `enigo.button(...)`, `enigo.move_mouse(...)`).
   - Remove `ControlTool` enum and OS-specific tool detection logic.

## Phase 2: System Tray (`desktop/tray.rs`)
**Current:** Uses `ksni`, which strictly implements the KDE Status Notifier Item specification over D-Bus (Linux only).
**New:** Uses `tray-icon`, a Tauri-backed cross-platform library.

1. **Dependencies:**
   - Remove `ksni` from the workspace and `cade-desktop` dependencies.
   - Add `tray-icon = "0.14"` to `crates/cade-desktop/Cargo.toml`.

2. **Implementation:**
   - Rewrite the tray initialization in `desktop/tray.rs` to build a `tray_icon::TrayIcon` with a native menu.
   - Map the tray menu events (e.g., "Quit", "Show CADE") using `tray_icon::menu::MenuEvent::receiver()` in the background async task.
   - Ensure the icon asset is loaded correctly via the `icon` module for Windows/macOS/Linux formats.

## Phase 3: OS Notifications (`desktop/notify.rs`)
**Current:** Uses `notify-rust` with default Linux D-Bus features.
**New:** Multi-platform feature enablement.

1. **Dependencies:**
   - Update `notify-rust` in `Cargo.toml` to explicitly include macOS and Windows features (e.g., `features = ["mac_os", "d"]`). 
   - *Note:* `notify-rust` uses WinRT on Windows and `mac-notification-sys` on macOS.

2. **Implementation:**
   - Ensure `send_notification` does not rely on any Linux-specific hints or actions that are unsupported on Windows/macOS. 
   - Test fallback behavior if the OS notification center is disabled by the user.

## Phase 4: Screen Capture (`desktop/capture.rs`)
**Current:** Uses `xcap`.
**New:** No code changes required.
- `xcap` is already fully cross-platform (DXGI/GDI on Windows, CoreGraphics on macOS, X11/XCB on Linux). Ensure it compiles successfully on all target OS environments during CI.

## Phase 5: CI & Validation
1. Update `.github/workflows/ci.yml` (or equivalent) to build the workspace on `windows-latest` and `macos-latest` runners.
2. Ensure the `desktop` feature flag can be successfully enabled on all platforms.
3. Validate that `cade-desktop` maintains its acyclic, isolated architecture (no new dependencies leaking into `cade-core` or `cade-agent`).