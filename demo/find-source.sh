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

search() { grep -rnE --include='*.go' "$@" "$ROOT" || true; }
place() { cut -d: -f1,2 <<<"$1"; }

camel="$(echo "$name" | awk -F'[_-]' '{ for (i = 1; i <= NF; i++) printf "%s%s", toupper(substr($i, 1, 1)), substr($i, 2) }')"
named="$(search "^[[:space:]]*[A-Za-z0-9_]+[[:space:]]*=[[:space:]]*\"$name\"" | head -1)"
const="$(sed -E 's/^[^:]*:[^:]*:[[:space:]]*([A-Za-z0-9_]+).*/\1/' <<<"$named")"

registered="" calls="" fn="$camel"
if [[ -n "$named" ]]; then
  registered="$(search "(^|[^A-Za-z0-9_])${const}[[:space:]]*:[[:space:]]*[A-Za-z0-9_]+|Register[A-Za-z]*\([A-Za-z0-9_]+,.*Name:[[:space:]]*$const([^A-Za-z0-9_]|$)" | head -1)"
  [[ -z "$registered" ]] || fn="$(sed -E "s/.*${const}[[:space:]]*:[[:space:]]*([A-Za-z0-9_]+).*/\1/; s/.*Register[A-Za-z]*\(([A-Za-z0-9_]+),.*/\1/" <<<"$registered")"
  calls="$(search -w "$const" | grep -vF "$(place "$named"):" | grep -vF "$(place "$registered"):" | head -3)"
fi

hit="$(search "^func $fn\(" | head -1)"
[[ -n "$hit" ]] || hit="$named"
[[ -n "$hit" ]] || hit="$(grep -rnF --include='*.go' "\"$name\"" "$ROOT" | head -1 || true)"
[[ -n "$hit" ]] || { echo "the sample has nothing named \`$name\`" >&2; exit 1; }

place "$hit"
echo "$ROOT"

[[ -n "$registered" ]] || exit 0
while IFS= read -r call; do
  [[ -z "$call" ]] || printf '%s\t%s\t%s\n' "$(place "$call")" "$ROOT" "the workflow asks for $const"
done <<<"$calls"
printf '%s\t%s\t%s\n' "$(place "$registered")" "$ROOT" "the worker registers $fn as $name"
printf '%s\t%s\t%s\n' "$(place "$hit")" "$ROOT" "$fn runs"
