#!/bin/sh
# Installs the `release` binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/404mat/release/main/scripts/install.sh | sh
#
# Supported: macOS (arm64, x86_64) and Linux (x86_64, aarch64).
# Override the destination with RELEASE_INSTALL_DIR.

set -eu

REPOSITORY="404mat/release"
INSTALL_DIR="${RELEASE_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
  echo "error: $*" >&2
  exit 1
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin)
      case "$arch" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) fail "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64) echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) fail "unsupported Linux architecture: $arch" ;;
      esac
      ;;
    *)
      fail "unsupported operating system: $os"
      ;;
  esac
}

download() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$1" -O "$2"
  else
    fail "curl or wget is required"
  fi
}

target="$(detect_target)"
tag="${RELEASE_VERSION:-}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ -z "$tag" ]; then
  latest="$(curl -fsSL "https://api.github.com/repos/${REPOSITORY}/releases/latest" 2>/dev/null || true)"
  tag="$(printf '%s' "$latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"

  [ -n "$tag" ] || fail "could not determine the latest release; set RELEASE_VERSION"
fi

archive="release-${tag}-${target}.tar.gz"
url="https://github.com/${REPOSITORY}/releases/download/${tag}/${archive}"

echo "Downloading ${url}"
download "$url" "$tmp/$archive"

tar -xzf "$tmp/$archive" -C "$tmp"

mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/release-${tag}-${target}/release" "$INSTALL_DIR/release"

echo "Installed release to ${INSTALL_DIR}/release"

case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *) echo "Add ${INSTALL_DIR} to your PATH to run \`release\`." ;;
esac
