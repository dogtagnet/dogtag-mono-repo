#!/usr/bin/env bash
# Stop everything started by demo-up.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
[ -f "$ROOT/.demo/pids" ] || { echo "no .demo/pids"; exit 0; }
while read -r pid; do [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; done < "$ROOT/.demo/pids"
rm -f "$ROOT/.demo/pids"
echo "demo stopped."
