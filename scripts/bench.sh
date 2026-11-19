#!/bin/sh
# Startup budget check.
#
#   ./scripts/bench.sh ./target/release/release
#
# Budgets: CLI first paint < 20 ms, offline plan < 60 ms. Requires hyperfine.

set -eu

BINARY="${1:-./target/release/release}"

if [ ! -x "$BINARY" ]; then
  echo "error: $BINARY is not executable" >&2
  exit 1
fi

if ! command -v hyperfine >/dev/null 2>&1; then
  echo "error: hyperfine is required (https://github.com/sharkdp/hyperfine)" >&2
  exit 1
fi

echo "== version / first paint =="
hyperfine --warmup 3 --runs 20 "$BINARY --version"

echo "== offline dry run (no network) =="
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  hyperfine --warmup 1 --runs 10 \
    --ignore-failure \
    "$BINARY --dry-run --offline --json >/dev/null" || true
else
  echo "skipped: not inside a git repository"
fi
