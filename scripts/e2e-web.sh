#!/usr/bin/env bash
# Launch the browser end-to-end suites. ONE command, from a fresh worktree, with nothing served first.
#
#   make e2e-web                      # every portal
#   make e2e-web ONLY=vet,groomer     # a subset
#
# WHY THIS EXISTS
#
# The Playwright specs were unlaunchable: each config expects a portal somebody has already served,
# and three of them additionally expect a backend. So every crew reported "could not run" and the
# suites were decoration. This script stands up what they need, runs them, tears down what it started,
# and reports counts - and refuses loudly when it cannot.
#
# THE PROXY TRAP, WHICH IS WHY THE SAFETY HERE IS STRUCTURAL AND NOT "THE SPECS MOCK"
#
# `vite preview` and `vite dev` both honour `server.proxy`, so serving a portal on a port of your own
# does NOT give you a backend of your own: `/api` still proxies to `VITE_*_API_PROXY`, whose default
# is the captain's stack. A crew that carefully picked a spare port has already driven his live
# government API on ROAX chain 135 this way, creating five real records.
#
# "The spec mocks" is NOT the safety property and must never be treated as one:
# `government.spec.ts` reads `/api/health` through `page.request`, which BYPASSES `page.route`
# entirely. So the defence is configuration, applied to every portal whether it mocks or not:
#
#   * every portal's PROXY target is repointed at a CLOSED PORT. `/api` is where every relative
#     request lands, mocked or not, so this is the one override that cannot be bypassed by a spec
#     reaching around `page.route`. Nothing real is reachable, by the network stack rather than by
#     the specs' good behaviour.
#   * `VITE_CENTRAL_API_BASE` is repointed at that same closed port. Its default is the ABSOLUTE
#     `http://localhost:39742`, NOT `/api`, so no proxy override touches it - overriding a portal's
#     own base alone leaves central calls aimed at a shared backend with nothing in between.
#   * the government portal instead points both at the hermetic backend this script starts.
#
# WHY THE PORTALS' OWN API BASE IS LEFT RELATIVE, WHICH LOOKS LIKE THE WEAKER CHOICE
#
# Setting `VITE_VET_API_BASE` to an absolute closed-port url is strictly safer in isolation and was
# tried first. It makes every request CROSS-ORIGIN, and a `page.route` fulfil carries no CORS headers,
# so the browser blocks the mocked response and the spec fails: measured, 11 of the vet suite's 22.
# A runner that reports 11 failures for a reason that exists only inside the runner is worse than
# useless - it trains a reader to disbelieve the suite. Same-origin `/api` keeps the mocks working,
# and the proxy override above is what makes it safe, so nothing is given up.
#
# Set at SERVER-START time, because `import.meta.env` is substituted then - exporting a variable
# before `playwright test` would do nothing at all.
#
# WHY DEV SERVERS RATHER THAN `vite preview`
#
# `preview` needs a `vite build` per portal first, and `build` runs `tsc --noEmit` - so a typecheck
# error anywhere would abort the e2e run for a reason that has nothing to do with the e2e. Dev serves
# the same app from the same source with the same env substitution, and starts in about a second.
#
# WHAT IT REFUSES TO DO
#
# It never kills a process it did not start, and never matches one by name or path (`pkill -f
# target/release/government-api` has destroyed the captain's live service three times - every checkout
# builds the same binary to the same relative path). It records each PID it spawns and kills exactly
# those. If a port it wants is occupied it stops and says so rather than clearing it.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/e2e.sh
source "$ROOT_DIR/scripts/lib/e2e.sh"
E2E_SUITE="e2e-web"
cd "$ROOT_DIR"

# Ports. A block of our own, well away from every port the repo's own stacks bind (39741, 41873/4,
# 43617/8, 44831/2, 45931, 46001) so a run can never collide with the captain's stack.
readonly PORT_VET_WEB=47103
readonly PORT_GROOMER_WEB=47104
readonly PORT_OWNER_WEB=47105
readonly PORT_GOV_WEB=47106
readonly PORT_GOV_API=47101
# Nothing listens here, and the preflight proves it. Every API base for a MOCKED portal points at it,
# so an un-mocked request is refused by the network stack rather than reaching something real.
readonly PORT_SINK=47199
# No path: `VITE_CENTRAL_API_BASE` is a bare origin, and a central call must fail to CONNECT rather
# than resolve to something. That is exactly what happens today when admin-api is not running, so the
# specs see the behaviour they were written against - guaranteed, instead of by luck.
readonly SINK="http://127.0.0.1:${PORT_SINK}"

