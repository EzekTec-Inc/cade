# CADE Keybindings Reference

Complete keyboard shortcuts for the CADE terminal UI, organised by category.

---

## Text Editing

| Key | Action |
|-----|--------|
| **Ctrl+A** · **Home** | Move cursor to start of buffer |
| **Ctrl+E** · **End** | Move cursor to end of buffer |
| **Alt+←** · **Ctrl+←** | Move one word left |
| **Alt+→** · **Ctrl+→** | Move one word right |
| **←** · **→** | Move one character left / right |
| **Ctrl+U** | Delete from cursor to start of buffer |
| **Ctrl+K** | Delete from cursor to end of current line |
| **Ctrl+W** | Delete word backwards (like bash) |
| **Backspace** | Delete character before cursor |
| **Delete** | Delete character at cursor |
| **Ctrl+Z** | Undo last edit (up to 100 levels) |
| **Ctrl+Y** | Redo last undone edit |

---

## Submission & Navigation

| Key | Action |
|-----|--------|
| **Enter** | Submit message to agent |
| **Shift+Enter** · **Alt+Enter** · **Ctrl+Enter** | Insert newline (multi-line mode) |
| **Ctrl+Enter** | Queue follow-up while agent is running |
| **Ctrl+C** | Cancel current input (or interrupt agent turn) |
| **Ctrl+D** | Exit CADE *(only when input field is empty)* |
| **↑** | Previous history entry *(or move cursor up one visual row)* |
| **↓** | Next history entry *(or move cursor down one visual row)* |
| **Esc** | Clear input field |

---

## Completion & Special Input

| Key / Prefix | Action |
|-------------|--------|
| **Tab** | Complete filesystem path at cursor |
| **Shift+Tab** | Cycle permission mode backward |
| **@** *(at start or after space)* | Open fuzzy file picker |
| **/** *(at start)* | List slash commands (e.g. `/context`, `/memory`) |
| **!command** | Run shell command; output shown locally **and** forwarded to the LLM |
| **!!command** | Run shell command silently; output shown locally only |

### `@` File Picker

While the picker overlay is open:

| Key | Action |
|-----|--------|
| Type | Filter matches |
| **↑** / **↓** | Navigate results |
| **Enter** | Insert selected path into input |
| **Backspace** | Remove last filter character; empty → dismiss |
| **Esc** | Dismiss without inserting |

---

## Viewport Navigation

| Key | Action |
|-----|--------|
| **Shift+K** | Scroll conversation up (10 rows) |
| **Shift+J** | Scroll conversation down (10 rows) |
| **Ctrl+O** | Toggle expand / collapse all tool outputs |

---

## Question & Permission Modals

When the agent asks a yes/no question or requests permission:

| Key | Action |
|-----|--------|
| **y** · **Enter** | Approve / yes |
| **n** | Deny / no |
| **Esc** | Dismiss (treated as no) |

---

## Paste Handling

| Key | Action |
|-----|--------|
| **Ctrl+V** | Paste from OS clipboard — image or text |
| **Alt+V** | Paste from OS clipboard (Windows Terminal / WSL alternative) |

**Text pastes**: Large pastes (> 10 lines) are collapsed into a compact marker such as
`[paste #1 +50 lines]` to keep the input field usable.  The full text is
transparently expanded when you press **Enter** to submit.

**Image pastes**: When the clipboard contains a PNG or JPEG, CADE inserts a
`[image #1: 640×480]` placeholder.  On submit the full base64-encoded image is sent
to the LLM as a vision attachment (supported on Claude, GPT-4o, and Gemini models).

---

## Platform Notes

### macOS (iTerm2 / Terminal.app / Ghostty)
- **Cmd+Z** / **Cmd+Shift+Z** are intercepted by macOS; use **Ctrl+Z** / **Ctrl+Y** instead.
- Word navigation via **Option+←** / **Option+→** sends `Alt+Arrow` which CADE recognises.

### Windows (Windows Terminal / WSL)
- **Ctrl+Enter** is the most reliable multi-line key in Windows Terminal.
- **Shift+Enter** works in Windows Terminal with the extended keyboard protocol enabled.
- Some terminals map **Ctrl+←** / **Ctrl+→** to OS-level word jump; **Alt+←** / **Alt+→** are a reliable fallback.

### Linux (Kitty / WezTerm / GNOME Terminal)
- All keybindings work as documented.
- **Ctrl+Z** suspends the process in some shells; CADE runs in raw mode so this is intercepted correctly as undo.

---

## Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | List available slash commands |
| `/context` | Show context window usage |
| `/memory` | Inspect agent memory blocks |
| `/clear` | Clear conversation and reset context |
| `/exit` | Exit CADE |

Run `/help` inside CADE for the full up-to-date list.
