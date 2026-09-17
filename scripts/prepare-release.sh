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
  # Count candidates that already exist as a tag *or* as a prepared branch: a candidate
  # whose PR has not been merged yet has no tag, and reusing its number collides with it.
  n=$( { git tag -l "v$next-rc.*"
         git ls-remote --heads origin "release/v$next-rc.*" 2>/dev/null | sed 's|.*/||'
       } | sort -u | wc -l)
  next="$next-rc.$((n + 1))"
fi

echo "$current -> $next" >&2

# The version lives in workspace.package and again in each internal dependency, which has
# to carry a version for crates.io. Both must move together or `cargo publish` refuses.
sed -i "0,/^version = \".*\"$/s//version = \"$next\"/" Cargo.toml
sed -i -E "s#(tmprl-[a-z]+ = \{ path = \"crates/tmprl-[a-z]+\", version = \")[^\"]+#\1$next#" Cargo.toml
cargo check --quiet >&2   # rewrites Cargo.lock

# The changelog section: whatever sits under Unreleased, plus the merges since the last
# tag, so a release always says what changed even when nobody wrote it down by hand.
last_tag=$(git tag -l 'v*' --sort=-v:refname | head -1)
range=${last_tag:+$last_tag..HEAD}
commits=$(git log "${range:-HEAD}" --no-merges --format='- %s' \
  --grep='^feat' --grep='^fix' --grep='^perf' --grep='^docs' -E | sed 's/([^)]*)//' || true)

python3 - "$next" "$commits" <<'PY' >&2
import datetime, pathlib, sys

version, commits = sys.argv[1], sys.argv[2]
path = pathlib.Path("CHANGELOG.md")
text = path.read_text()
heading = f"## {version} — {datetime.date.today():%Y-%m-%d}"

# A hand-written Unreleased section wins: it says what changed in the words someone chose.
# With none, open a section from the commit subjects, which is better than an empty entry.
if "## Unreleased" in text:
    text = text.replace("## Unreleased", heading, 1)
else:
    body = commits.strip() or "- No user-visible changes."
    text = text.replace("# Changelog\n", f"# Changelog\n\n{heading}\n\n{body}\n", 1)

path.write_text(text)
print(f"changelog: {heading}", file=sys.stderr)
PY

echo "$next"
