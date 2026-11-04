# Releasing

Two buttons in the **Actions** tab, no local git needed.

## 1. Prepare release

**Actions → Prepare release → Run workflow.**

| Input | |
|---|---|
| `bump` | `patch`, `minor` or `major` |
| `channel` | `release`, or a prerelease: `alpha`, `beta` or `rc` (`1.0.0-alpha.1`, `0.2.0-rc.1`) |

It bumps `workspace.package.version` and the three `tmprl-*` versions beside it, refreshes
`Cargo.lock`, updates the changelog, and opens a PR.

Review the PR, edit the changelog if the wording is thin, and merge it.

Locally, the same thing: `scripts/prepare-release.sh patch`, `… minor rc`, `… major alpha`, `… 0.4.0`.

### The changelog, and candidates

A candidate does **not** touch `CHANGELOG.md`. What is coming stays under `## Unreleased`
until the real version ships, because a section per candidate buries the release it was
rehearsing for, and leaves that release with an entry its candidates already emptied.

A real release promotes `## Unreleased` to `## X.Y.Z — date`, folding in any section an
earlier candidate for that same version did claim. With nothing written down it falls back to
the `feat:` / `fix:` / `perf:` commits since the last *release* (candidates are skipped), so
a release always says what changed. `scripts/changelog.py tidy` folds stray candidate
sections back; both it and `promote` only move bullets, never reword them.

### Releasing after a candidate

`patch` on top of `0.1.2-rc.4` gives **`0.1.2`**, the release the candidates were for, not
`0.1.3`. So: Prepare release with `patch` + `release`, merge, then Release with `v0.1.2`.
Pick `minor` or `major` only when the change deserves it — the candidates do not decide.

### The road to 1.0

| Step | Prepare release with | Gives |
|---|---|---|
| first alpha | `major` + `alpha` | `1.0.0-alpha.1` |
| another alpha | `patch` + `alpha` | `1.0.0-alpha.2` |
| the beta, API frozen | `patch` + `beta` | `1.0.0-beta.1` |
| a candidate | `patch` + `rc` | `1.0.0-rc.1` |
| 1.0 | `patch` + `release` | `1.0.0` |

An alpha may still change a key, a command or a flag, and says so in the changelog. From the
first beta, `docs/public-api.txt` does not change until 2.0. Every prerelease is a GitHub
prerelease only, as below.

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

Keys, commands, `config.toml` / `keys.toml` / `views.toml` / `theme.toml` / `dashboard.toml`
keys and CLI flags are the public
API here; Rust types are not. All four crates share one version, so any release moves them all.

`docs/public-api.txt` lists every command id, default binding and flag, and a test fails when
the code no longer matches it. A change to it is a change to the API: rerun with
`TMPRL_UPDATE_API=1 cargo test -p tmprl public_api`, say so in the changelog, and bump as the
table above says. Config keys are not in it, because they are parsed by hand; they are
promised by `docs/INTERFACE.md`.

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
