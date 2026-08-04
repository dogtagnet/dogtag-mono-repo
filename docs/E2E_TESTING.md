# End-to-end testing

The end-to-end suites are the gate.
There is no validation pipeline any more, so a change is defended by the tests that run against it and by nothing else.

Each surface has ONE command, and each can be run from a fresh worktree with nothing served first.

```bash
make e2e-web              # every portal: vet, groomer, owner, government
make e2e-web ONLY=vet     # one of them
make e2e-ios              # builds the app, boots a simulator, runs the Maestro flows
make e2e-android          # builds the apk, installs on a connected arm64 device, runs Maestro
make e2e                  # all three
```

## Three outcomes, and the third is the point

| exit | meaning |
|---|---|
| `0`  | the suite RAN and every test passed |
| `1`  | the suite RAN and something FAILED - a code finding |
| `78` | the suite DID NOT RUN - a prerequisite is genuinely absent |

**78 prints an unmissable banner, names the specific missing prerequisite, and is never 0.**

That third outcome is the whole reason these runners exist.
A skipped suite reporting success is the defect this fleet keeps finding: most recently both mobile unit modules failed to COMPILE and read as "no failures", because a module that does not run reports nothing, and nothing is indistinguishable from success.

Two rules follow, and both are enforced rather than documented:

- **Zero tests is a FAILURE, never "nothing to run."**
  Playwright exits 0 when a filter matches nothing, and a Gradle `BUILD SUCCESSFUL` can mean no test executed at all.
  Counts therefore come from a machine-readable report - Playwright's JSON reporter, Maestro's JUnit XML - never from scraped console text, and a total of zero is refused.
- **A SKIPPED test is a FAILURE too.**
  These runners configure everything the specs gate on, so a skip means that setup did not hold, not that the test was inapplicable.
  `government.spec.ts` and both `unlock.spec.ts` really do call `test.skip()` when the portal is not served with `VITE_DEMO_MODE=1`, and `government.spec.ts` skips again when no issuer is configured - so this is a live trap, not a theoretical one.

`78` is the same exit code and the same "THE CHECK DID NOT RUN" banner that `scripts/ensure-ts-prereqs.sh` already uses.
A second dialect for the same idea is how a caller comes to handle one and not the other.

## `make e2e-web`

Starts a vite dev server per portal on ports **47103-47106**, a hermetic `government-api` on **47101**, runs the Playwright specs against them, and stops everything it started.

### It cannot reach a real backend, and that is structural

`vite preview` and `vite dev` both honour `server.proxy`, so serving a portal on a port of your own does **not** give you a backend of your own: `/api` still proxies to `VITE_*_API_PROXY`, whose default is the captain's stack.
A crew that carefully picked a spare port has already driven his live government API on ROAX chain 135 this way, creating five real records.

**"The specs mock" is not the safety property.**
`government.spec.ts` reads `/api/health` through `page.request`, which bypasses `page.route` entirely.
So the defence is configuration:

- every portal's **proxy target** is repointed at a closed port (47199), which the preflight proves is closed.
  `/api` is where every relative request lands, mocked or not, so this is the one override a spec cannot reach around.
- **`VITE_CENTRAL_API_BASE`** is repointed at that same closed port.
  Its default is the absolute `http://localhost:39742`, *not* `/api`, so no proxy override touches it.
- the government portal points at the hermetic backend this script starts instead, and that backend's `/health` is checked for `simulated: true`, `chainId: null` and no signer **before a single spec runs**.

The portals' own API base is deliberately left relative.
Setting it to an absolute closed-port url is safer in isolation and was tried first: it makes every request cross-origin, a `page.route` fulfil carries no CORS headers, and the browser blocks the mocked response - 11 of the vet suite's 22 failed for a reason that existed only inside the runner.
Same-origin `/api` keeps the mocks working and the proxy override is what makes it safe.

### The government backend is simulated