ALL_SUITES=(vet groomer owner government)
SUITES=()

usage() {
  cat <<'USAGE'
usage: scripts/e2e-web.sh [--only a,b,c] [--list]

  --only    comma-separated subset of: vet, groomer, owner, government
  --list    print the suites and what each needs, then exit
  --cleanup stop servers a previous run recorded but did not get to stop (see .e2e-web.pids)
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only) IFS=',' read -r -a SUITES <<< "${2:-}"; shift 2 ;;
    --cleanup) cleanup_stale "$ROOT_DIR/.e2e-web.pids"; exit 0 ;;
    --list)
      printf 'vet         mocked backend + mocked RPC; needs only a served portal\n'
      printf 'groomer     mocked backend + mocked RPC; needs only a served portal\n'
      printf 'owner       mocked RPC, no backend at all; needs only a served portal\n'
      printf 'government  needs a government-api; this script builds and runs a hermetic one\n'
      printf '            (GOV_CHAIN_BACKEND=mem: a SIMULATED chain, nothing reaches ROAX)\n'
      exit 0 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
[[ ${#SUITES[@]} -eq 0 ]] && SUITES=("${ALL_SUITES[@]}")

for s in "${SUITES[@]}"; do
  case "$s" in
    vet|groomer|owner|government) ;;
    *) printf 'unknown suite: %s\n' "$s" >&2; usage >&2; exit 2 ;;
  esac
done

wants() { local w="$1" s; for s in "${SUITES[@]}"; do [[ "$s" == "$w" ]] && return 0; done; return 1; }

E2E_PIDFILE="$ROOT_DIR/.e2e-web.pids"
arm_teardown

# ---------------------------------------------------------------------------------------------
# Preflight. Everything that can be established BEFORE anything is started, so a missing
# prerequisite costs a second rather than surfacing halfway through a suite.
# ---------------------------------------------------------------------------------------------
step "preflight"
need_cmd node "Install Node 22+ (the repo pins >=22)."
need_cmd pnpm "Install pnpm 10 - \`corepack enable && corepack prepare pnpm@10.19.0 --activate\`."
need_cmd curl "Install curl; the runner uses it to wait for each server to become ready."
need_cmd lsof "Install lsof; the runner uses it to prove a port is free before binding it."

# The SDK's gitignored dist/ is how @dogtag/ui resolves @dogtag/standard's types, so a cold worktree
# cannot serve any portal until it exists. `ensure-ts-prereqs.sh` already knows how to satisfy that
# and, crucially, how to tell an environment failure from a branch defect - so defer to it rather than
# re-deciding that here. Exit 78 from it means the same thing it means here, so it is passed through.
if [[ ! -f packages/dogtag-standard-ts/dist/index.d.ts || ! -d node_modules ]]; then
  info "cold worktree - installing dependencies and building the SDK (once; ~6s warm, longer cold)"
  set +e
  bash scripts/ensure-ts-prereqs.sh --sdk-dist
  prereq_rc=$?
  set -e
  [[ $prereq_rc -eq "$E2E_DID_NOT_RUN" ]] && exit "$E2E_DID_NOT_RUN"
  [[ $prereq_rc -ne 0 ]] && failed "TypeScript prerequisites could not be satisfied and the cause is in this branch (see above)."
fi

PW="$ROOT_DIR/stacks/vet/web/node_modules/.bin/playwright"
[[ -x "$PW" ]] || could_not_run \
  "the Playwright CLI (expected at stacks/vet/web/node_modules/.bin/playwright)" \
  "Run \`pnpm install --frozen-lockfile\`."

# A browser binary is a separate download from the npm package, and its absence is the single most
# common reason this suite has not run. Probe it rather than discovering it inside the first spec.
if ! "$PW" install --dry-run chromium >/dev/null 2>&1; then
  could_not_run "the Playwright chromium browser build" "Run \`$PW install chromium\` (a few hundred MB, once per machine)."
fi
if ! ls "${PLAYWRIGHT_BROWSERS_PATH:-$HOME/Library/Caches/ms-playwright}"/chromium-* >/dev/null 2>&1; then
  could_not_run "the Playwright chromium browser build" "Run \`$PW install chromium\` (a few hundred MB, once per machine)."
fi

require_free_port "$PORT_SINK" "the closed-port sink that keeps mocked portals off any real backend"
wants vet        && require_free_port "$PORT_VET_WEB"     "the vet portal"
wants groomer    && require_free_port "$PORT_GROOMER_WEB" "the groomer portal"
wants owner      && require_free_port "$PORT_OWNER_WEB"   "the owner wallet"
wants government && require_free_port "$PORT_GOV_WEB"     "the government portal"
wants government && require_free_port "$PORT_GOV_API"     "the hermetic government backend"

GOV_BIN="$ROOT_DIR/target/release/government-api"
if wants government; then
  need_cmd cargo "Install Rust (rustup); the government suite needs a government-api binary."
  info "building government-api (release; ~20s warm)"
  cargo build -p government-api --release >"$E2E_TMPDIR/gov-build.log" 2>&1 || {
    tail -30 "$E2E_TMPDIR/gov-build.log" >&2
    failed "government-api failed to build (log above). That is a code finding, not an environment one."
  }
  [[ -x "$GOV_BIN" ]] || failed "cargo reported success but $GOV_BIN is absent."
fi
info "preflight OK"

# ---------------------------------------------------------------------------------------------
# Servers
# ---------------------------------------------------------------------------------------------

# Start a portal's vite dev server. The env is supplied by the caller; the port is forced on the
# COMMAND LINE because every config pins `strictPort` on its own port, which the captain's stack may
# hold. Note vite is invoked directly rather than through `pnpm --filter <pkg> dev -- --port N`: that
# form passes `--` through literally, vite never sees the port, and strictPort then fails on the
# config's own one.
#
# THE ADDRESS CONFIGURATION COMES FROM THE DEPLOY LEDGER, and a portal genuinely needs it.
#
# `$4` names a `scripts/gen-deployment-env.sh` target, which projects
# `contracts/deployments/roax.json` onto that portal's `VITE_*_ADDR` names. Nothing is transcribed
# here: a redeploy repoints this runner with no edit, and a key that stops existing fails loudly
# instead of silently naming a dead contract.
#
# It is REQUIRED, not decoration. Every address ships blank and fallback-free, and a consumer must
# treat `""` as could-not-check and refuse - so an unconfigured portal makes the credential-verify
# panel decline before it reads anything, and `verify-credential.spec.ts` then times out on a verdict
# that never renders. Measured: 7 of the vet suite's 22. Nothing reaches ROAX regardless, because the
# specs intercept the RPC; what the address does is let the panel get as far as asking.
start_portal() {
  local dir="$1" port="$2" label="$3" env_target="${4:-}"
  local log="$E2E_TMPDIR/$label.log"
  local envfile="$E2E_TMPDIR/$label.env"
  : > "$envfile"
  if [[ -n "$env_target" ]]; then
    bash "$ROOT_DIR/scripts/gen-deployment-env.sh" "$env_target" > "$envfile" 2>"$E2E_TMPDIR/$label.env.err" || {
      cat "$E2E_TMPDIR/$label.env.err" >&2
      failed "could not project the deploy ledger onto $env_target's variables (see above).
  The ledger is the only source of an address; this runner will not substitute a literal."
    }
  fi
  # `set -a` + source: the file is produced by our own script and holds only KEY=VALUE plus comments.
  ( cd "$ROOT_DIR/$dir" && set -a && . "$envfile" && set +a \
      && exec node_modules/.bin/vite --port "$port" --strictPort ) >"$log" 2>&1 &
  local pid=$!
  track_pid "$pid"
  # BOTH address families, in one loop: vite dev binds IPv6-only here, so a 127.0.0.1-only probe
  # reports a perfectly healthy server as dead and burns the whole timeout doing it.
  wait_for_http "$pid" "$label" "$log" 90 "http://127.0.0.1:$port/" "http://[::1]:$port/"
  PORTAL_URL="$E2E_READY_URL"
  info "$label on $PORTAL_URL (pid $pid)"
}

# The hermetic government backend. TWO flags, because they are two axes and collapsing them is a
# defect this repo already paid for: GOV_DEMO_MODE picks the ephemeral store and the demo token,
# GOV_CHAIN_BACKEND picks the chain. GOV_DEMO_MODE alone runs a demo store against the LIVE chain.
# DEMO_MODE=1 is separate again - without it the H2 production guard refuses to boot, which reads as a
# secrets problem and is really a missing flag (it normally arrives from contracts/.env).
start_government_api() {
  local log="$E2E_TMPDIR/government-api.log"
  # The FACTORY the simulated chain indexes issuances under, from the deploy ledger rather than a
  # literal. It is what makes `rootIssuer(root)` resolve, and so what lets the MANDATORY
  # issuer-whitelist pillar answer at all: unset, the pillar is permanently indeterminate and the
  # three-pillar assertion in `government.spec.ts` cannot pass on any simulated serve.
  # Nothing is dialled - the chain is `mem` - so this configures a map key, not a network target.
  # ...and the AUTHORITY whose grant log the pillar folds. Both come from the ledger rather than a
  # literal. The clone must be able to NAME a governing registry or the pillar degrades to
  # `unresolved`, which is a different claim from "the log recorded no grant".
  local GOV_FACTORY_ADDR GOV_REGISTRY_ADDR
  GOV_FACTORY_ADDR="$(bash "$ROOT_DIR/scripts/gen-deployment-env.sh" vet \
    | sed -n 's/^FACTORY_ADDR=//p' | head -1)"
  GOV_REGISTRY_ADDR="$(bash "$ROOT_DIR/scripts/gen-deployment-env.sh" vet-web \
    | sed -n 's/^VITE_PROVIDER_REGISTRY_ADDR=//p' | head -1)"
  [[ -n "$GOV_FACTORY_ADDR" && -n "$GOV_REGISTRY_ADDR" ]] || failed "the deploy ledger published no
  DogTagIssuerFactory / ProviderRegistry, so the issuer pillar could not be configured. The ledger is
  the only source; this runner will not substitute a literal."
  # The two issuer addresses are SIMULATED-CHAIN placeholders, not deployed contracts: on MemChain
  # there is no clone to name, and the ledger publishes none (providerCount is 0). The same
  # unmistakably-synthetic value scripts/e2e-roles.sh already uses for its hermetic run.
  #
  # They must be SET, and that is the whole reason this block exists: with them unset `/health`
  # reports both issuers null, `government.spec.ts` calls `test.skip()`, and the run reports success
  # having verified nothing - the exact outcome this script is built to make impossible.
  (
    cd "$ROOT_DIR" && exec env \
      PORT="$PORT_GOV_API" \
      DEMO_MODE=1 \
      GOV_DEMO_MODE=1 \
      GOV_CHAIN_BACKEND=mem \
      TRAVEL_CLEARANCE_ISSUER_ADDR=0x1111111111111111111111111111111111111111 \
      EU_HEALTH_CERT_ISSUER_ADDR=0x2222222222222222222222222222222222222222 \
      FACTORY_ADDR="$GOV_FACTORY_ADDR" \
      ISSUER_REGISTRY_ADDR="$GOV_REGISTRY_ADDR" \
      DEPLOYMENT_URL="http://127.0.0.1:$PORT_GOV_API" \
      "$GOV_BIN"
  ) >"$log" 2>&1 &
  local pid=$!
  track_pid "$pid"
  wait_for_http "$pid" "government-api" "$log" 60 "http://127.0.0.1:$PORT_GOV_API/health" "http://[::1]:$PORT_GOV_API/health"
  info "government-api on :$PORT_GOV_API (pid $pid, SIMULATED chain)"

  # Prove the backend is the simulated one before a single spec runs. A government-api that came up
  # LIVE would anchor real records on ROAX, and by the time a spec noticed it would be too late.
  local health; health="$(curl -fsS "http://127.0.0.1:$PORT_GOV_API/health")"
  python3 - "$health" <<'PY' || failed "the government backend did not come up on a simulated chain - refusing to run specs that issue."
import json, sys
h = json.loads(sys.argv[1])
ok = h.get("simulated") is True and h.get("chainId") is None and h.get("canSign") is not True
if not ok:
    sys.stderr.write("  /health says: simulated=%r chainId=%r canSign=%r\n"
                     % (h.get("simulated"), h.get("chainId"), h.get("canSign")))
    sys.exit(1)
issuers = h.get("issuers") or {}
missing = [k for k, v in issuers.items() if not v]
if missing:
    sys.stderr.write("  no issuer configured for %s - specs would SKIP rather than run\n" % ", ".join(missing))
    sys.exit(1)
PY
  info "verified: simulated chain, no signer, both issuers configured"
}

# ---------------------------------------------------------------------------------------------
# Running a suite
# ---------------------------------------------------------------------------------------------
TOTAL_PASSED=0 TOTAL_FAILED=0 TOTAL_SKIPPED=0
declare -a RESULT_LINES=()

# Run one portal's Playwright project and take the counts from its JSON report. The list reporter is
# kept for live output, but it is NOT what the counts come from: scraping console text is how a runner
# comes to believe a suite ran when it did not.
run_suite() {
  local label="$1" dir="$2" base_env_var="$3" base_url="$4"
  local json="$E2E_TMPDIR/$label.json"
  step "running $label specs against $base_url"
  set +e
  ( cd "$ROOT_DIR/$dir" \
    && env "$base_env_var=$base_url" PLAYWRIGHT_JSON_OUTPUT_NAME="$json" \
       node_modules/.bin/playwright test --reporter=list,json )
  local rc=$?
  set -e

  [[ -f "$json" ]] || failed "$label produced no JSON report, so it cannot be shown to have run
  (playwright exited $rc). Treating an exit code alone as evidence is what lets a suite that
  never executed report success."

  local counts; counts="$(python3 - "$json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
s = r.get("stats") or {}
print("%d %d %d %d" % (s.get("expected", 0), s.get("unexpected", 0),
                       s.get("flaky", 0), s.get("skipped", 0)))
PY
)"
  local passed failedn flaky skipped
  read -r passed failedn flaky skipped <<< "$counts"

  assert_ran "$label" "$((passed + failedn + flaky + skipped))"

  TOTAL_PASSED=$((TOTAL_PASSED + passed))
  TOTAL_FAILED=$((TOTAL_FAILED + failedn + flaky))
  TOTAL_SKIPPED=$((TOTAL_SKIPPED + skipped))
  RESULT_LINES+=("$(printf '  %-12s %3d passed  %3d failed  %3d skipped' "$label" "$passed" "$((failedn + flaky))" "$skipped")")

  # A SKIP here is not the spec being polite - this runner controls the whole environment, so a skip
  # means a prerequisite this script promised to supply was not supplied. Reporting that as a pass is
  # precisely the "could not run, looked green" outcome the suite exists to prevent.
  if [[ "$skipped" -gt 0 ]]; then
    failed "$label SKIPPED $skipped test(s).
  This runner configures everything these specs gate on, so a skip means that setup did not
  hold - not that the test was inapplicable. Investigate rather than accepting it: a skipped
  test verifies nothing and must never be counted as passing."
  fi
  if [[ $rc -ne 0 || "$failedn" -gt 0 || "$flaky" -gt 0 ]]; then
    failed "$label: $failedn failed, $flaky flaky (playwright exit $rc)."
  fi
  info "$label: $passed passed, 0 failed"
}

