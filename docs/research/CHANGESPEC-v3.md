# DogTag v3 Change-Spec — On-chain Proof-of-Verification (consent + Groth16)

> NORMATIVE for the v3 doc-update pass. Consolidates research **10** (Groth16), **11** (consent/registry), **12** (integration). Update agents apply this to `architecture.md`, `implementation.md`, `BUILD_PROMPT.md` using the canonical names in §0. Where this conflicts with earlier docs, **this wins**; preserve existing normative sections (arch §13, impl §11) and extend them (arch §13.7, impl §11.8).

## 0. Canonical names & enums (use EXACTLY)

- New contracts: **`VerificationRegistry`** (records verification events; normal + ZK paths), **`Groth16Verifier`** (snarkjs-generated, BN254), **`ConsentKeyRegistry`** (binds a user's BabyJubjub consent key → their secp256k1 `userWallet` via one-time on-chain `ecrecover`).
- `DogTagIssuer` gains: `zkCommit(bytes32 rKec, bytes32 rZk)` (issuer-only) + `ZkCommitment(rKec, rZk)` event + `kecOf[rZk] → rKec` mapping, so the registry maps a public ZK root back to the keccak issuance root and reuses the existing `isValid` gate.
- New event type: **`Verification`** (a.k.a. Presentation) — a first-class credential-presentation event (generalizes the xlsx "Travel Request" / "DOT Airline Form Presentation"). Keyed by `purpose = keccak256(label)`, e.g. `GROOMING_INTAKE`, `TRAVEL_PRESENTATION`, `AIRLINE_CHECKIN`, `VET_INTAKE`.
- **Verifier capability is gated separately from issuer roles**: `IssuerRegistry.isWhitelistedFor(keccak256("VERIFY:"||purpose), relayer)` — a groomer can verify without being an issuer.
- EIP-712 **`VerificationConsent`** struct: `{ uint256 dogTagId, bytes32 recordType, bytes32 credentialRoot, address relayer, address subject, uint256 nonce, uint256 deadline }`. Domain `{ name:"DogTag", version:"1", chainId:135, verifyingContract: VerificationRegistry }`.
  - **Normal path:** `credentialRoot = rKec`; signed by the user's **secp256k1** wallet (ECDSA, EIP-712) → registry does `ECDSA.recover == subject`.
  - **ZK path:** `credentialRoot = rZk` (the Poseidon root); signed by the user's **EdDSA-BabyJubjub** consent key (cheap in-circuit), which is pre-bound to `subject` (their secp256k1 wallet) in `ConsentKeyRegistry`.
- **Canonical nullifier** (shared `consumed` set across BOTH paths so one consent = one attestation): `nullifier = Poseidon(dogTagId, relayer, subject, nonce)`. ZK path: in-circuit public signal. Normal path: validated on-chain via an audited Solidity Poseidon lib. **MUST be a public signal, never derived from proof bytes** (Groth16 proofs are malleable — snarkjs #383); the registry **range-checks all public signals** (snarkjs #358).
- ZK public signals (order): `[ dogTagId, relayer, subject, nullifier, rZk ]`.
- Mobile signing keys: secp256k1 wallet (existing, for ECDSA consent + EIP-712) **plus** a derived **BabyJubjub consent key** (for ZK consent), registered once via `ConsentKeyRegistry`.
- New monorepo components: `circuits/` (circom + Groth16 setup + generated verifier), `contracts/src/{VerificationRegistry,Groth16Verifier,ConsentKeyRegistry}.sol`, `crates/dogtag-prover-rs/` (ark-circom + ark-groth16 proving service), a `consent` module in **both** SDKs (UniFFI-exported), `verification_records` Mongo collection.

## 1. The rewired verification flow (research 12)

The **mobile-user self-import path is UNCHANGED** (`verify(..., mode:"self-import")`). The **verifier** (groomer/vet/airline/gov) gains an on-chain attestation leg:

```
1. verifier: POST /verify/session/start {purpose} -> QR + one-time JWT carrying {verifierWallet (relayer), purpose, challenge, recordType}
2. mobile: user reviews -> signs EIP-712 VerificationConsent over the credential ROOT (never the salted data):
     {dogTagId, recordType, credentialRoot, relayer, subject=userWallet, nonce, deadline}
     - normal path: ECDSA (secp256k1) over rKec
     - ZK path:     EdDSA-BabyJubjub over rZk  (key bound via ConsentKeyRegistry)
   POST central /v1/verify/consent  -> relayed to verifier POST /verify/consent/submit
3. verifier backend builds:
     - NORMAL proof: reuse the 3-pillar verify(...,mode:"third-party") on the disclosed doc; OR
     - GROTH16 proof: dogtag-prover-rs builds the witness + proof (no raw data on chain)
4. verifier submits on-chain via the existing dual-signing prepare/confirm (hardened-confirm §11.6):
     - VerificationRegistry.recordVerification(consent, userSig)         // normal
     - VerificationRegistry.recordVerificationZK(a,b,c, [dogTagId,relayer,subject,nullifier,rZk])  // ZK
   -> emits Verified(dogTagId, relayer, subject, recordType/purpose, ts); consumes nullifier
```

**Import vs verification are DECOUPLED.** `/import/pull` (off-chain operational data) stays as-is. The new `/verify/*` is the on-chain attestation. NORMAL mode can compose both (disclosed doc drives import + attestation); **ZK mode = verification with no data import at all** (privacy-maximal, the default for sensitive purposes).

## 2. Contracts (research 10 + 11) — apply to arch §4, impl §2/§11

**`VerificationRegistry`** (custom — **not** EAS; EAS isn't on ROAX, can't express relayer-bound-in-sig, has no Groth16 path; borrow only its EIP-712 delegation shape):
```solidity
mapping(bytes32 => bool) public consumed;                 // nullifier -> used (shared by both paths)
bool public restrictToWhitelistedRelayers = true;          // admin toggle: require IssuerRegistry VERIFY: whitelist
event Verified(uint256 indexed dogTagId, address indexed relayer, address indexed subject, bytes32 purpose, bytes32 nullifier, uint256 ts);

function recordVerification(VerificationConsent c, bytes userSig) external {   // NORMAL
    require(block.timestamp <= c.deadline && msg.sender == c.relayer);
    if (restrictToWhitelistedRelayers) require(registry.isWhitelistedFor(keccak256("VERIFY:"||purpose), msg.sender));
    require(ECDSA.recover(_hashTypedDataV4(hash(c)), userSig) == c.subject);
    require(sbt.ownerOf(c.dogTagId) == c.subject);                 // pet belongs to the consenting user
    require(DogTagIssuer(issuerFor(c.recordType)).isValid(c.credentialRoot));   // rKec issued & not revoked
    bytes32 nf = poseidon4(c.dogTagId, c.relayer, c.subject, c.nonce); require(!consumed[nf]); consumed[nf]=true;
    emit Verified(c.dogTagId, c.relayer, c.subject, purpose, nf, block.timestamp);
}
function recordVerificationZK(uint[2] a, uint[2][2] b, uint[2] c, uint[5] pub) external {  // pub=[dogTagId,relayer,subject,nullifier,rZk]
    require(uint160(pub[1]) == uint160(uint(msg.sender)));         // relayer == caller
    if (restrictToWhitelistedRelayers) require(registry.isWhitelistedFor(keccak256("VERIFY:"||purpose), msg.sender));
    for (p in pub) require(p < SNARK_SCALAR_FIELD);               // range-check ALL public signals (#358)
    bytes32 nf = bytes32(pub[3]); require(!consumed[nf]); consumed[nf]=true;   // nullifier is a PUBLIC signal (#383)
    require(zkVerifier.verifyProof(a,b,c,pub));
    require(DogTagIssuer(issuerFor(...)).isValid(kecOf[bytes32(pub[4])]));     // map rZk->rKec, reuse isValid
    emit Verified(pub[0], address(uint160(pub[1])), address(uint160(pub[2])), purpose, nf, block.timestamp);
}
```
- **Relayer pattern:** plain "relayer submits a signed message" — **no EIP-2771** (a forwarder could spoof `msg.sender`, defeating the relayer binding) and **no ERC-4337** here (AA is reserved for the *owner's* gas-sponsored wallet). The relayer is bound *into* the consent + public signals.
- `Groth16Verifier`: snarkjs `zkey export solidityverifier`; BN254; ~211k gas verify, ~240–270k total.
- `ConsentKeyRegistry`: `bindConsentKey(babyJubPubKey, ecdsaSig)` → `ecrecover` proves the secp256k1 wallet authorizes that BabyJubjub key; `keyOf[wallet]` used by the ZK path's subject↔key linkage.

## 3. ZK circuit (research 10) — apply to new `circuits/`, arch §4.7, impl §11.8

- **BN254 Groth16**, ~12–18k constraints, sub-second proving.
- **Public:** `dogTagId, relayer, subject, nullifier, rZk`. **Private:** leaf values/salts/typeTags/keyPath-hashes, the **Poseidon** Merkle path, `consentNonce`, the EdDSA-BabyJubjub signature, the user's BabyJubjub consent pubkey.
- **Proves:** (a) private leaves → Poseidon root `rZk`; (b) the credential's `dogTagId` leaf == public `dogTagId`; (c) `subject`'s consent key signed `(dogTagId, relayer, rZk, nonce)` (EdDSA); (d) `nullifier == Poseidon(dogTagId, relayer, subject, nonce)`. The circuit does **NOT** prove `isValid` — the registry re-checks `isValid(rKec)` on-chain via `kecOf[rZk]`.
- **keccak issuance untouched:** SDK `wrapDocument` computes a **second Poseidon root `rZk` over the same leaf set** and the issuer calls `zkCommit(rKec, rZk)` at issuance (or lazily). `rKec`/`isValid`/§3 standard are byte-for-byte unchanged.
- **Rust proving:** `ark-circom` + `ark-groth16` (pure Rust, integrated witness-gen; rapidsnark only as a documented escape hatch).
- **Trusted setup:** reuse Hermez/Perpetual Powers of Tau (phase 1) + a **multi-party phase-2 (≥3 contributors) ending in a public random beacon**; publish transcript; pin the `.zkey` hash; ship in the prover image. A compromised phase-2 lets a party *forge attestations*, not leak data — and the core three-pillar trust model does not depend on the ZK setup at all.

## 4. Generalization & event taxonomy — apply to arch §3.6/§5, impl §1

Promote to a first-class **`Verification`/`Presentation`** event for any verifier (groomer/vet/airline/gov), keyed by `purpose`. This realizes the xlsx "Credential Presentation Event" rows. Verifier capability via the `VERIFY:` whitelist namespace, **distinct from issuer roles**.

## 5. Privacy / GDPR (research 11 + 12) — apply to arch §11/§13.7, impl §4.5/§11.8

- Both paths publish `subject` (userWallet) + `dogTagId` on-chain → permanent **behavioral linkage** (which user was verified by which business, when) = pseudonymous personal data, **DPIA scope**.
- Mitigations (normative): **(1) ZK is the default** for sensitive purposes (drops `recordType`+`credentialRoot` from chain — only the tuple + nullifier); **(2) fresh per-pet address** as `subject` (bounds to one pet, not the person's portfolio); **(3) ZK-v2 path may publish the `nullifier` *instead of* `subject`** (the interface already carries it) — the only variant that severs the user-address link; **(4)** consent receipts kept **off-chain, deletable**, in the existing crypto-shred erasure flow; **(5)** prefer a **permissioned chain / no public explorer**. The normal path is the more-exposing fallback for when an on-chain `credentialRoot` commitment is genuinely required. Refresh the **mandatory DPIA** to cover the verification-event linkage.
- Consent receipts + nullifier records are added to the erasure scope (`verification_records`, off-chain consent copies).

## 6. Section-by-section change map (research 12)

**architecture.md:** §1 (add 4th capability: on-chain proof-of-verification); §3.6 (the `VerificationConsent` artifact + `Verification` event type); §4.1 (add `VerificationRegistry`/`Groth16Verifier`/`ConsentKeyRegistry`; `DogTagIssuer.zkCommit`); §4.3 (`VERIFY:` whitelist namespace); new **§4.7** (verification contracts + circuit overview); §4.6 (interaction map: add the verify leg); §5 (presentation/verification as a recorded event; mobile verify still 3-pillar+contextual ownership); §9 (`verification_records`, consent receipts); §10 (mobile consent-signing UI + BabyJubjub key); §11 + new **§13.7** (privacy of on-chain verification events).

**implementation.md:** §0 (layout: `circuits/`, `crates/dogtag-prover-rs/`); §1 (consent EIP-712 + Poseidon `rZk` in `wrapDocument` + UniFFI consent module); §2 (the three new contracts + `zkCommit`); §3 (`/verify/session/start`, `/verify/consent/submit`, prover integration; **keep `/import/pull` decoupled**; submit via §11.6 prepare/confirm); §4 (central `/v1/verify/consent` relay + consent receipts + erasure scope); §5 (verifier portal "Verify" UI: QR, normal/ZK toggle, on-chain status); §6 (mobile consent signing, BabyJubjub key, `ConsentKeyRegistry` binding); §9 (tests: circuit, registry, nullifier, both paths); new **§11.8** (normative corrected code for the registry, circuit signals, consent, nullifier-sharing, range-checks).

**BUILD_PROMPT.md:** add a **non-negotiable principle** (on-chain proof-of-verification: consent binds the relayer; nullifier is a public signal & shared across paths; range-check public signals; keccak issuance untouched, parallel Poseidon `rZk` for ZK); a new **Phase 2.5** (circuits + Groth16 setup + `VerificationRegistry`/`ConsentKeyRegistry`); additions to Phase 1 (consent + `rZk` in SDK/vectors), Phase 3 (`/verify/*` + prover), Phase 4 (consent relay + erasure), Phase 5 (verifier Verify UI), Phase 6 (mobile consent + BabyJubjub), Phase 8 (behavioral-privacy gate + ZK-default check).

## 7. Flagged decision points (surface to the user, do not block)
1. **Parallel Poseidon `rZk` at issuance** (vs proving over keccak): adopted — keeps EVM issuance untouched; adds a Poseidon root + `zkCommit` event per issuance.
2. **On-chain Poseidon for the normal-path nullifier** (to share one `consumed` set across both paths) — small gas cost; audit to confirm lib choice. Alternative: separate per-path nullifier domains (accepts double-record across paths).
3. **ZK-v2 nullifier-instead-of-subject** to sever the on-chain user-address link — recommended as the privacy endgame; v1 publishes `subject`.
