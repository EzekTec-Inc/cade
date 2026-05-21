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
LATEST_RELEASE=$(curl -s https://api.github.com/repos/${REPO}/releases/latest | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

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
curl -L -# -o "$TMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

echo "[3/4] Extracting binaries..."
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

# 5. Install Binaries
echo "[4/4] Installing to $INSTALL_DIR..."
mv "$TMP_DIR/cade" "$INSTALL_DIR/"
mv "$TMP_DIR/cade-server" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/cade" "$INSTALL_DIR/cade-server"

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
echo "Starting CADE for the first time..."

# 6. Run CADE
"$INSTALL_DIR/cade"
