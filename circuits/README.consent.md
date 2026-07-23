# DogTag consent circuit (`consent.circom`) - owner-unlinkable consent (M2)

Groth16 circuit proving **owner-unlinkable consent**: that a **hidden** pet owner consented to a
**disclosed** relayer for a **disclosed** purpose, revealing **nothing** about the owner.
This is the protocol's ZK consent primitive - the one circuit every on-chain verification runs on.
It landed as redesign milestone **M2** of the owner-hidden rework (captain-approved spec in `data/dogtag-zkverify-z2`).
Built with circom 2.1.9 + snarkjs 0.7.6 + circomlib 2.0.5.

`DogTagConsent` **superseded** the retired owner-revealing `verification.circom`, which
exposed `subject` (the owner) + `keyHash` as public signals. That owner-revealing circuit source has
since been retired (its build products + ceremony transcript remain as historical provenance). The
shared `NodeHash`/`LessThanField`/`LessEqThanField` templates were **copied verbatim** into
[`lib/merkle_inclusion.circom`](lib/merkle_inclusion.circom) (kept bit-identical so the two circuits
agreed on the integer sort inside `hashNode`), NOT refactored out of the then-frozen circuit.

> **DEV setup vs the real ceremony.** `scripts/setup-consent.sh` is the throwaway DEV trusted setup
> (self-generated ptau, forgeable) used only by the circuit test below. The **real testnet-grade phase-2
> ceremony + VK freeze (M3) is DONE**: `scripts/ceremony-consent.sh` (public Hermez pow-17 ptau + a single
> testnet contribution + a public drand beacon) produced the pinned VK/zkey and `Groth16VerifierConsent.sol`
> — see [`../docs/CEREMONY_TRANSCRIPT.consent.md`](../docs/CEREMONY_TRANSCRIPT.consent.md).
> The on-chain `recordVerificationZK` wiring (`VerificationRegistryConsent` paired with `DogTagSBTConsent`, first deployed + verified as M5) is now the **sole live verification path**: every shipped consumer proves against this circuit, and the retired owner-revealing registry serves nothing.
> The current addresses live in [`../contracts/deployments/roax.json`](../contracts/deployments/roax.json) / `contracts/.env` (the disposable testnet will be wiped and redeployed fresh in the pending redeploy slice, so treat the env-configured pair as canonical).
> See the "Build + test" and "M4 binding" notes below.

## What it proves

`DogTagConsent(depth)` (instantiated `main = DogTagConsent(6)` — inclusion paths up to 6 siblings, so
trees up to `2^6 = 64` leaves; the per-tag profile tree is 3 owner leaves + credential attributes, well under
64). Unlike the retired `verification.circom`, this circuit does **NOT** re-derive the tree — it folds three
**owner leaves** up caller-supplied M1 inclusion **paths** (private inputs) to the public root `R`:

- **(1..3)** Recompute the three reserved owner leaves `Poseidon5([DS_LEAF, keyPath, salt, typeTag,
  value])` with **pinned `keyPath`+`typeTag`** (see schema below) and fold each up its inclusion path
  (the M1 `Sibling | Promote` engine, via `MerkleInclusion`) to a root; assert all three reach the
  **same** `R` (`incKey.root === R`, `incSecret.root === R`). The consent-key leaf's value is
  `Poseidon2(Ax, Ay)`, binding the in-tree key to the EdDSA signer.
- **(4)** EdDSA-BabyJubjub verify `(Ax, Ay, S, R8x, R8y)` over `M = Poseidon5([dogTagId, purpose,
  relayer, deadline, consentNonce])` (no domain tag) via circomlib `EdDSAPoseidonVerifier`.
- **(5)** `nullifier = Poseidon6([DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer,
  consentNonce])`.
- **(6)** range-check `relayer < 2^160` (public address; compared on-chain by the registry).

`dogTagId ↔ R` is bound **ON-CHAIN** by M4 (`profileRoot(dogTagId) == R`), **NOT** in this circuit.

