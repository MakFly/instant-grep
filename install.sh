#!/usr/bin/env bash
set -euo pipefail

# instant-grep installer — downloads the latest release binary for your platform
# Usage: curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash

REPO="MakFly/instant-grep"

# When running under sudo, resolve the real user's home directory.
# Only use getent (reads /etc/passwd); never fall back to shell expansion
# since `eval echo ~$SUDO_USER` is a command-injection sink if SUDO_USER
# is attacker-controlled.
if [ -n "${SUDO_USER:-}" ]; then
  REAL_HOME=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6 || true)
  if [ -z "$REAL_HOME" ]; then
    echo "Could not resolve home directory for SUDO_USER=$SUDO_USER via getent" >&2
    exit 1
  fi
else
  REAL_HOME="$HOME"
fi

# Detect existing ig location — update in-place if found
if [ -z "${IG_INSTALL_DIR:-}" ]; then
  EXISTING_IG=$(command -v ig 2>/dev/null || true)
  if [ -n "$EXISTING_IG" ]; then
    # Resolve symlinks to get the real path
    EXISTING_IG=$(readlink -f "$EXISTING_IG" 2>/dev/null || realpath "$EXISTING_IG" 2>/dev/null || echo "$EXISTING_IG")
    INSTALL_DIR="$(dirname "$EXISTING_IG")"
  else
    INSTALL_DIR="$REAL_HOME/.local/bin"
  fi
else
  INSTALL_DIR="$IG_INSTALL_DIR"
fi

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      aarch64|arm64) ARTIFACT="ig-linux-aarch64" ;;
      x86_64)        ARTIFACT="ig-linux-x86_64" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
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

# Clean up stale binaries in other locations
for dir in "$REAL_HOME/.local/bin" "$REAL_HOME/.cargo/bin"; do
  other="$dir/ig"
  if [ -f "$other" ] && [ "$(readlink -f "$other" 2>/dev/null || realpath "$other" 2>/dev/null || echo "$other")" != "$(readlink -f "$INSTALL_DIR/ig" 2>/dev/null || realpath "$INSTALL_DIR/ig" 2>/dev/null || echo "$INSTALL_DIR/ig")" ]; then
    echo "  → Removed stale binary: $other"
    rm -f "$other"
  fi
done

# Check PATH
if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
  echo ""
  echo "Add to your shell config:"
  echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi

echo ""
echo "Ready! Try: ig \"hello\" ."

# Auto-configure AI CLI agents
echo ""
"$INSTALL_DIR/ig" setup
