# Running CADE on Windows

## Prerequisites

### 1. **Rust Toolchain**
   - Download and install [Rust for Windows](https://rustup.rs/) — the MSVC toolchain is recommended
   - Verify: `rustc --version` and `cargo --version` in PowerShell/CMD

### 2. **Git for Windows** (Required)
   - [Download Git for Windows](https://git-scm.com/download/win)
   - This provides:
     - `git` binary (needed for version control tools)
     - **`patch` utility** (bundled with Git for Windows — needed for `apply_patch` tool)
   - During installation, select "Add Git Bash to PATH" (or add manually to `PATH`)
   - Verify: `git --version` and `patch --version` in PowerShell

### 3. **API Key**
   - Get an Anthropic API key from [console.anthropic.com](https://console.anthropic.com)
   - Keep it handy — you'll need it to start the server

### 4. **PowerShell 5.1+ or CMD.exe**
   - CADE uses `cmd.exe /C` on Windows for shell commands
   - Both PowerShell and CMD work; CMD is slightly faster (~5ms vs ~100ms per command)

---

## Installation & Build

### Step 1: Clone or Extract the Repository
```powershell
# If you have the source as a ZIP or tar archive
cd C:\Users\YourName\Desktop
# Extract/unzip CADE to a local directory

# Or clone from GitHub
git clone https://github.com/EzekTec-Inc/CADE.git
cd CADE
```

### Step 2: Build the Project
```powershell
# Debug build (faster, no optimizations):
cargo build

# Release build (slower, optimized):
cargo build --release
```

**Expected output:**
```
   Compiling cade-core v0.2.0
   Compiling cade-agent v0.2.0
   ...
    Finished `release` profile [optimized] target(s) in 2m 30s
```

The binaries will be in:
- **Debug:** `target\debug\cade.exe` and `target\debug\cade-server.exe`
- **Release:** `target\release\cade.exe` and `target\release\cade-server.exe`

---

## Running CADE

### Option A: Start Server + CLI (Recommended)

**Terminal 1 — Start the Server:**
```powershell
# PowerShell or CMD
set ANTHROPIC_API_KEY=sk-ant-...your-key-here...
.\target\release\cade-server.exe
```

Expected output:
```
[INFO] CADE server listening on http://127.0.0.1:8284
```

**Terminal 2 — Start the CLI:**
```powershell
.\target\release\cade.exe
```

The CLI auto-connects to the server on `localhost:8284`.

### Option B: Headless Mode (Single Command)
```powershell
set ANTHROPIC_API_KEY=sk-ant-...your-key-here...
.\target\release\cade.exe -p "Explain the Rust ownership system"
```

Output: Direct text response (no interactive UI).

---

## First Run

On first launch, CADE will:
1. Create a `.cade/` directory in the current working directory with settings
2. Create a `.cade/cade.db` file (SQLite database for agent state)
3. Generate a random agent ID and remember it in `.cade/session.json`
4. Load available tools (bash, read_file, write_file, edit_file, grep, glob, etc.)

On subsequent launches, CADE resolves your agent in this order:
1. **CLI flags** (`--agent <id>`, `--name <query>`, `--new-agent`)
2. **Local project agent** from `.cade/session.json`
3. **Global last agent** from `~/.cade/settings.json`
4. **Create new** (fallback)

Then you can start typing prompts:
```
You: Fix the bug in src/main.rs where...
Agent: I'll analyze the code and apply a fix...
[Agent uses read_file, grep, edit_file tools]
```

---

## Important Windows-Specific Notes

### ✅ Works Great on Windows
- ✓ All file operations (read, write, edit, apply_patch)
- ✓ Git commands (`git status`, `git log`, etc.)
- ✓ Desktop notifications (via Windows Toast)
- ✓ Screen capture (via Windows capture APIs)
- ✓ Input control (keyboard/mouse simulation)

### ⚠️ Shell Commands (`bash` tool)

**Key difference on Windows:**

On Linux/macOS, CADE runs commands with `bash -c`.  
On Windows, CADE runs commands with `cmd.exe /C`.

This means:
- Windows batch syntax works: `dir`, `echo`, `powershell -Command ...`, etc.
- Bash-isms don't work: `ls`, `cat`, `echo $VAR`, `if [ ... ]`, `&&`, `||` (mostly)

**Examples that work on Windows:**
```
cade> run: dir /s src
cade> run: powershell -Command "Get-ChildItem -Recurse src"
cade> run: git status
cade> run: cargo build
```

**Examples that don't work:**
```
# ❌ Bash syntax
cade> run: ls -la src          # Use 'dir /s src' instead
cade> run: cat README.md       # Use 'type README.md' instead
cade> run: if [ -f file ]; then echo "yes"; fi  # Use batch IF instead
```

**Workaround — Use PowerShell:**
```
cade> run: powershell -Command "Get-Content README.md | Select-String 'bug'"
```

### ⚠️ `patch` Utility

If `patch` is not found:
```
apply_patch: the `patch` utility was not found. 
Install Git for Windows (includes `patch`) or use WSL.
```

**Solution:** Ensure Git for Windows is installed and `patch` is in your `PATH`.

Verify:
```powershell
patch --version
```

If it's not found, add Git's `bin` directory to `PATH`:
```powershell
# Find Git installation
Get-ChildItem -Path "C:\Program Files*" -Filter "Git" -Type Directory

# Add to PATH (PowerShell):
$env:Path += ";C:\Program Files\Git\usr\bin"
```

### ⚠️ Environment Variables

On Windows, use `set` (CMD) or `$env:` (PowerShell):

**CMD:**
```cmd
set ANTHROPIC_API_KEY=sk-ant-...
set CADE_MODEL=anthropic/claude-opus
cade.exe
```

**PowerShell:**
```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
$env:CADE_MODEL = "anthropic/claude-opus"
.\cade.exe
```

---

## Configuration Files

After first run, check `.cade/session.json`:

```json
{
  "last_agent_id": "...",
  "execution_backend": "local",
  "permission_mode": "default"
}
```

For global settings, edit `~/.cade/settings.json`:
```json
{
  "store_api_key": true,
  "default_model": "anthropic/claude-sonnet-4-5",
  "permission_mode": "default"
}
```

---

## Troubleshooting

### Problem: `cargo build` fails with linker errors
**Solution:** Ensure you have the MSVC build tools installed. Download [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

### Problem: `ANTHROPIC_API_KEY` not recognized
**Solution:** Use `set` (CMD) or `$env:` (PowerShell) before launching. Check with `echo %ANTHROPIC_API_KEY%` (CMD) or `$env:ANTHROPIC_API_KEY` (PowerShell).

### Problem: Can't find `cade.exe` after build
**Solution:** Binaries are in `target\release\` (or `target\debug\`). Use the full path:
```powershell
.\target\release\cade.exe
```

Or add `target\release\` to your `PATH` for quick access:
```powershell
$env:Path += ";$pwd\target\release"
cade.exe
```

### Problem: Bash commands don't work
**Solution:** Remember Windows uses `cmd.exe /C`, not `bash`. Use batch/PowerShell syntax, or prefix with `powershell -Command`:
```
cade> run: powershell -Command "ls src | Measure-Object"
```

### Problem: `apply_patch` fails with "patch not found"
**Solution:** Install Git for Windows and ensure `patch` is in `PATH`:
```powershell
git --version    # Should work
patch --version  # Should work
```

If `patch --version` fails, reinstall Git for Windows and select the option to add Unix tools to PATH.

---

## Next Steps

1. **Read the [README.md](README.md)** for CLI usage and command reference
2. **Read the [ARCHITECTURE.md](ARCHITECTURE.md)** for internal architecture and data flow
3. **Try a simple prompt:**
   ```
   cade> List all Rust files in src/ and show their line counts
   ```
4. **Set up your project context** via `/init` or `/memory set project ...`
5. **Use slash commands** — type `/help` inside CADE for full reference

---

## Cross-Platform Features

With the recent cross-platform implementation (commit `18d194d3`), CADE now properly detects and uses:

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Shell commands | `bash -c` | `bash -c` | `cmd.exe /C` |
| Notifications | D-Bus (urgency) | macOS sound | Windows Toast |
| Screen capture | ✓ | ✓ | ✓ |
| Input control | ✓ | ✓ | ✓ |
| File operations | ✓ | ✓ | ✓ |
| Git integration | ✓ | ✓ | ✓ |
| Docker backend | ✓ | ✓ | ✓ (requires Docker Desktop) |
| SSH backend | ✓ | ✓ | ✓ |

---

## Support & Feedback

Found a bug or have a feature request? Open an issue on [GitHub](https://github.com/EzekTec-Inc/CADE/issues) or use the `/feedback` command inside CADE.
