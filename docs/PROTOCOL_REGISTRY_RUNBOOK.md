# ProtocolRegistry deploy + publish runbook (M-4)

Deploying the `ProtocolRegistry` and publishing the single owner-hidden protocol version.
That version is keyed by the internal version string `dogtag-levelb/1` (artifact axis: `dogtag-levelb-artifacts/1`) - an internal identifier, not a product label.
Its keccak keys the on-chain registry, so the string is never renamed.

**Status: NOT PUBLISHED on the launch set.**
`ProtocolRegistry` is deployed - its address is in `contracts/deployments/roax.json` - but it carries no published discovery set, and this runbook has not been run against it.
Verified on chain 2026-08-03: `getDiscoverySet(keccak256("dogtag-levelb/1"))` reverts with the contract's own named reason **`"unknown discovery set"`**, which is a deliberate fail-closed answer rather than an error.
(Note the getter is `getDiscoverySet`. Calling a name the contract does not have - `getContractSet`, say - also reverts, but with empty returndata, and reading THAT as "nothing is published" would be a dispatcher refusal mistaken for an answer.)
The ledger's `_publication` note records the same thing.

An earlier registry did carry a published, active `dogtag-levelb/1` on both axes; that instance is superseded and is not a write target.
Read any "EXECUTED" statement about publication as describing that earlier instance, not this one.

Consequence, and it is the intended state rather than a gap to paper over: an app validating a platform's version claim against this anchor resolves nothing and **fails closed**.
That is correct for an unpublished deployment - a discovery anchor that answered before anyone published would be asserting a version nobody staged.

This document is the reproducible procedure for publishing.
Executing it requires the governance/publisher key and is the captain's to authorize and run.

**The deployed `PUBLISH_TIMELOCK` is 3600 seconds (1 hour)**, read from the chain on 2026-08-03, and it is IMMUTABLE.
So propose and execute are separated by a real hour on this deployment - plan for the wait rather than expecting an immediate execute.

## Why this is the long pole

The app-side anchor validation (M-4 PR3) resolves its `TrustedAnchor` from
`ProtocolRegistry.getDiscoverySet` + `getActiveArtifactSet`.
There is no alternative source: the signed-manifest fallback is not usable because
`DOGTAG_MANIFEST_PUBKEY_HEX` is `None` (`crates/dogtag-prover-rs/src/manifest.rs`) and no client
fetches `/protocol/manifest`.
On-chain resolution is therefore the only anchor path.

Publication is a **two-phase, timelocked** operation. Mainnet keeps the full 2-day governance window;
this ROAX deployment sits at the contract's 1-hour floor, so the two transactions are an hour apart.

### The timelock is immutable and selected at deploy time

`ProtocolRegistry` stores `PUBLISH_TIMELOCK` as an immutable constructor value. The deploy script
provides the safe environment policy around that value:

- `PUBLISH_TIMELOCK_SECS` defaults to `172800` (2 days).
- Without `TESTNET_DEPLOY=true`, the script **requires exactly 2 days** and refuses zero, short, or
  otherwise non-default values. This is the loud mainnet guard; never set `TESTNET_DEPLOY` for a
  mainnet deployment.
- With `TESTNET_DEPLOY=true`, a testnet may choose a shorter value **but never zero**: the CONSTRUCTOR
  enforces a 1-hour floor (`MIN_PUBLISH_TIMELOCK_SECONDS`), so a zero-delay registry is now
  unrepresentable rather than merely discouraged. This ROAX deployment sits at that floor, 3600s.

The selected value cannot be changed after deployment. The admin-transfer timelock remains a separate
fixed 2-day governance control and is not affected by these variables.

## Prerequisites

- `PUBLISHER_KEY` / `GOV_KEY` for the Phase-2 governance authority (`0x8E27E117…`, `deployments/roax.json` `_governance`).
- `ROAX_RPC` pointing at the ROAX endpoint.
- Foundry installed.
- **The `minAppVersion` number must be locked first** — see "Version coherence" below.

## Step 1 — deploy the registry

**There is no separate registry-deploy script any more.** `DeployProtocolRegistry.s.sol` was removed;
`Deploy.s.sol` stands the registry up together with the rest of the launch set, and it is the same
script that carries the timelock policy described above (`PUBLISH_TIMELOCK_SECS`, `TESTNET_DEPLOY`).

So on an already-deployed chain there is nothing to do in this step - take the address from the
ledger:

```sh
export PROTOCOL_REGISTRY=$(python3 -c "import json;print(json.load(open('contracts/deployments/roax.json'))['ProtocolRegistry'])")
```

Deploying a NEW chain's registry means running `Deploy.s.sol`, which deploys all ten contracts; see
`docs/DEPLOY.md`. On mainnet, verify the script's printed delay says `172800 seconds` and testnet mode
`false` before recording anything.

## Step 2 — propose (starts the configured timelock)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

This stages **three** records in one batch - the `dogtag-levelb/1` contract set, its
`dogtag-levelb-artifacts/1` artifact set, and their binding. Their timelocks run **concurrently**, so
this is still a two-phase rollout, not three sequential waits.
On this deployment every ETA is the proposal block timestamp plus 3600 seconds.

The script prints each ETA. Record them; Phase 2 is invalid before the latest one elapses.

## Step 3 — execute (once the printed ETAs are reached)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

Sets are executed before bindings, because `executeArtifactBinding` requires both sides to already be
published. On this deployment that is an hour after Step 2 confirms; mainnet must wait the full 2 days. The script echoes back `active` and `minAppVersion` for `dogtag-levelb/1` — check them.

## Verification

```sh
cast call $PROTOCOL_REGISTRY "getDiscoverySet(bytes32)" $(cast keccak "dogtag-levelb/1") --rpc-url $ROAX_RPC
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
being gated.
(The `levelB*` / `LEVEL_B_*` code identifiers mirror the internal version key `dogtag-levelb/1` and,
like it, are never renamed.)
M-4 PR4 locks all four values to **`1.4.0`**. Step 2 must publish that exact floor;
re-publishing a corrected `minAppVersion` costs a fresh propose plus the registry's immutable
timelock (2 days on mainnet; 1 hour on this ROAX deployment).

## Rotating artifacts later

A zkey rotation does **not** re-run this script. It is `proposeArtifactSet(<new …-artifacts/2>)` +
`proposeArtifactBinding(levelBId(), <new id>)`, the timelock, then the two executes. No `ContractSet`
is written, so no trio address moves and nothing is redeployed.