## Public-signal ordering (ORDER IS LOAD-BEARING for M4 calldata; verified via `build/consent.sym`)

snarkjs orders public signals by wire index, and circom gives **OUTPUT signals the lowest wire
indices** — so, as in the retired `verification.circom`, **all seven public signals are declared as OUTPUTS** (in
this exact order) to fix the vector. The echoed outputs (`out* <== in*`) are fully constrained by
their private input, so exposing them as outputs does not weaken soundness.

```
pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
```

**No `subject`, no `keyHash`.** The owner never appears in the public signals — that owner-unlinkability
is the whole point of the owner-hidden model.

## Reserved owner-leaf schema (M5 issuance MUST match exactly)

The per-tag tree has three **private** owner-control leaves plus disclosable attribute leaves. The
circuit **pins** the `keyPath` field and `typeTag` of the three reserved leaves to fixed constants:

| leaf          | keyPath string     | typeTag   | value slot                              |
|---------------|--------------------|-----------|-----------------------------------------|
| owner-address | `owner.address`    | 5 (Bytes) | app-supplied owner address as a field   |
| consent-key   | `owner.consentKey` | 5 (Bytes) | `Poseidon2(Ax, Ay)` (keyHash)           |
| owner-secret  | `owner.secret`     | 5 (Bytes) | random secret field (= nullifier secret)|

The exact pinned `fieldOf(keyPath)` constants live in `consent.circom` and are re-derived + asserted
against the SDK `fieldOfKeyPath()` by `scripts/test-consent.mjs` (a drift guard); AGENTS.md carries the
canonical table. These three reserved leaves write a **raw field directly** into the value slot (NOT
the length-prefixed `fieldOfValue` byte-fold used for disclosable attribute leaves).

**The M5 device-side builder that matches this schema is
[`crates/dogtag-standard-rs/src/profile_tree.rs`](../crates/dogtag-standard-rs/src/profile_tree.rs)**
(`build_profile_tree`, reached from the holder apps via the `buildProfileTreeHex` FFI). It keeps the
raw-field encoding in a separate `hash_reserved_leaf` (folding it back into `hash_leaf` would build
leaves this circuit can never prove), and it **rejects** an attribute whose `keyPath` derives to a
reserved one - `TypeTag::Bytes` IS the pinned typeTag, so such an attribute would be a second
circuit-acceptable owner-secret leaf, which is the D5 break the pinning below exists to prevent. The
owner-secret, the consent key and the reserved leaves' salts are all derived from the wallet **seed**
there (each bound to `dogTagId`, so one wallet's tags share none of them), not an RNG, so
restoring the recovery phrase regenerates them (rebuilding `R` also needs the credential's attribute
leaves, which are not seed-derivable); the circuit is indifferent - it takes both as opaque fields.
See [`../docs/MOBILE_OWNER_SECRET.md`](../docs/MOBILE_OWNER_SECRET.md).

**Why pinning `keyPath` is load-bearing (soundness, not cosmetic):** if `keyPath` were a free prover
input, a prover could point the owner-secret inclusion proof at any other in-tree leaf, set
`ownerSecret` to that leaf's value, and mint a **second valid nullifier for one signed consent** —
breaking replay protection (D5). Pinning forces the unique real leaf. `test-consent.mjs` test (e)
exercises exactly this substitution and asserts it fails.

**What pinning does NOT do:** it does not bound the *number* of reserved triples in a tree. "The unique
real leaf" is true only because `build_profile_tree` builds exactly one
`(owner.address, owner.consentKey, owner.secret)` triple - an assumption about the tree, not a circuit
constraint. Keeping it that way is normative (**P-e**): see
[`../docs/DELEGATION.md`](../docs/DELEGATION.md) §5, which binds every future issuance entry point, and
§3.2, which is why delegation ships as a separate circuit rather than a second triple in `R`.

## `recordType` is prover-asserted (not consent-signed)

`recordType` (`pub[5]`) is **not** in `M` and **not** in the nullifier, so the owner's EdDSA consent
does not attest it. It is safe because only the owner's app can generate this proof (it needs the
private leaves + salts, not merely the signature), so the app — not the relayer — chooses it; Groth16
still binds it to the specific proof. **M4 must treat `recordType` as a prover-supplied label, not an
owner-attested field.**

