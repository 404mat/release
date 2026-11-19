#!/bin/sh
# Installs a `released` command that always runs the local build.
#
# The command is a symlink into the cargo target directory, so a plain
# `cargo build -p release` (or `cargo build --release -p release`) immediately
# changes what `released` runs. Nothing is copied, so there is no reinstall
# step and no chance of running a stale binary.
#
# The real `release` command is intentionally untouched: it is only ever
# installed the official way (scripts/install.sh, Homebrew, a release binary).
#
#   ./scripts/install-dev.sh            # debug build (fastest compile loop)
#   ./scripts/install-dev.sh --release  # release build (production performance)
#   ./scripts/install-dev.sh --no-build # only (re)create the symlink
#   ./scripts/install-dev.sh --uninstall
#
# Environment:
#   RELEASE_DEV_BIN_DIR   where to put the symlink (default: ~/.local/bin)
#   CARGO_TARGET_DIR      respected when set

set -eu

PROFILE=debug
BUILD=1
UNINSTALL=0

usage() {
  name=$(basename "$0")
  cat <<EOF
Usage: $name [--debug | --release] [--no-build] [--uninstall]

  --debug        symlink the debug build (default)
  --release      symlink the release build
  --no-build     do not run cargo, only create the symlink
  --uninstall    remove the released symlink
  -h, --help     show this help
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --debug)
      PROFILE=debug
      ;;
    --release)
      PROFILE=release
      ;;
    --no-build)
      BUILD=0
      ;;
    --uninstall)
      UNINSTALL=1
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO_ROOT/target}
BIN_DIR=${RELEASE_DEV_BIN_DIR:-$HOME/.local/bin}
LINK="$BIN_DIR/released"
# Left behind by earlier versions of this script; cleaned up on install/uninstall.
LEGACY_LINK="$BIN_DIR/release-dev"
BINARY="$TARGET_DIR/$PROFILE/release"

if [ "$UNINSTALL" -eq 1 ]; then
  if [ -L "$LINK" ]; then
    rm -f "$LINK"
    echo "Removed $LINK"
  else
    echo "No released symlink at $LINK."
  fi

  if [ -L "$LEGACY_LINK" ]; then
    rm -f "$LEGACY_LINK"
    echo "Removed legacy $LEGACY_LINK"
  fi

  exit 0
fi

if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
  echo "error: $LINK exists and is not a symlink; refusing to overwrite it." >&2
  echo "Remove it yourself if that is really what you want." >&2
  exit 1
fi

cd "$REPO_ROOT"

if [ "$BUILD" -eq 1 ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is not installed" >&2
    exit 1
  fi

  echo "Building $PROFILE binary..."

  if [ "$PROFILE" = release ]; then
    cargo build --release -p release
  else
    cargo build -p release
  fi
fi

if [ ! -x "$BINARY" ]; then
  echo "error: $BINARY does not exist. Run without --no-build." >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
ln -sfn "$BINARY" "$LINK"

if [ -L "$LEGACY_LINK" ] && [ "$LEGACY_LINK" != "$LINK" ]; then
  rm -f "$LEGACY_LINK"
  echo "Removed legacy $LEGACY_LINK"
fi

echo "Linked $LINK -> $BINARY"

if [ "$PROFILE" = release ]; then
  REBUILD="cargo build --release -p release"
else
  REBUILD="cargo build -p release"
fi

echo "Live reload: run '$REBUILD' and released picks up the new binary."
echo "The official 'release' command is left alone."

case ":$PATH:" in
  *":$BIN_DIR:"*)
    echo "Try: released --version"
    ;;
  *)
    echo "warning: $BIN_DIR is not on your PATH; add it to use released from anywhere:"
    echo "  export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac
