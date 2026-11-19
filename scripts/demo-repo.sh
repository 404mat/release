#!/bin/sh
# Builds a disposable repository with realistic history, for exercising the TUI
# and the changelog renderer against something bigger than three commits.
#
#   ./scripts/demo-repo.sh [path]
#
# Defaults to $TMPDIR/release-demo. The repository has a local bare remote, so
# the branch is "synced" and pushes work, but no GitHub remote, so the TUI will
# report that GitHub was not used.
#
# Fully deterministic: same commits, same dates, same messages every run.

set -eu

TMP_ROOT=${TMPDIR:-/tmp}
TMP_ROOT=${TMP_ROOT%/}

DEMO=${1:-$TMP_ROOT/release-demo}
DEMO=${DEMO%/}
REMOTE="$DEMO-remote.git"

rm -rf "$DEMO" "$REMOTE"

git init --bare -q "$REMOTE"
mkdir -p "$DEMO"
cd "$DEMO"
git init -q --initial-branch=main
git config user.name "Demo Maintainer"
git config user.email "demo@example.com"
git config commit.gpgsign false
git remote add origin "$REMOTE"

TICK=0

# Every commit gets a later date, so the log looks like months of work.
commit() {
  TICK=$((TICK + 1))
  stamp=$(date -j -u -v+"${TICK}"d -f "%Y-%m-%d %H:%M:%S" \
    "2025-01-06 09:00:00" "+%Y-%m-%dT%H:%M:%S +0000")
  GIT_AUTHOR_DATE="$stamp" GIT_COMMITTER_DATE="$stamp" git commit -q "$@"
}

write() {
  mkdir -p "$(dirname "$1")"
  printf '%s\n' "$2" > "$1"
}

# path|content|message. A path of "-" means "message only".
# Every message is also appended to docs/changelog.md, which guarantees that each
# commit has a real diff and gives the history a changelog of its own.
history() {
  while IFS='|' read -r file body message; do
    [ -n "${message:-}" ] || continue

    printf -- "- %s\n" "$message" >>docs/changelog.md

    if [ "$file" != "-" ]; then
      write "$file" "$body"
    fi

    git add -A
    commit -m "$message"
  done
}

cat > package.json <<'JSON'
{
  "name": "orbit",
  "version": "1.2.1",
  "description": "A demo project with real history",
  "license": "MIT",
  "scripts": {
    "test": "node --test"
  }
}
JSON

cat > package-lock.json <<'JSON'
{
  "name": "orbit",
  "version": "1.2.1",
  "lockfileVersion": 3
}
JSON

cat > .gitignore <<'EOF'
node_modules
dist
EOF

mkdir -p docs src test

cat > docs/changelog.md <<'EOF'
# Changelog
EOF

cat > docs/readme.md <<'EOF'
# Orbit

A small orbital mechanics library.
EOF

git add -A
commit -m "Initial commit"

# --- v0.1.0: the first usable version ---------------------------------------
history <<'EOF'
src/parse.cjs|module.exports = { parse };|Add the parser skeleton
-||Add tests for the parser (patch)
src/format.cjs|module.exports = { format };|Add output formatting (patch)
-||Fix a crash on empty input (patch)
-||Document the parser API
EOF

git tag v0.1.0

