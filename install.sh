#!/usr/bin/env bash
set -euo pipefail

# instant-grep installer
# Layout (option B — single user-facing binary):
#   ~/.local/bin/ig                        ← C shim, in PATH (the only thing the user sees)
#   ~/.local/share/ig/bin/ig-rust          ← Rust backend, hidden (invoked by the shim)
#
# Sudo install:
#   /usr/local/bin/ig
#   /usr/local/share/ig/bin/ig-rust
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

# Determine install dirs ──────────────────────────────────────────────────────
# IG_INSTALL_DIR overrides the shim location; SHARE_DIR is derived from it.
if [ -n "${IG_INSTALL_DIR:-}" ]; then
  BIN_DIR="$IG_INSTALL_DIR"
elif [ -n "${SUDO_USER:-}" ] || [ "$(id -u)" = "0" ]; then
  BIN_DIR="/usr/local/bin"
else
  # Detect existing shim location for in-place upgrade
  EXISTING_IG=$(command -v ig 2>/dev/null || true)
  if [ -n "$EXISTING_IG" ]; then
    EXISTING_IG=$(readlink -f "$EXISTING_IG" 2>/dev/null || realpath "$EXISTING_IG" 2>/dev/null || echo "$EXISTING_IG")
    BIN_DIR="$(dirname "$EXISTING_IG")"
  else
    BIN_DIR="$REAL_HOME/.local/bin"
  fi
fi

# Derive SHARE_DIR from BIN_DIR
case "$BIN_DIR" in
  /usr/local/bin)              SHARE_DIR="/usr/local/share/ig/bin" ;;
  /opt/homebrew/bin)           SHARE_DIR="/opt/homebrew/share/ig/bin" ;;
  "$REAL_HOME/.local/bin")     SHARE_DIR="$REAL_HOME/.local/share/ig/bin" ;;
  "$REAL_HOME/.cargo/bin")     SHARE_DIR="$REAL_HOME/.local/share/ig/bin" ;;
  *)                           SHARE_DIR="$REAL_HOME/.local/share/ig/bin" ;;
esac

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

ARTIFACT_RUST="${ARTIFACT}-rust"

# Get latest release tag ──────────────────────────────────────────────────────
TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$TAG" ]; then
  echo "Failed to fetch latest release tag"
  exit 1
fi

URL_SHIM="https://github.com/$REPO/releases/download/$TAG/$ARTIFACT"
URL_RUST="https://github.com/$REPO/releases/download/$TAG/$ARTIFACT_RUST"

echo "Installing instant-grep $TAG ($ARTIFACT)..."
echo "  shim    → $BIN_DIR/ig"
echo "  backend → $SHARE_DIR/ig-rust"

mkdir -p "$BIN_DIR" "$SHARE_DIR"

# Download shim
if ! curl -fsSL "$URL_SHIM" -o "$BIN_DIR/ig"; then
  echo "✗ Failed to download $ARTIFACT" >&2
  exit 1
fi
chmod +x "$BIN_DIR/ig"

# Download Rust backend (if release ships it; legacy releases may not)
if curl -fsSL "$URL_RUST" -o "$SHARE_DIR/ig-rust" 2>/dev/null; then
  chmod +x "$SHARE_DIR/ig-rust"
else
  echo "  ⚠ $ARTIFACT_RUST not in release — backend missing, shim will refuse to run"
  echo "    This usually means you hit an old release; ask the maintainer to re-release."
  rm -f "$SHARE_DIR/ig-rust"
fi

# Stable codesign identifier on macOS — prevents TCC from re-prompting for
# file-access permissions after every `ig update`. Without this, the ad-hoc
# identifier embeds the binary hash (e.g. `ig-5555494468fc...`), so each
# rebuild looks like a brand-new app to the TCC database and BTM service.
# Bundle ID `dev.makfly.ig` is stable across releases; only the CDHash
# changes, and TCC keys off the identifier when the team is unset.
if [ "$OS" = "Darwin" ]; then
  for bin in "$BIN_DIR/ig" "$SHARE_DIR/ig-rust"; do
    [ -f "$bin" ] || continue
    codesign --force --sign - --identifier dev.makfly.ig "$bin" 2>/dev/null \
      || echo "  ⚠ codesign failed for $bin (TCC may re-prompt on next launch)"
  done
fi

# Migration: clean up legacy ig-rust placed next to the shim ──────────────────
for legacy in "$REAL_HOME/.local/bin/ig-rust" "$REAL_HOME/.cargo/bin/ig-rust" "/usr/local/bin/ig-rust"; do
  if [ -f "$legacy" ]; then
    echo "  → Removing legacy backend: $legacy"
    rm -f "$legacy" 2>/dev/null || true
  fi
done

# Migration: clean up stale ig binaries elsewhere in the user's PATH ──────────
for dir in "$REAL_HOME/.local/bin" "$REAL_HOME/.cargo/bin"; do
  other="$dir/ig"
  if [ -f "$other" ]; then
    other_canon=$(readlink -f "$other" 2>/dev/null || realpath "$other" 2>/dev/null || echo "$other")
    bin_canon=$(readlink -f "$BIN_DIR/ig" 2>/dev/null || realpath "$BIN_DIR/ig" 2>/dev/null || echo "$BIN_DIR/ig")
    if [ "$other_canon" != "$bin_canon" ]; then
      echo "  → Removed stale shim: $other"
      rm -f "$other"
    fi
  fi
done

# Verify ──────────────────────────────────────────────────────────────────────
if "$BIN_DIR/ig" --version >/dev/null 2>&1; then
  echo "✓ Installed: $("$BIN_DIR/ig" --version)"
elif [ -x "$SHARE_DIR/ig-rust" ] && "$SHARE_DIR/ig-rust" --version >/dev/null 2>&1; then
  echo "✓ Installed (backend only): $("$SHARE_DIR/ig-rust" --version)"
  echo "  Note: shim could not invoke backend; check \$IG_BACKEND or paths."
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
