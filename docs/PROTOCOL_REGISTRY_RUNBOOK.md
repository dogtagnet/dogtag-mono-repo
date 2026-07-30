# ProtocolRegistry deploy + publish runbook (M-4)

Deploying the `ProtocolRegistry` and publishing the single owner-hidden protocol version.
That version is keyed by the internal version string `dogtag-levelb/1` (artifact axis: `dogtag-levelb-artifacts/1`) - an internal identifier, not a product label.
Its keccak keys the on-chain registry, so the string is never renamed.

**Status: EXECUTED on ROAX (2026-07-23, the r8 fresh redeploy).**
`ProtocolRegistry` is deployed at `0xf5492A671E69b1A13f7Fd123C021830eB1ea8081` (recorded in `contracts/deployments/roax.json`), with `PUBLISH_TIMELOCK = 0` (the explicit testnet opt-in below) and the single version published and **active** on both axes: `dogtag-levelb/1` + `dogtag-levelb-artifacts/1` + their binding (propose → immediate execute).

This document remains the reproducible procedure (a future environment, or a mainnet deploy with the full 2-day timelock).
Executing it requires the governance/publisher key and is the captain's to authorize and run.

**This document is about the DEPLOYED generation-1 registry.**
The generation-2 discovery layer is a separate, built-but-undeployed contract with a different record and a non-zero timelock floor: see `docs/PROTOCOL_REGISTRY_V2.md`.
The `PUBLISH_TIMELOCK = 0` recorded above is the specific defect that registry exists to correct, and it is correctable only at deploy time because the value is immutable.

## Why this is the long pole

The app-side anchor validation (M-4 PR3) resolves its `TrustedAnchor` from
`ProtocolRegistry.getContractSet` + `getActiveArtifactSet`.
There is no alternative source: the signed-manifest fallback is not usable because
`DOGTAG_MANIFEST_PUBKEY_HEX` is `None` (`crates/dogtag-prover-rs/src/manifest.rs`) and no client
fetches `/protocol/manifest`.
On-chain resolution is therefore the only anchor path.

Publication is a **two-phase, timelocked** operation. Mainnet keeps the full 2-day governance window;
the ROAX testnet deploy uses the explicit zero-delay path so propose and execute can run immediately
for iteration (still as two separate transactions).

### The timelock is immutable and selected at deploy time

`ProtocolRegistry` stores `PUBLISH_TIMELOCK` as an immutable constructor value. The deploy script
provides the safe environment policy around that value:

- `PUBLISH_TIMELOCK_SECS` defaults to `172800` (2 days).
- Without `TESTNET_DEPLOY=true`, the script **requires exactly 2 days** and refuses zero, short, or
  otherwise non-default values. This is the loud mainnet guard; never set `TESTNET_DEPLOY` for a
  mainnet deployment.
- With `TESTNET_DEPLOY=true`, a testnet may deliberately choose a shorter value. The ROAX deployment
  uses `PUBLISH_TIMELOCK_SECS=0` for immediate execution.

The selected value cannot be changed after deployment. The admin-transfer timelock remains a separate
fixed 2-day governance control and is not affected by these variables.

## Prerequisites

- `PUBLISHER_KEY` / `GOV_KEY` for the Phase-2 governance authority (`0x8E27E117…`, `deployments/roax.json` `_governance`).
- `ROAX_RPC` pointing at the ROAX endpoint.
- Foundry installed.
- **The `minAppVersion` number must be locked first** — see "Version coherence" below.

## Step 1 — deploy the registry

ROAX testnet (fast path):

```sh
TESTNET_DEPLOY=true PUBLISH_TIMELOCK_SECS=0 \
forge script contracts/script/DeployProtocolRegistry.s.sol:DeployProtocolRegistry \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $GOV_KEY
```

`admin` and `publisher` both default to the governance authority; override with the `ADMIN` /
`PUBLISHER` env vars if they must differ.

Mainnet (safe default; do not set either timelock env var):

```sh
forge script contracts/script/DeployProtocolRegistry.s.sol:DeployProtocolRegistry \
  --rpc-url $MAINNET_RPC --broadcast --legacy --private-key $GOV_KEY
```

The script prints the selected delay and whether testnet mode was enabled. On mainnet, verify the
output says `172800 seconds` and `false` before recording the address.

For the ROAX deployment, record the deployed address in `contracts/deployments/roax.json` under
`ProtocolRegistry`, and export it for the next steps:

```sh
export PROTOCOL_REGISTRY=<deployed address>
```

## Step 2 — propose (starts the configured timelock)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

This stages **three** records in one batch - the `dogtag-levelb/1` contract set, its
`dogtag-levelb-artifacts/1` artifact set, and their binding. Their timelocks run **concurrently**, so
this is still a two-phase rollout, not three sequential waits.
On the ROAX zero-delay deployment, every ETA equals the proposal block timestamp.

The script prints each ETA. Record them; Phase 2 is invalid before the latest one elapses.

## Step 3 — execute (once the printed ETAs are reached)

```sh
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

Sets are executed before bindings, because `executeArtifactBinding` requires both sides to already be
published. With the ROAX zero-delay deploy, run this immediately after Step 2 confirms. Mainnet must
wait the full 2 days. The script echoes back `active` and `minAppVersion` for `dogtag-levelb/1` — check them.

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
being gated.
(The `levelB*` / `LEVEL_B_*` code identifiers mirror the internal version key `dogtag-levelb/1` and,
like it, are never renamed.)
M-4 PR4 locks all four values to **`1.4.0`**. Step 2 must publish that exact floor;
re-publishing a corrected `minAppVersion` costs a fresh propose plus the registry's immutable
timelock (2 days on mainnet; immediate on the ROAX fast-path deployment).

## Rotating artifacts later

A zkey rotation does **not** re-run this script. It is `proposeArtifactSet(<new …-artifacts/2>)` +
`proposeArtifactBinding(levelBId(), <new id>)`, the timelock, then the two executes. No `ContractSet`
is written, so no trio address moves and nothing is redeployed.