# --- v0.2.0: a backwards compatible feature release -------------------------
history <<'EOF'
src/units.cjs|module.exports = { units };|Add unit conversion helpers (minor)
-||Support au and light seconds
-||Fix rounding in distance output (#12)
-||Refactor the parse loop for clarity
-||Handle trailing whitespace (#15)
docs/api.md|# API|Add API documentation
-||Update the readme examples (patch)
EOF

git tag v0.2.0

# --- v1.0.0: a merged feature branch ---------------------------------------
git switch -q -c feature/search

history <<'EOF'
src/search.cjs|module.exports = { search };|Add a search index
-||Match on partial names
-||Bound the search results
docs/search.md|# Search|Document search usage
-||Fix an off by one in the index (patch)
EOF

git switch -q main
TICK=$((TICK + 1))
stamp=$(date -j -u -v+"${TICK}"d -f "%Y-%m-%d %H:%M:%S" \
  "2025-01-06 09:00:00" "+%Y-%m-%dT%H:%M:%S +0000")
GIT_AUTHOR_DATE="$stamp" GIT_COMMITTER_DATE="$stamp" \
  git merge -q --no-ff feature/search -m "Merge branch 'feature/search'"

git tag -a v1.0.0 -m "### Minor Changes

- Add a search index: #21
- Match on partial names: #22

### Patches

- Fix an off by one in the index: #24"

# --- v1.1.0 -----------------------------------------------------------------
history <<'EOF'
src/cache.cjs|module.exports = { cache };|Add an in memory cache (minor)
-||Evict entries after 60 seconds
-||Cache the parsed bodies
-||Fix a memory leak in the cache (patch)
-||Add benchmarks
-||Speed up formatting by 30 percent (patch)
-||Handle NaN coordinates (#31)
-||Validate inputs before parsing (patch)
docs/cache.md|# Cache|Document the cache
-||Tidy the test helpers (ignore)
EOF

git tag v1.1.0

# --- v1.1.1 -----------------------------------------------------------------
history <<'EOF'
-||Fix a regression in unit conversion (patch)
-||Guard against empty search terms (patch)
-||Release 1.1.1
EOF

git tag v1.1.1

# A commit whose title is just a version: the tool filters these out.
write src/version.txt "1.1.1"
git add -A
commit -m "1.1.1"

# --- v1.2.0: a body-only commit and PR references ---------------------------
history <<'EOF'
src/stream.cjs|module.exports = { stream };|Add streaming input (minor)
-||Keep order when streaming
-||Fix a deadlock on large inputs (#42)
-||Propagate errors from the reader
-||Add stream tests
docs/stream.md|# Streaming|Document streaming (patch)
-||Remove an unused import
-||Bump the minimum runtime version (major)
EOF

# A commit with no subject at all: the tool falls back to the body.
write src/retry.cjs "module.exports = { retry };"
git add -A
printf '\nImplement retry logic\n\n* retry on 500\n* retry on timeouts\n' >"$DEMO/.git/RETRY_MSG"
TICK=$((TICK + 1))
stamp=$(date -j -u -v+"${TICK}"d -f "%Y-%m-%d %H:%M:%S" \
  "2025-01-06 09:00:00" "+%Y-%m-%dT%H:%M:%S +0000")
GIT_AUTHOR_DATE="$stamp" GIT_COMMITTER_DATE="$stamp" \
  git commit -q --cleanup=verbatim -F "$DEMO/.git/RETRY_MSG"
rm -f "$DEMO/.git/RETRY_MSG"

git tag v1.2.0

# --- v1.2.1 -----------------------------------------------------------------
history <<'EOF'
-||Fix a rounding edge case (patch)
-||Handle unicode names (#51)
-||Make the error messages useful (patch)
-||Add a regression test for unicode
-||Document the release process
EOF

git tag v1.2.1

# --- unreleased work: what the TUI will classify ----------------------------
history <<'EOF'
src/offline.cjs|module.exports = { offline };|Add offline mode (minor)
-||Keep working when the index is stale
-||Speed up parsing by two times (patch)
-||Drop the legacy entry point (major)
-||Update dependencies (#128)
-||Fix flaky stream test (#131)
-||Add a json output flag (minor)
-||Remove dead code (ignore)
-||Improve error messages (#140)
-||Cover the retry path with tests
-||Extract the retry policy
docs/offline.md|# Offline|Document offline mode (patch)
EOF

# Two branches that were never merged, so the branch picker has something to show.
git switch -q -c next
write src/next.cjs "module.exports = { next };"
git add -A
commit -m "Start the next major line"

write src/next.cjs "module.exports = { next, more };"
git add -A
commit -m "Sketch the new API"

git switch -q -c experiment/ui
write src/ui.cjs "module.exports = { ui };"
git add -A
commit -m "Prototype a status line"

git switch -q main

git push -q -u origin main --tags
git push -q origin next experiment/ui

echo "repo:      $DEMO"
echo "remote:    $REMOTE"
echo "commits:   $(git rev-list --count HEAD)"
echo "tags:      $(git tag | tr '\n' ' ')"
echo "branches:  $(git branch --format='%(refname:short)' | tr '\n' ' ')"
echo
echo "Next:"
echo "  cd $DEMO"
echo "  released              # TUI"
echo "  released patch --dry-run --offline"
