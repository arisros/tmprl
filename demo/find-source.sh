#!/usr/bin/env bash
# The sample's source resolver for `gf`: finds where the thing under the cursor is named
# in the sample's own code.
#
# tmprl hands it one JSON object on stdin (docs/INTERFACE.md, "Going to the source") and
# opens the editor on the `path:line` it prints, started in the directory on the second
# line. A real one would also work out which version of the code was running; here there
# is only the one.
set -euo pipefail

ROOT="${DEMO_ROOT:-$(cd "$(dirname "$0")" && pwd)}"
# The top-level `name`. The payloads further in have fields called `name` too, so this is
# read as JSON, not searched for.
if command -v jq >/dev/null 2>&1; then
  name="$(jq -r '.name // empty')"
else
  name="$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("name") or "")')"
fi
[[ -n "$name" ]] || { echo "nothing under the cursor has a name" >&2; exit 1; }

# The function that does the work, when the name follows the sample's convention:
# `book_carrier` is done by `func BookCarrier(`.
camel="$(echo "$name" | awk -F'[_-]' '{ for (i = 1; i <= NF; i++) printf "%s%s", toupper(substr($i, 1, 1)), substr($i, 2) }')"
hit="$(grep -rnE --include='*.go' "^func $camel\(" "$ROOT" | head -1 || true)"
# Else where the name is declared, before anywhere it is merely used.
[[ -n "$hit" ]] || hit="$(grep -rnE --include='*.go' "=[[:space:]]*\"$name\"" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || hit="$(grep -rnF --include='*.go' "\"$name\"" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || { echo "the sample has nothing named \`$name\`" >&2; exit 1; }

echo "$hit" | cut -d: -f1,2
echo "$ROOT"
