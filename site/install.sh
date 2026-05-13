#!/usr/bin/env sh
# Heirloom installer — https://heirloom-webb.web.app/install
# Usage:
#   curl -sSL https://heirloom-webb.web.app/install | sh
#
# Tries in order:
#   1. Pre-built binary from latest GitHub release
#   2. cargo install --git (requires Rust toolchain)
#   3. clear instructions to clone + build manually
set -e

REPO="MayonaiseLover/heirloom"
BIN="heirloom"
INSTALL_DIR="${HEIRLOOM_INSTALL_DIR:-/usr/local/bin}"

# ──────────────────────────────────────────────────────
have() { command -v "$1" >/dev/null 2>&1; }

fetch() {
  if have curl; then curl -sSL "$1"
  elif have wget; then wget -qO- "$1"
  else echo ""; fi
}

# ──────────────────────────────────────────────────────
# detect OS + arch
# ──────────────────────────────────────────────────────
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  linux)  TARGET_OS="linux" ;;
  darwin) TARGET_OS="macos" ;;
  *)
    echo "✗ unsupported OS: $OS"
    echo "  Build from source: https://github.com/$REPO#building-from-source"
    exit 1 ;;
esac

case "$ARCH" in
  x86_64 | amd64)  TARGET_ARCH="x86_64" ;;
  aarch64 | arm64) TARGET_ARCH="aarch64" ;;
  *)
    echo "✗ unsupported architecture: $ARCH"
    echo "  Build from source: https://github.com/$REPO#building-from-source"
    exit 1 ;;
esac

# ──────────────────────────────────────────────────────
# attempt 1: latest GitHub release
# ──────────────────────────────────────────────────────
echo "→ looking for a pre-built release for $TARGET_OS/$TARGET_ARCH ..."
RELEASE_JSON=$(fetch "https://api.github.com/repos/$REPO/releases/latest" || true)
LATEST=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
HAS_ASSETS=$(echo "$RELEASE_JSON" | grep -c '"browser_download_url"' || true)

case "$TARGET_OS" in
  macos) TRIPLE="${TARGET_ARCH}-apple-darwin" ;;
  linux) TRIPLE="${TARGET_ARCH}-unknown-linux-musl" ;;
esac

if [ -n "$LATEST" ] && [ "$HAS_ASSETS" -gt 0 ]; then
  ARCHIVE="${BIN}-${LATEST}-${TRIPLE}.tar.gz"
  URL="https://github.com/$REPO/releases/download/$LATEST/$ARCHIVE"

  echo "→ trying $URL ..."
  TMP=$(mktemp -d)
  trap "rm -rf $TMP" EXIT

  if have curl; then
    HTTP_CODE=$(curl -sSL -w "%{http_code}" -o "$TMP/$ARCHIVE" "$URL" || echo "000")
  else
    wget -qO "$TMP/$ARCHIVE" "$URL" && HTTP_CODE="200" || HTTP_CODE="404"
  fi

  if [ "$HTTP_CODE" = "200" ] && [ -s "$TMP/$ARCHIVE" ]; then
    echo "→ extracting ..."
    tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
    BIN_PATH=$(find "$TMP" -type f -name "$BIN" | head -1)
    if [ -n "$BIN_PATH" ]; then
      chmod +x "$BIN_PATH"
      if [ -w "$INSTALL_DIR" ]; then
        mv "$BIN_PATH" "$INSTALL_DIR/$BIN"
      else
        echo "→ need sudo to write to $INSTALL_DIR ..."
        sudo mv "$BIN_PATH" "$INSTALL_DIR/$BIN"
      fi
      TEAM_BIN=$(find "$TMP" -type f -name "heirloom-team-server" | head -1)
      if [ -n "$TEAM_BIN" ]; then
        chmod +x "$TEAM_BIN"
        if [ -w "$INSTALL_DIR" ]; then mv "$TEAM_BIN" "$INSTALL_DIR/heirloom-team-server"
        else sudo mv "$TEAM_BIN" "$INSTALL_DIR/heirloom-team-server"; fi
      fi
      echo "✓ installed $($INSTALL_DIR/$BIN --version 2>/dev/null || echo "$BIN $LATEST")"
      SUCCESS=1
    fi
  fi
fi

# ──────────────────────────────────────────────────────
# attempt 2: cargo install --git (build from source)
# ──────────────────────────────────────────────────────
if [ -z "${SUCCESS:-}" ]; then
  echo ""
  echo "  No pre-built binary available yet for $TRIPLE."
  echo ""
  if have cargo; then
    echo "→ found Rust toolchain, building from source via cargo ..."
    echo "  (this takes 2-3 minutes the first time)"
    echo ""
    cargo install --git "https://github.com/$REPO" --bin heirloom heirloom-cli
    cargo install --git "https://github.com/$REPO" --bin heirloom-team-server heirloom-team || true
    SUCCESS=1
  else
    cat <<EOF
  Rust toolchain not found. You have two options:

  Option 1 — Install Rust, then re-run this script:
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
      source "\$HOME/.cargo/env"
      curl -sSL https://heirloom-webb.web.app/install | sh

  Option 2 — Clone and build manually:
      git clone https://github.com/$REPO
      cd heirloom
      cargo install --path crates/heirloom-cli
      cargo install --path crates/heirloom-team

EOF
    exit 1
  fi
fi

# ──────────────────────────────────────────────────────
# greet
# ──────────────────────────────────────────────────────
cat <<EOF

  Get started:
    heirloom init
    heirloom ingest fs --path ~/Documents/notes
    heirloom search "anything you remember"

  Connect to Claude / Cursor / Antigravity:
    https://github.com/$REPO/blob/main/docs/INTEGRATIONS.md

  Run as MCP server:
    heirloom serve

EOF
