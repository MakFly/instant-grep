#!/usr/bin/env bash
set -euo pipefail

# instant-grep installer — downloads the latest release binary for your platform
# Usage: curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash

REPO="MakFly/instant-grep"
INSTALL_DIR="${IG_INSTALL_DIR:-$HOME/.local/bin}"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  ARTIFACT="ig-linux-x86_64" ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) ARTIFACT="ig-macos-aarch64" ;;
      x86_64)        ARTIFACT="ig-macos-x86_64" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

# Get latest release tag
TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$TAG" ]; then
  echo "Failed to fetch latest release tag"
  exit 1
fi

URL="https://github.com/$REPO/releases/download/$TAG/$ARTIFACT"

echo "Installing instant-grep $TAG ($ARTIFACT)..."
echo "  → $INSTALL_DIR/ig"

mkdir -p "$INSTALL_DIR"
curl -fsSL "$URL" -o "$INSTALL_DIR/ig"
chmod +x "$INSTALL_DIR/ig"

# Verify
if "$INSTALL_DIR/ig" --version >/dev/null 2>&1; then
  echo "✓ Installed: $("$INSTALL_DIR/ig" --version)"
else
  echo "✗ Installation failed"
  exit 1
fi

# Check PATH
if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
  echo ""
  echo "Add to your shell config:"
  echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
