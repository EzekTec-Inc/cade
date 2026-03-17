# Security Fix Implementation Plan

Based on the security review of the CADE workspace, the following four fixes will be implemented to secure the application against RCE, path traversal, API key exposure, and timing attacks.

## 1. Prevent Auto-Approval of Config/Skill Edits (RCE Mitigation)
**Target:** `src/permissions/mod.rs`
**Issue:** When the `AcceptEdits` permission mode is active, the agent can auto-approve edits to critical configuration files (e.g., `~/.cade/settings.json`), triggering a hot-reload of malicious MCP server commands (RCE).
**Implementation:**
- Modify `PermissionManager::auto_approve()` to explicitly block auto-approval (return `false`) if the tool is an edit/write operation AND the target path contains `.cade/settings.json`, `settings.local.json`, or `.skills/`.
- Ensure these sensitive paths always require explicit manual user approval via the TUI prompt, regardless of the active permission mode.

## 2. Fix Path Traversal in Skill Installation
**Target:** `src/skills/mod.rs` (`install_skill_from_url` function)
**Issue:** The `skill_id` is derived from the URL via `.rsplit('/').next()`. A malicious URL (e.g., `http://example.com/..`) evaluates to `..`, allowing the installer to write files outside the intended `.skills/` directory.
**Implementation:**
- Add explicit input validation to the derived `skill_id`.
- Ensure it contains only alphanumeric characters and dashes (`[a-zA-Z0-9\-]`).
- If the `skill_id` contains `.` or `/`, reject the installation with an error: `anyhow::bail!("Invalid skill ID derived from URL")`.

## 3. Secure Configuration File Permissions
**Target:** `src/settings/manager.rs` (`save_to_file` function)
**Issue:** `std::fs::write` creates files with default permissions (`0644` on Unix), exposing sensitive API keys (e.g., `CADE_API_KEY`) to other users on the same machine.
**Implementation:**
- Replace `std::fs::write(path, content)?` with `std::fs::OpenOptions` and `std::io::Write`.
- Use `std::os::unix::fs::OpenOptionsExt` to explicitly set `.mode(0o600)` (read/write for owner only).
- Apply this secure file creation to both the global `~/.cade/settings.json` and the project-local `.cade/settings.local.json`.

## 4. Mitigate Authentication Timing Attacks
**Target:** `src/server/api/auth.rs` (`auth_middleware` function)
**Issue:** The API key verification uses a standard string equality check (`token == expected`), which short-circuits. This enables theoretical timing attacks where an attacker guesses the token byte-by-byte.
**Implementation:**
- Import the `subtle` crate (which is already in the dependency tree, verified via `Cargo.lock` output during build).
- Use `subtle::ConstantTimeEq` to compare the `provided.as_bytes()` and `expected.as_bytes()` in constant time.
- E.g.: `let is_valid = provided.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1;`
