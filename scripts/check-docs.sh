#!/usr/bin/env bash
# Keep the code maps and the per-crate test counts in the docs true.
# --fix writes the test counts into the tables instead of failing on them.

set -euo pipefail

fix=0
[ "${1:-}" = "--fix" ] && fix=1

fail=0
complain() {
    echo "check-docs: $*" >&2
    fail=1
}

while IFS= read -r f; do
    head -n1 "$f" | grep -q '^//!' || complain "$f has no //! header saying what it is"
done < <(find crates -path '*/src/*' -name '*.rs' | sort)

map=docs/ARCHITECTURE.md
for f in crates/tmprl-core/src/*.rs crates/tmprl-client/src/ops/*.rs; do
    name=$(basename "$f" .rs)
    case "$name" in lib | mod) continue ;; esac
    grep -q "\`$name" "$map" || complain "$f is missing from the code map in $map"
done

table=crates/tmprl-tui/src/app/mod.rs
for f in crates/tmprl-tui/src/app/*.rs; do
    name=$(basename "$f" .rs)
    [ "$name" = mod ] && continue
    grep -q "^//! | \`$name\`" "$table" || complain "$f is missing from the table in $table"
done

for dir in crates/*/; do
    crate=$(basename "$dir")
    n=$(grep -rE '^\s*#\[(tokio::)?test' "$dir" | wc -l | tr -d ' ')
    for doc in README.md docs/ARCHITECTURE.md; do
        grep -qE "^\| \`$crate\` \|(.*\|)? $n \|" "$doc" && continue
        if [ "$fix" = 1 ]; then
            # The row's first cell that is only a number is the count, in both tables.
            CRATE="$crate" N="$n" perl -pi -e \
                's/^(\| `\Q$ENV{CRATE}\E` \|(?:[^|]*\|)*?) \d+ \|/$1 $ENV{N} |/' "$doc"
            echo "check-docs: $doc now says $crate has $n tests" >&2
        else
            complain "$doc does not say $crate has $n tests (scripts/check-docs.sh --fix writes it)"
        fi
    done
done

for f in changelog.d/*.md; do
    [ -e "$f" ] || continue
    [ "$(basename "$f")" = README.md ] && continue
    head -n1 "$f" | grep -q '^- ' || complain "$f does not start with a bullet (\`- \`)"
done

exit "$fail"
