#!/usr/bin/env bash
# Stop cloudflared tunnels started by scripts/tunnel-up.sh - by their RECORDED pids, never by name.
#
#   scripts/tunnel-down.sh            # every tunnel tunnel-up.sh recorded
#   scripts/tunnel-down.sh 41874      # just that port's tunnel
#
# Never `pkill cloudflared`: the pattern reaches whichever unrelated cloudflared it happens to hit
# (the operator may have others running), which is the same reason demo-down.sh kills only recorded
# pids. A tunnel this script did not start is not this script's to stop.
#
# Remember: stopping a tunnel retires its URL. The next tunnel-up.sh mints a NEW one, and any stack
# booted with the old URL keeps printing QRs no phone can reach until it is restarted onto the new
# one (docs/DEMO_CLICKS.md §0.7).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

stop_one() { # pidfile
  local f="$1" port pid
  port="$(basename "$f")"; port="${port#tunnel.}"; port="${port%.pid}"
  pid="$(cat "$f" 2>/dev/null || true)"
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    echo "  tunnel $port: stopped pid $pid"
  else
    echo "  tunnel $port: already gone (stale pidfile)"
  fi
  rm -f "$f"
}

if [ "$#" -gt 0 ]; then
  for port in "$@"; do
    f="$ROOT/.demo/tunnel.$port.pid"
    if [ -f "$f" ]; then stop_one "$f"; else
      echo "  tunnel $port: nothing recorded ($f absent) - if a tunnel is running, tunnel-up.sh did not start it; stop it where you started it, never by name" >&2
    fi
  done
  exit 0
fi

found=0
for f in "$ROOT"/.demo/tunnel.*.pid; do
  [ -e "$f" ] || continue
  found=1
  stop_one "$f"
done
[ "$found" -eq 1 ] || echo "no recorded tunnels (.demo/tunnel.*.pid absent) - nothing to stop"
