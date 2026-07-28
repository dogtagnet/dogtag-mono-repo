# Pinning the witness graph on-chain — operator runbook

This runbook covers ONE change: moving the owner-hidden witness graph (`consent.graph`) from
**unpinned** to **pinned**, on-chain and in the descriptor, as a single atomic step.

It is written to be executed by the operator who holds the publisher key.

**STATUS: EXECUTED on ROAX (chainId 135) on 2026-07-28.**
The graph is pinned on chain and in the descriptor, both sides in the same change.
What follows is kept as the procedure for the NEXT rotation, annotated with what the first run found.

| | |
|---|---|
| decision | in-place re-publish of `dogtag-levelb-artifacts/1` (NOT `…-artifacts/2`) |
| `proposeArtifactSet` | `0xa72f8a3bfcdf07f75c85e91cc5ae175dd255bdbfe03c497e609b480e36343576` (block 292176) |
| `executeArtifactSet` | `0xf052f1ffcdb511fd50b5cc800864aefb8c84c366494dd69600d35ac525928f7c` (block 292176) |
| signer | governance `0x8E27E117…` (holds `PUBLISHER_ROLE`; the deployer EOA does not) |
| script | `contracts/script/PinConsentWitnessGraph.s.sol` - artifact axis ONLY, two transactions |
| read back | `witnessMobileSha256 == 0x2f74d26b…f793`, `artifactSetId` unchanged, `active` still true |
| collateral | `ContractSet` untouched (`publishedAt` still `1784787356`), `artifactSetCount` still 1, binding unchanged |

The re-publish restamped the artifact set's own `publishedAt` from `1784787356` to `1785254312`.
That is inherent to re-publishing and is why the original stamp is recorded here - it is otherwise gone.

---

## Why this is a runbook and not a code change

The graph is committed and its bytes are attested in-repo
(`dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256`, enforced by
`graph_file_matches_attested_sha256`).
That closes the "which graph did this app prove with?" gap for anyone reading the repo.

What it did **not** do was put the graph's identity on-chain: before the run recorded above, the
published `ArtifactSet` carried `witnessMobileSha256 = 0`, which
`contracts/src/ProtocolRegistry.sol:129` defines as *unpinned*.
It now carries `2f74d26b…f793`, so that gap is closed for a reader of the CHAIN too.
The paragraphs below are why the two had to move together, and they govern the next rotation exactly
as they governed this one.

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

Assertions of the OLD unpinned state must move in the same commit. Update each to assert the new
truth; do **not** relax one into accepting either state - that would retire the only check keeping the
two sides in lockstep. The 2026-07-28 run touched:

- `crates/dogtag-prover-rs/src/manifest.rs` - `manifest_pins_come_from_the_descriptor` asserted
  `m.witness_mobile_sha256 == None`; it became `Some(artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256)`
  (the exact hash, not `is_some()`).
- `apps/ios/DogTagTests/AnchorResolverTests.swift` - `testDecodeArtifactSetGolden`'s golden blob is
  documented as the EXACT bytes `getActiveArtifactSet` returns, so its head-word 2 and the comment
  calling the graph "published 0/unpinned" both went stale. Substituting the real hash is offset-safe
  (a fixed-width head word) and nothing asserts on it - `AnchorResolver` decodes only `artifactSetId`,
  `minAppVersion` and `active`.

Two that deliberately did NOT change, so a future run does not "fix" them:

- `crates/dogtag-prover-rs/src/artifact.rs` - `descriptor_graph_pin_agrees_with_the_file` already
  handled both arms; the `Some` arm simply became the live one. Keep the `None` arm: a rotation passes
  back through it.
- `contracts/test/ProtocolRegistry.t.sol` - `test_propose_allows_unpinned_graph` passes its own local
  `bytes32(0)` and asserts the CONTRACT still accepts an unpinned graph. That property is unchanged by
  what ROAX happens to publish.

Verify with `cargo test -p dogtag-prover-rs` **and** `cargo test -p vet-api --test discovery_validation`
(the reconcile path), plus the iOS `DogTagTests` scheme if the fixture moved.

### Ordering

Publish on chain FIRST, read the set back, then land the descriptor. The reverse leaves the signed
manifest advertising a pin the chain does not carry, which every reconcile reports as a conflict.
The window between the two is real but currently harmless - nothing in production consumes the
reconcile path - so close it in the same task rather than relying on that.

## Step 2 — publish the artifact set on-chain

