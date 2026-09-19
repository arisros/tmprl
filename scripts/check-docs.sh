#!/usr/bin/env bash
#
# Keep the code maps honest.
#
# Every source file opens with a `//!` line saying what it is, and the maps that list
# files by hand (docs/ARCHITECTURE.md, app/mod.rs) name every file they cover. Both drift
# silently otherwise: the architecture doc's test counts were months stale before anyone
# looked. Run by CI; runnable by hand from the repository root.

set -euo pipefail

fail=0
complain() {
    echo "check-docs: $*" >&2
    fail=1
}

# 1. A header on every file.
while IFS= read -r f; do
    head -n1 "$f" | grep -q '^//!' || complain "$f has no //! header saying what it is"
done < <(find crates -path '*/src/*' -name '*.rs' | sort)

# 2. Every tmprl-core module and client operation in the architecture map.
map=docs/ARCHITECTURE.md
for f in crates/tmprl-core/src/*.rs crates/tmprl-client/src/ops/*.rs; do
    name=$(basename "$f" .rs)
    case "$name" in lib | mod) continue ;; esac
    grep -q "\`$name" "$map" || complain "$f is missing from the code map in $map"
done

# 3. Every app/ file in the table at the top of app/mod.rs.
table=crates/tmprl-tui/src/app/mod.rs
for f in crates/tmprl-tui/src/app/*.rs; do
    name=$(basename "$f" .rs)
    [ "$name" = mod ] && continue
    grep -q "^//! | \`$name\`" "$table" || complain "$f is missing from the table in $table"
done

exit "$fail"
