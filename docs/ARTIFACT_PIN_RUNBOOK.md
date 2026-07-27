# Pinning the witness graph on-chain — operator runbook

This runbook covers ONE change: moving the owner-hidden witness graph (`consent.graph`) from
**unpinned** to **pinned**, on-chain and in the descriptor, as a single atomic step.

It is written to be executed by the operator who holds the publisher key.
Nothing in this file has been run.

---

## Why this is a runbook and not a code change

The graph is now committed and its bytes are attested in-repo
(`dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256`, enforced by
`graph_file_matches_attested_sha256`).
That closes the "which graph did this app prove with?" gap for anyone reading the repo.

What it does **not** do is put the graph's identity on-chain.
The published `ArtifactSet` still carries `witnessMobileSha256 = 0`, which
`contracts/src/ProtocolRegistry.sol:129` defines as *unpinned*.

The descriptor field and the on-chain field are in **deliberate lockstep**, and the coupling is
load-bearing:

```
artifact.rs  witness_graph.sha256
  -> manifest.rs:209   Manifest.witness_mobile_sha256
  -> manifest.rs:433   reconcile() via cmp_opt
```

`cmp_opt` treats `(Some, None)` as a **conflict**.
So flipping the descriptor alone would make the signed manifest advertise a pin the chain does not
carry, and every reconcile would report a disagreement that is really just a half-applied rollout.
Flipping the chain alone would leave the manifest silent about an artifact the anchor now pins.

Hence: **one atomic step, both sides, or neither.**

---

## The value being published

| | |
|---|---|
| artifact | `circuits/build/consent.graph` |
| size | 1,546,215 bytes |
| SHA-256 | `2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793` |
| artifact set | `dogtag-levelb-artifacts/1` (`keccak256`) |

Re-derive it yourself rather than trusting this file:

```bash
shasum -a 256 circuits/build/consent.graph
```

It must equal the value above and
`crates/dogtag-prover-rs/src/artifact.rs::LEVEL_B_V1_WITNESS_GRAPH_SHA256`.
`cargo test -p dogtag-prover-rs --lib artifact::` asserts exactly that, so a green run is the check.

---

## Step 0 — if the GRAPH ITSELF changed (rotation)

Skip this if you are only publishing the existing graph's hash.

If `consent.circom` changed and the graph was rebuilt, the attested hash moves. It lives in **one**
place by design — `LEVEL_B_V1_WITNESS_GRAPH_SHA256` in
`crates/dogtag-prover-rs/src/artifact.rs`. `scripts/vendor-mobile-artifacts.sh` reads it from there
rather than duplicating it, so there is no second literal to forget.

```bash
shasum -a 256 circuits/build/consent.graph      # new value
# update LEVEL_B_V1_WITNESS_GRAPH_SHA256 in crates/dogtag-prover-rs/src/artifact.rs
cargo test -p dogtag-prover-rs --lib artifact::  # graph_file_matches_attested_sha256 must pass
git add -f circuits/build/consent.graph
```

Also re-check the doc-level copies of the value, which are prose and not read by any code:
`docs/MOBILE_BUILD.md` §4 and this file's table below.

A rebuilt graph also means the zkey/VK may have moved — if the circuit changed, the whole artifact
set rotates, not just the graph, and that is a `…-artifacts/2` publication (see Step 2).

## Step 1 — flip the descriptor pin

In `crates/dogtag-prover-rs/src/artifact.rs`, inside `LEVEL_B_V1_DESCRIPTOR`:

```rust
    witness_graph: ArtifactFile {
        rel_path: "consent.graph",
        sha256: Some(LEVEL_B_V1_WITNESS_GRAPH_SHA256),
    },
```

`descriptor_graph_pin_agrees_with_the_file` then enforces descriptor↔file agreement automatically —
a stale or hand-typed hash fails there rather than in the field.

One test asserts the CURRENT unpinned state and must be updated in the same commit:

- `crates/dogtag-prover-rs/src/manifest.rs` — `manifest_pins_come_from_the_descriptor` asserts
  `m.witness_mobile_sha256 == None`. It becomes
  `Some(artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256)`.

Update it to assert the new truth. Do **not** relax it into accepting either state — that would
retire the only check that keeps the two sides in lockstep.

## Step 2 — publish the artifact set on-chain

`PublishProtocolVersions.s.sol` takes every pin from a mandatory env var (no stale-network
fallbacks). The only value that changes is `CONSENT_WITNESS_MOBILE_SHA256`, which is currently `0`:

```bash
cd contracts

export CONSENT_ZKEY_SHA256=0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868
export CONSENT_WITNESS_MOBILE_SHA256=0x2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793
export CONSENT_R1CS_SHA256=0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7
export CONSENT_WASM_SHA256=0x482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6
export DOGTAG_ARTIFACTS_URL=...   # unchanged; must match what is already published

forge script script/PublishProtocolVersions.s.sol \
  --rpc-url "${ROAX_RPC:-https://devrpc.roax.net}" \
  --broadcast
```

Confirm the remaining three hashes still match the descriptor before broadcasting — they are
unchanged by this work, but publishing re-states all four:

```bash
shasum -a 256 circuits/build/consent_final.zkey circuits/build/consent.r1cs \
              circuits/build/consent_js/consent.wasm
```

### Rotation, not mutation

Whether this is an in-place re-publish of `dogtag-levelb-artifacts/1` or a NEW
`…-artifacts/2` + re-pointed binding is a protocol-lifecycle decision, and it is the reason this
step is not scripted here.

`ProtocolVersions.sol` documents the intended shape: *"Rotating a zkey means authoring a NEW artifact
set (`…-artifacts/2`) and re-pointing the binding."*
Pinning the graph does not change any artifact's bytes — it only publishes an identity that was
previously omitted — so an in-place re-publish is defensible.
But if the deployed registry treats a published set as immutable, this needs `…-artifacts/2` and a
binding update, and then `artifactSetId` changes, which the apps DO decode
(`AnchorResolver.kt` / `.swift` read `artifactSetId` and `minAppVersion`).

**Check the deployed registry's publish semantics before choosing.** Getting this wrong is the one
step here that can strand an app on an anchor it cannot resolve.

## Step 3 — verify

```bash
cargo test -p dogtag-prover-rs                       # descriptor + manifest agree
cd contracts && forge test --match-contract ProtocolRegistry
```

Then read the published set back from the chain and confirm `witnessMobileSha256` is the value
above, not `0`.

---

## What this still does NOT give you

Pinning makes the graph's identity **published**. It does not make it **app-enforced**.

The mobile resolvers do not verify the bundled graph against the anchor, by design:
`AGENTS.md` records that bundled-artifact integrity is the package signature's job, not a runtime
hash check, and `ZkeyAsset.kt` / `ZkeyAsset.swift` carry no hash fields.
The apps also do not currently decode `witnessMobileSha256` at all — `AnchorResolver` reads only
`artifactSetId`, `minAppVersion` and `active`.

So after this runbook the chain states which graph is authoritative, and the repo attests which
graph is committed, but an app shipping a divergent graph would not detect it at runtime.
Closing that requires a runtime hash check on the mobile side, which is a deliberate reversal of a
documented design decision and belongs in its own change.
