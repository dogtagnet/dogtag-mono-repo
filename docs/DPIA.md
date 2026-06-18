# DogTag — Data Protection Impact Assessment (DPIA)

> **Status:** mandatory and living document (impl §11.1 / §13.7; architecture §13). It MUST be
> refreshed whenever the on-chain data model, the verification subsystem, or the erasure flow change.
> This DPIA covers GDPR (esp. Arts. 5, 6, 9, 17, 25, 35) and CCPA/CPRA data-subject deletion.
>
> **Headline finding:** DogTag is engineered so that **no personal data is ever written on-chain in
> the clear.** What the ledger holds is **salted cryptographic commitments**, status flags,
> timestamps, and **non-personal references**. Even so, three categories of on-chain artefact are
> **pseudonymous personal data** under GDPR and are assessed below, together with the
> **crypto-shredding** mitigation — which is **risk mitigation, NOT a regulator-blessed safe
> harbour.**

---

## 1. Processing overview

| | |
|---|---|
| **Controller (issuance)** | the self-hosted business (vet/groomer) that issues a credential |
| **Controller (central)** | DogTag (we host) for discovery, whitelisting, mobile API, appointments |
| **Processors** | MongoDB (self-hosted per stack, internal-only), ROAX chain (public ledger) |
| **Data subjects** | pet **owners** (natural persons). Pets are not data subjects, but a pet's data is linkable to its owner |
| **Lawful bases** | issuance/verification: **contract** (Art. 6(1)(b)) + **consent** (Art. 6(1)(a), per-purpose `Consent`/`ConsentReceipt`); Art. 9 service-attestation data (assistance-animal status): **explicit consent**, off-chain only |

## 2. On-chain personal-data inventory

Nothing personal is stored in plaintext on-chain. The following on-chain artefacts are nonetheless
in DPIA scope as **pseudonymous personal data** because they can, in principle, be correlated to a
person:

| On-chain artefact | What it is | Why it is in scope | Mitigation |
|---|---|---|---|
| **Salted commitment** (Merkle root `R`) | `issue(R)` anchors a salted-Merkle root; per-field random **16-byte salts are off-chain**. | A salted hash of personal data is **still personal data** (Recital 26); an *unsalted* hash of a low-entropy 15-digit microchip is brute-forceable. | Per-field salting (the privacy mechanism, not just anti-forgery); salts are off-chain, encrypted, **shreddable**. |
| **wallet ↔ SBT link** (`ownerOf(dogTagId)==wallet`) | the soulbound token binds a pet's `dogTagId` to the owner's wallet address. | An EVM address is **pseudonymous personal data**; a live `ownerOf` link associates a pet with a controllable wallet. | **Fresh per-pet derived address** (§5) — breaks cross-pet enumeration; **SBT burn** on erasure drops the live link. |
| **verification-event linkage** | `Verified(dogTagId, relayer, subject, purpose, nullifier, ts)` — which pet was presented to which business, when. | `subject`+`dogTagId`+`relayer`+`ts` is **behavioural pseudonymous personal data** (who verified whom). | **ZK-default for sensitive purposes** (no `recordType`/`credentialRoot` on chain); **fresh per-pet `subject`** bounds linkage to one pet; off-chain consent copies are deletable. |

**`dogTagId` is non-personal by construction.** It is a random/sequential identifier allocated at
mint — it is **NEVER** `keccak256(microchip)` and **NEVER** `Poseidon(microchip)` (asserted on-chain
by `test_dogTagId_is_not_hash_of_microchip` and off-chain by the Phase-8 gate
`gate_pii_off_chain.rs`). `keyOf[subject]` (the bound BabyJubjub consent key) is also in scope and is
covered by verifier-side erasure (`ownerId→verifier` index, impl §11.10(j)).

## 3. Off-chain personal data (the deletable store)

All owner PII, pet profile docs, credential **salts + cleartext values**, Art. 9 service
attestations, relayed `VerificationConsent` copies, consent receipts, and `verification_records` live
**off-chain**, each **encrypted under a per-record DEK** (AES-256-GCM; `crates`/stacks `crypto.rs`).
Mongo is **internal to the compose network only** — never published to the host (see `docs/DEPLOY.md`
and each `docker-compose.yml`).

