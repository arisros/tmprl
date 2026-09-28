#!/usr/bin/env bash
#
# Bump the workspace version and write the changelog section for it.
#
# Called by .github/workflows/prepare-release.yml, and runnable by hand:
#
#   scripts/prepare-release.sh patch          # 0.1.0 -> 0.1.1
#   scripts/prepare-release.sh minor rc       # 0.1.0 -> 0.2.0-rc.1
#   scripts/prepare-release.sh 0.4.0          # an exact version
#
# Prints the new version on stdout. Everything else goes to stderr, so the workflow can
# read the version straight out of it.
set -euo pipefail

bump="${1:?usage: prepare-release.sh <patch|minor|major|X.Y.Z> [rc]}"
channel="${2:-release}"

cd "$(dirname "$0")/.."

current=$(grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
base=${current%%-*}
IFS=. read -r major minor patch <<<"$base"

case "$bump" in
  major) next="$((major + 1)).0.0" ;;
  minor) next="$major.$((minor + 1)).0" ;;
  # A patch on top of 0.1.1-rc.2 is 0.1.1: the release the candidates were for.
  patch) if [ "$current" = "$base" ]; then next="$major.$minor.$((patch + 1))"; else next="$base"; fi ;;
  [0-9]*) next="$bump" ;;
  *) echo "unknown bump: $bump" >&2; exit 2 ;;
esac

if [ "$channel" = "rc" ]; then
  # One past the highest candidate that exists as a tag *or* as a prepared branch: a
  # candidate whose PR has not been merged yet has no tag, and reusing its number collides
  # with it. The highest, not a count: numbers are skipped when a candidate is abandoned
  # (there is a v0.1.1-rc.2 and no rc.1), and counting then hands out a number in use.
  n=$( { git tag -l "v$next-rc.*"
         git ls-remote --tags origin "v$next-rc.*" 2>/dev/null | sed 's|.*/||; s/\^{}$//'
         git ls-remote --heads origin "release/v$next-rc.*" 2>/dev/null | sed 's|.*/||'
       } | sed -n 's/.*-rc\.\([0-9]*\)$/\1/p' | sort -n | tail -1)
  next="$next-rc.$((${n:-0} + 1))"
fi

# A version that is already tagged has already been released; preparing it again writes
# a second changelog section for it and a PR whose tag the Release button then refuses.
if git tag -l "v$next" | grep -q . \
  || git ls-remote --tags origin "v$next" 2>/dev/null | grep -q .; then
  echo "v$next is already tagged; pick another bump" >&2
  exit 1
fi

echo "$current -> $next" >&2

# The version lives in workspace.package and again in each internal dependency, which has
# to carry a version for crates.io. Both must move together or `cargo publish` refuses.
sed -i "0,/^version = \".*\"$/s//version = \"$next\"/" Cargo.toml
sed -i -E "s#(tmprl-[a-z]+ = \{ path = \"crates/tmprl-[a-z]+\", version = \")[^\"]+#\1$next#" Cargo.toml
cargo check --quiet >&2   # rewrites Cargo.lock

# The changelog. A candidate leaves it alone: `## Unreleased` keeps saying what is coming,
# and the entry lands under the real version when that ships. Promoting instead per candidate
# gave every release a section its candidates had already emptied.
if [ "$channel" = "rc" ]; then
  echo "changelog: untouched, this is a candidate for $base" >&2
else
  # Whatever is under Unreleased, plus any section a candidate for this version did claim.
  scripts/changelog.py promote "$next" >&2

  # An empty entry means nobody wrote one, so fall back to the user-visible commits since
  # the last release. docs, chore, ci and test commits change nothing a user runs.
  if grep -qF -- "- No user-visible changes." CHANGELOG.md; then
    last_release=$(git tag -l 'v*' --sort=-v:refname | grep -v -- '-rc\.' | head -1)
    range=${last_release:+$last_release..HEAD}
    commits=$(git log "${range:-HEAD}" --no-merges --format='- %s' \
      --grep='^feat' --grep='^fix' --grep='^perf' -E | sed 's/([^)]*)//' || true)
    if [ -n "$commits" ]; then
      python3 - "$commits" <<'PY' >&2
import pathlib, sys
path = pathlib.Path("CHANGELOG.md")
path.write_text(path.read_text().replace("- No user-visible changes.", sys.argv[1].strip(), 1))
print("changelog: filled from the commit subjects", file=sys.stderr)
PY
    fi
  fi
fi

echo "$next"
