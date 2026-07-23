# DogTag - Data Protection Impact Assessment (DPIA)

> **Status:** mandatory and living document (impl §11.1 / §13.7; architecture §13).
> It MUST be refreshed whenever the on-chain data model, the verification subsystem, or the erasure flow change.
> This DPIA covers GDPR (esp. Arts. 5, 6, 9, 17, 25, 35) and CCPA/CPRA data-subject deletion.
>
> **Headline finding:** DogTag is engineered so that **no personal data is ever written on-chain in the clear.**
> What the ledger holds is **salted cryptographic commitments**, status flags, timestamps, and **non-personal references**.
> The tag is minted to a **neutral custodian** (no owner wallet address on-chain), and the on-chain `Verified` event **carries no subject**.
> Even so, the on-chain artefacts assessed in §2 are **pseudonymous personal data** under GDPR, and are assessed together with the **crypto-shredding** mitigation - which is **risk mitigation, NOT a regulator-blessed safe harbour.**

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

Nothing personal is stored in plaintext on-chain.
A DogTag credential (the tag) is a Merkle tree with a single Poseidon root `R`.
The owner is a **hidden leaf**: three reserved leaves (owner-address, consent-key, owner-secret) plus the disclosable pet-attribute leaves fold into `R` **on the owner's device**, and the issuer seals only `R` on-chain via `DogTagSBTConsent.mintCustodial(dogTagId, R)` - a custodial mint that takes **no recipient address**.
There is no on-chain consent-key registry: the consent key lives inside the tree as the hidden `owner.consentKey` leaf, so no key-registry artefact exists to assess.
The following on-chain artefacts are nonetheless in DPIA scope as **pseudonymous personal data** because they can, in principle, be correlated to a person:

| On-chain artefact | What it is | Why it is in scope | Mitigation |
|---|---|---|---|
| **Salted commitment** (Merkle roots, incl. the tag root `R`) | `issue(R)` anchors a salted-Merkle root; per-field random **16-byte salts are off-chain**. The tag root `R` additionally commits to the hidden owner triple, which never leaves the device and is never seen by the issuer. | A salted hash of personal data is **still personal data** (Recital 26); an *unsalted* hash of a low-entropy 15-digit microchip is brute-forceable. | Per-field salting (the privacy mechanism, not just anti-forgery); salts are off-chain, encrypted, **shreddable**; the owner leaves exist only on the owner's device. |
| **Custodial tag token** (`dogTagId` → write-once `profileRoot`) | the soulbound token binds a pet's `dogTagId` to `R`. `mintCustodial(id, root)` takes no `to` parameter, so the owner's wallet is not expressible in the calldata and never reaches contract storage or an event. | `dogTagId` is a **stable per-tag pseudonymous identifier**: everything anchored to the tag (its root, its verifications) shares it, and a tag is attributable to its issuing vet via the issuer/root-index anchoring. | No owner address exists on-chain to correlate; the id is an opaque Poseidon field-hash of an allocated decimal handle, never derived from the microchip or any owner attribute (see below); two tags of one owner are mutually unlinkable. |
| **Verification-event linkage** | `Verified(dogTagId, relayer, purpose, nullifier, deadline, ts)` - which **tag** was presented to which business, when. The event **carries no subject**: the owner is proven in zero knowledge and appears in no public signal, no contract storage, and no event. | `dogTagId`+`relayer`+`ts` is **behavioural pseudonymous personal data**: all of a tag's verifications share its `dogTagId`, so a chain observer can link one tag's verification history **across relayers**, even though no event names the owner. | The owner is never identified on-chain; per-tag key derivation keeps two tags of the same owner mutually unlinkable; nullifiers are one-time values that reveal nothing; off-chain verification copies are deletable (§4). |

**`dogTagId` contains no owner or pet data by construction.**
It is the opaque Poseidon field-hash of a random/sequential decimal handle allocated at issuance.
It is **never** derived from the microchip number or from any owner attribute, so it cannot be reversed into either.
It is still a stable per-tag pseudonym, which is why the linkage rows above remain in scope rather than being dismissed.

### 2.1 Who still learns the owner - the off-chain boundary of "owner-hidden"