## 4. Right-to-erasure — crypto-shredding (the load-bearing mitigation)

Erasure (GDPR Art. 17 / CCPA deletion, both via the same 45-day flow) executes `erase(ownerId,
scope)`:

1. **Destroy the per-record DEK** (crypto-shred). Every ciphertext copy — DB, oplog, WAL, backups,
   importer caches — becomes **permanently undecryptable**, including the **salts**. "Scrub the salt
   from every replica" is only tractable as **key destruction**.
2. **Delete the off-chain row** (defence in depth), **including `verification_records` + consent
   receipts**.
3. **Propagate erasure central → every business backend** (the issuing vet is the GDPR controller).
4. **Burn the SBT** to drop the live `ownerOf ↔ wallet` link.

After this, the on-chain salted commitment **remains** but is **unlinkable**: with the salt
unrecoverable, the low-entropy preimage can no longer be tied to the commitment, and the per-pet
address is unlinked. This is verified by the Phase-8 gate `gate_erasure_unlinkability.rs` (post-erase
**decrypt fails** for both credential salts and `verification_records`).

> ⚠️ **This is a documented RISK MITIGATION, not a regulator-blessed safe harbour.** The immutable
> ledger entry is not deleted; we render it unlinkable. Whether a supervisory authority accepts
> crypto-shredding as satisfying Art. 17 for an immutable ledger is **unsettled**. We disclose this to
> data subjects and do not represent erasure as physical deletion of the on-chain commitment.

## 5. Data-minimisation & privacy-by-design measures (Art. 25)

- **Nothing personal on-chain in the clear** — only salted commitments, status, timestamps, non-personal DIDs/accreditation refs.
- **Fresh per-pet derived address** — each pet's SBT mints to a distinct address; the ZK `subject` IS that per-pet address, so verification linkage is bounded to **one pet, not the owner's portfolio** (asserted by `gate_behavioral_privacy.rs`).
- **ZK is the default for sensitive purposes** — `/verify/session/start` defaults `mode` to `"zk"` when unspecified; the Groth16 path records **no** `recordType`/`credentialRoot` on chain. The normal ECDSA path is the **fallback** only when an on-chain `credentialRoot` commitment is genuinely required.
- **Per-purpose consent** — `Consent`/`ConsentReceipt` bind a lawful basis + retention clock; withdrawal triggers retention re-evaluation → erase.
- **Custody isolation** — `/admin/*` custody endpoints are localhost/session-bound, never publicly exposed.

## 6. Residual risks

| Residual risk | Assessment | Treatment |
|---|---|---|
| **Copy-proliferation** before erasure | A third party who scanned/disclosed a credential may hold an off-chain copy outside our control. | Short-lived share JWTs (`exp=180s`, one-time `jti`); data-minimised disclosure; ZK path discloses no raw data. Cannot guarantee third-party deletion. |
| **Immutable-ledger permanence** | The salted commitment + `Verified` tuple persist forever; crypto-shred makes them unlinkable but not absent. | Disclosed to subjects; crypto-shred + per-pet address + ZK-default minimise residual attributability. Not a safe harbour (§4). |
| **Address re-correlation** | Chain analytics could attempt to cluster a per-pet address. | Fresh per-pet addresses; gas sponsorship so owners hold no PLASMA (no funding trail); no native send/receive in v1. |
| **Low-entropy preimage** | A 15-digit microchip is brute-forceable if a hash of it were ever unsalted on chain. | Salting is mandatory and per-field; `dogTagId` is never a hash of the chip; gate-tested. |

## 7. Sign-off & review triggers

Refresh this DPIA on any change to: the on-chain data model, the verification subsystem
(`VerificationRegistry`/circuit/`ConsentKeyRegistry`), the erasure scope/flow, the address-derivation
scheme, or the lawful-basis/consent model. The Phase-8 gate tests
(`gate_pii_off_chain`, `gate_erasure_unlinkability`, `gate_behavioral_privacy`, dual-signing parity)
are the CI guardrails for the claims in §2, §4, and §5.
