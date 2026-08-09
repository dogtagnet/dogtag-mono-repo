#!/usr/bin/env bash
# Start a cloudflared quick tunnel IN THE BACKGROUND, the way demo-up.sh runs everything else:
# nohup'd, logged to .demo/, its pid RECORDED so it can be stopped by pid and never by name.
#
#   scripts/tunnel-up.sh              # tunnel to the vet api (port 41874)
#   scripts/tunnel-up.sh 43618        # tunnel to another local port (groomer, prover, government)
#
# Prints the https://<sub>.trycloudflare.com URL once cloudflared reports it, plus the exact
# demo-up.sh line that boots the stack onto it. Idempotent: if this port's recorded tunnel is still
# alive, its existing URL is printed instead of starting a second one (a fresh run would mint a NEW
# URL and force a stack restart for nothing).
#
# Stop with scripts/tunnel-down.sh — which kills only the pid recorded here. A foreground
# `cloudflared tunnel --url ...` occupies the terminal and never prints its own pid, so the stop
# instruction had no input; and `pkill cloudflared` is forbidden in this repo for the same reason
# every pattern kill is: it reaches whichever unrelated cloudflared it happens to hit.
#
# Files (per port, so several roles can be tunnelled at once):
#   .demo/tunnel.<port>.log   cloudflared's own output — the URL is read back out of this
#   .demo/tunnel.<port>.pid   the ONE pid tunnel-down.sh may kill
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${1:-41874}"
case "$PORT" in
  *[!0-9]*) echo "ERROR: '$PORT' is not a port. Usage: scripts/tunnel-up.sh [port]" >&2; exit 1 ;;
esac
mkdir -p "$ROOT/.demo"
LOG="$ROOT/.demo/tunnel.$PORT.log"
PIDFILE="$ROOT/.demo/tunnel.$PORT.pid"

command -v cloudflared >/dev/null 2>&1 || {
  echo "ERROR: cloudflared is not installed (brew install cloudflared). See docs/DEMO_CLICKS.md §0.6." >&2
  exit 1
}

# The env var demo-up.sh takes for this port (docs/TUNNELING.md owns the map).
var_for_port() {
  case "$1" in
    41874) echo "VET_PUBLIC_URL" ;;
    43618) echo "GROOMER_PUBLIC_URL" ;;
    41875) echo "PROVER_PUBLIC_URL" ;;
    44832) echo "GOV_PUBLIC_URL" ;;
    *) echo "" ;;
  esac
}

# `-a`: cloudflared's log carries escape bytes that make grep call it binary and answer
# "Binary file matches" instead of the URL. `|| true`: no-match grep exits 1, which
# set -e/pipefail would turn into a silent script death inside the poll loop.
url_from_log() { { grep -a -Eo 'https://[a-z0-9-]+\.trycloudflare\.com' "$LOG" 2>/dev/null || true; } | head -1; }

print_next_step() { # url
  local var; var="$(var_for_port "$PORT")"
  echo "  tunnel URL: $1"
  if [ -n "$var" ]; then
    echo "  boot the stack onto it:  LAN_IP=\$(ipconfig getifaddr en0) $var=$1 scripts/demo-up.sh"
  fi
  echo "  stop it later:           scripts/tunnel-down.sh $PORT"
}

# Idempotence: a live recorded tunnel keeps its URL — every fresh cloudflared run mints a NEW one,
# and a new URL means every QR already drawn is dead and the stack must be rebooted onto it.
if [ -f "$PIDFILE" ]; then
  pid="$(cat "$PIDFILE" 2>/dev/null || true)"
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    url="$(url_from_log)"
    echo "tunnel for port $PORT is already running (pid $pid)"
    if [ -n "$url" ]; then print_next_step "$url"; else
      echo "  ...but its URL is not in $LOG - it may still be connecting; re-run in a few seconds." >&2
    fi
    exit 0
  fi
  # recorded pid is dead: stale files from an earlier run — start fresh.
  rm -f "$PIDFILE"
fi

: > "$LOG"
nohup cloudflared tunnel --url "http://localhost:$PORT" >"$LOG" 2>&1 &
pid=$!
echo "$pid" > "$PIDFILE"
echo "started cloudflared for http://localhost:$PORT (pid $pid, log $LOG)"

# cloudflared prints the quick-tunnel URL within a few seconds of connecting; wait for it rather
# than telling the operator to go grep a log.
for _ in $(seq 1 30); do
  url="$(url_from_log)"
  [ -n "$url" ] && { print_next_step "$url"; exit 0; }
  sleep 1
done

echo "ERROR: no trycloudflare URL appeared in $LOG after 30s - the tunnel did not come up." >&2
echo "Last log lines:" >&2
tail -5 "$LOG" >&2 || true
kill "$pid" 2>/dev/null || true
rm -f "$PIDFILE"
exit 1