`GOV_DEMO_MODE=1` and `GOV_CHAIN_BACKEND=mem` are **two axes**, and collapsing them is a defect this repo has already paid for: `GOV_DEMO_MODE` alone picks the ephemeral store and runs against the LIVE chain.
`DEMO_MODE=1` is separate again - without it the production secrets guard refuses to boot, which reads as a secrets problem and is really a missing flag.

Both `*_ISSUER_ADDR` values are simulated-chain placeholders, the same unmistakably-synthetic form `scripts/e2e-roles.sh` already uses.
They must be set: with them unset, `/health` reports both issuers null and `government.spec.ts` **skips**, so the run would report success having verified nothing.

### Addresses come from the deploy ledger

Each portal's `VITE_*_ADDR` config is projected from `contracts/deployments/roax.json` by `scripts/gen-deployment-env.sh`.
Nothing is transcribed: a redeploy repoints the runner with no edit, and a key that stops existing fails loudly.

This is required, not decoration.
Every address ships blank and fallback-free and a consumer must treat `""` as could-not-check and refuse - so an unconfigured portal makes the credential-verify panel decline before it reads anything, and `verify-credential.spec.ts` times out on a verdict that never renders.

### Teardown

Every process the runner starts is recorded by PID and killed by that PID.
Nothing is ever matched by name or path: `pkill -f target/release/government-api` has destroyed the captain's live service three times, because every checkout of this monorepo builds the same binary to the same relative path.

If a port it wants is occupied it **stops and says so** rather than clearing it.

bash defers a trap until the current foreground command returns, so a hard kill during a long `playwright test` can leave servers behind.
For that case the runner keeps a ledger at `.e2e-web.pids` (gitignored):

```bash
scripts/e2e-web.sh --cleanup
```

It re-verifies each recorded PID's command line before killing it, so a recycled PID is left alone.

### Not included

`stacks/admin/web` has no Playwright suite - its coverage is the mounted unit suite under `stacks/admin/web/test`, which runs in `pnpm test`.

## `make e2e-ios` / `make e2e-android`

Both build a **debug** build (the ZK self-test card is `#if DEBUG` / `BuildConfig.DEBUG`, so a release build proves nothing), install it, and run every flow in `apps/<platform>/maestro/`.

**iOS never runs `xcodegen`.**
The committed `project.pbxproj` references the two gitignored proving artifacts as bundle resources, and xcodegen enumerates the `DogTag/` folder - so regenerating in a checkout that has not vendored them silently drops both Copy-Bundle-Resources entries, and the app then builds clean and proves with nothing.

**Android checks the ABI rather than assuming it.**
The prover ships only `arm64-v8a`/`armeabi-v7a`, so an x86_64 emulator cannot load it; it would fail deep inside the flow with an `UnsatisfiedLinkError` that reads like a product defect rather than like the wrong emulator.

### A flow that needs a real deployment is excluded, and SAID

`apps/ios/maestro/nearby_result_rows.yaml` is tagged `requires-deployment`: it asserts rendered
provider rows, and the directory host is the fixed production constant `AppConfig.centralApi` with no
debug override, so a dev machine cannot render one.
It used to live below a divider inside `nearby_scope_separation.yaml`, which meant a local run of the
half that CAN run always went red on the half that cannot - a could-not-run wearing the costume of a
failure, and a red nobody can act on is a red everybody learns to ignore.

The runner excludes that tag by default and **prints what it left out**, with the reason and the flag
that runs it (`--include-deployment`).
Excluded is not silent: a run that bounds its coverage has to log what it dropped, or "OK" reads as
"covered everything".

### A Maestro HARNESS failure is a could-not-run, not a flow failure

