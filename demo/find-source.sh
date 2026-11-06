#!/usr/bin/env bash
# The sample's source resolver for `gf`: finds where the thing under the cursor is named
set -euo pipefail

ROOT="${DEMO_ROOT:-$(cd "$(dirname "$0")" && pwd)}"
# Read as JSON: payloads further in have `name` fields too.
if command -v jq >/dev/null 2>&1; then
  name="$(jq -r '.name // empty')"
else
  name="$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("name") or "")')"
fi
[[ -n "$name" ]] || { echo "nothing under the cursor has a name" >&2; exit 1; }

camel="$(echo "$name" | awk -F'[_-]' '{ for (i = 1; i <= NF; i++) printf "%s%s", toupper(substr($i, 1, 1)), substr($i, 2) }')"
hit="$(grep -rnE --include='*.go' "^func $camel\(" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || hit="$(grep -rnE --include='*.go' "=[[:space:]]*\"$name\"" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || hit="$(grep -rnF --include='*.go' "\"$name\"" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || { echo "the sample has nothing named \`$name\`" >&2; exit 1; }

echo "$hit" | cut -d: -f1,2
echo "$ROOT"
