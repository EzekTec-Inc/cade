# Refactoring & Remediation Plan: CADE Clipboard & Paste Infrastructure

This document outlines the architectural plan to fix and optimize CADE's clipboard and pasting mechanics. It implements direct clipboard image harvesting (bypassing terminal standard input limits) and introduces high-velocity key throttling to prevent large code pastes from corrupting the prompt buffer.

---

## 1. Direct Clipboard Image Harvesting (`crates/cade-tui/src/app/clipboard.rs`)

**Goal:** Allow users to paste raw image pixels (e.g. from a screenshot utility) directly into CADE TUI using `Ctrl+V`, bypassing the physical stdout/stdin text-only limits of terminal emulators.

*   [ ] **Step 1.1: Implement `read_clipboard_image`**
    *   Add a function inside `crates/cade-tui/src/app/clipboard.rs`:
        ```rust
        pub(crate) fn read_clipboard_image() -> Option<(String, u32, u32)> {
            let mut cb = arboard::Clipboard::new().ok()?;
            let img = cb.get_image().ok()?;
            
            // Encode raw pixels to PNG bytes in memory
            let mut png_bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut png_bytes, img.width as u32, img.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(&img.bytes).ok()?;
            drop(writer);

            // Encode to base64
            use base64::Engine;
            let b64 = base64::prelude::BASE64_STANDARD.encode(&png_bytes);
            Some(("image/png".to_string(), img.width as u32, img.height as u32, b64))
        }
        ```
*   [ ] **Step 1.2: Bind `Ctrl+V` & `Alt+V` to Clipboard Reader**
    *   In `crates/cade-tui/src/app/input.rs` inside `TuiApp::handle_key_input`, intercept:
        *   `KeyCode::Char('v')` / `KeyCode::Char('V')` with `KeyModifiers::CONTROL` (Ctrl+V)
        *   `KeyCode::Char('v')` / `KeyCode::Char('V')` with `KeyModifiers::ALT` (Alt+V)
    *   If triggered:
        *   Attempt to read an image via `read_clipboard_image()`. If successful, call `self.handle_image_paste()` and set `draw_dirty = true`.
        *   If no image is in the clipboard, fallback to reading text via `arboard::Clipboard::get_text()`, and call `self.editor.handle_paste(&text)`.

---

## 2. Input Throttling for Non-Bracketed Code Pastes (`crates/cade-tui/src/app/input.rs`)

**Goal:** Prevent high-velocity simulated keypresses (during terminal emulator pastes where Bracketed Paste Mode is disabled) from flooding the TUI event loop, spawning corrupt autocomplete menus, or dropping characters.

*   [ ] **Step 2.1: Add `last_keypress_instant` Tracker**
    *   Add `last_keypress: std::time::Instant` and `is_pasting: bool` fields to `TuiApp` inside `crates/cade-tui/src/app/mod.rs`.
*   [ ] **Step 2.2: Implement High-Velocity Key Detection**
    *   At the start of `handle_key_input` inside `input.rs`, calculate the elapsed time since the previous keypress:
        ```rust
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_keypress);
        self.last_keypress = now;

        // If key events are arriving less than 3ms apart, we are in a simulated paste flood
        let is_flooding = delta.as_millis() < 3;
        ```
*   [ ] **Step 2.3: Suppress Autocomplete & Redraws Mid-Flood**
    *   If `is_flooding` is true, **bypass/suppress the autocomplete overlay search** and skip immediate frame redraws.
    *   Buffer the incoming characters inside `self.editor` and trigger a single, debounced frame redraw once the key velocity slows down back to human typing thresholds (e.g. `delta.as_millis() >= 50`).

---

## 3. Verification & Quality Gates

*   [ ] **Step 3.1: Compilation and Warnings**
    *   Verify the project compiles cleanly under strict lints:
        `cargo check --workspace`
        `cargo clippy --workspace -- -D warnings`
*   [ ] **Step 3.2: Unit and Regression Tests**
    *   Add unit tests in `crates/cade-tui/src/app/app_tests.rs` verifying that high-velocity simulated typing triggers the throttling state and correctly preserves the input text buffer without spawning overlay menus.
    *   Execute full workspace test suite:
        `cargo test --workspace`