## Constraints

`DogTagConsent(6)` has **~38.5k** non-linear constraints, 0 public inputs, 7 public outputs. The DEV
ptau is **power 16** (`2^16 = 65536 ≥ 38501`).

## Build + test

```
bash scripts/ceremony-consent.sh   # M3: real testnet ceremony -> committed VK/zkey + Groth16VerifierConsent.sol
npm run test-consent               # witness/proof round-trip + R-parity + negatives + keyPath-substitution + D5
npm run gen-consent-fixture        # M4: real proofs vs the committed zkey -> contracts/test/consent-fixture.json
npm run build-consent              # ⚠ DEV/throwaway setup — OVERWRITES the committed M3 zkey/VK with a forgeable key; avoid
```

> **`gen-consent-fixture` takes two env overrides (M-3), both defaulting to the original values** so a
> bare run still targets the committed fixture:
> `CONSENT_FIXTURE_RELAYER` (default `0x1111…1111`) and `CONSENT_FIXTURE_OUT` (default
> `contracts/test/consent-fixture.json`). They exist because **`relayer` is bound into BOTH the EdDSA
> message `M` and the nullifier**, so a proof can only ever be submitted by the address it names - the
> committed fixture's `0x1111…1111` is fine for Foundry (it `prank`s) but unsubmittable by any key we
> hold, making it useless for the Rust relayer E2E. Hence the second committed fixture
> `contracts/test/consent-fixture-anvil.json`, the same witness rebound to anvil account 0. Changing a
> public INPUT does **not** move the VK - same ceremony key, same verifier contract. `CONSENT_FIXTURE_OUT`
> is what keeps a rebound variant from ever overwriting the fixture the Foundry suite pins.
>
> Both fixtures are **semantically, not byte-wise, reproducible**: Groth16 proving draws fresh
> randomness per run, so re-running the generator reproduces every public value (`pub`, `R`, `nullifier`,
> `recordType`, `deadline`, …) but emits different `a`/`b`/`c`. A diff in the proof elements is expected,
> not artifact drift - do not regenerate a committed fixture merely to "check" it.

Since M3, `build/consent.r1cs`, `build/consent_final.zkey`, `build/consent_verification_key.json` and
`build/consent_js/consent.wasm` are **committed**, so `npm run test-consent` runs the full suite against
the **real production key** (33/33 green: round-trip verify + R-parity + negatives + D5). It still
**SKIPs cleanly (exit 0)** if those artifacts are absent, so it never reds an unbuilt checkout — it is a
**standalone heavy manual gate**, intentionally **not** wired into `make test`.

> **`build-consent`/`setup-consent.sh` produce a DEV key that MUST NOT be deployed.** Its
> `Groth16Verifier.consent.dev.sol` comes from a locally-generated power-of-tau with a single
> contributor and a throwaway beacon (the operator knows the toxic waste, so the key is forgeable).
> The **real (M3) VK** is the committed `build/consent_verification_key.json` (sha256 `27879dd7…`) paired
> with `build/consent_final.zkey` (sha256 `f83a111f…`) and `Groth16VerifierConsent.sol`, produced by
> `scripts/ceremony-consent.sh` — see [`../docs/CEREMONY_TRANSCRIPT.consent.md`](../docs/CEREMONY_TRANSCRIPT.consent.md).
> Running `build-consent` again would OVERWRITE those committed files with a forgeable dev key — don't.
> Both the on-chain verifier (M3) and the registry that verifies against it (M4) are now deployed, so an
> overwrite would also invalidate `contracts/test/consent-fixture.json` against the deployed VK.

## M4 binding — SHIPPED