Maestro's Android driver talks to an on-device gRPC server, and when that server does not come up the
run reports `[Failed] <flow> (0s)` - indistinguishable on the console from a flow whose first assertion
was false.
Those are different answers: one is a finding about the product, the other about the machine, and
reporting the second as the first sends a reader hunting a regression that does not exist.
So the runner passes `--debug-output` to a directory it controls, and a narrow set of signatures
(`Not able to reach the gRPC server`, `StatusRuntimeException: UNAVAILABLE`, driver-launch failures)
turns the run into an exit-78 could-not-run that quotes the line it matched.
The set is deliberately narrow, like `ensure-ts-prereqs.sh`'s: stamping a real break "environment"
teaches a reader to wave the next one through, which is the worse error of the two.

**KNOWN, on this lab machine: the Android flow could not be completed.**
The runner does everything up to and including installing the APK on an arm64 emulator, and then
Maestro 2.6.1 either stalls at driver setup or fails with `io.grpc.StatusRuntimeException: UNAVAILABLE`
- four attempts, including after uninstalling and letting it reinstall its driver cleanly.
That is Maestro against this AVD, outside the flow and outside the runner.
It is recorded here rather than papered over: **the Android pass path is unverified**, and the CI job
for it remains `workflow_dispatch`-only on a self-hosted runner.
What IS verified is every step up to the handoff, plus both honest refusals - no device (78) and the
deadline (a bounded, loud stop rather than an indefinite wait).

### A hang is a failure, not a wait

Maestro is run under a deadline (600s iOS, 900s Android) and stopped by recorded PID if it overruns.
This was found by using the runner, not by imagining it: Maestro 2.6.1 stalled on a cold Android
emulator after logging `Selected device` and installing its driver, and made no progress for over half
an hour.
Without a deadline the runner waits with it forever, printing nothing - the worst outcome here, because
it is not a wrong answer but *no* answer.
`timeout(1)` is deliberately not used: it is GNU coreutils and absent from a stock macOS, so relying on
it would make the guard silently do nothing on the platform this repo is developed on.

`ANDROID_HOME` is *resolved* (environment, then the conventional locations) rather than required - `adb` lives at `~/Library/Android/sdk` on a normal macOS install while the variable is routinely unset, so demanding it would refuse a machine that is perfectly capable.

### What each refuses on

| missing | reported as |
|---|---|
| Maestro not on PATH | 78, with the install command |
| no simulator / no connected device | 78, and Android lists the AVDs that exist on the machine |
| an unauthorized adb device | 78, distinguished from "none attached" |
| a non-ARM device | 78, naming the ABI it found |
| `consent_final.zkey` / `consent.graph` / `roax.json` absent | 78, naming the file and `make vendor-mobile-artifacts` or `make gen-mobile-config` |
| `DogTagFFI.xcframework` absent | 78, with the cargo + `xcodebuild -create-xcframework` recipe |
| `jniLibs/arm64-v8a/*.so` absent | 78, with the `cargo ndk` line, including `--features prover` |

Those artifacts are all gitignored, so a fresh checkout has none of them and the refusal path is the normal first experience.
A build failure *after* the preflight is reported as a **code finding** (exit 1), because every prerequisite was established before that point.

## Adding a spec

Keep it in the portal's existing `e2e/` directory - the runner discovers whatever is there through each package's own `playwright.config.ts`, so a new spec file needs no wiring.

If it needs the backend mocked, use the existing `page.route(/^https?:\/\/[^/]+\/api\//, ...)` shape.
A `**/api/**` glob wrongly swallows `@dogtag/ui`'s `src/api/*.ts` module scripts and breaks the mount.

**A fake must be keyed on the contract it is asked about**, not merely on the selector.
A fake that ignores which address it is dialled at cannot model "the hostile contract answers `true` while the clone the factory named answers `false`", which is how forged-issuer tests pass for the wrong reason.

**An unmodelled read must throw, not answer empty.**
`verify-credential.spec.ts` does this and it is why the suite is worth having: when rights became a bitmask, its `RightsSet` log went unmodelled, and answering `[]` would have read as "the registry recorded no grant" - a definite refusal of a genuine credential, an invented accusation rather than an invented pass, and no better for it.
Throwing turned a silent wrong verdict into a message naming the topic to add.
