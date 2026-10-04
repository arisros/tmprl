# Contributing

## Before proposing a feature

Read [docs/ROADMAP.md](docs/ROADMAP.md), including the *Not planned* table at the end. An item
gets on the roadmap when it passes two tests: it makes a workflow easier to understand,
navigate or operate from a terminal, and Temporal exposes the data it needs. A request that
fails the second test is recorded under *Not planned* with the reason, so check there before
opening an issue.

A bug needs no such check. Open an issue with the bug form.

## Prerequisites

| | |
|---|---|
| Rust 1.95 or newer | `rust-version` in `Cargo.toml`. CI builds and tests on stable, and checks 1.95 separately |
| the `temporal` CLI | Only for the integration tests. It carries the dev server they run against |

## What CI runs

CI sets `RUSTFLAGS=-D warnings`, so a compiler warning fails the build. Run these from the
repository root before pushing.

| Job | Command |
|---|---|
| check | `cargo fmt --all -- --check` |
| check | `scripts/check-docs.sh` |
| check | `cargo clippy --all-targets --all-features` |
| check | `cargo build --all-targets` |
| msrv | `cargo check --all-targets --all-features --locked`, on Rust 1.95 |
| macOS | `cargo test --all-features`, with no server |
| tests | `cargo llvm-cov --all-features --lcov --output-path lcov.info`, against a live dev server |

`cargo test --all-features` runs the same tests as the last row without the coverage report.

## Tests, with and without a server

Only the tests in `crates/tmprl-client/tests/live.rs` need a server. Everything else runs
with nothing but the toolchain.

```sh
cargo test                           # no server: the live tests print SKIP and pass
```

```sh
temporal server start-dev &
for i in 1 2 3; do
  temporal workflow start --task-queue ci --type CiWorkflow --workflow-id "ci-$i"
done
TMPRL_REQUIRE_SERVER=1 cargo test    # no server: the live tests fail
```

With no server reachable each live test skips, so `cargo test` passes on a machine that has
never run Temporal. Setting `TMPRL_REQUIRE_SERVER` turns that skip into a failure, which is
what CI does, so that a broken connection layer cannot pass as a green build. If you changed
`tmprl-client`, run it this way before pushing.

The seeding matters. A live test that finds too few workflows to assert on prints `SKIP` and
passes, and `TMPRL_REQUIRE_SERVER` does not change that. No worker is needed, the workflows
only have to exist.

The tests connect with the default profile, so `TEMPORAL_ADDRESS` points them at a server on
another port. CI runs them against two dev servers, the latest `temporal` CLI and an older one
pinned in `.github/workflows/ci.yml`.

## `scripts/check-docs.sh`

This is the check a first pull request usually fails. It enforces four things:

| Check | To satisfy it |
|---|---|
| Every `.rs` file under `crates/*/src/` starts with a `//!` line | Line 1 of a new file says what the file is |
| Every `crates/tmprl-core/src/*.rs` and `crates/tmprl-client/src/ops/*.rs` is named in `docs/ARCHITECTURE.md` | Add the file to the code map under [Where things live](docs/ARCHITECTURE.md#where-things-live) |
| Every `crates/tmprl-tui/src/app/*.rs` has a row in the table at the top of `crates/tmprl-tui/src/app/mod.rs` | Add the row |
| Each crate's row in the crate tables of `README.md` and `docs/ARCHITECTURE.md` carries its number of `#[test]` and `#[tokio::test]` attributes | Adding or removing a test changes the number in both tables |

The script prints the number it expects, for example
`check-docs: README.md does not say tmprl-core has 316 tests`.

## Design rules

[docs/ARCHITECTURE.md §10](docs/ARCHITECTURE.md#10-design-rules) has four rules, in priority
order, with the reasons: nothing blocks the input path, domain logic stays out of the render
path, new behaviour is a `Command`, and matches over protocol enums are exhaustive. Read the
section before writing code.

[Where things live](docs/ARCHITECTURE.md#where-things-live) in the same document lists every
file, and says how to follow one behaviour from its command id to the code that draws it.

## Public API

Per [docs/RELEASING.md](docs/RELEASING.md#which-bump), the public API is keys, command ids,
the keys of `config.toml`, `keys.toml`, `views.toml`, `theme.toml` and `dashboard.toml`, and
CLI flags. Rust types are not.

Adding one of those is a minor release. Removing or renaming one is a breaking change, so say
so in the pull request. [docs/INTERFACE.md](docs/INTERFACE.md) lists every binding and marks
it live or planned; keep it true when a binding changes.

## Commits

[Conventional Commits](https://www.conventionalcommits.org): `type(scope): subject`.

- The subject is imperative, lowercase and under 72 characters, with no trailing period.
- The scope is optional. The history uses the area touched: `feat(search)`, `fix(audit)`,
  `docs(readme)`.
- When `CHANGELOG.md` has nothing written for a release, the release falls back to the
  subjects of the `feat:`, `fix:` and `perf:` commits, so write those subjects for a user.

Maintainers cut releases; [docs/RELEASING.md](docs/RELEASING.md) has the process.

## Security

Do not report a vulnerability in a public issue. See [SECURITY.md](SECURITY.md).
