# CADE Self-Update Mechanism Proposal

Based on an investigation of the `pi-coding` agent and the current state of CADE, here is a proposal for a robust self-update mechanism.

## Current State of CADE
CADE currently utilizes the `self_update` Rust crate (in `crates/cade-cli/src/cli/update.rs`) to fetch the latest GitHub Release binaries and directly overwrite the local `cade` and `cade-server` executables. 

**Shortcomings:**
1. **Blind Overwrites:** It assumes a standalone binary installation. If a user installed CADE via `cargo install cade`, the updater blindly overwrites the cargo binary with a GitHub release binary, bypassing cargo entirely.
2. **Windows File Locks (EBUSY):** Windows strictly locks running executables (`cade-server.exe`). The current code explicitly acknowledges it cannot overwrite a running server:
   ```rust
   eprintln!("\r\n[!] Warning: Failed to update cade-server (it may be running and locked).");
   ```

---

## Inspiration from `pi-coding`
The `pi-coding` agent has a highly mature self-update system that CADE can learn from:
1. **Installation Method Detection:** It detects how it was installed (`npm`, `pnpm`, `bun`, `yarn`, or standalone binary).
2. **Delegation:** Instead of manually downloading files for package-managed installations, it delegates back to the original package manager (e.g., spawning `npm install -g @earendil-works/pi-coding-agent`).
3. **Windows Quarantine Pattern:** To bypass Windows EBUSY file locks on running native `.node` dependencies, `pi-coding` "quarantines" them. It renames the locked files into a `.pi-native-quarantine` directory. Windows allows renaming a locked file, which frees up the original path for the package manager to write the new files successfully.
4. **Permissions Checks:** It checks if the binary directory is writable before even attempting an update.

---

## Proposed Implementation for CADE

To modernize CADE's update system using the principles from `pi-coding`, we should implement the following steps:

### 1. Detect Installation Method
Before attempting any update, CADE should determine its provenance.
- **Cargo Install:** Check if the current executable path (`std::env::current_exe()`) resides inside `~/.cargo/bin`.
- **Standalone:** If not in a package manager directory, assume it is a standalone binary downloaded from GitHub.

### 2. Smart Update Delegation
Instead of always pulling from GitHub:
- **If Cargo:** Spawn a child process to run `cargo install cade --force`. This ensures the user's toolchain and package registry remain the source of truth.
- **If Standalone:** Continue using the `self_update` crate to pull from GitHub Releases.

### 3. Graceful Windows Executable Replacement (The Quarantine Pattern)
To fix the `cade-server.exe` lock issue on Windows, we should adopt the quarantine pattern:
1. When downloading the new `cade-server.exe`, do not try to overwrite the existing one directly.
2. Instead, **rename** the running `cade-server.exe` to `cade-server.exe.old` (or move it to a `.quarantine` folder). Windows allows renaming memory-mapped running executables.
3. Extract the newly downloaded binary into the now-empty `cade-server.exe` path.
4. During the next startup of CADE (or via a cleanup thread), quietly delete any leftover `.old` quarantine files if they are no longer locked.

### 4. Permissions Pre-flight Check
Before downloading MBs of binaries or invoking cargo, perform a quick write-test (or permissions check) in the directory of `std::env::current_exe()`. If the user installed CADE system-wide (e.g., `/usr/local/bin`) and is running the update without `sudo`, gracefully abort and suggest:
`"This installation is managed by a system directory. Please run the update command with sudo."` (Matching the fallback behavior in `pi-coding`).