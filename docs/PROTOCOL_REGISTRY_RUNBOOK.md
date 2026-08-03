# ProtocolRegistry deploy + publish runbook (M-4)

Deploying the `ProtocolRegistry` and publishing the single owner-hidden protocol version.
That version is keyed by the internal version string `dogtag-levelb/1` (artifact axis: `dogtag-levelb-artifacts/1`) - an internal identifier, not a product label.
Its keccak keys the on-chain registry, so the string is never renamed.

**Status: PUBLISHED on the launch set.**
`dogtag-levelb/1` is live and active on both axes with its binding, executed 2026-08-03.
`getDiscoverySet(keccak256("dogtag-levelb/1"))` returns a nine-word record; `getActiveArtifactSet` resolves; `minAppVersion` is `1.4.0`.
The registry's address, the publish transaction hashes and the reasoning are in `contracts/deployments/roax.json` (`_publication`, `_protocol_registry_redeploy`); they are deliberately not copied into this file.

Note the getter is `getDiscoverySet`.
Calling a name the contract does not have - `getContractSet`, say - reverts too, but at the DISPATCHER with empty returndata, and reading that as "nothing is published" would be a dispatcher refusal mistaken for an answer.
The named reason **`"unknown discovery set"`** is what an UNKNOWN key returns from this contract: a deliberate fail-closed answer rather than an error, and the thing to compare against when you check a key that should not be there.

The registry itself was REDEPLOYED before publishing, and the earlier instance is not a write target.
`PUBLISH_TIMELOCK` is immutable, and the original carried the then-mandatory 1-hour floor, which put an hour between propose and execute on every testnet iteration.

This document is the reproducible procedure for publishing.
Executing it requires the governance/publisher key and is the captain's to authorize and run.

**The steps below have already been run against this deployment**, which is what the status above records.
They are kept in the imperative because they are both the reproducible record of that run and the runbook for the next registry - but do not walk them here expecting a first publication.
`executeDiscoverySet` assigns unconditionally, so a re-run is an IN-PLACE re-publish: it restamps `publishedAt` and re-emits the event without adding a list entry.
That is a deliberate operation (publishing an omitted identity, say), never something to reach by following a runbook.

**The deployed `PUBLISH_TIMELOCK` is 0**, per the captain's 2026-08-03 ruling that testnet waits not at all and production keeps 2 days.
So propose and execute land back to back on this deployment - no wait to plan for.
The floor that used to make zero unrepresentable moved off the contract to `Deploy.validatePublishTimelock`; see "The timelock is immutable and selected at deploy time" below.

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
- With `TESTNET_DEPLOY=true`, a testnet may choose ANY value **including zero**. `MIN_PUBLISH_TIMELOCK`
  is 0, so a zero-delay registry is representable and this ROAX deployment uses it: a development chain
  deploys, publishes, tests and redeploys in one sitting, and a floor there buys nothing while costing
  every iteration. The guard is a RELOCATION, not a removal - it used to sit on the contract because a
  wrong immutable value could only be repaired by replacing the registry, and replacing it is routine
  now that a mobile rebuild-and-reinstall accompanies every full redeploy as standing process. The cost
  is stated rather than hidden: a direct `forge create` bypassing the script can pick any delay on any
  chain, and the script is the only production guard where it used to be defence in depth.

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
On this deployment `PUBLISH_TIMELOCK` is 0, so every ETA is the proposal block timestamp itself and Phase 2 follows immediately.

The script prints each ETA. On a deployment with a real delay, record them; Phase 2 is invalid before the latest one elapses.

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
timelock (2 days on mainnet; zero on this ROAX deployment, so a correction is one sitting).

## Rotating artifacts later

A zkey rotation does **not** re-run this script. It is `proposeArtifactSet(<new …-artifacts/2>)` +
`proposeArtifactBinding(levelBId(), <new id>)`, the timelock, then the two executes. No `ContractSet`
is written, so no trio address moves and nothing is redeployed.
