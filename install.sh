#!/usr/bin/env bash
set -e

# CADE Installation Script for Linux and macOS
REPO="EzekTec-Inc/cade"

echo "=========================================="
echo "    Installing CADE AI Coding Assistant   "
echo "=========================================="

# 1. Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

if [ "$ARCH" != "x86_64" ]; then
    if [ "$OS" = "Darwin" ] && [ "$ARCH" = "arm64" ]; then
        echo "Detected Apple Silicon (M1/M2/M3/M4). Using x86_64 binary via Rosetta."
        if ! /usr/bin/pgrep oahd >/dev/null 2>&1 && [ ! -f /Library/Apple/usr/libexec/oah/libRosettaRuntime ]; then
            echo "Note: If Rosetta 2 is not yet installed on this Mac, run:"
            echo "  softwareupdate --install-rosetta"
        fi
        ARCH="x86_64"
    else
        echo "Error: Unsupported architecture $ARCH. CADE currently provides pre-built binaries for x86_64."
        exit 1
    fi
fi

TARGET=""
case "$OS" in
    Linux)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    Darwin)
        TARGET="x86_64-apple-darwin"
        ;;
    *)
        echo "Error: Unsupported OS $OS"
        exit 1
        ;;
esac

ASSET_NAME="cade-${TARGET}.tar.gz"

# 2. Fetch Latest Release
echo "[1/4] Fetching latest release info..."
LATEST_RELEASE=$(curl -s -H "User-Agent: CADE-Installer" https://api.github.com/repos/${REPO}/releases/latest | grep '"tag_name":' | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST_RELEASE" ]; then
    # Fallback to redirect resolution in case of API rate limits
    LATEST_RELEASE=$(curl -sI "https://github.com/${REPO}/releases/latest" | grep -i "^location:" | head -n 1 | sed -E 's|.*/tag/([^/\r\n]+).*|\1|' | tr -d '\r\n')
fi

if [ -z "$LATEST_RELEASE" ]; then
    echo "Error: Could not determine latest release version."
    exit 1
fi
echo "Latest version: $LATEST_RELEASE"

DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_RELEASE}/${ASSET_NAME}"

# 3. Setup Directories
INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# 4. Download and Extract
echo "[2/4] Downloading $ASSET_NAME..."
if ! curl -fL -# -o "$TMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"; then
    echo ""
    echo "Error: Failed to download $ASSET_NAME from $DOWNLOAD_URL"
    echo "Pre-built binaries for $TARGET may not be published for release $LATEST_RELEASE yet."
    echo "You can build CADE directly from source with:"
    echo "  cargo install --path crates/cade-cli"
    echo "  cargo install --path crates/cade-server-bin"
    exit 1
fi

echo "[3/4] Extracting binaries..."
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

# Locate binaries (whether flat or inside nested directories)
CADE_BIN=$(find "$TMP_DIR" -type f -name "cade" | head -n 1)
SERVER_BIN=$(find "$TMP_DIR" -type f -name "cade-server" | head -n 1)

if [ -z "$CADE_BIN" ] || [ -z "$SERVER_BIN" ]; then
    echo "Error: Could not locate extracted binaries in archive."
    exit 1
fi

# 5. Install Binaries
echo "[4/4] Installing to $INSTALL_DIR..."
mv "$CADE_BIN" "$INSTALL_DIR/cade"
mv "$SERVER_BIN" "$INSTALL_DIR/cade-server"
chmod +x "$INSTALL_DIR/cade" "$INSTALL_DIR/cade-server"

# Strip macOS quarantine attribute if present
if [ "$OS" = "Darwin" ]; then
    xattr -d com.apple.quarantine "$INSTALL_DIR/cade" "$INSTALL_DIR/cade-server" 2>/dev/null || true
fi

# Ensure ~/.local/bin is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "Warning: $INSTALL_DIR is not in your PATH."
    echo "Please add the following line to your ~/.bashrc, ~/.zshrc, or ~/.profile:"
    echo "export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi

echo "=========================================="
echo "    CADE successfully installed!          "
echo "=========================================="

# 6. Run CADE if in an interactive terminal, otherwise prompt
if [ -t 0 ]; then
    echo "Starting CADE for the first time..."
    "$INSTALL_DIR/cade"
else
    echo "Run 'cade' in your terminal to start CADE!"
fi