The owner-hidden model assessed above is the **only** issuance and verification model; the retired owner-revealing path (which minted the tag to the owner's wallet and emitted a `subject` in its event) no longer exists.
This section scopes the owner-hidden claim precisely, because it is a claim about **downstream** parties, not about every party.

**"Owner-hidden" means hidden from DOWNSTREAM parties - the chain, verifiers, relayers - not from the issuing authority.**
The chain, the public signals, the `Verified` event, and the verifying operator never learn the owner.
Two parties still hold or can learn owner PII, and both are off-chain:

1. **The issuing vet holds owner identity by design.**
   The issuance session row carries the `ownerIdentity` block (name, country of identification, identification number) collected by the operator-gated `POST /profiles/issue/session/start`.
   That collection is **deliberate and justified**: the issuing authority legitimately holds the identity of the person it issues to - it is the basis on which the credential is issued at all.
   The owner-hidden design narrows who ELSE learns it, which is the privacy gain being claimed here - not a claim that the issuer forgets it.
   The block is off-chain store data and therefore already inside the §3 encrypted store and the §4 erasure flow, which is where a data subject's rights over it are exercised.
   **The identity is now COMMITTED into `R` as hidden, selectively-disclosable Merkle leaves (D1, implemented).**
   One `owner.identity.*` attribute leaf per non-blank field (`fullName`, `country`, `docNumber`), salted with a fresh vet-generated 16-byte salt, folded into `R` **on the owner's device** (the wallet seed never leaves the phone) and verified into `R` by the vet at custodial-bind (the attestation-integrity gate) before anything is anchored.
   The privacy properties of the leaf itself:
   - **On-chain and to every downstream party, nothing changes.** `R` is a Poseidon root; an identity leaf is indistinguishable from any other salted attribute leaf. The consent circuit is leaf-blind and its public signals carry no attribute values. The high-entropy issuer salt keeps low-entropy values (a country has ~200 possibilities) unguessable behind the root, and testing a guess also requires the non-public Merkle path.
   - **Disclosure is owner-initiated, per-leaf, and per-verifier.** A `ProfileDisclosure` envelope (Merkle openings of exactly the owner-picked leaves) may ride alongside a consent-proof submission; the verify handler checks each opening against the anchored `R` and the accompanying proof, records only the revealed **keyPaths** (never the values) on the audit row, and stores nothing on-chain.
   - **A plain reveal is a reveal.** The chosen verifier learns the disclosed value in cleartext - inherent to "revealing your name"; the binding to the consent proof stops third-party replay, not the recipient's knowledge.
   - **Who holds the openings off-chain:** the issuing vet (its session row keeps the `{keyPath, salt, value}` triples - the basis of the integrity gate) and the owner's device (its owner-secret store persists the same triples as disclosure openings, inside the mobile backup contract of `docs/MOBILE_OWNER_SECRET.md`).
2. **The server-prove fallback prover sees the proof witness.**
   Proving is on-device by default, and the on-device path leaks nothing.
   For devices that cannot run the Groth16 prover locally, the backend `POST /prove-consent` route is a **trusted-prover fallback**: the assembled circuit input it receives carries `ownerSecret` and `ownerAddress` by construction.
   A prover operator on that path can therefore name the owner and recompute that tag's nullifiers, linking the tag's verification history.
   Owner-unlinkability holds against a chain observer and against the relayer, and does **not** hold against the prover operator (trust-boundary note on `ConsentProver::prove` in `stacks/vet/api/src/prover.rs`; residual-risk row in §6).

One operator-side detail, recorded here as fact: each relayed verification writes a `VerifySession` audit row into the operator trail (`GET /verify/history`, `GET /verify/session/:id`).
The row is owner-blind by construction - `VerifySession` has no `subject` field, and the proof emits no public signal that could fill one - so it holds verifier operational metadata only: the `purpose`/`recordType` bytes32 words, the relayer address, the nullifier, a txHash, a status, and timestamps.
**Open for the DPO:** whether these rows are already covered by the `verification_records` erasure scope of §4, or need to be named there explicitly.
That is a compliance determination, not a documentation one, and it is deliberately not answered here.

## 3. Off-chain personal data (the deletable store)

All owner PII (including the issuance `ownerIdentity` block), pet profile docs, credential **salts + cleartext values**, Art. 9 service attestations, verification session rows and `verification_records`, and consent receipts live **off-chain**, each **encrypted under a per-record DEK** (AES-256-GCM; `crates`/stacks `crypto.rs`).
Mongo is **internal to the compose network only** - never published to the host (see `docs/DEPLOY.md` and each `docker-compose.yml`).
The owner-secret, the per-tag consent key, and the reserved-leaf salts are held on the owner's device only and are never transmitted - the one deliberate exception is the `/prove-consent` fallback witness (§2.1) - so they sit outside every processor's store.

## 4. Right-to-erasure — crypto-shredding (the load-bearing mitigation)

Erasure (GDPR Art. 17 / CCPA deletion, both via the same 45-day flow) executes `erase(ownerId,
scope)`:

1. **Destroy the per-record DEK** (crypto-shred). Every ciphertext copy — DB, oplog, WAL, backups,
   importer caches — becomes **permanently undecryptable**, including the **salts**. "Scrub the salt
   from every replica" is only tractable as **key destruction**.
2. **Delete the off-chain row** (defence in depth), **including `verification_records` + consent
   receipts**.
3. **Propagate erasure central → every business backend** (the issuing vet is the GDPR controller).
4. **On-chain, there is no owner link to drop.**
   The tag was minted to a neutral custodian, so no owner-wallet association ever existed on-chain.
   An erasure MAY additionally **burn the tag** (admin-gated `burn(id)`), after which verification fails closed via the registry's token-existence gate.
   Sharp edge: `burn` deliberately does **not** clear `profileRoot[id]` (that existence gate depends on it), so pair an erasure burn with the issuer's `revoke(R)` for defence in depth.
   The erasure is also **irreversible**: `mintCustodial` rejects any id whose `profileRoot` is already set, so a burned `dogTagId` can never be re-minted and re-animated.

After this, the on-chain salted commitment **remains** but is **unlinkable**: with the salt unrecoverable, the low-entropy preimage can no longer be tied to the commitment.
This is verified by the gate test `gate_erasure_unlinkability.rs` (admin stack): post-erase **decrypt fails** for both credential salts and `verification_records`.

> ⚠️ **This is a documented RISK MITIGATION, not a regulator-blessed safe harbour.** The immutable
> ledger entry is not deleted; we render it unlinkable. Whether a supervisory authority accepts
> crypto-shredding as satisfying Art. 17 for an immutable ledger is **unsettled**. We disclose this to
> data subjects and do not represent erasure as physical deletion of the on-chain commitment.

## 5. Data-minimisation & privacy-by-design measures (Art. 25)

- **Nothing personal on-chain in the clear** - only salted commitments, status, timestamps, non-personal references.
- **No owner address on-chain at all** - `mintCustodial(dogTagId, R)` takes no recipient, so the owner's wallet is not expressible in the calldata; the owner triple (address, consent key, owner secret) folds into `R` on the owner's device and never reaches the issuer, the chain, or the verifier.
- **The on-chain `Verified` event is produced only by a Groth16 consent proof and names no subject** - the owner is proven in zero knowledge; the retired path that carried a `subject` on-chain is gone. Selective disclosure to an operator is a Merkle inclusion check against `R` performed off-chain, and writes nothing on-chain.
- **Per-purpose, point-in-time consent** - each consent proof is cryptographically bound to `(dogTagId, purpose, relayer, deadline)` and is a point-in-time act. Off-chain `Consent`/`ConsentReceipt` records bind a lawful basis + retention clock; withdrawing processing consent triggers retention re-evaluation and erasure of the off-chain records (§4). A consent proof already recorded on-chain is a historical fact, not a standing permission.
- **Per-tag key isolation** - the owner-secret and consent key are derived on-device per `(wallet seed, dogTagId)`, so two tags of one owner share no key material and their nullifiers are mutually unlinkable.
- **Custody isolation** — `/admin/*` custody endpoints are localhost/session-bound, never publicly exposed.

## 6. Residual risks

| Residual risk | Assessment | Treatment |
|---|---|---|
| **Copy-proliferation** before erasure | A third party who scanned/disclosed a credential may hold an off-chain copy outside our control. | Short-lived share JWTs (`exp=180s`, one-time `jti`); data-minimised disclosure; the consent proof discloses no owner data. Cannot guarantee third-party deletion. |
| **Immutable-ledger permanence** | The salted commitment + the subject-less `Verified` tuple persist forever; crypto-shred makes them unlinkable but not absent. | Disclosed to subjects; crypto-shred + the owner-hidden event shape minimise residual attributability. Not a safe harbour (§4). |
| **Per-tag behavioural linkage** | All of a tag's verifications share its `dogTagId`, so a chain observer can assemble one tag's verification history **across relayers**; the tag is also attributable to its issuing vet via the issuer/root-index anchoring. The owner is never named, and two tags of one owner cannot be linked to each other. | Identity-unlinkability by construction (subject-less event, custodial mint, per-tag keys). Accepted residual: per-tag history linkage is public; disclosed to subjects. |
| **Trusted-prover exposure** (server-prove fallback) | A `/prove-consent` prover operator sees `ownerSecret` + `ownerAddress` in the witness, so it can name the owner and link that tag's verification history. The wallet seed never leaves the device, so the owner's other tags and future consents under them stay out of reach. | On-device proving is the default and leaks nothing; the fallback exists only for devices that cannot prove locally; disclosed here and in `docs/MOBILE_OWNER_SECRET.md` "Handling rules". |
| **Low-entropy preimage** | A 15-digit microchip is brute-forceable if a hash of it were ever unsalted on chain. | Salting is mandatory and per-field; `dogTagId` is an opaque field-hash of the allocated handle, never derived from the chip (§2). |

## 7. Sign-off & review triggers

Refresh this DPIA on any change to: the on-chain data model, the verification subsystem (`VerificationRegistryConsent` / `circuits/consent.circom`), the issuance/custody model, the erasure scope/flow, or the lawful-basis/consent model.
The CI guardrails: `gate_erasure_unlinkability.rs` (admin stack) holds the §4 claim, and `gate_dual_signing_parity.rs` (vet stack) holds signing parity.
The structural claims in §2 and §5 - no recipient parameter on the mint, no subject field in the `Verified` event - are properties of the contract interfaces themselves (`DogTagSBTConsent.sol`, `VerificationRegistryConsent.sol`), exercised by the contract test suite (`contracts/test/`).