# ---------------------------------------------------------------------------------------------
# Go
# ---------------------------------------------------------------------------------------------
if wants vet; then
  step "starting the vet portal"
  VITE_DEMO_MODE=1 VITE_CENTRAL_API_BASE="$SINK" VITE_VET_API_PROXY="$SINK" \
    start_portal stacks/vet/web "$PORT_VET_WEB" vet-web vet-web
  run_suite vet stacks/vet/web VET_URL "$PORTAL_URL"
fi

if wants groomer; then
  step "starting the groomer portal"
  VITE_DEMO_MODE=1 VITE_CENTRAL_API_BASE="$SINK" VITE_GROOMER_API_PROXY="$SINK" \
    start_portal stacks/groomer/web "$PORT_GROOMER_WEB" groomer-web groomer-web
  run_suite groomer stacks/groomer/web GROOMER_URL "$PORTAL_URL"
fi

if wants owner; then
  # The owner wallet has NO backend and NO vite proxy at all, so there is nothing to point away from
  # anything. Its config would otherwise start its own server with `reuseExistingServer: true`, which
  # would silently attach to a server this script did not start - hence OWNER_URL, which turns that off.
  step "starting the owner wallet"
  start_portal stacks/owner/web "$PORT_OWNER_WEB" owner-web owner-web
  run_suite owner stacks/owner/web OWNER_URL "$PORTAL_URL"
fi

if wants government; then
  step "starting the hermetic government backend"
  start_government_api
  step "starting the government portal"
  VITE_DEMO_MODE=1 VITE_GOV_API_PROXY="http://127.0.0.1:$PORT_GOV_API" \
    start_portal stacks/government/web "$PORT_GOV_WEB" government-web
  run_suite government stacks/government/web GOV_URL "$PORTAL_URL"
fi

step "summary"
printf '%s\n' "${RESULT_LINES[@]}"
printf '  %-12s %3d passed  %3d failed  %3d skipped\n' TOTAL "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED"
assert_ran "e2e-web (all suites)" "$((TOTAL_PASSED + TOTAL_FAILED + TOTAL_SKIPPED))"
[[ "$TOTAL_FAILED" -eq 0 ]] || failed "$TOTAL_FAILED test(s) failed."
printf '\ne2e-web: OK - %d tests passed across %d suite(s); every server this script started has been stopped.\n' \
  "$TOTAL_PASSED" "${#SUITES[@]}"
