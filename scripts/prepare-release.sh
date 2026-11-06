#!/usr/bin/env bash
# Bump the workspace version and write the changelog section for it.
# Prints only the new version on stdout; the workflow reads it from there.
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
  patch) if [ "$current" = "$base" ]; then next="$major.$minor.$((patch + 1))"; else next="$base"; fi ;;
  [0-9]*) next="$bump" ;;
  *) echo "unknown bump: $bump" >&2; exit 2 ;;
esac

if [ "$channel" = "rc" ]; then
  # The highest, not a count: abandoned candidates leave gaps in the numbers.
  n=$( { git tag -l "v$next-rc.*"
         git ls-remote --tags origin "v$next-rc.*" 2>/dev/null | sed 's|.*/||; s/\^{}$//'
         git ls-remote --heads origin "release/v$next-rc.*" 2>/dev/null | sed 's|.*/||'
       } | sed -n 's/.*-rc\.\([0-9]*\)$/\1/p' | sort -n | tail -1)
  next="$next-rc.$((${n:-0} + 1))"
fi

if git tag -l "v$next" | grep -q . \
  || git ls-remote --tags origin "v$next" 2>/dev/null | grep -q .; then
  echo "v$next is already tagged; pick another bump" >&2
  exit 1
fi

echo "$current -> $next" >&2

# Internal dependencies carry the version too, or cargo publish refuses.
sed -i "0,/^version = \".*\"$/s//version = \"$next\"/" Cargo.toml
sed -i -E "s#(tmprl-[a-z]+ = \{ path = \"crates/tmprl-[a-z]+\", version = \")[^\"]+#\1$next#" Cargo.toml
cargo check --quiet >&2   # rewrites Cargo.lock

if [ "$channel" = "rc" ]; then
  echo "changelog: untouched, this is a candidate for $base" >&2
else
  scripts/changelog.py promote "$next" >&2

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
