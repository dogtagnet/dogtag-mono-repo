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
| **wallet ↔ SBT link** (`ownerOf(dogTagId)==wallet`) | the soulbound token binds a pet's `dogTagId` to the owner's wallet address. **Level-A only** — a tag issued via the Level-B custodial route (§2.1) is minted to a neutral custodian, so `ownerOf` carries no owner meaning and this row does not apply to it. Every live tag today is Level-A. | An EVM address is **pseudonymous personal data**; a live `ownerOf` link associates a pet with a controllable wallet. | **Fresh per-pet derived address** (§5) — breaks cross-pet enumeration; **SBT burn** on erasure drops the live link. Level-B removes the link entirely (§2.1). |
| **verification-event linkage** | `Verified(dogTagId, relayer, subject, purpose, nullifier, ts)` — which pet was presented to which business, when. | `subject`+`dogTagId`+`relayer`+`ts` is **behavioural pseudonymous personal data** (who verified whom). | **ZK-default for sensitive purposes** (no `recordType`/`credentialRoot` on chain); **fresh per-pet `subject`** bounds linkage to one pet; off-chain consent copies are deletable. |

**`dogTagId` is non-personal by construction.** It is a random/sequential identifier allocated at
mint — it is **NEVER** `keccak256(microchip)` and **NEVER** `Poseidon(microchip)` (asserted on-chain
by `test_dogTagId_is_not_hash_of_microchip` and off-chain by the Phase-8 gate
`gate_pii_off_chain.rs`). `keyOf[subject]` (the bound BabyJubjub consent key) is also in scope and is
covered by verifier-side erasure (`ownerId→verifier` index, impl §11.10(j)).

### 2.1 Queued refresh - Level-B owner-blind verification (pending M7)

> **This refresh is QUEUED, not performed.** The assessment above, and in §4, describes the **live**
> system and is accurate as written; nothing in it is re-scored here.

Milestone M4 deployed a **Level-B owner-blind verification path**
(`VerificationRegistryConsent`, `contracts/src/VerificationRegistryConsent.sol`) that is **NOT YET
LIVE**.
The live verification subsystem remains the Level-A `VerificationRegistry`, which is what this DPIA
assesses.
The cutover is **M7**. Its M5 on-chain prerequisite is now deployed + verified: `DogTagSBTConsent`
`0x96Cba458…` is paired with `VerificationRegistryConsent` `0xb9B313C1…` on ROAX. The pair is **NOT
LIVE and wired to no SHIPPED consumer** (M-2/M-3 added off-by-default backend routes against each half
- see the status update below), so nothing below is in effect: issuance today still mints the SBT
to the owner's wallet exactly as §2 and §4 assess it. M7 must perform this DPIA refresh before cutover.

**M-2 / M-3 status update - both Level-B routes now exist, but nothing assessed here changes.** M-2 added
`POST /profiles/issue/custodial-bind` to the vet stack, the owner-hidden path that anchors a
device-built `R` and seals it with `mintCustodial`; M-3 added its verification counterpart,
`POST /verify/consent/levelb`, which relays an owner-hidden consent proof to
`VerificationRegistryConsent`. Neither moves this assessment, for three independent reasons: each is
**off by default** (unset `SBT_CONSENT_ADDR` / `PROFILE_ISSUER_ADDR` → 503; unset
`VERIFICATION_REGISTRY_CONSENT_ADDR` → 503), **no shipped device posts to either**, and both are
**additive** - the Level-A `POST /profiles/issue/bind` still serves every live issuance and still mints
to the owner's wallet, and the Level-A `POST /verify/consent/submit` still serves every live
verification. The trigger for the re-scoring below is Level-B carrying **real** issuance and
verification traffic, not the routes' existence.