The snarkjs Solidity verifier exposes `verifyProof(a, b, c, pub[7])`. Per the spec,
`recordVerificationZK` binds the proof to the real tag by requiring `pub[4] /*R*/ ==
profileRoot(pub[0] /*dogTagId*/)` (the **only** place `dogTagId ↔ R` is checked), enforces
`deadline >= block.timestamp`, consumes `pub[3] /*nullifier*/`, and emits an **owner-blind**
`Verified` event (no `subject`/`keyHash`).

**The M5 pair is deployed, verified, and LIVE:** `DogTagSBTConsent` (the custodial SBT with a write-once
`profileRoot`) is paired with `VerificationRegistryConsent`, which verifies against the M3 ceremony verifier `Groth16VerifierConsent`.
Both runtimes and constructor wiring match the compiled source; the addresses come from `contracts/deployments/roax.json` / `contracts/.env` (the disposable testnet will be wiped and redeployed fresh in the pending redeploy slice).
Because the registry's `sbt` is immutable, this deployment superseded the earlier M4 registry deploy, which was permanently bound to the retired owner-revealing SBT and was never live (zero `Verified` events).
`contracts/script/DeployCustodialIssuance.s.sol` deploys the canonical pair; the M4 `DeployConsentRegistry.s.sol` (now removed) is superseded.
The retired subject-bearing registry and owner-revealing SBT sources are deleted from the repo, and this pair is the sole verification path in the codebase; the retired deployed instances persist on-chain until the pending wipe/fresh-redeploy slice.
The device-side tree builder that *produces* an `R` owner-privately (`profile_tree.rs`, above) is the issuance counterpart: vet-api's `POST /profiles/issue/custodial-bind` (the **sole** issuance bind) accepts a device-built `R` and mints owner-hidden via `issue(R)` + `mintCustodial`.
On the verification end, `POST /v1/verify/consent` carries a proof built against *this* circuit to `VerificationRegistryConsent` via the 4-arg `recordVerificationZK(a,b,c,pub[7])`, reading `recordType`/`deadline` out of `pub[5]`/`pub[6]` rather than inventing them.
Both routes fail closed when unconfigured (issuance needs `SBT_CONSENT_ADDR` + `PROFILE_ISSUER_ADDR`, verification needs `VERIFICATION_REGISTRY_CONSENT_ADDR` - else 503).
Details: AGENTS.md "M5 as-built" + "M5 app-side" + the M-2 custodial-issuance-bridge and M-3 unified-submission notes; `roax.json` `_m5_custodial_issuance`.

`contracts/test/ConsentRegistry.t.sol` proves a REAL proof from the committed production zkey verifies
through it, using the committed
`contracts/test/consent-fixture.json` (regenerate with `npm run gen-consent-fixture`). The deployed
runtime is byte-identical to this source. An earlier deploy (`0x57A2998…`, now
`VerificationRegistryConsent_preErasureGate_legacy`) predated the `ownerOf` token-existence gate and was
redeployed rather than left stale; it was never live. Full details: AGENTS.md's
`VerificationRegistryConsent` (M4) section; circuit details: AGENTS.md's `DogTagConsent` circuit (M2) section.

## M3 VK-freeze checkpoint — REVIEWED, VK FROZEN

`M` is `Poseidon5` and shares arity + first slot with the leaf hash `Poseidon5([DS_LEAF=1, …])` when
`dogTagId == 1`. This was the last point to reconsider `M`'s preimage structure before locking the VK.
**Conclusion: no exploit exists** (EdDSA needs the private key; leaves are never signed), the
public-signal order/count was re-verified from the freshly compiled circuit (7 outputs, 0 public inputs,
`nPublic == 7`), and the captain-approved spec fixes `M` in this exact form (no DS tag) — a domain tag
would require changing the spec, this circuit, and M7's app proof-gen together, and re-running the
ceremony. The VK is therefore **frozen** against `consent.circom` as merged in #42. See
[`../docs/CEREMONY_TRANSCRIPT.consent.md`](../docs/CEREMONY_TRANSCRIPT.consent.md) "M3 VK-freeze checkpoint".
