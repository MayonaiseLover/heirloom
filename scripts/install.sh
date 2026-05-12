#!/usr/bin/env bash
# Heirloom installer.
#
# Usage:
#   curl -sSL https://heirloom.web.app/install | sh
#
# Behavior:
#   - Detects OS + arch
#   - Downloads the matching release binary from GitHub
#   - Installs to ~/.local/bin/heirloom (overridable via HEIRLOOM_PREFIX)
#   - Prints next steps

set -euo pipefail

REPO="heirloom-dev/heirloom"
PREFIX="${HEIRLOOM_PREFIX:-$HOME/.local/bin}"

die() { printf "\033[1;31merror:\033[0m %s\n" "$*" >&2; exit 1; }
info() { printf "\033[1;36m::\033[0m %s\n" "$*"; }

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$os" in
    linux)
      case "$arch" in
        x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) die "unsupported linux arch: $arch" ;;
      esac
      ;;
    darwin)
      case "$arch" in
        x86_64) echo "x86_64-apple-darwin" ;;
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        *) die "unsupported macOS arch: $arch" ;;
      esac
      ;;
    *) die "unsupported OS: $os (try cargo install for Windows)" ;;
  esac
}

latest_tag() {
  curl -sSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
    | head -n1
}

main() {
  command -v curl >/dev/null || die "curl is required"
  command -v tar >/dev/null  || die "tar is required"

  local target tag url tmp archive
  target="$(detect_target)"
  tag="${HEIRLOOM_VERSION:-$(latest_tag)}"
  [ -n "$tag" ] || die "could not determine latest version — set HEIRLOOM_VERSION"

  info "installing heirloom $tag for $target"

  archive="heirloom-$tag-$target.tar.gz"
  url="https://github.com/$REPO/releases/download/$tag/$archive"
  tmp="$(mktemp -d)"
  trap "rm -rf $tmp" EXIT

  curl -sSfL -o "$tmp/$archive" "$url" \
    || die "download failed: $url"

  tar xzf "$tmp/$archive" -C "$tmp"
  mkdir -p "$PREFIX"
  install -m 0755 "$tmp/heirloom-$tag-$target/heirloom" "$PREFIX/heirloom"

  info "installed to $PREFIX/heirloom"
  case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *)
      printf "\033[1;33m!! \033[0m %s is not in your PATH. Add this to your shell profile:\n" "$PREFIX"
      printf "    export PATH=\"%s:\$PATH\"\n" "$PREFIX"
      ;;
  esac

  printf "\nNext: \033[1mheirloom init\033[0m\n"
}

main "$@"
