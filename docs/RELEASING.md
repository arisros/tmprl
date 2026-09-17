# Releasing

Two buttons in the **Actions** tab, no local git needed.

## 1. Prepare release

**Actions → Prepare release → Run workflow.**

| Input | |
|---|---|
| `bump` | `patch`, `minor` or `major` |
| `channel` | `release`, or `rc` for a candidate (`0.2.0-rc.1`) |

It bumps `workspace.package.version` and the three `tmprl-*` versions beside it, refreshes
`Cargo.lock`, turns the changelog's `## Unreleased` section into `## X.Y.Z — date`, and opens
a PR. With no Unreleased section it writes one from the `feat:` / `fix:` / `perf:` / `docs:`
commits since the last tag, so a release always says what changed.

Review the PR, edit the changelog if the generated wording is thin, and merge it.

Locally, the same thing: `scripts/prepare-release.sh patch`, `… minor rc`, `… 0.4.0`.

## 2. Release

**Actions → Release → Run workflow**, with the tag from that PR, e.g. `v0.1.1`.

The run creates the tag, then:

1. builds `tmprl` for Linux and macOS, x86_64 and aarch64
2. creates the GitHub Release with the tarballs and `tmprl-installer.sh`
3. pushes `Formula/tmprl.rb` to [`arisros/homebrew-tap`](https://github.com/arisros/homebrew-tap)
4. runs `publish-crates.yml`, which publishes the four crates to crates.io

A tag with a suffix (`v0.2.0-rc.1`) makes a GitHub prerelease and skips Homebrew and
crates.io, so candidates can be tested without publishing.

Leaving the tag as `dry-run` builds everything and publishes nothing.

Note: pushing a tag by hand no longer releases anything. `dispatch-releases` in
`dist-workspace.toml` moved the trigger to this button, so the tag and the release are made
by the same run and cannot disagree.

## Which bump

While the version starts with `0.`, the middle number is the breaking one.

| Change | Bump |
|---|---|
| Bug fix, docs, internals | patch |
| New feature, new config key or binding | minor |
| A key, command, config key or flag removed or renamed | minor while 0.x, major after 1.0 |

Keys, commands, `config.toml` / `keys.toml` / `views.toml` keys and CLI flags are the public
API here; Rust types are not. All four crates share one version, so any release moves them all.

## Before a release

```sh
cargo publish --workspace --dry-run   # packaging and metadata
dist plan                             # what the release will build
```

After changing `[dist]` in `dist-workspace.toml`, run `dist generate` so `release.yml`
matches it; CI's plan job fails when they differ.

## Secrets

| Secret | Used for |
|---|---|
| `HOMEBREW_TAP_TOKEN` | fine-grained PAT, Contents: write on `arisros/homebrew-tap` |
| `CARGO_REGISTRY_TOKEN` | crates.io token, publish-new and publish-update, pattern `tmprl*` |

Both expire. A release that fails at the publish step with `Bad credentials` or a 401 needs
new tokens, then **Re-run failed jobs** on that run.
