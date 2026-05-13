#!/usr/bin/env sh
# Heirloom installer — https://heirloom.web.app/install
# Usage: curl -sSL https://heirloom.web.app/install | sh
set -e

REPO="MayonaiseLover/heirloom"
BIN="heirloom"
INSTALL_DIR="${HEIRLOOM_INSTALL_DIR:-/usr/local/bin}"

# ──────────────────────────────────────────────────────
# detect OS + arch
# ──────────────────────────────────────────────────────
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  linux)  TARGET_OS="linux" ;;
  darwin) TARGET_OS="macos" ;;
  *)
    echo "error: unsupported OS: $OS"
    echo "Download manually from: https://github.com/$REPO/releases/latest"
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64 | amd64) TARGET_ARCH="x86_64" ;;
  aarch64 | arm64) TARGET_ARCH="aarch64" ;;
  *)
    echo "error: unsupported architecture: $ARCH"
    echo "Download manually from: https://github.com/$REPO/releases/latest"
    exit 1
    ;;
esac

# ──────────────────────────────────────────────────────
# resolve latest version tag from GitHub API
# ──────────────────────────────────────────────────────
echo "→ fetching latest Heirloom release..."
if command -v curl >/dev/null 2>&1; then
  LATEST=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
elif command -v wget >/dev/null 2>&1; then
  LATEST=$(wget -qO- "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
else
  echo "error: need curl or wget"
  exit 1
fi

if [ -z "$LATEST" ]; then
  echo "error: could not determine latest version"
  echo "Download manually from: https://github.com/$REPO/releases/latest"
  exit 1
fi

echo "→ latest version: $LATEST"

# ──────────────────────────────────────────────────────
# assemble download URL
# cargo-dist produces:  heirloom-v1.0.1-aarch64-apple-darwin.tar.gz
# ──────────────────────────────────────────────────────
case "$TARGET_OS" in
  macos) TRIPLE="${TARGET_ARCH}-apple-darwin" ;;
  linux) TRIPLE="${TARGET_ARCH}-unknown-linux-musl" ;;
esac

ARCHIVE="${BIN}-${LATEST}-${TRIPLE}.tar.gz"
URL="https://github.com/$REPO/releases/download/$LATEST/$ARCHIVE"

# ──────────────────────────────────────────────────────
# download and install
# ──────────────────────────────────────────────────────
TMP=$(mktemp -d)
trap "rm -rf $TMP" EXIT

echo "→ downloading $ARCHIVE ..."
if command -v curl >/dev/null 2>&1; then
  curl -sSL "$URL" -o "$TMP/$ARCHIVE"
else
  wget -qO "$TMP/$ARCHIVE" "$URL"
fi

echo "→ extracting..."
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"

# find the binary (may be nested in a dir)
BIN_PATH=$(find "$TMP" -type f -name "$BIN" | head -1)
if [ -z "$BIN_PATH" ]; then
  echo "error: binary not found in archive"
  exit 1
fi
chmod +x "$BIN_PATH"

# ──────────────────────────────────────────────────────
# place binary
# ──────────────────────────────────────────────────────
if [ -w "$INSTALL_DIR" ]; then
  mv "$BIN_PATH" "$INSTALL_DIR/$BIN"
else
  echo "→ need sudo to write to $INSTALL_DIR..."
  sudo mv "$BIN_PATH" "$INSTALL_DIR/$BIN"
fi

# ──────────────────────────────────────────────────────
# also install heirloom-team-server if present in archive
# ──────────────────────────────────────────────────────
TEAM_BIN=$(find "$TMP" -type f -name "heirloom-team-server" | head -1)
if [ -n "$TEAM_BIN" ]; then
  chmod +x "$TEAM_BIN"
  if [ -w "$INSTALL_DIR" ]; then
    mv "$TEAM_BIN" "$INSTALL_DIR/heirloom-team-server"
  else
    sudo mv "$TEAM_BIN" "$INSTALL_DIR/heirloom-team-server"
  fi
  echo "→ installed heirloom-team-server → $INSTALL_DIR/heirloom-team-server"
fi

# ──────────────────────────────────────────────────────
# verify + greet
# ──────────────────────────────────────────────────────
if ! command -v "$BIN" >/dev/null 2>&1; then
  echo ""
  echo "  installed → $INSTALL_DIR/$BIN"
  echo "  (add $INSTALL_DIR to your PATH if not already there)"
else
  echo "→ installed $($BIN --version)"
fi

echo ""
echo "  Get started:"
echo "    heirloom init"
echo "    heirloom ingest fs --path ~/Documents/notes"
echo "    heirloom search \"anything you remember\""
echo ""
echo "  Docs: https://github.com/$REPO#readme"
echo "  MCP:  heirloom serve"
echo ""
