#!/usr/bin/env bash
# The sample cluster for tmprl: a local Temporal dev server, the sample's workers and codec
# server, and shipments to look at.
#
#   demo/run.sh up            start the server, the codec server and the workers
#   demo/run.sh seed [n]      start n shipments (default 300), the same ones every time
#   demo/run.sh tmprl [...]   run tmprl against it, with the sample's own config
#   demo/run.sh down          stop everything and forget the data
#
# Needs the `temporal` CLI and Go. Nothing leaves the machine and every name, address and
# phone number in the data is invented.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
RUN="$HERE/.run"
ADDRESS="${DEMO_ADDRESS:-127.0.0.1:7244}"
CODEC="${DEMO_CODEC:-127.0.0.1:7799}"
mkdir -p "$RUN"

alive() { [[ -f "$RUN/$1.pid" ]] && kill -0 "$(cat "$RUN/$1.pid")" 2>/dev/null; }

start() {
  local name="$1"; shift
  if alive "$name"; then return; fi
  nohup "$@" >"$RUN/$name.log" 2>&1 &
  echo $! >"$RUN/$name.pid"
}

case "${1:-}" in
  up)
    (cd "$HERE" && go build -o "$RUN/bin/" ./cmd/...)
    start server temporal server start-dev --ip "${ADDRESS%:*}" --port "${ADDRESS#*:}" --headless --log-level warn
    for _ in $(seq 1 40); do
      temporal operator namespace list --address "$ADDRESS" >/dev/null 2>&1 && break
      sleep 0.5
    done
    start codec "$RUN/bin/codec" -listen "$CODEC"
    start worker "$RUN/bin/worker" -address "$ADDRESS"
    echo "sample cluster on $ADDRESS, codec on http://$CODEC"
    ;;
  seed)
    "$RUN/bin/seed" -address "$ADDRESS" -n "${2:-300}" "${@:3}"
    ;;
  tmprl)
    shift
    # A shell that exports TEMPORAL_ADDRESS would override the profile.
    # The config names the resolver by where this checkout is.
    mkdir -p "$RUN/config"
    for f in "$HERE"/config/config.toml "$HERE"/config/dashboard.toml; do
      sed "s|@DEMO@|$HERE|g" "$f" >"$RUN/config/$(basename "$f")"
    done
    env -u TEMPORAL_ADDRESS -u TEMPORAL_NAMESPACE -u TEMPORAL_API_KEY -u TEMPORAL_TLS \
      TMPRL_CONFIG_DIR="$RUN/config" DEMO_ROOT="$HERE" \
      "${TMPRL:-tmprl}" --profile sample --config "$HERE/config/temporal.toml" "$@"
    ;;
  down)
    for name in worker codec server; do
      if alive "$name"; then kill "$(cat "$RUN/$name.pid")" 2>/dev/null || true; fi
      rm -f "$RUN/$name.pid"
    done
    ;;
  *)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
