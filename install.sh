#!/usr/bin/env bash
set -euo pipefail

# instant-grep installer
# Layout (v1.20+, single-binary):
#   ~/.local/bin/ig       ← the only binary
#
# Sudo install:
#   /usr/local/bin/ig
#
# Pre-v1.20 installs used a C shim at ~/.local/bin/ig that invoked
# ~/.local/share/ig/bin/ig-rust. This script migrates those automatically
# by removing the stale backend dir + any stray ig-rust binaries.
#
# Usage: curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash

REPO="MakFly/instant-grep"

# When running under sudo, resolve the real user's home directory via getent.
if [ -n "${SUDO_USER:-}" ]; then
  REAL_HOME=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6 || true)
  if [ -z "$REAL_HOME" ]; then
    echo "Could not resolve home directory for SUDO_USER=$SUDO_USER via getent" >&2
    exit 1
  fi
else
  REAL_HOME="$HOME"
fi

# Determine install dir ───────────────────────────────────────────────────────
if [ -n "${IG_INSTALL_DIR:-}" ]; then
  BIN_DIR="$IG_INSTALL_DIR"
elif [ -n "${SUDO_USER:-}" ] || [ "$(id -u)" = "0" ]; then
  BIN_DIR="/usr/local/bin"
else
  # Detect existing binary location for in-place upgrade
  EXISTING_IG=$(command -v ig 2>/dev/null || true)
  if [ -n "$EXISTING_IG" ]; then
    EXISTING_IG=$(readlink -f "$EXISTING_IG" 2>/dev/null || realpath "$EXISTING_IG" 2>/dev/null || echo "$EXISTING_IG")
    BIN_DIR="$(dirname "$EXISTING_IG")"
  else
    BIN_DIR="$REAL_HOME/.local/bin"
  fi
fi

# Detect platform ─────────────────────────────────────────────────────────────
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

# Get latest release tag ──────────────────────────────────────────────────────
TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$TAG" ]; then
  echo "Failed to fetch latest release tag"
  exit 1
fi

URL="https://github.com/$REPO/releases/download/$TAG/$ARTIFACT"

echo "Installing instant-grep $TAG ($ARTIFACT)..."
echo "  → $BIN_DIR/ig"

mkdir -p "$BIN_DIR"

# Download the single binary
if ! curl -fsSL "$URL" -o "$BIN_DIR/ig"; then
  echo "✗ Failed to download $ARTIFACT" >&2
  exit 1
fi
chmod +x "$BIN_DIR/ig"

# Stable codesign identifier on macOS — prevents TCC from re-prompting for
# file-access permissions after every `ig update`. Without `-i`, the ad-hoc
# identifier embeds the binary hash, so each rebuild looks like a brand-new
# app. Bundle ID `dev.makfly.ig` is stable across releases.
if [ "$OS" = "Darwin" ]; then
  codesign --force --sign - --identifier dev.makfly.ig "$BIN_DIR/ig" 2>/dev/null \
    || echo "  ⚠ codesign failed (TCC may re-prompt on next launch)"
fi

# Migration from pre-v1.20 shim+backend layout ────────────────────────────────
# Remove the legacy ig-rust backend wherever it might have been installed,
# and the now-empty share dir. This keeps the user's PATH clean and stops
# `ig` from accidentally invoking a stale backend (its absence is benign —
# v1.20+ binary is self-contained).
for legacy_rust in \
  "$REAL_HOME/.local/share/ig/bin/ig-rust" \
  "$REAL_HOME/.local/bin/ig-rust" \
  "$REAL_HOME/.cargo/bin/ig-rust" \
  "/usr/local/share/ig/bin/ig-rust" \
  "/usr/local/bin/ig-rust" \
  "/opt/homebrew/share/ig/bin/ig-rust"; do
  if [ -f "$legacy_rust" ]; then
    echo "  → Removing legacy backend: $legacy_rust"
    rm -f "$legacy_rust" 2>/dev/null || true
  fi
done
# Tidy now-empty share dirs (best-effort)
for legacy_share in \
  "$REAL_HOME/.local/share/ig/bin" \
  "$REAL_HOME/.local/share/ig" \
  "/usr/local/share/ig/bin" \
  "/usr/local/share/ig" \
  "/opt/homebrew/share/ig/bin" \
  "/opt/homebrew/share/ig"; do
  [ -d "$legacy_share" ] && rmdir "$legacy_share" 2>/dev/null || true
done

# Migration: clean up stale ig binaries elsewhere in the user's PATH ──────────
for dir in "$REAL_HOME/.local/bin" "$REAL_HOME/.cargo/bin"; do
  other="$dir/ig"
  if [ -f "$other" ]; then
    other_canon=$(readlink -f "$other" 2>/dev/null || realpath "$other" 2>/dev/null || echo "$other")
    bin_canon=$(readlink -f "$BIN_DIR/ig" 2>/dev/null || realpath "$BIN_DIR/ig" 2>/dev/null || echo "$BIN_DIR/ig")
    if [ "$other_canon" != "$bin_canon" ]; then
      echo "  → Removed stale ig: $other"
      rm -f "$other"
    fi
  fi
done

# Verify ──────────────────────────────────────────────────────────────────────
if "$BIN_DIR/ig" --version >/dev/null 2>&1; then
  echo "✓ Installed: $("$BIN_DIR/ig" --version)"
else
  echo "✗ Installation failed" >&2
  exit 1
fi

# PATH check ──────────────────────────────────────────────────────────────────
if ! echo "$PATH" | grep -q "$BIN_DIR"; then
  echo ""
  echo "Add to your shell config:"
  echo "  export PATH=\"$BIN_DIR:\$PATH\""
fi

echo ""
echo "Ready! Try: ig \"hello\" ."

# Auto-configure AI CLI agents + shell hook
echo ""
"$BIN_DIR/ig" setup || true

# Auto-install the global daemon (launchd/systemd-user) so it survives reboots
# and auto-restarts after every `ig update`. Opt-out with IG_NO_DAEMON_INSTALL=1.
if [ "${IG_NO_DAEMON_INSTALL:-0}" != "1" ]; then
  echo ""
  echo "Installing global daemon (auto-start on login)..."
  if ! "$BIN_DIR/ig" daemon install; then
    echo "  ⚠ daemon install failed. Run 'ig daemon install' manually later."
  fi
fi
