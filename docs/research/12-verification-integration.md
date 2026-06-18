# 12 — On-Chain Verification Events: Consent, ZK, and the VerificationRegistry (INTEGRATION & IMPACT)

> ⚠️ **SUPERSEDED for normative details.** This early brief uses an outdated 3-signal public set, no nullifier, old consent field names, and "CHANGESPEC-v2" references. The **normative source of truth is `CHANGESPEC-v3.md` + `implementation.md §11.8/§11.9` + `architecture.md §4.7/§13.8`** (7 public signals `[dogTagId, purpose, relayer, subject, nullifier, keyHash, rZk]`, `purpose`-bound consent, pinned Poseidon nullifier). Read this only for high-level flow/rationale.

> **Status:** v1 design proposal. Companion research: ZK design in [`10-zk-groth16.md`](./10-zk-groth16.md), consent/registry in [`11-consent-attestation.md`](./11-consent-attestation.md) (assumed). This brief specifies how a new **on-chain verification / presentation event** rewires the existing groomer/business flow, generalizes to any verifier, the new monorepo components, the section-by-section change map for `architecture.md` and `implementation.md`, the privacy/GDPR impact, and the build-order/phase impact.
>
> **Normative posture:** this brief follows the existing precedence rules — `architecture.md §13` / `implementation.md §11` (audit remediations) win on conflict, and `CHANGESPEC-v2 §0` canonical names/enums are mandatory. The new event type **extends** those sections; it does not replace them.

---

## 0. The feature in one sentence

A **verifier** (groomer, vet-as-verifier, airline, government) records an **on-chain transaction proving they validated** a user's vaccination certificate or DogTag SBT, where the user authorizes the act with a **wallet-signed EIP-712 consent** (over the credential, capturing the verifier's address), and the verifier's backend produces **either a normal proof OR a Groth16 ZK-proof** before submitting to a new **`VerificationRegistry`** contract.