One M-3 detail to CARRY INTO the M7 refresh, recorded here as fact rather than re-scored: when the
Level-B route is wired, it writes a `VerifySession` audit row into the **same** operator trail as
Level-A (`GET /verify/history`, `GET /verify/session/:id`). The row is owner-blind by construction -
`VerifySession` has no `subject` field, and Level-B emits no public signal that could fill one - so it
holds verifier operational metadata only: the `purpose`/`recordType` bytes32 words, the relayer address,
the nullifier, a txHash, a status, and timestamps. No owner identifier. On an unwired stack the route
503s before any row is written, so nothing is created at all.
**Open for the M7 refresh / DPO:** whether these rows are already covered by the `verification_records`
erasure scope of §4, or need to be named there explicitly. That is a compliance determination, not a
documentation one, and it is deliberately not answered here.

> **Scope the owner-blindness claim precisely - "owner-hidden" means hidden from DOWNSTREAM parties,
> not from the issuing authority.** Level-B removes the owner from the chain, from the public signals,
> from the `Verified` event, and from what a verifier or relayer ever sees. It does **not** - and is not
> intended to - remove the owner from the issuing vet's own records: the session row still carries the
> `ownerIdentity` block (name, country of identification, identification number) collected by the
> operator-gated `POST /profiles/issue/session/start`. That collection is **deliberate and justified**:
> the issuing authority legitimately holds the identity of the person it issues to (it is the basis on
> which the credential is issued at all), exactly as it does on the Level-A path. Level-B narrows who
> ELSE learns it, which is the privacy gain being claimed here - not a claim that the issuer forgets it.
> The block is off-chain store data and therefore already inside the §3 encrypted store and the §4
> erasure flow, which is where a data subject's rights over it are exercised.
>
> The Level-B handler builds no verifiable credential, so it does not currently read that block. That is
> a property of the current implementation stage, not a signal that the data is surplus: owner identity
> is **planned** to be committed into `R` as a hidden, selectively-disclosable Merkle leaf
> (identity-as-leaf), putting it on a path further INTO the design. That leaf work is **planned, not
> implemented** - when it lands, this section must be revisited to assess the leaf itself.

When Level-B goes live, the living-document rule in the status block above fires and the following
MUST be re-scored.
This is a forward-looking notice only; none of it is in effect today.

1. **verification-event linkage** (§2) - the Level-B `Verified` event drops `subject` entirely, so
   the recorded mitigation "fresh per-pet `subject` bounds linkage to one pet" would no longer apply
   as written.
   The owner would be proven in zero knowledge and would not appear in the public signals, in
   contract storage, or in the event.
   The row therefore needs a genuine **risk re-scoring**, not a text edit.
2. **wallet ↔ SBT link** (§2) - under Level-B decision D1 the tag would be minted to a **neutral
   custodian**, so `ownerOf` would carry no owner meaning and the pet/wallet association changes
   character.
   That lands with custodial issuance: `DogTagSBTConsent.mintCustodial(id, root)` takes no `to`
   parameter at all, so the owner's wallet is not expressible in the calldata and never reaches
   contract storage or an event.
   The contract side is M5; the issuer route that drives it is M-2's
   `POST /profiles/issue/custodial-bind`, which correspondingly accepts **no wallet and no signature**
   (there is no recipient for a signature to attest, and accepting one would restore the very link
   being removed) — see the M-2 status note above for why this is not yet in effect.
3. **erasure flow** (§4, step 4 "burn the SBT") - the Level-B registry reads `ownerOf` for token
   **existence** only and never for owner identity, so an erasure burn would fail verification closed
   **only** by virtue of that existence gate.
   Sharp edge: `burn` does **not** clear `profileRoot[id]` on either SBT (Level-A `DogTagSBT` or Level-B
   `DogTagSBTConsent`, where the mapping is deliberately never cleared because that existence gate
   depends on it), so an erasure burn should be paired with `revoke(R)` for defence in depth.
   Under Level-B the erasure is also **irreversible**: `mintCustodial` rejects any id whose
   `profileRoot` is already set, so a burned `dogTagId` can never be re-minted and re-animated.

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