**Do NOT use `PublishProtocolVersions.s.sol` for this.** It is the FIRST-ROLLOUT script and publishes
the version on BOTH axes plus the binding: its Propose phase sends `proposeContractSet` +
`proposeArtifactSet` + `proposeArtifactBinding` and its Execute phase the three matching executes -
**six transactions**, four of which re-publish a `ContractSet` and a binding that a graph pin does not
change. That is not cosmetic: `executeContractSet` restamps `publishedAt` and re-emits
`ContractSetPublished`, rewriting the trio's on-chain provenance for a change that moves no trio
address. (The command previously printed here also named no contract and no phase, so it could not
have run as written.)

Use `contracts/script/PinConsentWitnessGraph.s.sol`, which sends exactly **two** transactions, both on
the artifact axis. It takes every pin from a mandatory env var (no stale-network fallbacks); the only
value that changes is `CONSENT_WITNESS_MOBILE_SHA256`.

Its guard is the part worth keeping: because a re-publish restates ALL of the set's fields, the script
reads the CURRENT on-chain record and refuses to broadcast unless every field except
`witnessMobileSha256` is unchanged and the graph is currently unpinned. A stale env var therefore
cannot rewrite a pin or the base URL under cover of "pinning the graph", and the script is structurally
incapable of performing a rotation.

```bash
cd contracts

export PROTOCOL_REGISTRY=0xf5492A671E69b1A13f7Fd123C021830eB1ea8081   # deployments/roax.json
export CONSENT_ZKEY_SHA256=0xf83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868
export CONSENT_WITNESS_MOBILE_SHA256=0x2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793
export CONSENT_R1CS_SHA256=0x828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7
export CONSENT_WASM_SHA256=0x482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6
export DOGTAG_ARTIFACTS_URL=https://artifacts.dogtag.io/levelb1   # must match what is already published

forge script script/PinConsentWitnessGraph.s.sol:PinConsentWitnessGraph \
  --rpc-url "${ROAX_RPC:-https://devrpc.roax.net}" \
  --broadcast --legacy --private-key "$GOVERNANCE_PRIVATE_KEY"
```

ROAX needs `--legacy`, and the signer must hold `PUBLISHER_ROLE` - that is the governance key
`0x8E27E117…`, **not** the deployer EOA, which does not hold it. Note `PUBLISHER_ROLE` is
`keccak256("PUBLISHER")`, not `keccak256("PUBLISHER_ROLE")`; deriving it from the variable name gives a
role nobody holds and makes a correctly-configured signer look unauthorized.

Where the publish timelock is non-zero the script proposes only, prints the ETA, and says the pin is
NOT live until a later `executeArtifactSet`. ROAX was deployed with the zero-timelock testnet opt-in
(`PUBLISH_TIMELOCK() == 0`, verified on the deployed contract), so both ran back to back.

Confirm the remaining three hashes still match the descriptor before broadcasting — they are
unchanged by this work, but publishing re-states all four:

```bash
shasum -a 256 circuits/build/consent_final.zkey circuits/build/consent.r1cs \
              circuits/build/consent_js/consent.wasm
```

### Rotation, not mutation - RESOLVED: in-place re-publish

Whether this is an in-place re-publish of `dogtag-levelb-artifacts/1` or a NEW
`…-artifacts/2` + re-pointed binding is a protocol-lifecycle decision, and it is why this step was
left unscripted.

**For a graph PIN it is an in-place re-publish**, decided 2026-07-28 on this evidence:

* The deployed registry does NOT treat a published set as immutable. `executeArtifactSet` assigns
  `artifactSets[id] = a` unconditionally and uses `isNew` only to decide whether to append to
  `artifactSetList`; `ArtifactSetPublished(id, isNew)` exists precisely to tell a re-publish from a
  first publication, and `contractSetList`'s own doc says "a swap-republish does not duplicate its id".
* That was confirmed against the DEPLOYED bytecode, not just the source, by rehearsing the exact two
  transactions on an `anvil --fork-url` of ROAX before broadcasting: the set came back with the graph
  pinned, `artifactSetId` unchanged, `artifactSetCount` still 1, the binding untouched and the
  `ContractSet` untouched. The live run then reproduced that read-back exactly.
* Pinning changes NO artifact's bytes - it publishes an identity that was previously omitted - so
  nothing an app fetches is different afterwards.
* `…-artifacts/2` would MOVE `artifactSetId`, which both mobile `AnchorResolver`s decode, and would
  require a binding update. That is real strand-the-app risk bought for no benefit.

**A genuine zkey/circuit rotation is still `…-artifacts/2` + a re-pointed binding**, exactly as
`ProtocolVersions.sol` says - different bytes, so apps must be able to tell the sets apart. Do not
generalise the in-place decision above beyond the pin-an-omitted-identity case.

**Check the deployed registry's publish semantics before choosing**, rather than inheriting this
answer: it is a property of the deployed instance, and getting it wrong is the one step here that can
strand an app on an anchor it cannot resolve. A fork rehearsal answers it empirically in minutes.

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