This is, in xlsx-taxonomy terms, the first-class implementation of a **Credential Presentation Event** (today only narrated as "Travel Request" / "DOT Service Animal Airline Form Presentation" — strings 8/9 and 16/17 of the workbook's *Unique Events* sheet) — promoted from prose to an on-chain, consent-gated, optionally-private artifact.

---

## 1. How it rewires the groomer import/verification flow

### 1.1 Today (baseline)

Two existing flows are relevant:

- **Mobile-user on-chain verification (UNCHANGED).** The owner self-imports via the contextual four-fragment `verify(...,mode:"self-import")` (impl §11.3 / §6.5): three authenticity pillars **+** `ownerOf(dogTagId)==myWalletAddress`. **This feature does not touch it.**
- **Groomer import FROM user (impl §3.5 `/import/pull`, §5.2).** Today the user shows a one-time JWT QR against the **central** API (`aud:"dogtag-business"`); the groomer's backend `GET`s the **raw wrapped doc**, runs `verify(doc,{...,mode:"third-party"})` (ownership `NOT_APPLICABLE`), and on `valid` **upserts `clients`/`pets_cache`** (off-chain import). **Nothing is recorded on-chain by the groomer.** Verification is ephemeral and private to the groomer's Mongo.

The gaps the feature closes: (a) no durable, tamper-evident, *third-party-auditable* proof that a verification happened; (b) the groomer receives the **raw credential** (maximal data exposure) even when it only needs a yes/no; (c) no explicit, signed, purpose-bound **user consent** capturing *who* verified *what*.

### 1.2 New flow (consent → proof → on-chain record)

The mobile-user self-import path is untouched. The groomer/verifier path is rewired as follows. New endpoints are marked **NEW**; the verifier-side ones live on the **business backend** (`stacks/groomer/api`, generalized to vet/airline/gov stacks), the user-side ones on the **central backend** (`stacks/admin/api`, mobile API).

```
PRESENTATION / VERIFICATION EVENT  (verifier = groomer; generalizes to vet/airline/gov)

(1) [UNCHANGED] mobile-user on-chain verification of its own credential — self-import path (§11.3).

(2) Verifier shows QR + one-time JWT  — business backend:
    POST /verify/session/start { recordTypes:[VACCINATION|DOG_PROFILE...], mode:"normal"|"zk" }   [NEW]
       -> verifierAddress = activeSigner()          # the address that will submit the on-chain tx
          challenge = uuid()                        # nonce, one-time, short TTL
          jwt = sign_eddsa({ iss, sub: sessionId, aud:"dogtag-verify", scope:"present:credential",
                             verifierAddress, challenge, recordTypes, exp ~180s, jti })
          QR = https://<verifier-host>/v?t=<jwt>&s=<sessionId>
       -> { sessionId, qrUrl }

(3) User sends a SIGNED CONSENT (EIP-712) over the credential — NOT the raw data — central/mobile:
    Mobile scans QR, reads verifierAddress + challenge + recordTypes from the JWT.
    Mobile builds the EIP-712 `VerificationConsent` (domain: chainId 135, verifyingContract = VerificationRegistry):
       VerificationConsent {
         dogTagId, merkleRoot,          # the credential being attested (root, NOT the salted data)
         verifier: verifierAddress,     # CAPTURES the verifier's wallet address (binds who verified)
         userWallet,                    # the pet's owner address (fresh per-pet, §11.1)
         recordType, purpose,           # e.g. keccak256("GROOMING_INTAKE"); xlsx event label
         challenge, nonce, deadline
       }
    Wallet (embedded-MPC default / BIP-39) signs -> consentSig.
    POST /v1/verify/consent { sessionId, verifierApiBase, consent, consentSig }   [NEW, central]
       require auth(owner); store Consent + ConsentReceipt (purpose, lawfulBasis, grantedAt) (§9 / §4.5)
       relay to the verifier:
         POST <verifierApiBase>/verify/consent/submit { consent, consentSig, disclosure }   [NEW, business]
       # `disclosure`: in NORMAL mode the user may attach the selectively-disclosed wrapped doc
       #   (or just the root + fields the verifier needs); in ZK mode NO raw doc is sent — only the
       #   consent + the inputs the prover needs (see §3 ZK signals).

(4) Verifier backend generates a NORMAL proof OR a Groth16 ZK-proof — business backend:
    POST /verify/consent/submit { consent, consentSig, disclosure }   [NEW]
       require operator session
       verify EIP-712 consentSig recovers to consent.userWallet
       require consent.verifier == activeSigner()      # consent is bound to THIS verifier
       require sessions[consent.sessionId].challenge == consent.challenge (one-time)
       # NORMAL mode: run the existing three-pillar verify() over the disclosed wrapped doc
       #   verdict = verify(doc,{rpc,dns,mode:"third-party"})  ; require verdict.valid
       #   proofBlob = { kind:"normal", verdict, merkleRoot, consentSig }
       # ZK mode: build a Groth16 proof with the circuits/ prover
       #   public signals = (dogTagId, groomerRelayer, userWallet)   # see §3 / §6 of the spec
       #   private witness = merkle membership / isValid path / consent binding
       #   proofBlob = { kind:"groth16", proof, publicSignals }
       -> { proofId, kind, proofBlob }

(5) Verifier submits the on-chain tx to the NEW VerificationRegistry — business backend:
    Mirrors the dual-signing prepare/confirm pattern (§3.8/§11.6) — build is ALWAYS server-side,
    only sign+broadcast differs by signingMode.
    POST /verify/prepare { proofId }   [NEW]
       calldata =
         NORMAL: recordVerification(dogTagId, merkleRoot, recordType, userWallet, consentHash, consentSig)
         ZK:     recordVerificationZk(zkProof, publicSignals=(dogTagId, groomerRelayer, userWallet))
       if signingMode=="wallet": return { unsignedTx:{ to:VERIFICATION_REGISTRY, data:calldata, value:0, chainId:135 } }
       else (backend): txHash = sign_and_send(activeSigner, ROAX_RPC, VERIFICATION_REGISTRY, calldata)
    POST /verify/confirm { proofId, txHash }   [NEW]
       # hardened like §11.6 confirm: derive signer from tx (never the body); bind tx.to/input/value/chainId;
       # require Groth16Verifier accepted on-chain (ZK) OR the VerificationAttested event matches; N confirmations.
       require event VerificationAttested(dogTagId, verifier=signer, recordType, ts) present
       require signer == consent.verifier
       save verification_record{ verificationId, dogTagId, verifier, recordType, kind, txHash, consentReceiptId, status:"attested" }

(6) ZK public signals = (dogTagId, groomerRelayer, userWallet).   # see §3 + the ZK brief (10-zk-groth16.md)
```

### 1.3 Does the groomer still import the off-chain profile/vaccination data IN ADDITION, or instead? — RECONCILED

**Both — but they become two distinct, independently-invokable operations, and the on-chain verification does NOT require the off-chain import.** Reconciliation:

- **`/import/pull` (impl §3.5) stays** as the off-chain *data ingestion* operation (groomer needs the pet's name, vaccine status, owner contact to actually run the appointment → upserts `clients`/`pets_cache`). It is **operationally optional** for the new verification event.
- **The new `/verify/*` flow is the on-chain *attestation* operation.** Its purpose is a durable, third-party-auditable proof that "verifier X validated dogTagId Y's recordType Z at time T, with the owner's signed consent." It does **not** by itself populate `clients`/`pets_cache`.
- **The two compose.** In **NORMAL** mode, the consent submission MAY carry the selectively-disclosed wrapped doc, so the same payload both (a) drives the off-chain import (reuse the `/import/pull` upsert) and (b) feeds the three-pillar verify that backs the on-chain record. In **ZK** mode, the verifier deliberately receives **no raw data** — only the proof + public signals — so there is **on-chain verification WITHOUT off-chain import** (this is the privacy-maximal path: the groomer proves "this pet's rabies cert is valid and the owner consented" without ever holding the cert).

**Net rule:** _verification (on-chain) and import (off-chain) are decoupled. Import is data; verification is proof. The new feature adds the proof leg and makes the data leg optional (and, in ZK mode, absent)._ This is the key integration correction — `/import/pull` must NOT be conflated with or replaced by `/verify/*`.

---

## 2. Generalization — a first-class Presentation / Verification event type

The user said "groomer," but the existing system already treats the business backend as generic (`stacks/vet`, `stacks/groomer`, and "later governments/airlines" per arch §1; the xlsx *Unique Issuers* sheet already lists **USDA APHIS, EU Competent Authority, CDC, Department of Transportation**). The verification event must therefore be **verifier-agnostic**.

### 2.1 Map onto the existing xlsx event taxonomy

The xlsx *Unique Events* sheet already distinguishes **two event classes**:

| xlsx event | Class | Today |
|---|---|---|
| `Dog Tag`, `Add Vet`, `Vaccination`, `Service Animal Airline Form`, `Add Service Dog` | **Credential Issuance Event** | Implemented (anchor a Merkle root via `DogTagIssuer.issue`) |
| **`Travel Request`** ("…presented to US Gov and European Gov to be verified") | **Credential Presentation Event** | Prose only — no on-chain artifact |
| **`DOT Service Animal Airline Form Presentation`** ("…presented to Airlines to be verified") | **Credential Presentation Event** | Prose only — no on-chain artifact |
| `Travel Denial` / `Travel Clearance` | **Outcome of a presentation** | Outcome → may *issue* an `EU_HEALTH_CERT` (existing issuance) |
| `CDC Dog Import Form` | Off-chain event | Off-chain only |

**Recommendation:** promote "Credential Presentation Event" to a **first-class `PRESENTATION` / `VERIFICATION` event type** with an on-chain record in `VerificationRegistry`. The groomer's "grooming intake verification," the airline's "DOT form presentation," the government's "travel request," and a vet-as-verifier check are all the **same event type** with a different `purpose`/`recordType` label and a different verifier address. Concretely:

- Add a canonical event/purpose enum alongside `recordType` (CHANGESPEC §0): `purpose` labels e.g. `GROOMING_INTAKE`, `TRAVEL_REQUEST`, `DOT_AIRLINE_PRESENTATION`, `BOARDING_INTAKE`, `VET_CROSS_CHECK` (on-chain = `keccak256(label)`, exactly like `recordType`).
- The `Travel Clearance` outcome stays an **issuance** event (issue `EU_HEALTH_CERT`) — i.e. a presentation event can *chain into* an issuance event. The registry records the presentation; the existing `DogTagIssuer` records any resulting issued credential. They remain separate contracts/records.
- The verifier need not be whitelisted as an **issuer** — verification is a *different capability*. See §3.2 (the registry gates "may attest a verification," distinct from `IssuerRegistry`'s "may issue a root").

**Naming:** treat the contract as **`VerificationRegistry`** and the event type as **`Presentation`/`Verification`** (use one consistently in CHANGESPEC §0 — recommend `Verification` as the entity, `Presentation` as the human/xlsx-facing label). This avoids overloading "import" and keeps the issuance/presentation split that the xlsx already implies.

---

## 3. New monorepo components

All paths relative to `dogtag-mono-repo/` (layout per impl §0).

| Component | Path | What it is |
|---|---|---|
| **ZK circuits package** | `circuits/` *(new top-level, sibling of `contracts/`)* | circom sources + Groth16 trusted-setup (phase-1 ptau + phase-2 per-circuit `zkey`) + generated artifacts. Subdirs: `circuits/src/verification.circom`, `circuits/build/` (r1cs, wasm, zkey), `circuits/scripts/{compile,setup,export-verifier}.sh`, `circuits/test/`. Public signals fixed to **(dogTagId, groomerRelayer, userWallet)** per the feature spec. Full design: `10-zk-groth16.md`. |
| **`VerificationRegistry` contract** | `contracts/src/VerificationRegistry.sol` | Records presentation/verification events. `recordVerification(...)` (normal) + `recordVerificationZk(zkProof, publicSignals)` (calls the verifier). Emits `VerificationAttested(dogTagId, verifier, recordType/purpose, kind, ts)`. Gated by a **verifier-capability check** (see below). Stores `consentHash` (NOT the consent contents). |
| **`Groth16Verifier` contract** | `contracts/src/Groth16Verifier.sol` | snarkjs/circom-generated `verifyProof(a,b,c,publicSignals)`. **Generated artifact** — checked in, regenerated by `circuits/scripts/export-verifier.sh`; do not hand-edit. Referenced by `VerificationRegistry.recordVerificationZk`. |
| **Verifier-capability gate** | `contracts/src/IssuerRegistry.sol` (extend) **or** a small `VerifierRegistry` mapping inside `VerificationRegistry` | "May this address attest a verification for this purpose?" Recommend reusing `IssuerRegistry` with a new role/recordType namespace (e.g. `isWhitelistedFor(keccak256("VERIFY:"+purpose), signer)`) so the existing admin approval queue + multi-address-per-entity model (§4.3) is reused, and a groomer who is *not* an issuer can still be a *verifier*. |
| **Proving service (Rust)** | `crates/dogtag-prover-rs/` *(new crate)* + wired into business APIs (`stacks/{groomer,vet}/api`) | Builds the witness from the consent + credential, runs Groth16 proving (e.g. `ark-groth16`/`ark-circom`, or shells `snarkjs` with the `zkey`), returns `{proof, publicSignals}`. Also the NORMAL-mode path (reuse `dogtag-standard-rs::verify`). Kept server-side only (like custody), under operator-session auth. |
| **EIP-712 consent in the SDK** | `crates/dogtag-standard-rs/src/consent.rs` + `packages/dogtag-standard-ts/src/consent.ts` | Canonical `VerificationConsent` struct + EIP-712 domain (chainId 135, `verifyingContract = VerificationRegistry`) + `hashConsent()` + `verifyConsentSig()`. **Byte-identical TS/Rust**, with a `consent` section in `testvectors.json`. Exposed over **UniFFI** so mobile signs the *same* struct the backend verifies. |
| **Mobile consent-signing UI** | `apps/android/...` + `apps/ios/...` (Verify/Scan flow) | New "Present credential to verifier" screen: scan verifier QR → show *who* is asking + *what* + *why* (verifierAddress, recordTypes, purpose) → sign `VerificationConsent` with the in-app wallet → POST to central `/v1/verify/consent`. Reuses the §6.4 wallet module; the consent is a DogTag-domain EIP-712 sign (like `recover()`'s `Claim`), **never** a connected dApp. |
| **Consent/verification records (Mongo)** | central `consents`/`consent_receipts` (§9.1, extend) + business `verification_records` (new, §9.2) | Off-chain audit of consents (lawful basis) and attestations (txHash, consentReceiptId, kind). |
| **Deploy + addresses** | `contracts/script/Deploy.s.sol` (extend) + `deployments/roax.json` (+ `VERIFICATION_REGISTRY`, `GROTH16_VERIFIER`) + `.env.example` per business stack (`VERIFICATION_REGISTRY_ADDR`, `GROTH16_VERIFIER_ADDR`) | |

> **Trusted-setup caveat (for `10-zk-groth16.md`):** Groth16 needs a per-circuit trusted setup; document the ceremony (or use a known-good universal ptau + per-circuit phase-2) and that the verifying key is pinned in the checked-in `Groth16Verifier.sol`. Circuit changes ⇒ new setup ⇒ redeploy verifier ⇒ rotate `GROTH16_VERIFIER_ADDR`.

---

## 4. Impact on each existing doc section (section-by-section change map)

### 4.1 `architecture.md`

| Section | Change | How |
|---|---|---|
| **§1 / §1.1** | Add **`circuits`** to the products table; note verification is a first-class capability. | Add row: `ZK circuits | circom + Groth16 | shared | circuits/`. |
| **§3 (data standard)** | Add the **`VerificationConsent` artifact** (EIP-712) as a new standard artifact alongside the wrapped doc; note it commits to the **root**, never the salted `data`. | New §3.7 "Verification consent (EIP-712)" with the struct + domain. |
| **§4.1 (contract set)** | Add **`VerificationRegistry`** and **`Groth16Verifier`** to the contract list. | Two new bullets. |
| **§4.3 (IssuerRegistry)** | Add a **verifier-capability** namespace (reuse `isWhitelistedFor(keccak256("VERIFY:"+purpose), signer)`), distinct from issuer roles. | New paragraph: a verifier need not be an issuer. |
| **NEW §4.7 (`VerificationRegistry`)** | Specify `recordVerification` / `recordVerificationZk`, the `VerificationAttested` event, `consentHash` storage, the Groth16 hookup, and the (dogTagId, groomerRelayer, userWallet) public signals. | New subsection. |
| **§4.6 (interaction map)** | Add the **PRESENT / VERIFY** sequence (the §1.2 flow) next to ISSUE / SHARE / FETCH+VERIFY. | New block in the diagram. |
| **§5 (verification pipeline)** | Clarify the new event is a **separate, on-chain attestation** layered on the existing third-party `verify()` (which it reuses in NORMAL mode); ZK mode replaces raw-doc disclosure with a proof. The three authenticity pillars + contextual ownership are unchanged. | New §5.5 "On-chain verification events (presentation)". |
| **§9 (data model)** | Add **`verification_records`** (business) and extend **`consents`/`consent_receipts`** (central) to cover verification consents. | Add fields/collections. |
| **§10 (mobile)** | Add **consent-signing** to the wallet/Verify flow; the DogTag-domain EIP-712 consent signs like `recover()`. | New bullet in §10.1 + §6.4-equivalent. |
| **§11 (privacy)** | Add the **behavioral-linkage** analysis (§5 below) + ZK as a data-minimization control + DPIA additions. | Expand §11.1. |
| **§13 (remediations)** | Add a **§13.7** normative block: consent EIP-712 binding rules, registry gating, ZK setup/verifier pinning, confirm-hardening reuse, behavioral-privacy mitigations. | New subsection (extend, don't replace). |

### 4.2 `implementation.md`

| Section | Change | How |
|---|---|---|
| **§0 (monorepo layout)** | Add `circuits/` and `crates/dogtag-prover-rs/`. | Edit the tree. |
| **§1 (SDK)** | Add **§1.10 EIP-712 consent** (`hashConsent`, `verifyConsentSig`, domain) to both SDKs + testvectors `consent` section + UniFFI export. | New subsection. |
| **§2 (contracts)** | Add **`VerificationRegistry.sol`** + **`Groth16Verifier.sol`** bodies; extend `script/Deploy.s.sol` (deploy verifier, deploy registry pointing at it + IssuerRegistry; write addrs). | New §2.6/§2.7 + edit §2.5. |
| **§3 (business backend)** | Add the **`/verify/*` endpoints** (`session/start`, `consent/submit`, `prepare`, `confirm`) mirroring the §3.8/§11.6 dual-signing + hardened-confirm pattern; wire the prover. **Explicitly keep `/import/pull` (§3.5) as the decoupled off-chain import** and note ZK mode imports nothing. | New §3.9 + a reconciliation note on §3.5. |
| **§4 (central backend)** | Add `POST /v1/verify/consent` (auth owner → store Consent/ConsentReceipt → relay to verifier). Wire verification consents into the consent/erasure model (§4.5). | New §4.6. |
| **§5 (portals)** | Add a **verifier UI** (start session/QR, show proof + tx, NORMAL/ZK toggle) to the groomer portal (and generic business portals). | Edit §5.2. |
| **§6 (mobile)** | Add the **consent-signing flow** to the Verify/scan path (§6.4 wallet reuse). Note: self-import (§6.5) is **unchanged**. | New §6.6. |
| **§9 (testing)** | Add: consent EIP-712 cross-language parity; Groth16 proof verifies on-chain; registry rejects unbound/replayed consent; NORMAL vs ZK both produce a valid attestation; verifier-capability gating. | Add cases. |
| **§11 (remediations, NORMATIVE)** | Add **§11.8** with the corrected/hardened code: consent verification (recover==userWallet, verifier==activeSigner, one-time challenge), confirm hardening reuse, registry `recordVerification*`, prover boundary (server-only, operator-session). | New subsection (extend). |
| **§10 (build order)** | Insert the ZK/registry/consent phase (see §6 below). | Edit the list. |

### 4.3 `CHANGESPEC-v2.md` (and the `BUILD_PROMPT.md` it drives)

Add a **§7** "On-chain verification events (presentation)" to the change-spec: canonical `Verification`/`Consent` entity names, the `purpose` enum, the `VerificationConsent` EIP-712 struct, the NORMAL/ZK modes, and the per-file update tasks — so future doc-update passes treat it as normative.

---

## 5. Privacy / GDPR impact

### 5.1 What the feature adds to the on-chain surface

Each on-chain verification event links **`userWallet ↔ dogTagId ↔ verifier ↔ timestamp`** (and, in normal mode, `recordType`/`purpose`). This is **new behavioral/relationship data** on an immutable, globally-replicated ledger: *who* got *what* verified, by *whom*, *when*, and *how often*. Per the existing model (arch §11.1, research 07), the **wallet↔SBT link is already pseudonymous personal data in DPIA scope**; this feature **amplifies** it from a static binding to a **time-series of interactions** (a behavioral graph), which is more sensitive and harder to mitigate.

### 5.2 Interaction with the existing privacy model + fresh-per-pet-address mitigation

- **Fresh-per-pet address (§11.1, §13.6) partially mitigates** cross-*pet* enumeration but **does not** stop building a behavioral profile of a *single pet*: every verifier that the same pet visits links to the **same** per-pet `userWallet`. An observer can see "this pet was verified by groomer A, airline B, vet C over time." The per-pet address bounds the blast radius to one pet, not to one event.
- **`recordType`/`purpose` on-chain leaks category** (e.g. a `SERVICE_ATTESTATION` verification would betray Art. 9 special-category context). **Hard rule (extends §11.1's "never on-chain" list):** do **not** put service-animal/disability `purpose` labels on-chain in cleartext; for Art. 9-adjacent verifications use the **ZK path** (no recordType in clear) or a non-revealing purpose hash, and keep the human-readable purpose off-chain in the consent receipt.
- **The EIP-712 consent itself must never be stored on-chain in cleartext** — store only `consentHash`; the consent contents (with its purpose/PII) live in the off-chain `ConsentReceipt`, inside the crypto-shredding erasure scope (§11.6).
- **ZK mode is the principal privacy control.** With public signals = (dogTagId, groomerRelayer, userWallet), the chain learns *that* a valid credential of some kind was verified and consented to, **without** the credential's root, fields, or recordType. This is the data-minimization answer for the behavioral-linkage problem. **Recommendation:** default Art. 9-adjacent and government/airline presentations to **ZK**; allow NORMAL only where the verifier legitimately needs the disclosed fields off-chain anyway.
- **Erasure (§11.6) extends but cannot fully cover this.** The off-chain `Consent`/`ConsentReceipt`/`verification_records` are deletable/crypto-shreddable and the SBT burn drops the live `ownerOf` link, but the **on-chain `VerificationAttested` events are immutable** (like `RootIssued`/`Locked`). They remain `(userWallet, dogTagId, verifier, ts)` tuples forever. Mitigation: the per-pet address + burn make the tuple **unlinkable to the natural person** once off-chain links are destroyed — **same posture as today: a documented mitigation, NOT a regulator-blessed safe harbour.** ZK mode reduces what is anchored in the first place.

### 5.3 DPIA additions (mandatory; extends arch §11.1 / §13.5)

The DPIA must now additionally record:
1. **New processing:** on-chain verification/presentation events create a behavioral interaction graph keyed on the pseudonymous per-pet `userWallet`.
2. **Lawful basis + purpose limitation:** each event is gated by an explicit, withdrawable, purpose-bound **EIP-712 consent** with a `ConsentReceipt` (consent is the lawful basis; tie to §4.5 consent/retention).
3. **Data minimization:** ZK mode (no root/recordType on-chain) as the default for sensitive purposes; cleartext `purpose` for Art. 9 contexts is **prohibited** on-chain.
4. **Verifier as a (joint) controller** for the attestation it records; cross-controller erasure propagation (central → business, §11.6) must cover `verification_records` and `consent_receipts`.
5. **Residual risk:** immutable `VerificationAttested` tuples; copy-proliferation of consents; unreachable third-party caches — explicitly logged as residual, with per-pet-address + burn + ZK as the mitigations, refreshed whenever on-chain fields or chain topology change.

---

## 6. Build-order / phase impact (BUILD_PROMPT)

The ZK circuit + `VerificationRegistry` + consent flow slot in as **a new phase between contracts and backends**, with additions threaded into the existing mobile and business-portal phases. Proposed edits to BUILD_PROMPT's phased plan:

- **Phase 1 (SDKs):** add the **EIP-712 `VerificationConsent`** struct + `hashConsent`/`verifyConsentSig` to both SDKs and a `consent` block in `testvectors.json` (it's part of the deterministic trust core and is consumed by mobile + backend). UniFFI-export it.
- **Phase 2 (contracts):** add **`VerificationRegistry`** + the verifier-capability gate (IssuerRegistry namespace) and the `VerificationAttested` event + Foundry tests (consent-bound, replay-rejected, capability-gated). The **`Groth16Verifier`** is generated in the new ZK phase, so Phase 2 deploys the registry with the verifier address wired in afterward (or deploy registry, then `setVerifier`).
- **NEW Phase 2.5 — ZK circuit + Groth16 setup (between contracts and backends):** scaffold `circuits/` (circom `verification.circom`, public signals (dogTagId, groomerRelayer, userWallet)), run the trusted setup (ptau + per-circuit zkey), generate + check in `Groth16Verifier.sol`, deploy it, wire it into `VerificationRegistry`. Acceptance: a proof generated off-chain `verifyProof`s true on-chain; a tampered witness fails. **This is the natural home for the bulk of the feature's contract+crypto risk.**
- **Phase 3 (business backend):** add the **`/verify/*` endpoints** (session/start, consent/submit, prepare, confirm) + wire **`crates/dogtag-prover-rs`** (NORMAL reuse of `verify()`; ZK proving). Reuse the dual-signing `prepare`/`confirm` + hardened-confirm machinery. Keep `/import/pull` decoupled. Operator-session auth on all `/verify/*`.
- **Phase 4 (central backend):** add `POST /v1/verify/consent` (store Consent/ConsentReceipt, relay to verifier); extend the consent/retention/erasure model to cover verification consents and `verification_records`; extend cross-backend erasure propagation.
- **Phase 5 (portals):** add the **verifier UI** (start session/QR, NORMAL/ZK toggle, show proof + on-chain tx) to the groomer (and generic business) portal.
- **Phase 6 (mobile):** add the **consent-signing flow** to the Verify/scan path (DogTag-domain EIP-712 sign via the §6.4 wallet; show who/what/why). Self-import (§6.5) is **unchanged**.
- **Phase 8 (hardening):** add a **behavioral-privacy gate**: assert no Art. 9 `purpose` is ever anchored in cleartext; ZK is the default for sensitive purposes; consent is verified on-chain (recover==userWallet, verifier==signer, one-time challenge); erasure covers `verification_records`/`consent_receipts` and propagates to business backends; the on-chain `VerificationAttested` tuple is acknowledged as immutable pseudonymous data in DPIA scope.
- **Non-negotiable principles:** add a 9th — "Verification is consented and minimized: every on-chain verification event is gated by a wallet-signed EIP-712 consent that captures the verifier address; prefer ZK so the chain learns the minimum; verification (on-chain proof) is decoupled from import (off-chain data)."

---

## 7. Summary of the integration-accurate decisions

1. **Decoupling:** on-chain **verification** (proof) ≠ off-chain **import** (data). `/import/pull` stays; `/verify/*` is new; ZK mode does verification with **no** import.
2. **Consent binds the verifier:** EIP-712 `VerificationConsent` over the **root** (never salted data), capturing `verifier`, `userWallet`, `dogTagId`, `purpose`, one-time `challenge`; signed by the in-app wallet; verified on submit (recover==userWallet, verifier==activeSigner) and re-bound on confirm (signer==consent.verifier).
3. **Generalization:** one first-class **`Verification`/`Presentation`** event type (the xlsx "Credential Presentation Event") for groomer/vet/airline/gov, keyed by a `purpose` enum; verifier-capability is gated separately from issuer roles (reuse `IssuerRegistry` namespace).
4. **New components:** `circuits/`, `VerificationRegistry.sol` + `Groth16Verifier.sol`, `crates/dogtag-prover-rs`, SDK `consent` module (TS+Rust+UniFFI), mobile consent UI, `verification_records` Mongo.
5. **Reuse, don't reinvent:** the dual-signing `prepare`/`confirm` + hardened-confirm pattern (§3.8/§11.6) and the consent/erasure model (§4.5/§11.6) carry the new flow.
6. **Privacy:** new behavioral linkage on-chain; ZK is the data-minimization default for sensitive purposes; never anchor Art. 9 `purpose` in cleartext; DPIA + erasure scope extended; immutable attestation tuples are documented residual risk.
7. **Phasing:** a new **Phase 2.5** (circuit + Groth16 setup + registry) between contracts and backends, with additions to Phases 1, 3, 4, 5, 6, 8.
