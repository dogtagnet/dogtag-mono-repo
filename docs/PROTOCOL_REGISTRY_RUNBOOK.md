# ProtocolRegistry deploy + publish runbook (M-4)

Deploying the `ProtocolRegistry` and publishing the `dogtag-levela/1` + `dogtag-levelb/1` records.

**Status: NOT YET RUN.**
`ProtocolRegistry` is absent from `contracts/deployments/roax.json`, so no anchor exists on ROAX today.

This document is the prepared procedure only.
Executing it requires the governance/publisher key and is the captain's to authorize and run.

## Why this is the long pole

The app-side anchor validation (M-4 PR3) resolves its `TrustedAnchor` from
`ProtocolRegistry.getContractSet` + `getActiveArtifactSet`.
There is no alternative source: the signed-manifest fallback is not usable because
`DOGTAG_MANIFEST_PUBKEY_HEX` is `None` (`crates/dogtag-prover-rs/src/manifest.rs`) and no client
fetches `/protocol/manifest`.
On-chain resolution is therefore the only anchor path.

Publication is a **two-phase, timelocked** operation, so it cannot be completed in one sitting.

### The timelock is FIXED at 2 days, not configurable

Checked as part of M-4, because a shortened testnet timelock would remove the wall-clock gate:

- `PUBLISH_TIMELOCK` is `uint256 public constant = 2 days` (`contracts/src/ProtocolRegistry.sol`).
- The constructor takes only `(admin, publisher)` — no timelock parameter.
- `DeployProtocolRegistry.s.sol` exposes only the `ADMIN` and `PUBLISHER` env vars.

So there is **no deploy-time knob**. Shortening it for ROAX would mean editing a governance-critical
contract to add an immutable constructor parameter, which would also make testnet diverge from
production governance semantics. The recommendation is **not** to do that, and to schedule the 2-day
wait instead.

Note this does **not** block local development or CI: an Anvil-based e2e can advance chain time
directly. The 2-day wait applies only to the real ROAX publication.

## Prerequisites

- `PUBLISHER_KEY` / `GOV_KEY` for the Phase-2 governance authority (`0x8E27E117…`, `deployments/roax.json` `_governance`).
- `ROAX_RPC` pointing at the ROAX endpoint.
- Foundry installed.
- **The `minAppVersion` number must be locked first** — see "Version coherence" below.

## Step 1 — deploy the registry

```sh
forge script contracts/script/DeployProtocolRegistry.s.sol:DeployProtocolRegistry \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
```

`admin` and `publisher` both default to the governance authority; override with the `ADMIN` /
`PUBLISHER` env vars if they must differ.

Then record the deployed address in `contracts/deployments/roax.json` under `ProtocolRegistry`, and
export it for the next steps:

```sh
export PROTOCOL_REGISTRY=<deployed address>
```

## Step 2 — propose (starts the 2-day timelock)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

This stages **six** records in one batch — both contract sets, both artifact sets, and both bindings.
Their timelocks run **concurrently**, so this is still a two-phase rollout, not six sequential waits.

The script prints each ETA. Record them; Phase 2 is invalid before the latest one elapses.

## Step 3 — execute (after the timelock elapses)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

Sets are executed before bindings, because `executeArtifactBinding` requires both sides to already be
published. The script echoes back `active` and `minAppVersion` for both levels — check them.

## Verification

```sh
cast call $PROTOCOL_REGISTRY "getContractSet(bytes32)" $(cast keccak "dogtag-levelb/1") --rpc-url $ROAX_RPC
cast call $PROTOCOL_REGISTRY "getActiveArtifactSet(bytes32)" $(cast keccak "dogtag-levelb/1") --rpc-url $ROAX_RPC
```

Both `active` bits must read true. The app's validator requires **both** independently and fails
closed if either is false.

## Version coherence — lock this number before Step 2

Four places must agree, or the app fails closed with `AppTooOld`:

| Where | What |
|---|---|
| `apps/ios/project.yml` | `MARKETING_VERSION` |
| `apps/android/app/build.gradle.kts` | `versionName` |
| `contracts/script/ProtocolVersions.sol` | `levelBArtifacts().minAppVersion` |
| `crates/dogtag-prover-rs/src/manifest.rs` | `LEVEL_B_ARTIFACT_RELEASE.min_app_version` |

The last two are mirrors of each other and are what gets published here; the first two are the build
being gated. The app version is chosen in M-4 PR4 — **do not run Step 2 until it is locked**, since
re-publishing a corrected `minAppVersion` costs a fresh propose plus the full 2-day timelock.

Today's published floor is `1.4.0` while both apps build as `0.1`, so every app would currently fail
closed. PR4 resolves this.

## Rotating artifacts later

A zkey rotation does **not** re-run this script. It is `proposeArtifactSet(<new …-artifacts/2>)` +
`proposeArtifactBinding(levelBId(), <new id>)`, the timelock, then the two executes. No `ContractSet`
is written, so no trio address moves and nothing is redeployed.
