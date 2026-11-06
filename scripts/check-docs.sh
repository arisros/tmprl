#!/usr/bin/env bash
# Keep the code maps and the per-crate test counts in the docs true.

set -euo pipefail

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
        grep -qE "^\| \`$crate\` \|(.*\|)? $n \|" "$doc" \
            || complain "$doc does not say $crate has $n tests"
    done
done

exit "$fail"
