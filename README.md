# Release

Release turns the commits since your last release into a [GitHub Release](https://help.github.com/articles/creating-releases/), or into an annotated Git tag. It groups changes by type, writes the changelog, bumps `package.json`, commits, tags, pushes, and creates the release — or, for a tag-only release, stops after the tag. It ships as a single binary for macOS and Linux, and needs only `git` and an authenticated [`gh`](https://cli.github.com), which it reads its token from: credentials are never stored.

```bash
curl -fsSL https://raw.githubusercontent.com/404mat/release/main/scripts/install.sh | sh
```

Run `release` inside a repository for the interactive terminal UI, where you classify commits, watch the notes build up, and review every side effect before confirming. For scripts and agents, name the bump instead: `release patch`, `release minor`, `release major`, or `release pre beta` for a prerelease. Leaving the bump out releases the tag that already exists, `--tag-only` writes an annotated tag instead of a release, `--dry-run` prints what would happen without touching anything, and `--json` returns a single machine-readable document on stdout. `release help` lists every option.

Each run fetches first, so the release is always computed from fresh refs. The prefix of the latest tag is preserved (`v1.2.3` → `v1.2.4`), and a bump refuses to start when `package.json` and that tag disagree, so a version can never be skipped silently. When the network or a push fails, the exact git or HTTP error is reported rather than swallowed, and the work already done is left in a state you can inspect.

Behaviour is configured in an optional `release.toml` — remote, release branch, prerelease suffix, editor — or through a hook executable, which receives the generated notes and metadata as JSON on stdin and returns the changelog to publish.
