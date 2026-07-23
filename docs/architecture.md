# DogTag Ecosystem — Architecture

> Status: v4 design (Poseidon-unified credential commitment — single root `R`; issuance + dual-signing + wallet + granular SBT + on-chain proof-of-verification, audit-remediated). Source research: [`docs/research/`](./research) (briefs `01`–`13`, audits `01`–`12`, `CHANGESPEC-v2`/`-v3`/`-v4`). **Normative precedence: §13 overrides §1–§12; highest-numbered §13 sub-section wins (§13.9 = v4.1, latest); `CHANGESPEC-v4` (Poseidon unification) overrides all earlier hash/dual-root wording on conflict.** Reference UI/data: [`references/`](../references).
> Chain: **ROAX** (EVM, chainId `0x87` = **135**, native gas token **PLASMA**, RPC `https://devrpc.roax.net`, explorer `https://explorer.roax.net` — Blockscout-style). RPC was returning `502` at design time; treat liveness as a deploy-time pre-check.

---

## 1. Vision & scope

DogTag is a **pet-credentialing ecosystem**. Pet owners hold their pets' identity, health, service, and travel records in a mobile app. Veterinarians and groomers (and later governments/airlines) run software that **issues and consumes verifiable credentials** about pets. Credentials are **anchored on-chain** as a **single Poseidon Merkle root `R`** (the credential commitment - leaf hash, Merkle tree, and consent nullifier are all Poseidon; §3.3/§3.4) and **verifiable three ways**: cryptographic integrity, on-chain issuance/revocation status, and **DNS-bound issuer identity** - the OpenAttestation trust triangle, implemented here **from scratch** with a **language-agnostic, JSON-free canonicalization**. The pet OWNER never appears on-chain: a DogTag is minted to a neutral custodian (`DogTagSBTConsent.mintCustodial(dogTagId, R)` - no recipient address, write-once `profileRoot`), and the owner exists only as hidden leaves of the tag's tree, folded into `R` on the owner's device (§4.2).

### The two proof primitives (normative model)

Every flow in the protocol is one of two proofs:

- **A MERKLE PROOF is issuance.**
  Verifying a record's or credential's integrity, and selectively disclosing its fields, is a Merkle inclusion check against the anchored Poseidon root `R` (or a record root).
  Redaction (`obfuscate`) moves a leaf hash into `privacy.obfuscated[]`; the root never changes (§3.5).
- **A ZK PROOF is consent.**
  A Groth16 proof over `circuits/consent.circom` (`DogTagConsent(6)`) proves, without revealing the owner, that the owner of tag `dogTagId` consented to a verification for `purpose` bound to the verifying operator (`relayer`), yielding public signals `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
  `VerificationRegistryConsent` checks the proof on-chain (`msg.sender == relayer`, `R == profileRoot(dogTagId)`) and emits a subject-less `Verified` event (§4.7).

The tag's OWNER is a HIDDEN LEAF: three reserved leaves (owner-address, consent-key, owner-secret) plus the disclosable pet-attribute leaves fold into `R` on the owner's device, and the issuer seals only `R` on-chain.
Records (vaccinations, certificates, boarding) are OWNERLESS issuer-anchored attribute trees, bound to a tag via `dogTagId -> profileRoot`; owner-hidden is a property of the TAG, not of each record.

### System capabilities

1. **Issue** verifiable credentials (Merkle roots anchored on-chain, DNS-bound issuer).
2. **Verify** a credential's authenticity (3 pillars; §5).
3. **Import / share** records (QR + one-time tokens, off-chain operational data).
4. **On-chain proof-of-verification** - a verifier (groomer/vet/airline/gov) records on-chain, **with the owner's ZK-proven consent**, that it validated a credential - without the chain or the verifier ever learning who the owner is (the Groth16 consent proof above; no credential data on chain). Verifier capability is gated by a `VERIFY:` whitelist namespace, **separate from issuer roles** (a groomer can verify without being an issuer). See §3.6, §4.3, §4.7, §13.7.

### 1.1 Products in this monorepo

| Product | Tech | Who runs it | Folder |
|---|---|---|---|
| Pet-owner app (Android) | Kotlin + Jetpack Compose | End users | `apps/android` |
| Pet-owner app (iOS) | Swift + SwiftUI | End users | `apps/ios` |
| Pet-owner (holder) wallet (web) | React+Vite SPA, **no backend** (holds credentials in localStorage) | End users (browser) | `stacks/owner/web` |
| Vet portal stack | React+Vite SPA + Rust API + MongoDB | **Each vet, self-hosted** (or we host) | `stacks/vet` |
| Groomer portal stack | React+Vite SPA + Rust API + MongoDB | **Each groomer, self-hosted** (or we host) | `stacks/groomer` |
| Admin / central stack | React+Vite SPA + Rust API + MongoDB | **We (protocol)** | `stacks/admin` |
| Smart contracts | Solidity + Foundry | Deployed once to ROAX | `contracts` |
| Data standard SDK (TS) | TypeScript | Shared (portals, web) | `packages/dogtag-standard-ts` |
| Data standard SDK (Rust) | Rust crate | Shared (all backends) | `crates/dogtag-standard-rs` |
| Shared UI | React component lib | Shared (portals) | `packages/ui` |

### 1.2 The "two backend server types" model

- **Business backend** (vet/groomer): **self-sovereign, self-hosted, one instance per business.** Holds its own keys, its own MongoDB, its own domain. Signs and broadcasts its own on-chain transactions. The vet/groomer never sees web3, gas, or a wallet — the frontend just POSTs to its own backend.
- **Central backend** (admin stack): **one instance, run by us.** Powers the pet-owner mobile apps (accounts, pet ownership, discovery) **and** the protocol admin functions: the **business registry/directory** (discovery + each business's API URL), and **issuer whitelisting** (the on-chain gate). The central backend is the **system of record for appointments**.

```
                         ┌──────────────────────────────────────────┐
                         │        ROAX blockchain (EVM, 135)          │
                         │ DogTagSBTConsent · IssuerRegistry · Issuers│
                         └───────▲───────────────▲──────────────▲─────┘
        whitelist tx (admin)     │   issue/revoke│  read (verify)│
                                 │               │               │
   ┌──────────────────┐   ┌──────┴──────┐  ┌─────┴───────┐  ┌────┴────────┐
   │  Central / Admin  │   │  Vet stack  │  │ Groomer stk │  │ Mobile apps │
   │  (we host)        │   │ (self-host) │  │ (self-host) │  │ (devices)   │
   │ • mobile-user API │   │ • Rust API  │  │ • Rust API  │  │ • Android   │
   │ • business registry│  │ • own keys  │  │ • own keys  │  │ • iOS       │
   │ • whitelisting    │   │ • MongoDB   │  │ • MongoDB   │  │             │
   │ • appt source-of- │   │ • SPA       │  │ • SPA       │  │             │
   │   truth           │   └─────────────┘  └─────────────┘  └─────────────┘
   │ • MongoDB · SPA   │          ▲                 ▲              │  │
   └───────▲───────────┘          │ booking sync    │ booking sync │  │ scan QR,
           │ discovery, booking,   └─────────────────┘              │  │ verify,
           │ ownership             ◀── direct record fetch (QR/JWT) ─┘  │ import
           └────────────────────────────────────────────────────────────┘
```

---

## 2. Network & deployment topology

> Status: the contract set targets **ROAX (chainId 135)** - the deployment ledger is
> `contracts/deployments/roax.json`; demo runbook in `docs/DEMO.md`. The testnet is disposable:
> with the owner-revealing path retired, it is wiped and redeployed fresh (decision D5), so no
> pre-unification records survive. This section remains the normative design.

Every business stack and the admin stack is a self-contained **Docker Compose** project. **Uncommon, non-overlapping host ports** (server already hosts other apps). MongoDB is **never** published to the host — internal to each compose network only.

| Stack | web (SPA/nginx) | api (Rust) | mongo (internal only) |
|---|---|---|---|
| admin (ours) | **39741** | **39742** | 39743 (internal) |
| vet | **41873** | **41874** | 41875 (internal) |
| groomer | **43617** | **43618** | 43619 (internal) |

- Externally exposed: each stack's `web` port, and its `api` port (mobile apps + cross-backend sync call the API directly). Mongo bound to the compose network only.
- Each business stack sits behind the operator's own TLS reverse proxy / domain (`https://vet.example.com` → `web`/`api`). The **domain is the identity anchor** for DNS verification.
- The admin stack is reachable at our fixed domain (e.g. `https://api.dogtag.io`) which mobile apps are configured against.

See `implementation.md` §Docker for the compose files and `.env` schema.

---

## 3. The DogTag Open Pet Credential standard (data layer)

This is the **open-sourced, library-agnostic** core. Identical results in TypeScript, Rust, and Solidity (and circom — §4.7). Full rationale: [`research/02-attestation.md`](./research/02-attestation.md), [`research/01-data-standards.md`](./research/01-data-standards.md), [`research/13-poseidon-unification.md`](./research/13-poseidon-unification.md).

> **Hash policy (v4 - Poseidon-unified).** The **credential commitment uses Poseidon** over BN254 - the leaf hash, the Merkle tree, and the consent nullifier (so the same root `R` is provable in-circuit). **keccak256 (Ethereum Keccak, padding `0x01`, not NIST SHA3-256 `0x06`) is retained ONLY where the EVM/ECDSA standards mandate it and the value never enters a circuit as a raw keccak digest:** (1) Ethereum address derivation and transaction signing at the EVM boundary; (2) pure namespacing keys - `recordType = keccak256(label)`, the `VERIFY:` whitelist keys, and the clone `salt`. A keccak label that must enter the circuit (`purpose`, `recordType`) is reduced `mod r` once at the field boundary. Everything that is part of the credential commitment or enters the circuit is Poseidon.

### 3.1 Wrapped-document shape

A credential, once issued, is a **wrapped document**:

```jsonc
{
  "version": "dogtag/1.0",
  "data": { /* the salted credential fields — see 3.2 */ },
  "signature": {
    "type": "DogTagMerkleProof",
    "targetHash": "0x…",   // merkle root of THIS document's leaves
    "proof": [],            // sibling hashes to reach the batch root (empty for single-doc)
    "merkleRoot": "0x…"     // value anchored on-chain (== targetHash when proof is empty)
  },
  "issuer": {
    "name": "Seaport Animal Hospital",
    "domain": "vet.seaport.example",        // DNS identity
    "documentStore": "0x…",                 // issuer contract address
    "recordType": "VACCINATION"
  },
  "privacy": { "obfuscated": [] },           // hashes of redacted leaves (selective disclosure)
  "protocol": {                              // OPTIONAL provenance block (M7 §4.2); absent on pre-M7 docs, carried BESIDE R (never inside it)
    "chainId": 135,
    "version": "dogtag-levelb/1",            // the internal protocol version key (an internal identifier, not a product label)
    "verificationRegistry": "0x…",           // routing key: which verification registry this record targets
    "issuerClone": "0x…",                    // == issuer.documentStore
    "issuerSigner": "0x…"                     // CLAIM of who issued; validated vs on-chain issuedBy[R], never treated as authority
  }
}
```

> **The optional `protocol` block (M7 provenance, M7 §4.2)** records which protocol/contract the credential was created on and who issued it, carried **BESIDE `merkleRoot` - never inside `R` or the ZK proof** - so a receiver can route to the right registry/clone. Pre-M7 docs omit it and stay verifiable. The `version` string is the internal protocol version key whose keccak keys the on-chain `ProtocolRegistry`; it is a load-bearing identifier and is never renamed. `issuerSigner` is a routing **hint** (the issuer's claim), never authority: `verify()` may validate it against on-chain `issuedBy[R]` and always re-derives issuance against `issuer.documentStore`, never the untrusted block.

> **Single-record now, batch later** (your decision): `proof` is empty and `merkleRoot == targetHash` today. When batching is added, `targetHash` stays the per-document root, `proof` carries batch siblings, and `merkleRoot` becomes the batch root. **The anchored value is always a `bytes32` root and verification always calls `isValid(root)` — no format break.**

### 3.2 Salted leaves (selective disclosure)

Every **scalar field** of the credential becomes its own Merkle leaf, salted with **16 random bytes** so individual fields can later be redacted without changing the root, and so values aren't guessable from their hash. This is your `{ "uuid:value" }` idea, hardened.

A leaf is the tuple `(keyPath, salt, typeTag, value)`:

- `keyPath`: dotted path, e.g. `credentialSubject.microchip.code`. **NFC-normalized**, UTF-8.
- `salt`: 16 raw random bytes (one per field; stored in `data` so the holder can prove a field).
- `typeTag` (`uint8`): `0=null, 1=bool, 2=string, 3=integer, 4=decimal, 5=bytes`. **Mandatory** so `"5"` (string) ≠ `5` (integer).
- `value`: canonical bytes per type (see 3.3).

`data` stores each field as its salted, type-tagged string so it's self-describing and human-inspectable:

```jsonc
"data": {
  "credentialSubject": {
    "microchip": { "code": "a3f1…(16-byte salt hex):2:985141006580311" },
    "weightHistory[0].value": "9b22…:4:22.7"
  }
}
```

### 3.3 Canonical leaf hashing (the algorithm — Poseidon)

The credential commitment is **Poseidon over BN254 field elements** (not a byte-string hash), because the same leaves must be proven in the Groth16 circuit (§4.7). Poseidon hashes field elements `<254` bits, so byte strings (`keyPath`, `salt`, `value`) are first packed into fields via the injective, length-bound **`fieldOf`/`bytesToField`** map (component-hash approach — circomlib Poseidon arity is compile-time-constant, so we fold limbs through Poseidon rather than raw-absorbing):

```
fieldOf(bytes x) -> field:                      // injective, length-bound, multi-limb
   b = u64be(len(x)) ‖ x                         // 8-byte big-endian length prefix
   limbs = split b into 31-byte big-endian limbs (each < 2^248 < p, no wraparound)
   acc = DS_BYTES; for L in limbs: acc = Poseidon(acc, fieldFromLimb(L))   // domain-sep fold
   return acc
fieldOf(scalar uint) -> field: the integer reduced into [0,p)   // 15-digit chip, timestamps, typeTag, addresses(uint160) all fit one field

hashLeaf = Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt16), fieldOf(typeTag), fieldOf(value))   // 5 inputs → circomlib Poseidon width t=6
```

- The 8-byte big-endian length prefix inside `fieldOf` is what kills intra-leaf second-preimage ambiguity (replacing the old per-component `uint32` length prefixes).
- **`encodeValue` is UNCHANGED** — the NFC normalization, the pinned decimal grammar, the no-float guard, and the mandatory `typeTag` are identical to the prior spec; only the final hashing changes from keccak to Poseidon. Value encoding rules (deterministic across languages), unchanged:
  - `null` → empty bytes.
  - `bool` → `0x00`/`0x01`.
  - `string` → **NFC-normalized** UTF-8 bytes.
  - `integer` → decimal ASCII, **no leading zeros, no `-0`** (arbitrary precision; covers microchip IDs and lot numbers as exact integers — never floats).
  - `decimal` → fixed decimal **string** (e.g. weight `"22.7"`), normalized (no trailing zeros beyond significant, single canonical form). **Native floats are forbidden.**
  - `bytes` → raw bytes.

> Implementations of `fieldOf`/`encodeValue` are byte-identical across TS, Rust, and Solidity; the final Poseidon uses the pinned instantiation in §3.4 (one parameter set, four libraries, CI anchor vector). The `fieldOf` packing is a hand-rolled limb fold (not `abi.encode`) so it is trivially identical in all environments without an ABI codec.

### 3.4 Merkle tree build (Poseidon)

```
1. Compute hashLeaf (§3.3) for every field.
2. Sort leaf hashes ascending as integers in [0,p). (deterministic order, ignores field order)
3. Build bottom-up:
     parent = Poseidon( DS_NODE, min(a,b), max(a,b) )      // 3 inputs; commutative
     a,b compared as integers in [0,p)
   - A lone odd node is PROMOTED unchanged to the next level (never duplicated).
4. Root of the tree = the credential commitment R (== targetHash for this document).
   - Single-leaf document: R = that one leaf hash.
```

- **Commutative sorted-pair (`min`/`max`)** ⇒ proofs are unordered sibling sets; on-chain verification needs no left/right bits. The in-circuit ordered tree applies the same sortPair+mux so the proven root equals the SDK's `R`. Odd-promotion and the single-leaf root are preserved exactly.
- **Domain tags** replace the old keccak `0x00`/`0x01` bytes and are used as the **first input slot** (NOT a capacity IV, to stay on the exact circomlib API across all four libs): `DS_LEAF=1`, `DS_NODE=2`, `DS_BYTES=3`, `DS_NULLIFIER=4`. They prevent leaf/node/byte/nullifier-domain confusion.

> **Pinned Poseidon instantiation.** ONE circomlib BN254 parameter set — x⁵ S-box, `R_F=8`, per-`t` `R_P`, seed `"poseidon"`, circomlib MDS/round-constants. **Four pinned libraries:** circom → circomlib `Poseidon`; TS → **`poseidon-lite`**; Rust → **`light-poseidon`** (`new_circom(n)`, Veridise-audited); Solidity → **`poseidon-solidity`** (`PoseidonT3..T7`). **CI MUST assert bit-identical output across all four** against the anchor vector `poseidon([1,2]) = 0x115cc0f5...189a` (circomlibjs has historically drifted — pin + test). Full rationale: [`research/13-poseidon-unification.md`](./research/13-poseidon-unification.md).

### 3.5 Selective disclosure / obfuscation

To redact a field while keeping the same root: move the field's **leaf hash** into `privacy.obfuscated[]`, delete its cleartext from `data`. Verifier recomputes the leaf set as `(hashes of remaining fields) ∪ privacy.obfuscated`, rebuilds the tree, and gets the **same `targetHash`**. Lets a pet owner share, e.g., rabies status without revealing owner address.

### 3.6 Credential schemas (W3C VC 2.0 envelope)

All credentials wrap in **W3C Verifiable Credentials Data Model 2.0**. The envelope is canonicalized exactly per §3.2–§3.4 (Poseidon salted leaves; **we do NOT adopt JSON-LD/RDF canonicalization** — SMART Health Cards / EU DCC lesson: anchor only a hash/root, never RDF-canonicalize). Envelope fields (canonical, per CHANGESPEC §0):

- `@context`: **URI array** — `["https://www.w3.org/ns/credentials/v2", "<DogTag context URI>"]`. **Human prose never goes in `@context`** — it goes in `description`.
- `type`: **token array**, e.g. `["VerifiableCredential","RabiesVaccinationCertificate"]`.
- `id`, `issuer`, `validFrom`, `validUntil`, `credentialSubject`.
- `credentialStatus`: revocation pointer — **mirrors the on-chain `isValid(root)`** (revocation is first-class).
- `credentialSchema`: schema reference.
- Legal/trust meta on **every** credential: `attestationType`, `signatureTrustTier` (`accredited_authority`|`licensed_vet`|`self_attested`), `legalEffect` (`evidentiary`), `legalBasisVersion`, `jurisdiction`.

Record types map to the xlsx **Unique Events** (`recordType` on-chain = `keccak256(label)`):

| `recordType` | Credential | Issuer | Anchored by |
|---|---|---|---|
| `DOG_PROFILE` | DogTag pet identity (mints SBT) | DogTag protocol | central or self-host |
| `VACCINATION` | Rabies/other vaccine certificate | Vet | vet stack |
| `SERVICE_ATTESTATION` | Service/assistance attestation (trust-tiered) | Vet/trainer/handler | vet stack |
| `TRAVEL_CLEARANCE` | Intra-EU travel clearance | EU competent authority (future) | future stack |
| `EU_HEALTH_CERT` | EU Annex IV health certificate (USDA-endorsed) | USDA APHIS (future) | future stack |
| `DOT_SERVICE_FORM` | DOT service-animal air form (self-attested) | Handler (off-chain trust) | **off-chain only** |
| `CDC_IMPORT_FORM` | CDC dog import form | **Off-chain only** (app + email) | not on-chain |

#### Finalized field sets (canonical names per CHANGESPEC §0)

**`Owner` entity — first-class, off-chain PII only, never on-chain.** `{name, addresses[], phones[], email, emergencyContact, contactUpdatedOn}`. The **record-custodian (the issuing vet/clinic — legal owner of the record) is distinct from the pet-owner** (information-access rights). `Dog.ownershipHistory[]{ownerId, from, to}`.

**Dog identity** (`DOG_PROFILE`, mints the SBT): `dogTagId` (SBT tokenId), `name`, `species` (top-level), `breedVbo` (Vertebrate Breed Ontology id, e.g. `VBO:0200798`) + `breedLabel`, `sex` (`male`|`female`) **separate from** `neuterStatus` (`intact`|`neutered`|`spayed`), `dateOfBirth` (derive age - drop free-text age), `colour`, `distinctiveFeatures`, `weightHistory[]{value, unit:"kg"|"lb", measuredOn}` (unit-bearing + dated), `microchip`, `photoHashes[]` (off-chain blobs, hash only).
The tag's tree is built **on the owner's device** (`build_profile_tree`): the disclosable pet-attribute leaves above plus three reserved HIDDEN owner leaves - `owner.address` (the wallet address as an opaque field), `owner.consentKey` (leaf value `keyHash = Poseidon(Ax, Ay)` of the per-tag consent key), and `owner.secret` (the nullifier secret) - all derived from the wallet seed and never sent to the issuer.
Like the rest of the profile they are salted Merkle leaves, never on chain in cleartext.
**`ownerIdentity{countryOfIdentification, identification, name}`** - the human behind the device (ISO country + gov-ID/passport number + official name as on the ID) - is **vet-entered at issue time** (§4.3 vets-issue-dog-tags; implementation §3.11) and stays off-chain with the issuing vet (the record-custodian); committing it into `R` as a hidden, selectively-disclosable leaf is planned but not yet implemented (see `DPIA.md`).
**Attribute leaves are additive and do NOT change the consent circuit** (§4.7): the circuit proves only the three reserved owner leaves' inclusion in `R` (tree depth 6, up to 64 leaves), so adding or changing disclosable attribute leaves never changes the circuit's public signals.

**`microchip` object** (never a float, never a bare number): `{code: string(15), standard: enum("ISO_11784_11785","OTHER"), implantDate, bodyLocation}`. `implantDate` mandatory (EU/VEHCS enforce "vaccination date ≥ implant date").

**Rabies / vaccine block** (coded, hashes identically across jurisdictions — EU DCC lesson): `vaccineProductCode` (USDA APHIS Veterinary Biologics PCN) + `vaccineProductName` + `vaccineManufacturer` (separate from product), `batchLotNumber`, `vaccinationDate`, `validFrom`, `validUntil`, `nextDueDate` (CDC + VEHCS require "date next due"), `authorizedVet`, `series` (`primary`|`booster`), optional `titer{labId, sampledAt, resultIUml}`. **The vaccine credential references `dogTagId` only — it does NOT copy name/breed/etc.** (stop duplicating identity → reduces drift + on-chain hash payload).

**Service/assistance attestation** (`SERVICE_ATTESTATION`) — a **trust-tiered attestation, not a boolean**: `assistanceType` (`service_dog`|`emotional_support`|`none`; ESA distinct from service dog), `issuerTrustTier` (`adi_accredited`|`licensed_pro`|`handler_self_attestation`|`unverified_registry`), `taskDescription`, `legalContext[]` (`ADA`|`ACAA`|`FHA`). **Special-category (GDPR Art. 9) data — off-chain only, NEVER hashed on-chain.** No `disability_verified` field.

**Issuer accreditation** (mandatory, structured — not free text): `usdaNan` (6-digit National Accreditation Number), `nvapCategory`, `license{number, jurisdiction, expiry}`, `aphisEndorsement{vehcsRef, endorsedAt}` (for exports). Export certs are **layered/multi-issuer** (accredited vet → APHIS VEHCS endorsement chain).

**Schema invariants enforced at issuance** (encode as validators in both SDKs):

- **Microchip `code`**: `^[0-9]{15}$` (ISO 11784/11785), conditional (required for EU + CDC paths). Cross-credential join key.
- **Rabies mandatory fields**: `vaccineProductCode`, `vaccineProductName`, `vaccineManufacturer`, `batchLotNumber`, `vaccinationDate`, `validFrom`, `validUntil`, `nextDueDate`, `authorizedVet`. (Omitting name/manufacturer/batch = EU non-compliance.)
- **Validity invariants**: `microchip.implantDate` ≤ `vaccinationDate`; animal ≥12 weeks at vaccination (EU); `validFrom = vaccinationDate + 21 days` for a primary series (booster-aware — continuous boosters skip the wait); titer `resultIUml` ≥0.5 when applicable; EU AHC valid 10 days→entry then 4 months; CDC receipt valid 6 months; CDC dogs ≥6 months at entry.
- **DOT form** is **handler self-attestation under 18 U.S.C. §1001** — issuer is the holder, not a vet; off-chain only; record only that an attestation exists, never "verified disability".
- **Legal posture is evidentiary, not authoritative** — `legalBasisVersion`/`jurisdiction` versioned (EU 2013 acts are being recodified).

Full field tables per document type: [`research/01-data-standards.md`](./research/01-data-standards.md).

#### Verification artifacts (proof-of-verification)

A **`Verification`** event (a.k.a. **Presentation**) is a first-class credential-presentation event - it generalizes the xlsx "Credential Presentation Event" rows (e.g. the **Travel Request** / **DOT Airline Form Presentation**). It records, on-chain, that some verifier validated a credential with the owner's consent. It is keyed by `purpose` (a keccak label reduced `mod r`), with labels such as `GROOMING_INTAKE`, `TRAVEL_PRESENTATION`, `AIRLINE_CHECKIN`, `VET_INTAKE`. The event itself lives on `VerificationRegistryConsent` (§4.7); the consent that authorizes it is the **ZK consent proof** below.

**Consent is a ZK proof, not a signed message.**
The owner's device proves, in zero knowledge, that the hidden owner of `dogTagId` consented to this one verification:

- The **per-tag consent key** (EdDSA-BabyJubjub, derived on-device from `(wallet seed, dogTagId)`) signs `M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)`.
  The key never appears on-chain: it lives inside the tag's tree as the `owner.consentKey` leaf (leaf value `keyHash = Poseidon(Ax, Ay)`), and the circuit proves that leaf's inclusion in `R`.
- The circuit also proves inclusion of the `owner.address` and `owner.secret` leaves in the same `R`, and computes the **nullifier** `Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)` - relayer-bound and subject-less.
  One signed consent = one recorded attestation: replaying it repeats the nullifier and is rejected on-chain; a fresh `consentNonce` is a new consent.
- Public signals (frozen order): `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
  There is no owner/subject signal and no key signal - nothing in the proof, the calldata, or the event names the owner.
- Consent is **not revocable**: a ZK consent proof is a point-in-time act, and there is nothing to un-give.
  Records and credentials, by contrast, are revocable by their issuer (§4.4).

> **Historical note.** The retired owner-revealing path authorized verifications with an EIP-712 `VerificationConsent` typed-data signature naming a `subject` (the owner's wallet), plus an on-chain consent-key registry binding a single wallet-scoped BabyJubjub key. Both are removed: consent is proven in-ZK or not at all.

### 3.7 `dogTagId` encoding — operator handle vs on-chain field element

A dog tag has **two encodings of the same id**, and they must never be confused:

- The **handle** — a small numeric, operator-facing id (e.g. `"3"`). It is what the vet wizard allocates, what the human reads, and **the value of the credential's `credentialSubject.dogTagId` leaf** (a tag-3 Integer scalar).
- The **on-chain id** = `field_of_value(Integer(handle))` — a single BN254 field element obtained by hashing the handle's canonical encoded value through the §3.3 `fieldOf` map. This is the value used **everywhere on-chain and in-proof**.

**Why they must coincide.** `consent_assemble` field-hashes the handle **exactly once** and uses that identical field element for both the circuit's public `dogTagId` input and the `build_profile_tree` KDF binding that produces `R` (`crates/dogtag-standard-rs/src/consent_assemble.rs`).
The custodial mint (`mintCustodial(id, R)`) must use the same field as its `id`: the registry's owner-hidden binding is `R == profileRoot(dogTagId)`, so a mismatch anywhere fails closed at verification.
Anchoring the raw handle on-chain would break this binding.

| Used as | Value |
|---|---|
| Operator-facing id / human-readable handle | raw handle (`"3"`) |
| `credentialSubject.dogTagId` **leaf value** | raw handle (Integer scalar) |
| DOG_PROFILE SBT **token id** (`mintCustodial`, `profileRoot`) | `field_of_value(handle)` |
| Circuit **`pub[0]`** / §3.6 consent `dogTagId` | `field_of_value(handle)` |
| EdDSA message term + Poseidon **nullifier** term | `field_of_value(handle)` |
| Device **mint-confirmation poll key** (`profileRoot`) | `field_of_value(handle)` |

**One implementation, reused everywhere:** the FFI `dog_tag_id_field_hex` (`crates/dogtag-standard-rs/src/ffi.rs`), the `field-hash` bin, the backend's `routes.rs::onchain_dog_tag_id` (drives the mint, the id-allocation `profileRoot` collision check, and the post-mint read-back), the SDK's `consent_assemble`, and `dogTagIdFieldHex` in the mobile scan/import code (both Android and iOS) all compute the same field element. (This is **distinct from** the §11.1 rule that the handle must be a non-personal id - never any hash *of the microchip*. The handle is a random/sequential value; `field_of_value` is just its on-chain/in-circuit encoding.)

---

## 4. Smart-contract architecture

Solidity + Foundry, OpenZeppelin v5. Full snippets/signatures: [`research/03-chain-contracts.md`](./research/03-chain-contracts.md). Deploy/verify: §8.

### 4.1 Contract set

```
DogTagSBTConsent    - ERC-721 + ERC-5192 soulbound, custodially minted. One non-transferable token
                      per pet, minted to a neutral custodian (no recipient parameter);
                      write-once profileRoot(dogTagId) = R.
IssuerRegistry      - central AccessControl whitelist of issuer signing addresses (the gate).
DogTagIssuer        - per-record-type anchoring contract (implementation, cloned). Issues/revokes
                      bytes32 merkle roots; every write gated by IssuerRegistry.
DogTagIssuerFactory - deploys DogTagIssuer EIP-1167 clones (one per record type / per business);
                      doubles as the write-once rootIssuer[R] index (registerRoot at issue - §13.9).
VerificationRegistryConsent - records owner-blind Verification/Presentation events from Groth16
                      consent proofs; nullifier set; gated by the VERIFY: whitelist namespace (§4.3, §4.7).
Groth16VerifierConsent - snarkjs-generated BN254 verifier for consent.circom (the frozen ceremony VK; §4.7).
ProtocolRegistry    - the on-chain discovery anchor: two-axis (contract set + artifact set) version
                      records that apps validate platform claims against (impl §3.10c/§3.10d).
```

`DogTagIssuer` needs **no ZK-specific additions** (v4 - Poseidon unification). There is **ONE root `R`** (Poseidon) anchored by `issue(R)`; the circuit proves that same `R`, so `VerificationRegistryConsent` checks `DogTagIssuer.isValid(R)` **directly** on the public root (resolving the clone from the write-once `rootIssuer[R]` index - §13.9). The former dual-root machinery - `zkCommit(rKec, rZk)`, the `ZkCommitment` event, the `kecOf[rZk]` mapping, and the `zkIndex`/`cloneOf`/`issuerForAny` lookups - is **deleted** (CHANGESPEC-v4 §2). Issuance adds **zero** on-chain hashing (still just stores a `bytes32`).

### 4.2 `DogTagSBTConsent` - the pet identity (custodial, owner-hidden)

The "DogTag" factory: **issues an on-chain identity per chip/pet** that everyone references.
Every tag is minted to ONE neutral, immutable **custodian**: `mintCustodial(dogTagId, R)` takes NO recipient parameter, so the owner's wallet appears in neither the calldata, nor storage, nor any event.
Ownership is proven in-ZK against the write-once `profileRoot(dogTagId) = R` (§4.7), never by an `ownerOf` read - `ownerOf` always returns the custodian and carries no owner meaning.
The lifecycle (create / status / erasure) keeps the **least-privilege roles + originator binding + authority override** of [`research/09-sbt-lifecycle.md`](./research/09-sbt-lifecycle.md). (Normative refinements in §13.6.)

**Standards posture:** ERC-721 + ERC-5192 (permanently `locked`); `issuerOf` + issuer/verifier separation borrowed from ERC-5727 (vocabulary only); **ERC-5484's frozen mint-time `BurnAuth` is rejected** — it cannot express "the original issuer **OR** a *current* authority," and authority legitimately changes (a clinic closes, a regulator steps in). Status semantics follow W3C Bitstring Status List (status is *about* the credential; never destroys it).

**Granular action roles** (OZ v5 `AccessControlEnumerable` + `AccessControlDefaultAdminRules`, so the accredited set is publicly auditable **and** `DEFAULT_ADMIN_ROLE` moves only via a two-step, time-locked hand-off):
- `ISSUER_ROLE` - **create/mint** a DogTag (custodially).
- `AUTHORITY_ROLE` - **cross-issuer status transitions** (incl. `Deceased`); any current authority may act on any token. It has NO power over `profileRoot`: the root is write-once, so there is nothing for an authority to overwrite.
- `DEFAULT_ADMIN_ROLE` — protocol multisig (`AccessControlDefaultAdminRules`, two-step transfer + 3-day delay; see §13.1 H-3 + [`GOVERNANCE_MIGRATION.md`](./GOVERNANCE_MIGRATION.md)).

There is deliberately **no `RECOVERY_ROLE` and no profile-root setter**: recovery is re-issue (below), and `profileRoot` is write-once (below).
The retired owner-revealing contract's `RECOVERY_ROLE`/`UPDATER_ROLE` are gone with it.

**Originator binding + authority override** (resolves your deceased question): record `issuerOf[tokenId]` at mint, **immutable**. Status mutations are gated by `msg.sender == issuerOf[tokenId] || hasRole(AUTHORITY_ROLE, msg.sender)`. So the **original issuer can always act on its own tokens**, and **any *current* authority can act on any token** - which is exactly why marking a pet **`Deceased`** is an `AUTHORITY_ROLE`-or-original-issuer action (a death is often reported by a *different* accredited vet than the minter), **never** the owner - and under the custodial model there is no on-chain owner who could even call. Because authority membership is mutable, authority evolves without re-issuing tokens (impossible under ERC-5484's frozen value).

**Status model — soft status, NEVER burn** (`DogTagStatus` enum): `Active`, `Lost`, `TransferPending`, `Deceased`, `Revoked`. `Active↔Lost` and `Active↔TransferPending` are reversible; **`Deceased` and `Revoked` are terminal/irreversible**. We do **not** burn on death/revocation — burning would orphan every credential that references `tokenId` and break historical verifiability. `burn` is reserved for the **admin GDPR-erasure path only** (§13). Every transition emits `StatusChanged(tokenId, from, to, by, reason)`.

- `tokenId` (`dogTagId`) = the canonical pet identity; **allocated as a random/sequential non-personal id** — **never any hash of the microchip** (neither `keccak256(microchip)` nor `Poseidon(microchip)` — any hash of a low-entropy chip is brute-forceable on-chain — §11.1). All other credentials reference it.
- **Held by the neutral custodian**, never by the owner's wallet: the custodian never signs and never acts (tags are soulbound and the contract has no transfer path), so custody confers no authority and a custodian key compromise grants an attacker nothing. The owner's control lives in the hidden leaves of `R` (§3.6) and is exercised only through consent proofs.
- One SBT per microchip (uniqueness enforced off-chain by the issuing flow before mint).

**Write-once `profileRoot`.** `profileRoot[id]` is set once, at mint, and can never change: no setter exists, and a burned id can never be re-minted (`mintCustodial` rejects an id whose root is already set).
`R == profileRoot(dogTagId)` is the sole tag-to-owner binding - there are no owner-identity checks - so a mutable root would be a full forgery vector: anyone able to overwrite it could fold a tree whose reserved owner leaves they control and consent as any `dogTagId`.
The consequence is intended: a burned id is retired permanently, which is what makes `burn` a real GDPR erasure, and a tag's tree is frozen at issuance - any change to it is a fresh issuance under a new id.

**Recovery is re-issue, not rebind (D3).** There is **no `recover()`** and no signature-authorized owner rebind: a rebind names the new owner on-chain, which is exactly the linkage the owner-hidden model removes.
Recovery (a lost or rotated key, a corrected attribute) is a **fresh custodial issuance under a new `dogTagId` + new `R`** (device flow: `ProfileTreeStore.reissue`).
Referencing credentials do **not** survive the re-issue (accept-the-break, captain decision 2026-07-16): the owner re-obtains each fresh from its issuer under the new id, because re-anchoring a prior attestation to an id its issuer never signed would forge attestation applicability.
`POST /profiles/issue/custodial-bind` supplies the MECHANICAL half - a fresh session allocates a fresh `dogTagId` and the device posts the new `R` - but there is no re-issue-AWARE issuer flow: nothing marks the abandoned tag or links old to new, and per D3 that link must stay device-local anyway.
(The retired owner-revealing contract carried a two-signature `recover()` rebind gated by a `RECOVERY_ROLE`; that design is removed with it.)

> **DELEGATION IS NOT OWNER CHANGE - do not conflate them.** Everything above (the recovery re-issue) **replaces** the principal. **Delegation** - an owner authorizing a **non-owner** (a caretaker taking the pet to the groomer) for **scoped consent while remaining the owner** - is a separate mechanism, and it must never be implemented as a partial owner rebind. It is **decided and deferred**: delegation ships as a **separate delegate circuit** in which the delegate is authorized by an owner-signed message and is therefore **never committed in `R`**, routed as its own protocol version through the two-axis `ProtocolRegistry`; the owner consent circuit and its 7 public signals stay frozen. Because `profileRoot` is write-once, any attempt to express delegation *through* the tree collapses back into retire-and-re-issue. Full decision record, including the normative "exactly one reserved triple per `R`" invariant and the `DEPTH` item still to lock before the mainnet ceremony: [`DELEGATION.md`](./DELEGATION.md) (decided 2026-07-20).

Key functions:
```solidity
function mintCustodial(uint256 dogTagId, bytes32 profileRoot) external onlyRole(ISSUER_ROLE);
    // NO recipient parameter: mints to the immutable neutral custodian. Requires a non-zero root and
    // an id whose profileRoot is unset (single-use FOREVER, even after burn); records
    // issuerOf[dogTagId]=msg.sender; emits Locked + an owner-blind Issued
function setStatus(uint256 dogTagId, DogTagStatus s, string reason) external; // require msg.sender==issuerOf[id] || AUTHORITY_ROLE; Deceased/Revoked terminal; never an owner; emits StatusChanged
function profileRoot(uint256 dogTagId) external view returns (bytes32); // write-once; the tag<->R binding the verification registry checks
function status(uint256 dogTagId) external view returns (DogTagStatus);
function locked(uint256 tokenId) external pure returns (bool); // always true
function burn(uint256 tokenId) external onlyRole(DEFAULT_ADMIN_ROLE); // GDPR-erasure ONLY; profileRoot survives (the id stays retired); emits Burned
```

### 4.3 `IssuerRegistry` — the whitelist gate (central protocol control)

Implements your **"central protocol gates"** decision. AccessControl over Ownable so a compromised signer is revoked **O(1), globally** across all issuers.

```solidity
DEFAULT_ADMIN_ROLE  // DogTag protocol multisig/admin
// per-recordType, per-signer scoping (§13.1 C-2)
function whitelistFor(bytes32 recordType, address signer) external onlyRole(DEFAULT_ADMIN_ROLE);
function delistFor(bytes32 recordType, address signer)   external onlyRole(DEFAULT_ADMIN_ROLE);
function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
```

**Multiple addresses per issuer entity (one-to-many issuer → signers).** A single logical issuer (vet/clinic business) may sign with its **backend-derived address OR a browser-wallet (MetaMask/WalletConnect) address** (§6, dual signing modes) — these are different addresses, so **both must be whitelistable for the same issuer**. The contract grants a role to an *address*; an issuer can have many. Off-chain, an `issuer_entity` row links the business to its signing addresses (`issuer_signer{issuerEntityId, address, mode, recordTypes[], status}`); the contract has no concept of "the same vet".

**Invariant:** the **active signer must be `isWhitelistedFor(recordType, signer)`** for the record being issued.

Onboarding flow (off-chain → on-chain, also triggered on a signing-mode switch — see §6): a new signer address → vet submits `{issuerEntityId, address, mode, recordTypes, verifyPurposes[], USDA#, license#}` to the **central/admin backend** → admin verifies accreditation off-chain → admin calls `whitelistFor(recordType, addr)` per record type (and `whitelistFor(verify_key(purpose), addr)` per verify-purpose — see below) → app polls `isWhitelistedFor` until live. Only then can that address issue/verify. Delist inactive-mode addresses to avoid a stale, over-broad whitelist; backend key rotation = a new address to whitelist. **Whitelisting is the admin portal `approve_application`, not a script** — the only off-portal step is funding the signer with PLASMA gas.

**Vets issue dog tags (proper onboarding) - admin portal only approves + whitelists.** The admin/central portal does **NOT** register devices or mint dog tags: there is **no** `POST /v1/register`, **no** `GET/POST /v1/admin/owners`, **no** admin "Registered devices" / "Mint dog-tag" page, and the phone has **no** "Central API URL" setting - every host the device talks to comes from a scanned QR. The admin's only onboarding power is the apply→approve **whitelisting of vet/groomer signer addresses** (above).
The dog tag is issued by the **vet**, mirroring import/export: the phone **creates a self-custodial wallet** (Profile → "Create embedded wallet" → 24-word seed → secp256k1 address, §6.4); the vet "Register pet (issue dog tag)" wizard (operator enters `ownerIdentity{countryOfIdentification, identification, name}` + pet fields, demo-prefilled) → `POST /profiles/issue/session/start` → returns a one-time token + QR `<vetHost>/p/<token>` (32-hex, 180s TTL) + the allocated `dogTagId`.
The device scans `/p/<token>`, builds the profile tree **locally** from its wallet seed (the three hidden owner leaves + the pet-attribute leaves fold into root `R`; the owner-secret never leaves the device) and POSTs `<vetHost>/profiles/issue/custodial-bind { token, root }` - **no wallet address, no signature**: `mintCustodial` has no recipient, so there is nothing for a signature to attest, and sending a wallet would restore the very link this design removes. The one-time token IS the authorization.
The vet backend shape-checks `R` (opaque - the server has no seed and cannot recompute it), **anchors** it (`issue(R)` on the `DOG_PROFILE` `DogTagIssuer` clone, so `rootIssuer[R]` resolves), then **seals** it (`mintCustodial(field_of_value(handle), R)`), responding `{ dogTagId, onchainDogTagId, root, protocolVersion, status:"minting" }`.
The device polls the chain until `profileRoot(dogTagId) == R`, then imports its dog tag - **gasless for the device** (the vet pays).
This requires the vet signer to hold **`DogTagSBTConsent.ISSUER_ROLE`**, granted once by the protocol admin (demo: `scripts/demo-bootstrap.sh` does `grantRole(keccak256("ISSUER"), vetSigner)`); **`ISSUER_ROLE` is a trust escalation** - a holder can mint any id - so in production it is granted **only to accredited vets**. See implementation §3.11.

**`VERIFY:` whitelist namespace - verifier capability gated SEPARATELY from issuer roles.** The same per-(key, address) `isWhitelistedFor` machinery scopes who may record a verification: `VerificationRegistryConsent` checks `IssuerRegistry.isWhitelistedFor(keccak256("VERIFY:" || purpose), relayer)`. Because verify-capability lives in its own `VERIFY:`-prefixed key space, a **groomer can be authorized to verify a given `purpose` without holding any issuer role** (and an issuer is not implicitly a verifier). **Verifiers onboard through the same apply→approve flow as issuers:** the issuer application carries `verifyPurposes[]` (e.g. `grooming_intake/boarding_intake/daycare_access`), and `approve_application` whitelists `VERIFY:<purpose>` per address **on-chain** - `verify_key = keccak256(abi.encode("VERIFY:", keccak256(label) mod r))`, the purpose reduced mod BN254 `r` (`purpose_key`) so the registry stores/nullifies the same reduced value (§11.8). No demo-bootstrap/script cast for VERIFY.

### 4.4 `DogTagIssuer` — record anchoring (cloned per record type)

The OpenAttestation `DocumentStore` analog. **One clone per record type** (and per business, so each business's issuance is independently revocable/auditable).

```solidity
mapping(bytes32 => uint256) public issuedAt;   // 0 = not issued
mapping(bytes32 => uint256) public revokedAt;  // 0 = not revoked

modifier onlyWhitelisted() { require(registry.isWhitelistedFor(recordType, msg.sender)); _; }

function initialize(string name, bytes32 recordType, address registry) external; // clones have no ctor
function issue(bytes32 root)            external onlyWhitelisted;
function bulkIssue(bytes32[] roots)     external onlyWhitelisted;  // batch-ready
function revoke(bytes32 root)           external onlyWhitelisted;
function bulkRevoke(bytes32[] roots)    external onlyWhitelisted;
function isIssued(bytes32 root)  external view returns (bool);
function isRevoked(bytes32 root) external view returns (bool);
function isValid(bytes32 root)   external view returns (bool); // issued && !revoked
// events: RootIssued(root, msg.sender, ts), RootRevoked(root, msg.sender, ts)
```

- `isValid(root)` is the single verification entry point — same call for single-doc and future batched anchoring.
- `bulkIssue/bulkRevoke` already present so batching needs **no redeploy**.

### 4.5 `DogTagIssuerFactory` — clone deployer

```solidity
function createIssuer(string name, bytes32 recordType, bytes32 salt)
    external returns (address clone); // Clones.cloneDeterministic(impl, salt); then initialize()
function predictIssuer(bytes32 salt) external view returns (address);
```

- EIP-1167 minimal proxies via OZ `Clones` → ~95% deploy-gas savings vs full deploy; verify the implementation **once** on Blockscout; addresses pre-computable.
- Trade-off: clones are immutable + need `initialize()` (no constructor) — acceptable for intentionally-immutable anchoring contracts.

### 4.6 On-chain ↔ off-chain interaction map

**Pet-onboarding (dog-tag issuance) handshake - the operator types NO address, and no address ever crosses the wire.** The vet issues the dog tag; the **device supplies only the root `R`** it folded locally from its wallet seed. The vet's `ownerIdentity` 3 fields stay off-chain with the issuing vet (§3.6). Flow (verified against `stacks/vet/api/src/routes.rs`, `stacks/vet/web/src/pages/IssueDogTag.tsx`):

```
PET ONBOARDING (vet "Register pet" - the device supplies only the root R):
  vet operator → "Register pet" wizard (enters ownerIdentity{country,identification,name} + pet fields)
  vet web → vet API: POST /profiles/issue/session/start  (operator-session gated)
  vet API: allocate a numeric HANDLE (skip ids already TAKEN under field_of_value(handle) - the
           write-once profileRoot(id) is the taken-marker, and it SURVIVES a burn)
           persist ProfileIssueSession + a fresh 16-byte one-time BIND TOKEN (180s TTL)
           → {token, dogTagId(handle), sessionId, qr = <vetHost>/p/<token>}
  device scans /p/<token> → GET /p/{token} → session meta (NON-consuming) + unverifiedClaims
           {protocolVersion,chainId,verificationRegistry,issuerClone,purpose:"DOG_PROFILE"} — the platform's
           CONVENIENCE tier, validated against the dogtag discovery anchor before use (impl §3.10d)
  device: build the profile tree LOCALLY from the wallet seed → root R (never sends the owner-secret)
          POST /profiles/issue/custodial-bind {token, root}      ← NO walletAddress, NO signature:
            mintCustodial has no recipient, so there is nothing for a signature to attest and sending
            a wallet would restore the very link this design removes. The one-time token IS the authz.
  vet API: shape-check R (opaque - the server has no seed and CANNOT recompute it); verify the SBT +
           profile-issuer addresses are configured BEFORE consuming the token (a half-wired stack must
           not burn the QR); consume the token ATOMICALLY; re-check profileRoot(id) is unset (a sealed
           id is retired forever)
           RESPOND IMMEDIATELY {dogTagId, onchainDogTagId, root, protocolVersion, status:"minting"}
           THEN async (tokio::spawn): issue(R) on the DogTagIssuer clone FIRST (anchor - so
             rootIssuer[R] resolves), THEN mintCustodial(field_of_value(handle), R) (seal -
             irreversible; write-once profileRoot survives a burn, so a mint before a failing issue
             would retire the id forever)
             read back profileRoot==R && isValid(R) → "bound"+txHash (else "error"). Deliberately NO
             ownerOf comparison: the owner is the neutral custodian, and comparing it would reintroduce
             the linkage.
  device polls the chain under field_of_value(handle) until profileRoot == R, then offline-integrity-
           verifies and imports the dog-tag record (pet appears)
  operator portal polls GET /profiles/issue/session/{id} for pending → bound+txHash; the session
           carries NO walletAddress BY DESIGN (an owner-hidden issuance never learns one)
  ⇒ GASLESS for the device (the vet pays); requires DogTagSBTConsent.ISSUER_ROLE on the vet signer
    (demo: scripts/demo-bootstrap.sh grantRole(keccak256("ISSUER"), vetSigner) - a trust escalation,
     production-granted only to accredited vets).

ISSUE (vet issues a vaccination):
  vet frontend → vet API: POST /records {type:VACCINATION, fields, dogTagId}
  vet API: build wrapped doc (salt+leaves+merkle) → root
           sign+broadcast issuer.issue(root) with whitelisted key
           store wrapped doc + tx hash in vet MongoDB
           publish DNS check is operator's responsibility (TXT already set)
  vet API → frontend: {recordId, root, txHash}

SHARE (vet shows QR for a record):
  vet API mints EdDSA JWT scoped to recordId (exp ~2-5min, jti)
  QR = https://<vet-host>/r?t=<jwt>&i=<recordId>

FETCH + VERIFY (mobile scans):
  mobile parses QR → GET https://<vet-host>/records/{recordId}  (Bearer JWT)
  vet API checks JWT (sub==recordId, jti one-time) → returns wrapped doc
  mobile verifies 3 authenticity pillars (all required):
    integrity: recompute Poseidon leaves+merkle from data → == targetHash; proof→merkleRoot (R)
    issuance:  issuer.isValid(merkleRoot) via ROAX RPC (read-only)
    identity:  DNS TXT of issuer.domain lists issuer.documentStore + chainId
  + tag binding (the owner's own dog-tag import only):
    the device checks profileRoot(dogTagId) == R for its own tag; "is this mine" is answered
    DEVICE-LOCALLY (the tag's hidden owner leaves fold from this device's wallet seed) - never by
    an on-chain owner read (ownerOf is the neutral custodian and carries no owner meaning)
  mobile stores credential under the pet, grouped by recordType

EXPORT (device → groomer; on-chain proof-of-verification — decoupled from import; §4.7, §3.6):
  groomer API → POST /verify/session/start {purpose}
                 → QR carries {host, one-time token, groomerAddr=relayer} — low-density
                   https://<host>/x/<token>?a=<relayer>  (token, NOT a JWT; one-time, 180s)
  mobile scans → GET https://<host>/x/<token> → session meta {relayer,purpose,recordType,challenge}
                   + unverifiedClaims{protocolVersion,chainId,verificationRegistry,issuerClone,purpose}
                     — the platform's CONVENIENCE tier (claims, NOT authority): the phone resolves the
                     dogtag-owned discovery anchor (ProtocolRegistry / signed manifest) and validates them
                     before trusting any platform-supplied version/registry (impl §3.10d) — hard-stop on mismatch
          asserts groomerAddr(QR) == session relayer
          on-chain: isWhitelistedFor(VERIFY:purpose, groomerAddr)  (hard-stop if not a whitelisted groomer)
          DNS-verify the groomer (prod/remote): the host's domain must publish
            dogtag-verify=<groomerAddr> via DoH (Cloudflare) — SKIPPED for local hosts (§4.7)
  mobile: owner reviews → the app assembles the consent witness (consent_assemble) and PROVES ON-DEVICE:
            the per-tag consent key signs M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)
            and the phone generates the Groth16 consent proof locally - the raw record, the wallet
            seed, and the owner NEVER leave the phone
          → POST {proof:{a,b,c,pubSignals[7]}} to https://<host>/v1/verify/consent (with the token)
            ← NO consent/sig object, NO bind leg: there is no subject and no consent-key registry
              (the consent key lives INSIDE the tree), so there is nothing to bind and no owner slot
              a caller could fill. The proof self-authenticates.
  groomer backend (it receives only the proof, never the witness):
          preflight the registry's own requires pre-gas (field range; relayer range on the FULL
            element before narrowing; pub[relayer] == our custody signer; deadline > now+120s;
            art9; VERIFY: whitelist - checked unconditionally, deliberately stricter than the
            registry's toggleable restrictToWhitelistedRelayers so a mis-set flag cannot open the relay)
          drive the VerifySession audit row to "recording" - owner-blind by construction: the row has
            no subject field and the proof has no signal for one
          RESPOND IMMEDIATELY {status:"recording", protocolVersion, sessionId, registry, nullifier}
          THEN async: VerificationRegistryConsent.recordVerificationZK(a,b,c,
                        [dogTagId,purpose,relayer,nullifier,R,recordType,deadline])
                        // recordType+deadline are pub[5]/pub[6], proof-bound, never relayer-invented
                      → emits an owner-blind Verified(...) (no subject); row → "recorded"+txHash | "error"
  NOTE index public signals via public_signals::level_b (named constants; the module name mirrors the
    internal protocol version key, an internal identifier), never a bare literal - a misread slot
    fails silently downstream.
```

> **Import and verification are DECOUPLED.** `/import/pull` (off-chain operational data) stays as-is. The `/verify/*` leg is the on-chain attestation and imports **no data at all**: if a verifier needs the record contents, the owner separately discloses them through the import/share flow (with selective disclosure, §3.5).

> **Privacy model of EXPORT - what happens to the root `R`.**
> The phone proves the hidden owner leaves fold into `R` in-circuit and POSTs only `{proof, pubSignals}`.
> `R` is `pub[4]` - a public *signal* the chain range-checks and binds to `profileRoot(dogTagId)` - but the **raw record never reaches the groomer**, no salted cleartext is disclosed to anyone, and **no signal names the owner**.
> This is zero-knowledge against the groomer AND owner-blind on-chain: what becomes public is only `(dogTagId, purpose, relayer, nullifier, R, recordType, deadline)`.
> Residual linkage is pet-level, not owner-level: `dogTagId` and `R` are stable per tag, so one tag's verifications are linkable to each other (§11.1) - the OWNER is what stays hidden.

### 4.7 EXPORT contracts & ZK circuit (proof-of-verification)

Records that a groomer validated a credential **with the owner's consent, proven in zero knowledge** - the owner **exports** a proof to the groomer (the symmetric counterpart of IMPORT, §7). One registry, one `consumed` nullifier set, and the existing issuance/revocation gate.

**Proving is ON-DEVICE (canonical).** The **phone** assembles the consent witness (`consent_assemble`) and generates the Groth16 proof locally; it POSTs only `{proof, pubSignals}` - the groomer **never receives the raw record, the witness, or any owner identifier**. The backend `POST /prove-consent` route is the independent **server-prove fallback**: a trusted prover that sees the witness (so THAT service's operator could name the owner), while the verifier and the chain still never learn the owner. The `dogtag-prover-rs` crate also re-proves from a witness as a **test oracle** for the e2e scripts; neither is the canonical path.

**Export QR = one-time token (not a JWT).** The QR is `https://<host>/x/<token>?a=<relayer>` carrying the groomer wallet address + a one-time token + host (low-density). The phone resolves it via `GET /x/<token>`, on-chain-checks `isWhitelistedFor(VERIFY:purpose, groomerAddr)`, and (prod/remote) **DNS-verifies the groomer**: the host's domain must publish `dogtag-verify=<groomerAddr>` (mirrors the issuer TXT in `stacks/admin/api/src/dns.rs`, resolved via Cloudflare DoH) - **skipped for local hosts** (IP literal / `localhost` / `*.local` / LAN), the LOCAL demo. The proof is POSTed to `/v1/verify/consent` with the token (consumed only on a successful record). Full detail: [`research/10-zk-groth16.md`](./research/10-zk-groth16.md), [`research/11-consent-attestation.md`](./research/11-consent-attestation.md), [`research/12-verification-integration.md`](./research/12-verification-integration.md). Endpoint pseudocode: `implementation.md §3.9`.

**`VerificationRegistryConsent`** - custom, **not EAS** (EAS isn't on ROAX and has no Groth16 path). Owner-blind by construction: no ECDSA consent path (an owner-signed message would name a subject), no consent-key registry, no owner-identity checks - `ownerOf` is called once purely as a token-EXISTENCE gate whose return value is discarded.

```solidity
mapping(bytes32 => bool) public consumed;             // nullifier -> used
bool public restrictToWhitelistedRelayers = true;     // admin toggle: require VERIFY: whitelist
event Verified(uint256 indexed dogTagId, address indexed relayer,
               bytes32 purpose, bytes32 nullifier, uint256 deadline, uint256 ts);   // NO subject field

function recordVerificationZK(uint[2] a, uint[2][2] b, uint[2] c, uint[7] pub) external {
    // pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
    for (p in pub) require(p < SNARK_SCALAR_FIELD);                    // range-check ALL signals (#358)
    require(pub[2] < 2**160);                                          // address-typed signal fits 160 bits (L1)
    require(block.timestamp <= pub[6]);                                // deadline is proof-bound (pub[6])
    require(pub[5] != SERVICE_ATTESTATION_FIELD);                      // Art. 9: never verifiable on-chain (§13.8)
    require(address(uint160(pub[2])) == msg.sender);                   // relayer == caller
    if (restrictToWhitelistedRelayers)
        require(registry.isWhitelistedFor(keccak256("VERIFY:"||bytes32(pub[1])), msg.sender));
    require(bytes32(pub[4]) == sbt.profileRoot(pub[0]));               // THE owner-hidden binding: R == profileRoot(dogTagId)
    sbt.ownerOf(pub[0]);                                               // token-EXISTENCE gate only (reverts on burn);
                                                                       // return value DISCARDED - never compared
    require(zkVerifier.verifyProof(a, b, c, pub));
    bytes32 nf = bytes32(pub[3]); require(!consumed[nf]); consumed[nf] = true;   // nullifier is a PUBLIC signal (#383)
    address clone = rootIssuer[bytes32(pub[4])]; require(clone != address(0));   // resolve issuing clone FROM the root (§13.9)
    require(DogTagIssuer(clone).isValid(bytes32(pub[4])));             // revocation: isValid(R) directly on the public root
    emit Verified(pub[0], msg.sender, bytes32(pub[1]), nf, pub[6], block.timestamp);
}
```

(Abridged; the authoritative body is `contracts/src/VerificationRegistryConsent.sol`.)

- **Nullifier.** `nullifier = Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)` - a **public signal** output by the circuit (NEVER derived from proof bytes: Groth16 proofs are malleable, snarkjs #383), bound to the hidden `owner.secret` leaf proven ∈ `R`. Replaying one signed consent repeats the nullifier and is rejected; a fresh nonce is a new consent. There is no on-chain Poseidon: the chain only consumes the signal.
- **`dogTagId <-> R` is bound ON-CHAIN, not in-circuit.** `require(pub[4] == profileRoot(pub[0]))` is the only place the tag-root binding is checked; without it a prover could fold a tree they fully control and consent as any `dogTagId`.
- **Range-check every public signal** (`< SNARK_SCALAR_FIELD`) before use (snarkjs #358), and the address-typed `relayer` `< 2^160` on the **full** element before narrowing.
- **`recordType` and `deadline` are proof-bound public signals** (`pub[5]`/`pub[6]`), never relayer-supplied calldata. `recordType` is prover-asserted, not consent-signed (it is in neither `M` nor the nullifier): only the owner's app can build the proof, so the app - not the relayer - chooses it; treat it as a label bound to the proof, not an owner attestation.
- **Relayer pattern.** Plain "relayer submits a proof" - **no EIP-2771** (a forwarder could spoof `msg.sender`, defeating the relayer binding) and **no ERC-4337** here. The relayer is bound into the EdDSA message `M`, the nullifier, and the public signals, and enforced `== msg.sender`.
- **Verifier swap is time-locked:** `proposeZkVerifier` → `executeZkVerifier` after a real 2-day timelock (§13.9).

**`Groth16VerifierConsent`** - snarkjs `zkey export solidityverifier` from the frozen consent-ceremony `.zkey`; BN254; `verifyProof(a, b, c, uint[7] pub)`.

> **Retired.** The owner-revealing registry (with its ECDSA `recordVerification` path and subject-bearing `recordVerificationZK`), the non-consent `Groth16Verifier`, and the `ConsentKeyRegistry` (+ its gasless `bindConsentKeyFor` EIP-712 bind) are deleted. The consent key lives inside the tag's tree, so there is nothing to bind on-chain.

> **ASYNC-RECORD (the ROAX block-time fix).** ROAX blocks are ~12s apart, so an on-chain broadcast's receipt (~12–24s) exceeds the phone's HTTP submit timeout - a synchronous handler would have the phone close the TCP connection mid-broadcast, Axum cancels the future, and nothing records (session stuck `pending`). So the submit handler validates everything fast, persists `status:"recording"`, **responds immediately (no txHash yet)**, and runs the `recordVerificationZK` broadcast in a `tokio::spawn` that awaits the receipt. The device polls the session + the chain `consumed(nullifier)` for the terminal status. The one-time export token is consumed **only on a fully successful record**, so a failed record leaves the owner's QR retryable. (The same response-then-spawn pattern drives the §4.6 onboarding mint and is mirrored there explicitly.)

**The BN254 Groth16 consent circuit** (`circuits/consent.circom`, `DogTagConsent(6)`; proven via circom-prover/Arkworks on-device, `ark-circom` + `ark-groth16` server-side):

- **Public signals (frozen order):** `[ dogTagId, purpose, relayer, nullifier, R, recordType, deadline ]` - declared as circuit OUTPUTS in exactly this order. No subject, no keyHash: nothing names the owner.
- **Private:** the three reserved owner leaves' values + salts (owner-address; consent-key, whose leaf value is `keyHash = Poseidon(Ax, Ay)`; owner-secret), their Merkle inclusion paths (depth 6), the per-tag BabyJubjub consent pubkey `(Ax, Ay)`, the EdDSA signature `(R8x, R8y, S)`, and `consentNonce`.
- **Proves:** (1..3) each reserved leaf's inclusion in the same root `R`, under its PINNED keyPath + typeTag constants (pinning is load-bearing against keyPath substitution); (4) the in-tree consent key signed `M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)` (EdDSA-BabyJubjub); (5) `nullifier == Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)`; plus a 160-bit range check on `relayer`. The circuit does **NOT** prove `dogTagId <-> R` (bound on-chain, above) and does **NOT** prove `isValid` (the registry re-checks it on-chain). Soundness of the reserved-leaf binding additionally rests on the normative issuance invariant "exactly one reserved triple per `R`" ([`DELEGATION.md`](./DELEGATION.md) §5).

**ONE Poseidon root `R` (v4 - Poseidon unification).** `wrapDocument` (records) and `build_profile_tree` (the tag) each compute a **single** Poseidon root `R`; the issuer calls `issue(R)`. The circuit proves inclusion against that same `R` and the registry checks `isValid(R)` directly - there is no parallel keccak root, no `rZk`, and no `zkCommit`/`kecOf` mapping (CHANGESPEC-v4 §2). The §3 canonicalization standard (now Poseidon) and `isValid` are the single source of truth.

**Attribute leaves do NOT change this circuit.** The circuit proves only the three reserved owner leaves' inclusion (§3.6), so adding or changing disclosable `DOG_PROFILE` attribute leaves never changes the public signals or the proof shape (tree depth 6, up to 64 leaves).

**Trusted setup.** Phase 1 reuses Hermez / Perpetual Powers of Tau; the **consent phase-2 ceremony is done** (testnet-grade, multi-party, ending in a public random beacon) - transcript in [`CEREMONY_TRANSCRIPT.consent.md`](./CEREMONY_TRANSCRIPT.consent.md), and the committed VK/zkey (`consent_verification_key.json` / `consent_final.zkey`) are its output, hash-pinned and enforced at prover load. A compromised phase-2 lets a party *forge consents*, not leak data - and the core three-pillar trust model (§5) does not depend on the ZK setup at all. (The earlier ceremony transcript for the retired circuit remains valid history; the consent ceremony is the live one. A mainnet re-run is planned; `DEPTH` is the one ceremony-gated decision still open - [`DELEGATION.md`](./DELEGATION.md).)

---

## 5. Verification pipeline

A credential's **authenticity** rests on **three pillars** — it is VALID only if all three return VALID and none returns INVALID (OA-style fragment model; each fragment is tri-state `VALID | INVALID | ERROR` — a network/RPC error ≠ forged ≠ valid):

1. **Integrity** — recompute every leaf hash from `data` with the **pinned Poseidon** scheme (§3.3), union with `privacy.obfuscated`, rebuild the **Poseidon** Merkle tree (§3.4) → must equal `signature.targetHash`. Single-document credentials only: `signature.proof` MUST be empty, so `targetHash` **is** the credential root `R` (`signature.merkleRoot`); a non-empty `proof` is **rejected**, never folded (C1). (Pure, offline, in the SDK.)
2. **Issuance status** — read `DogTagIssuer(issuer.documentStore).isValid(merkleRoot)` over ROAX RPC. Must be `true` (issued, not revoked).
3. **Identity (DNS)** — resolve `issuer.domain` TXT records over DNS-over-HTTPS; one must read `dogtag net=ethereum chainId=135 addr=<documentStore>` (case-insensitive addr, matching chainId). Binds the human-trusted domain to the contract.

A fourth fragment, **ownership**, exists in the SDK for completeness but is **contextual - never part of the universal validity gate**:

4. **Ownership** - a tri-state fragment resolved from an on-chain owner read where a calling flow supplies a `userWalletAddress`; `NOT_APPLICABLE` otherwise, and never blocking `valid` for third parties.
   Under the custodial tag (§4.2) `ownerOf` returns the neutral custodian and carries no owner meaning, so no live flow gates on it: the owner's own dog-tag import instead checks `profileRoot(dogTagId) == R` and answers "is this mine" device-locally (the tag's hidden owner leaves fold from this device's wallet seed).
   Proving ownership to anyone else is the consent ZK proof (§4.7), never an address comparison.

The SDK exposes `verify(wrappedDoc, {rpc, dnsResolver, userWalletAddress?}) → { valid, fragments: {integrity, issuance, identity, ownership} }`. When `userWalletAddress` is absent (third-party verification), `ownership` resolves to `NOT_APPLICABLE` and never blocks `valid`. Both TS and Rust implement it identically; mobile apps call the Rust crate (via FFI/UniFFI) or a thin native port.

### 5.4 On-chain verifier note

`DogTagIssuer.isValid` only checks the **anchored root**. Merkle-proof checking happens **off-chain** in the SDK (cheaper, and the chain only needs the root). A `MerkleVerifierLib` mirrors the §3.4 Poseidon (`PoseidonT3`) domain-separated commutative node hash so any contract that wants on-chain proof verification can, but v1 does not require it.

### 5.5 Presentation / verification as a recorded on-chain event

A **verifier** presenting/validating a credential is now a **first-class, recorded on-chain event** - an owner-blind `Verified(...)` on `VerificationRegistryConsent`, authorized by the owner's ZK consent proof (§3.6, §4.7). This is the proof-of-verification capability; it is what realizes the xlsx Travel Request / DOT Airline presentation rows.

**The mobile-user self-import pipeline above is UNCHANGED.** The three authenticity pillars (integrity / issuance / identity), plus the owner's device-local tag binding for the dog tag itself (`profileRoot(dogTagId) == R`, §5), are exactly as in §5.1–§5.4. Proof-of-verification is an **additional, decoupled** leg used by third-party verifiers; it does not alter how a record is imported or how its authenticity is computed.

---

## 6. Signing modes (dual, mutually exclusive, switchable)

A vet/groomer anchors a merkle root (or mints an SBT) using **either** their own browser wallet **or** the self-hosted backend's custodied key. The two modes are **mutually exclusive, switchable at any time, and behaviourally identical** except for *who signs and who pays gas*. Full detail: [`research/08-wallet-integration.md`](./research/08-wallet-integration.md) (Part A) + [`research/04-custody-qr.md`](./research/04-custody-qr.md).

> **Decisive rule:** merkle-root / wrapped-document building is **ALWAYS server-side (shared SDK) — identical in both modes**. Only the final "sign + broadcast" step differs. This is what makes "what gets anchored" provably mode-independent.

### 6.1 `SigningStrategy` interface — two implementations

`signingMode` enum = `wallet` | `backend`, persisted **server-side** (so it follows the user across devices) via a **Settings radio toggle**. A `SigningStrategy` abstraction resolves the active mode; the credential-building code never knows which is active.

- **`WalletStrategy`** — browser wallet signs the backend's unsigned tx. Stack: **wagmi v2-era + viem 2 + Reown AppKit** (MetaMask via injected/EIP-6963; any WalletConnect v2 wallet via Reown). The user's own address signs and **pays PLASMA gas**.
- **`BackendStrategy`** — the self-hosted Rust backend holds an HD seed (per `04-custody-qr.md`), signs+broadcasts from a backend-derived address, and **pays gas from a funded key** (users issue "gaslessly"). Library: **Alloy** (`alloy`, `alloy-signer-local` with `mnemonic`+`keystore`); `ethers-rs` deprecated. Genesis state machine `UNINITIALIZED → PENDING_BACKUP → INITIALIZED`; age-encrypted seed at rest; unlock TTY > secrets file > env; seed in `secrecy`/`zeroize`, `mlock`'d, never logged. EIP-1559 by default, legacy `gas_price` fallback if ROAX lacks 1559 fee data.

### 6.2 Prepare / confirm + on-chain re-verification

- `POST /credentials/prepare` `{recordType, petTokenId, payload, mode}` → backend does the wrap + merkle + calldata for **both** modes and returns a `PreparedCredential` with an **unsigned tx** `{to, data, value, chainId: 135}` (wallet mode) **OR** signs + broadcasts itself and returns the `txHash` directly (backend mode).
- `POST /credentials/confirm` `{recordId, txHash, signer}` → backend **re-verifies on-chain** (the issuer's `RootIssued(merkleRoot, signer)` event + `issuedAt[merkleRoot] != 0`) before flipping the draft to `issued` — **a lying or buggy frontend cannot fake issuance**. The persisted row stores `{signingMode, signerAddress}` as **audit metadata only**; verification and downstream behaviour ignore them.

### 6.3 Whitelist, chain-add, switching UX

- **Whitelist:** the active signer must be `isWhitelistedFor(recordType, signer)` — see §4.3. `submit()` pre-flights via `eth_call` to fail fast (wallet mode = user pays gas on a revert). A new address (mode switch / second device / backend key rotation) is an **onboarding event** → admin approval queue → `whitelistFor` → poll until live.
- **Chain add:** viem `defineChain` (ROAX, chainId 135, PLASMA, RPC, explorer). `useSwitchChain` → `wallet_switchEthereumChain`; on error `4902` (chain unknown) fall back to `wallet_addEthereumChain` (EIP-3085, `chainId:'0x87'`).
- **Settings toggle / status panel:** wallet mode shows connected address + ROAX-chain check (offer "Switch to ROAX") + a per-recordType whitelist badge; backend mode shows genesis state (`INITIALIZED`/`LOCKED`) + the active address's **PLASMA balance**. Switching affects only *future* signing; broadcast records (have a `txHash`) are unaffected; in-flight **prepared** drafts are re-validated against the new active signer (merkleRoot is mode-independent, only the broadcast path changes). Block switching while a submit is pending.

---

## 7. QR record sharing — IMPORT & EXPORT (symmetric one-time-token flows)

The two QR directions are **symmetric**, each a low-density one-time token bound to a host:

- **IMPORT** (device ← vet): the QR carries `{one-time token, IP/host}`; the phone `GET`s the raw cert from `host + /r/<token>` (one-time, 180s, **deleted after first read**), then verifies the three authenticity pillars locally. This is how the code works today.
- **EXPORT** (device → groomer): the QR carries `{groomer wallet address, one-time token, IP/host}` as `https://<host>/x/<token>?a=<relayer>` - a **one-time token, NOT a JWT**. The phone resolves it via `GET /x/<token>`, **DNS-verifies the groomer** (prod/remote; skipped local), generates the consent ZK proof **on-device**, and POSTs `{proof: {a, b, c, pubSignals[7]}}` to `host + /v1/verify/consent` using the token (consumed on a successful record). Detail in §4.7.

The legacy embedded-JWT record path (`/r?t=<jwt>`) below remains only for IMPORT back-compat:

- **JWT alg:** EdDSA (Ed25519), a **per-deployment keypair separate from blockchain keys** (ES256 fallback). Lib `jsonwebtoken` 10.x.
- **Claims:** `iss`=deployment URL, `sub`=recordId (scoping anchor), `aud`=`dogtag-mobile`, `scope`=`read:record`, `iat`/`nbf`, `exp` ~2–5 min, `jti`.
- **Enforcement:** server checks `sub == path recordId`, scope, and a `jti` store (Redis/Mongo `SETNX … EX exp`) for **one-time use**; `leeway = 30s` for clock skew.
- **QR payload:** HTTPS deep link `https://<deployment-host>/r?t=<jwt>&i=<recordId>` — the **origin is the API base**, so the per-deployment URL requirement is satisfied by construction. ECC level M, byte mode, ~QR v6–10. `qrcode` crate.
- **Low-density variant (server-side one-time token):** the issuer→user QR MAY instead carry a SHORT server-side one-time token — `https://<deployment-host>/r/<32-hex>` (16 random bytes, no embedded JWT, no query string). The server maps `token → recordId` (exp ~180s) and resolves it via an unauthenticated `GET /r/:token` that **deletes the token on first read** (atomic remove == one-time), returning the same `wrappedDoc` body as `GET /records/{id}`. This gives the SAME one-time-use guarantee as the embedded record-JWT, but a far lower-density QR the phone camera can focus on. The legacy `/r?t=` JWT path remains for back-compat.
- **Two QR directions (see the IMPORT/EXPORT summary at the top of §7):**
  - **IMPORT — issuer → user** (vet shows QR; mobile pulls the record to import via `/r/<token>`).
  - **EXPORT — groomer → user** (groomer shows QR carrying `{groomerAddr, token, host}`; the owner exports an on-device ZK proof of a credential to the groomer via `/x/<token>` → `/v1/verify/consent`, §4.7). The user→business *operational profile share* (pet profile / vaccination status against the **central** API) still uses the one-time-JWT pattern, audience `dogtag-business`.

---

## 8. Calendar sync & appointments

Full detail: [`research/05-calendar-appointments.md`](./research/05-calendar-appointments.md).

### 8.1 Google Calendar two-way sync (per business backend)

- OAuth 2.0 web-server flow, `access_type=offline` + `prompt=consent` → refresh token; scope `calendar.events`.
- **Incremental sync tokens:** initial full `events.list` → `nextSyncToken`; thereafter `events.list?syncToken=…` returns only changes (incl. `status:"cancelled"` deletions). On **HTTP 410** discard token, wipe mirror, full resync.
- **Push:** `events.watch` webhook channels (~1 week, no auto-renew) — a ping just triggers an incremental list. Mandatory: periodic incremental-poll fallback + a channel-renewal cron.
- **Availability:** `freeBusy.query` for busy intervals.
- **Echo-loop avoidance:** platform-written events tagged `extendedProperties.private { dogtag.owned=1, dogtag.apptId, dogtag.rev }` + stored `etag`; on ingest, our own echoes are recognized and skipped. Mapping table `gcal_event_map(appointment_id ↔ google_event_id, etag, rev, direction)`. Untagged external events become **read-only busy blocks**, never appointments. Conflicts: **platform-wins**.

### 8.2 Appointment state machine

```
REQUESTED ──▶ CONFIRMED ──▶ COMPLETED
   │            │  │  └────▶ NO_SHOW
   │            │  └───────▶ CANCELLED
   └▶ DECLINED  └──────────▶ RESCHEDULED (stays CONFIRMED, new time)
Terminal: DECLINED, CANCELLED, COMPLETED, NO_SHOW
```

### 8.3 Cross-backend booking contract (central ↔ business)

The user owns the appointment in the mobile app (central backend); the business sees the same appointment on its self-hosted backend. **Central is the system of record**; business keeps an **idempotent replica** keyed by the same `appointmentId` + central-assigned monotonic `rev`.

- **Central → business:** `PUT /v1/appointments/{id}` (upsert), `/cancel`, `/reschedule`. Headers: `Idempotency-Key`, HMAC signature (shared secret established at business registration). `409 stale_rev` → reconcile.
- **Business → central:** `POST /v1/businesses/{businessId}/appointment-events` with `{appointmentId, rev, event, occurredAt}` for business-driven transitions (CONFIRMED/DECLINED/COMPLETED/NO_SHOW).
- **Catch-up:** both expose `GET /v1/appointments?updatedSince=…` to heal dropped callbacks.
- **Ordering:** apply-if-rev-newer; central arbitrates; terminal states (CANCELLED/DECLINED) win over CONFIRMED. Keeps add/remove/reschedule consistent on both backends **and** in mirrored Google Calendar.
- **Availability exposed to mobile** = working-hours grid − platform appointments − Google FreeBusy − capacity, with **soft slot holds** to prevent double-booking during the request window.

### 8.4 Discovery → booking flow

```
mobile → central: GET /v1/businesses?type=groomer&near=lat,lng
central → mobile: [{businessId, name, geo, services, apiBaseUrl, hmacKeyId}]
mobile → central: POST /v1/appointments {businessId, dogTagId, slot}
central: create appt (rev=1, REQUESTED) → PUT to business apiBaseUrl
business: store replica, notify staff
... business approves → POST appointment-events {CONFIRMED} → central → push to mobile
```

---

## 9. Data model (MongoDB)

### 9.1 Central / admin DB
- `users` — pet owners (auth, profile, push tokens; self-custodial wallet, **a fresh address derived per pet** `m/44'/60'/0'/0/{petIndex}` to avoid linking one person's whole pet history — §11.1).
- `owners` — **first-class `Owner` entity (off-chain PII only, encrypted, deletable, never on-chain):** `{ownerId, name, addresses[], phones[], email, emergencyContact, contactUpdatedOn}`. The pet-owner; distinct from the record-custodian (§9.2 `records.custodian`).
- `pets` — pet profile; `dogTagId` (SBT) once minted; `microchip{code,standard,implantDate,bodyLocation}` (code unique); `ownershipHistory[]{ownerId, from, to}`; cached profile root.
- `credentials` — references to credentials the user has imported (wrapped docs + verify cache, incl. `ownership` fragment).
- `consents` / `consent_receipts` — **`Consent`/`ConsentReceipt`** per-purpose records `{purpose, lawfulBasis, grantedAt, withdrawnAt, receiptId}`; drive retention + the erasure flow (§11).
- `businesses` — registry: `{businessId, type, name, geo, services, apiBaseUrl, domain, documentStores{recordType→addr}, signerAddresses[], hmacKeyId, status}`. **Non-personal discovery data.**
- `issuer_applications` — pending whitelist requests `{issuerEntityId, address, mode, recordTypes[], USDA#, license#, status}`.
- `appointments` — **source of truth** `{appointmentId, rev, userId, petId, businessId, state, slot, history[]}`.
- `verification_records` - proof-of-verification ledger `{dogTagId, relayer, purpose, recordType, nullifier, txHash, deadline, ts}` - a mirror of the on-chain `Verified` events (read from the chain; central is not in the verify loop and never sees a consent), which are **owner-blind** (no subject field exists to store). Owner-side consent receipts, where kept, are off-chain and deletable (crypto-shred erasure scope - §11/§13.7).

### 9.2 Business (vet/groomer) DB
- `keystore_meta` — genesis state, encrypted-seed location, derived accounts (addresses+labels only) — backend signing mode.
- `records` — issued wrapped documents `{recordId, recordType, dogTagId, wrappedDoc, root, txHash, blockNumber, explorerUrl, signingMode, signerAddress, custodian, retention{basis, clock}, status, label, notes, revokedTxHash/revokedBlockNumber/revokeExplorerUrl, invalidatedAt/invalidationReason, createdAt/updatedAt}` — each row bundles the credential with its **immutable on-chain proof** (tx hash, block number, explorer link); `status` ∈ prepared/confirming/issued/revoked/expired with **soft-invalidation only** (revoke keeps the row + issuance proof and adds a revoke-tx proof; `expired` is an off-chain-only transition; no delete endpoint). **`custodian` (the practice = legal record owner) is distinct from the pet-`Owner`.**
- `issuer_signers` — `{issuerEntityId, address, mode('wallet'|'backend'), recordTypes[], whitelistedTxHash, status}` — one issuer entity, many signing addresses (§4.3).
- `consents` / `consent_receipts` - per-purpose lawful-basis records (mirror of §9.1 for issuer-side processing); includes off-chain receipts for recorded verifications (deletable, erasure-scoped - §13.7).
- `verify_sessions` - verifier-side audit rows of recorded `Verified` events `{sessionId, relayer, purpose, recordType, challenge, status, nullifier, txHash, ts}` - **owner-blind by construction**: the row has no subject field and the consent proof has no signal that could fill one.
- `clients`, `pets_cache` — imported pet profiles/owners (groomer view).
- `appointments` — **replica** `{appointmentId, rev, state, slot, gcalEventId}`.
- `gcal_event_map`, `gcal_sync_state` — calendar sync bookkeeping.
- `jwt_jti` — one-time-use token ledger (or Redis).

---

## 10. Mobile architecture (themes)

- **Android:** Kotlin + Jetpack Compose, MVVM, Retrofit/Ktor, CameraX (QR), Maps Compose, EncryptedSharedPreferences/Keystore.
- **iOS:** Swift + SwiftUI, MVVM, async/await URLSession, AVFoundation (QR), MapKit, Keychain.
- **Verification:** shared Rust crate `dogtag-standard-rs` exposed via **UniFFI** to both platforms (single source of truth for canonicalization + Merkle + verify), avoiding two re-implementations.
- **Theming (mobile keeps its 7 themes — black/white/blue/red/pink/green/yellow, each with light+dark — unchanged):** a **semantic token layer** (`color.primary`, `color.secondary`, `color.surface`, `color.onPrimary`, …) with one palette per theme. Components reference **only semantic tokens**, never raw colors → switching theme swaps the palette, components unchanged. Android: `MaterialTheme` `ColorScheme` per theme + a `ThemeController`. iOS: an `@Environment` theme object + `Color` token extensions.
- **Navigation** mirrors the reference: bottom tabs **Verify · Travel · Home · Documents · Profile**; Home = pet card + grouped Credentials (Health / Service / Travel); add-record wizards with type pickers.

### 10.1 Mobile wallet (Settings) — Telegram-style self-custody

Under **Settings**, like Telegram's TON Space — a low-friction, recoverable, self-custodial EVM wallet. Full detail: [`research/08-wallet-integration.md`](./research/08-wallet-integration.md) (Part B).

- **Default = embedded MPC wallet** (MetaMask Embedded Wallets / Privy — real TSS, social/passkey login, **no seed-phrase UX** for non-crypto pet owners; the provider cannot sign alone).
- **Advanced/optional = raw BIP-39 self-custody export** (web3j 4.12.x on Android, web3swift 3.3.2 on iOS; derive `m/44'/60'/0'/0/0`) — gives crypto-natives a true exit/ownership story.
- **Storage = encrypt-then-store:** a hardware key in the **Secure Enclave (iOS) / StrongBox (Android)** encrypts the seed/secret; the ciphertext is stored in normal storage; decryption is **biometric-gated** (the Enclave/Keystore can't hold an arbitrary 256-bit seed directly). Require `biometryCurrentSet`/`setUserAuthenticationRequired` so re-enrolling biometrics invalidates the key; `…ThisDeviceOnly`/no auto-backup. **This bullet describes the WALLET SEED's key only.** The owner-hidden tag adds a **second, separate** on-device store - the per-tag owner-secret witness (`ProfileTreeStore`, both platforms) - whose key policy is deliberately different and must not be conflated: it is **not** `setUserAuthenticationRequired` (it gates on device-unlock, mirroring iOS `.completeFileProtection`, because it is read on ordinary display/proof paths where a per-read biometric would be a UX divergence); StrongBox is a best-effort top rung with documented fallbacks rather than a requirement; and its device-locality comes from `isExcludedFromBackup` (iOS) / `noBackupFilesDir` (Android - the manifest keeps `allowBackup="true"`, so the store does not rely on that flag). Contract: [`MOBILE_OWNER_SECRET.md`](./MOBILE_OWNER_SECRET.md).
- Shows **address + PLASMA balance**, send/receive; connects to external dApps (scan `wc:` URI) via **Reown WalletKit** (both platforms).
- **The pet's tag is NOT owned by this wallet on-chain** (custodial mint - §4.2): the wallet's role is to be the derivation root of the tag's hidden owner leaves. The seed derives, per tag, the owner-secret, the reserved-leaf salts, and the BabyJubjub consent key (one KDF preimage builder, bound to `(seed, dogTagId)`), so holding the seed IS holding the pet - no on-chain address comparison exists or is needed (§5). Recovery/transfer is a **fresh custodial issuance under a new `dogTagId` + new `R`** (§4.2 recovery-is-re-issue); there is no on-chain rebind. v1 prefers **gas sponsorship / AA so owners hold no PLASMA**.
- **Consent signing (proof-of-verification - §3.6/§4.7).** When a verifier requests a verification, the app shows a consent review screen; on approval it derives the **per-tag BabyJubjub consent key** from `(wallet seed, dogTagId)`, signs `M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)` (EdDSA), assembles the consent witness (`consent_assemble`, UniFFI-exported from the shared SDK), and generates the Groth16 consent proof on-device. The key is committed inside the tag's tree (the `owner.consentKey` leaf, `keyHash = Poseidon(Ax, Ay)`); there is **no on-chain key registry and no wallet-level consent key**.

### 10.2 Portal theming — light/dark

The **vet, groomer, and admin web portals** get a real user-switchable **light/dark theme toggle** (persisted), via `packages/ui` semantic tokens gaining light + dark palettes. Matches the groomer reference aesthetic (dark sidebar / light content) but as switchable light/dark — **portals are light/dark only, not the 7 mobile colorways**.

### 10.3 Mobile ZK proving - on-device, with a server-prove fallback

The consent proof (§4.7) is generated on-device from **ONE shared circuit-input assembly** (`consent_assemble`, the same named inputs every prover consumes), so on-device and server proving are byte-identical in what they prove.

- **On-device proving (canonical).** The phone runs Groth16 locally via **circom-prover** (Arkworks `ProofLib::Arkworks`) with a **circom-witnesscalc GRAPH witness** (`WitnessFn::CircomWitnessCalc`, `crates/dogtag-standard-rs/src/prover_ffi.rs`) - a pure-Rust, integer-width-correct interpreter of the circuit's field ops. The assets the app NEEDS are the frozen consent pair, `consent_final.zkey` + `consent.graph`, loaded by absolute path as runtime assets (the iOS project's bundling of this pair is finalized in the mobile-issuance/redeploy slice - see [`MOBILE_BUILD.md`](./MOBILE_BUILD.md)). The graph interpreter replaced rust-witness/wasm2c (w2c2), which miscompiled the circuit's i64 BN254 arithmetic on 32-bit ARM. The groomer never sees the raw record or the witness.
- **Server-prove fallback (`POST /prove-consent`).** A device that cannot run the on-device prover (e.g. 32-bit-only Android, which cannot produce a valid Groth16 proof with the Arkworks prover) POSTs its PRE-ASSEMBLED consent circuit input to a **trusted prover-service** - a `vet-api` instance compiled `--features prover` - and **submits the returned proof itself**, so the verifier still never sees the witness. The backend route exists today; the mobile wiring lands in a later slice. TRUST BOUNDARY: the assembled input carries `ownerSecret` + `ownerAddress`, so owner-unlinkability holds against the chain and the relayer, NOT against this service's operator - in production it is the **owner's own / owner-trusted prover**, never the groomer's.
- **rapidsnark is NOT viable** here (no armv7-android prebuilt) - the graph witness + Arkworks combination is what makes on-device proving correct on 64-bit ARM and the server fallback the only option on 32-bit.

---

## 11. Security model (summary)

- **On-chain trust:** only `IssuerRegistry`-whitelisted addresses can issue/revoke; whitelisting gated by off-chain accreditation review; compromised signer delisted globally O(1).
- **Identity:** DNS-TXT binds domain→contract; credential carries `domain` so verifier cross-checks.
- **Key custody:** seed never leaves the business backend; encrypted at rest; in-memory protections.
- **Record sharing:** short-lived, record-scoped, one-time JWTs; QR origin == API base.
- **Cross-backend:** HMAC-signed, idempotent, rev-ordered sync.
- **PII:** selective disclosure lets owners share minimal fields; central registry stores only non-personal business data; CDC import form stays **off-chain** (app + email only).
- **Privacy of pet data:** credential `data` lives off-chain (business + user), only Merkle roots on-chain.

### 11.1 Privacy & data-protection model (GDPR / UK GDPR / CCPA-CPRA)

Full detail: [`research/07-legal-privacy.md`](./research/07-legal-privacy.md). Two load-bearing constraints: (a) **owner PII must NEVER go on-chain**, and (b) a DogTag credential is **evidentiary, not self-authoritative**.

- **No personal data in cleartext or recoverable form on-chain.** On-chain holds only: salted commitments (salts off-chain), revocation/status, non-personal DIDs/keys, timestamps, schema/version, accreditation refs. **A salted commitment is itself personal data** (pseudonymisation, not anonymisation — ICO/EDPB), and **any unsalted hash of a low-entropy microchip number (15 digits) is brute-forceable → effectively reversible → personal data on an immutable ledger** — this is **hash-agnostic** (it holds whether the commitment is keccak or Poseidon; the hash function is not the protection). Hence per-field **16-byte random salts are the hiding term — the privacy mechanism, not just anti-forgery.** A globally-replicated ledger is also an independent GDPR Chapter V (cross-border transfer) problem — minimising on-chain personal data minimises on-chain transfer.
- **No owner wallet address appears on-chain at all** (custodial mint - §4.2): the tag is held by a neutral custodian, `Verified` events carry no subject, and the owner exists only as hidden, salted leaves inside `R`. What REMAINS pseudonymous personal data in DPIA scope is the **pet-level linkage**: `dogTagId` and `R` are stable per tag, so one tag's verification events are linkable to each other (which businesses saw this pet, when), and a tag resolves to its issuing vet via `rootIssuer[R]`/`issuedBy`. Owner-unlinkability is identity-level, not event-level - do not overstate it as full unlinkability, and do **not** claim "nothing personal on-chain" unqualified (a salted commitment is still personal data). SBT burn is part of the erasure flow (below). `dogTagId` (SBT tokenId) is allocated as a **non-personal** id - never **any hash of the microchip** (neither `keccak256(microchip)` nor `Poseidon(microchip)`; any hash of a low-entropy chip is brute-forceable). It is a random/sequential non-personal id.
- **Never on-chain (enumerated):** any owner PII (name/address/email/phone), document scans, **service-animal / disability indicators** (GDPR Art. 9 special category; CPRA sensitive PI) — service/assistance attestation data is off-chain only; and unsalted/low-entropy hashes of the microchip code or cert serials.
- **Right-to-erasure = destroy every copy of every per-field salt + delete the off-chain record + burn the SBT** so the on-chain commitment becomes **unlinkable** and the live `ownerOf → wallet` binding is dropped. The salt (16-byte CSPRNG, 128-bit) is the hiding term — even for low-entropy values an adversary must brute-force 2^128 salts — so destroying the salt unlinks **provided all copies are destroyed**. The weak link is **copy-proliferation**: the salt sits in cleartext in every distributed wrapped-doc `data` (issuer DB, holder device, importer caches, QR copies, backups/oplog). Implement erasure as **crypto-shredding**: encrypt salts/`data` under a per-record DEK at rest and destroy the DEK so all reachable ciphertext copies become undecryptable; copies the protocol can't reach (holder device, third-party importers) are DPIA residual risk. This is **risk-mitigation, NOT a regulator-blessed safe harbour** (CNIL: "close to" erasure; EDPB does not bless key-destruction as automatically satisfying Art. 17). A **DPIA is mandatory**, refreshed on any change to on-chain fields or chain topology; prefer a permissioned network where possible.
- **CCPA/GDPR delete endpoint (45-day SLA)** wired to the *same* off-chain delete + salt/key-destruction flow.
- **Consent + retention:** per-purpose `Consent`/`ConsentReceipt` records (lawful basis, withdrawable, timestamped — §9); `retention{basis, clock}` on credentials (default ≥5 yrs US / ≥3 yrs EU where silent).
- **Evidentiary legal posture + trust tiers:** a DNS-bound, chain-anchored W3C VC proves integrity/timing but carries **no eIDAS Art. 35 / ESIGN presumption** — authority is **extrinsic**, flowing from the accredited issuer (USDA-accredited vet / APHIS / competent authority). Encode `attestationType`, `signatureTrustTier`, `legalEffect` (`evidentiary`), `legalBasisVersion`, `jurisdiction`. The DOT form records that a **self-attestation under 18 U.S.C. §1001 exists** — never "verified disability". Never market the baseline as "legally binding / government-grade".
- **Record-custodian distinct from owner:** the practice/clinic owns the *record* (legal custodian); the pet-`Owner` has information-access rights — do not conflate (§9).
- **On-chain proof-of-verification creates behavioral linkage.** Recorded `Verified` events are owner-blind (no subject field), but still tie `dogTagId` ↔ verifier ↔ time - a pet-scoped behavioral trail that is **pseudonymous personal data in DPIA scope**. Mitigations are normative in **§13.7**.

---

## 13. Audit remediations (v1.1 — NORMATIVE; overrides §1–§12 on conflict)

Three independent audits ([`research/audit-01-contracts.md`](./research/audit-01-contracts.md), [`audit-02-crypto.md`](./research/audit-02-crypto.md), [`audit-03-systems.md`](./research/audit-03-systems.md)) found issues that **must** be resolved before any deploy. This section is the corrected design; `implementation.md §11` carries the corrected code/pseudocode.

### 13.1 Smart contracts (audit-01)
- **C-1 — lock the clone implementation.** `DogTagIssuer` gets `constructor(){ _disableInitializers(); }`. The implementation is the only Blockscout-verified address; leaving it initializable lets an attacker point `registry` at a malicious contract.
- **C-2 - per-record-type, per-address scoping (not one global boolean).** The single global `isWhitelisted` is replaced by `IssuerRegistry.isWhitelistedFor(bytes32 recordType, address signer)`. Each issuer clone checks `registry.isWhitelistedFor(recordType, msg.sender)`. SBT mint capability is a **dedicated** `DogTagSBTConsent.ISSUER_ROLE` (granted only to accredited vets), distinct from the record-issuer whitelist. A groomer key can never touch vaccination roots or pet profiles. (An earlier `IssuerRegistry.PROFILE_ISSUER_ROLE` was declared but never read by any contract - the capability always lived on the SBT - and has been removed.)
- **H-1 - originator binding.** `DogTagIssuer` records `issuedBy[root]=msg.sender` on `issue`; only the original issuer **or** protocol admin may `revoke` it. The SBT's profile root needs no update gate at all: `profileRoot` is **write-once** (§4.2), so the retired `setProfileRoot` originator-or-authority gate has nothing left to protect.
- **H-2 — `burn` is protocol-admin-only**, emits `Burned`, owners cannot self-burn (prevents orphaning referencing credentials).
- **H-3 - admin hardening (two-step everywhere + multisig migration).** `IssuerRegistry` (3-day), `VerificationRegistryConsent` (2-day) **and** `DogTagSBTConsent` (3-day) all use `AccessControlDefaultAdminRules` - `DEFAULT_ADMIN_ROLE` moves only via a two-step `begin/acceptDefaultAdminTransfer` with the delay; `DogTagIssuerFactory` uses `Ownable2Step`. Whitelist duty (`WHITELIST_ADMIN`) and role-admin duty (`DEFAULT_ADMIN_ROLE`) are split. **As-built status:** at deploy every contract's admin/owner is the **single deployer EOA** (`roax.json:admin`); a multisig is **not** yet live on-chain. The two-phase hand-off to a multisig (Safe or an equivalent threshold scheme) is shipped as code - `script/GovernanceMigration.sol`, `script/MigrateGovernance.s.sol`, the `GovernanceMigrationTest` proof, and the [`GOVERNANCE_MIGRATION.md`](./GOVERNANCE_MIGRATION.md) runbook - **pending the captain choosing signers and executing it** (a deliberate, irreversible ceremony, like the trusted-setup). (The retired pre-two-step SBT that predated this upgrade is deleted; every contract in the live set carries the two-step rules.)
- **M-1 — `createIssuer` is permissioned** (`onlyRole(DEFAULT_ADMIN_ROLE)`), salt = `keccak256(recordType, business)` to stop front-running/squatting.
- **M-4 — chain settings.** `evm_version = paris` everywhere (consistent); verify-reads wait **N block confirmations** (configurable; default 5) to tolerate reorgs — issuance status is only trusted past finality.
- **M (registry desync)** — `IssuerRegistry` is the single source of truth; no parallel bespoke mapping.

### 13.2 Canonicalization & Merkle (audit-02) — determinism is mandatory
- **Poseidon determinism across FOUR environments (v4).** The credential commitment is Poseidon (§3.3/§3.4); determinism now spans **circom / TS / Rust / Solidity** with the **single pinned circomlib BN254 instantiation** and pinned libraries (circomlib / `poseidon-lite` / `light-poseidon` / `poseidon-solidity`). **CI MUST assert bit-identical output across all four** against the anchor vector `poseidon([1,2]) = 0x115cc0f5...189a` (circomlibjs has historically drifted — pin + test). `encodeValue` and the `fieldOf` byte→field packing are byte-identical across all environments.
- **A1 — `canonicalDecimal` is pinned** to a closed ASCII grammar over the *input string*: `^-?(0|[1-9][0-9]*)(\.[0-9]+)?$`; strip fractional trailing zeros; drop a trailing dot; map `-0→0`; reject exponents/whitespace/`+`. (Covers weight `22.7`, titer `0.5`.)
- **A2 — typed input at the wrap boundary.** Numbers are **never** taken from a native float. Integers and decimals enter `wrapDocument` as **typed strings** (schema-driven), are carried as strings end-to-end, and `assertNotFloat` is a hard guard. `verify` never re-infers types — it reads the tag from the packed leaf.
- **A3 — Unicode pinned.** NFC normalization against a **pinned Unicode version** (stated in the SDK), unpaired surrogates rejected, NFC form stored in `data`. **Solidity participates at the node level only** — it never builds a leaf from a raw string.
- **C1 — invariant:** single-document verification **MUST rebuild the whole tree** and compare to `targetHash`; it must **never** trust `processProof` alone. (`processProof` is inclusion-only; position/shape unbound under commutative+odd-promotion.)
- **E2 — before enabling batching:** bind subtree size in the node hash — e.g. `hashNode = Poseidon(DS_NODE, subtreeLeafCount, min(a,b), max(a,b))` — or use ordered proofs for the batch layer. Not needed for single-doc v1, but the format reservation is documented now.
- **D1 — all three authenticity pillars required.** `fragments.integrity` alone proves nothing (an attacker can rewrite `data`+`targetHash` consistently); security rests on **pillar 2 (on-chain root)** + **pillar 3 (DNS)**. `verify` returns `valid` only if integrity + issuance + identity are all VALID (each tri-state `VALID|INVALID|ERROR`). The **`ownership` fragment is contextual, NOT part of the validity gate** (§5): it gates only the owner's self-import (`ownerOf(dogTagId) == userWalletAddress`) and is `NOT_APPLICABLE`/informational for third-party verification — otherwise every legitimate groomer/airline/vet import (none of whom are `ownerOf`) would falsely read INVALID. `obfuscated[]` entries are validated as 32-byte hashes that don't overlap live-leaf hashes; `dogTagId`, `@context[*]`, and `type[*]` are **non-obfuscatable**.
- **F2a — `flatten`/keyPath grammar is pinned** (load-bearing, since keyPath is hashed): dotted object keys, array indices as `[i]` base-10, reserved characters rejected, empty containers defined; shipped as test vectors.
- **F2b — packed-value parse splits on the first two colons only** (`salt:tag:value`), since values contain `:` (timestamps). 
- Salts: CSPRNG, unique per field, 16 bytes, cleartext in `data` (removed on obfuscation).

### 13.3 Systems, auth & standards (audit-03)
- **C-1 — `GET /share/{ref}` (central, user→business) mirrors the business-side asserts exactly:** `sub == ref`, `aud == "dogtag-business"`, scope check, and one-time `jti` consumption. Closes token replay + audience confusion.
- **C-2 — `appointment-events` ownership binding:** resolve the HMAC key by the path `businessId`, and require `appointment.businessId == path businessId`. A business can only act on its own appointments.
- **H — operator auth model (business backends):** a portal session/auth layer protects `/records`, `/revoke`, `/import/*`, `/calendar/*`. Custody endpoints (`/genesis/*`, `/unlock`, `/accounts`) live under an **`/admin`** namespace, bound to localhost/admin-session only, and `/unlock` is rate-limited (brute-force oracle). Custody is **never** on the public API surface.
- **H — central is the sole `rev` allocator.** Businesses never assign `rev` (prevents rev-tie split-brain). Business→central events carry the last-seen rev; central allocates the next.
- **H — DNS legitimacy.** Onboarding **verifies the TXT record before whitelisting**; the mobile verifier cross-checks the scanned `domain`/`documentStore` against the **admin-written central registry** (operator controls their own domain+contract+TXT+QR, so internal consistency ≠ legitimacy — the registry is the trust root for "is this a real vet").
- **H — schema validator corrected:** microchip `^[0-9]{15}$` is **conditional** (required for EU + CDC paths; optional for DOT/profile/pre-2011-tattoo/low-risk CDC); the `validFrom = vaccinationDate + 21d` rule is **booster-aware** (continuous boosters skip the wait); titer ≥0.5 IU/ml + timing windows, CDC age-≥6-months-**at-entry**, and echinococcus 24–120h are **fully coded**, not elided.
- **Registry self-write is impossible** — only admin approval writes `documentStores`/`domain`/whitelist.
- **Delisting is forward-only** (Medium, important): `isValid(root)` checks issued && !revoked, **not** registry membership, so a delisted signer's already-issued roots still verify VALID. Compromise response therefore requires an **admin revoke path** over the affected roots (mass-revoke), not just delisting.
- **`jti` one-time use is atomic** (unique-index insert / `SET NX`), never read-then-write.
- **Google echo discriminator is `etag`-primary** (not `rev`), so human edits in Google aren't silently dropped.
- **Verdict tri-state:** each pillar is `VALID | INVALID | ERROR` (network/RPC error ≠ forged ≠ valid).

### 13.4 Canonical naming (resolves doc-to-doc drift)
- Rabies fields (canonical, per CHANGESPEC §0): `vaccineProductCode`, `vaccineProductName`, `vaccineManufacturer`, `batchLotNumber`, `vaccinationDate`, `validFrom`, `validUntil`, `nextDueDate`, `authorizedVet`, `series` (`primary`|`booster`).
- VC `type` canonical string: `RabiesVaccinationCertificate` (validator matches this, not `"Vaccination"`).
- `recordType`: human label in docs/registry; **on-chain it is `keccak256(label)`** (e.g. `keccak256("VACCINATION")`). SDK exposes the mapping.
- JWT `exp`: **180s** default (configurable 120–300s) — single source of truth.
- Custody endpoints are under `/admin/genesis/*`, `/admin/unlock`, `/admin/accounts`.

### 13.5 v2 normative items (dual signing, wallet ownership, privacy)

These extend (do not replace) §13.1–§13.4 and the canonical names/enums in CHANGESPEC §0.

- **Dual-signing confirm re-verification.** `POST /credentials/confirm` MUST re-verify on-chain — the issuer's `RootIssued(merkleRoot, signer)` event **and** `issuedAt[merkleRoot] != 0` — before flipping a draft to `issued`. A lying/buggy frontend (wallet mode) cannot fake issuance. Merkle/wrapped-doc building is **always server-side, identical in both modes**; `{signingMode, signerAddress}` are audit-only.
- **Owner tag binding.** `ownership` stays a **contextual, tri-state fragment** (§5) that never gates third-party verification - validity for third parties rests on the three authenticity pillars only. Under the custodial tag (§4.2) `ownerOf` returns the neutral custodian, so no live flow compares it: the owner's own dog-tag import checks `profileRoot(dogTagId) == R` and the "is this mine" answer is device-local (seed possession). (Historical: the retired owner-owned SBT gated self-import on `ownerOf(dogTagId) == userWalletAddress` and changed owners via a consensual `recover()` re-bind; both are removed - recovery is re-issue, §13.6.)
- **PII-off-chain rule (qualified).** No recoverable personal data on-chain. Even a salted commitment is personal data; **any hash of a low-entropy microchip number is brute-forceable (hash-agnostic - keccak or Poseidon alike)**. Per-field 16-byte random **salts are the hiding term, the privacy mechanism**. Service/disability (Art. 9) data is off-chain only. No owner wallet address appears on-chain (custodial mint - §4.2/§11.1); the residual DPIA-scoped linkage is pet-level (`dogTagId`/`R` stable per tag). `dogTagId` is non-personal (never any hash of the microchip - neither `keccak256(microchip)` nor `Poseidon(microchip)`). Do not ship the unqualified "nothing personal on-chain" wording.
- **Multi-address whitelist.** `IssuerRegistry` supports **multiple signing addresses per issuer entity** (one-to-many). Invariant: the active signer MUST be `isWhitelistedFor(recordType, signer)`. A mode switch / second device / backend key rotation introduces a new address → admin approval queue → `whitelistFor` → poll until live; pre-flight `eth_call` to fail fast; delist inactive-mode addresses to avoid stale over-broad whitelisting.
- **MPC wallet storage.** Mobile default is an **embedded MPC wallet** (TSS — provider can't sign alone); raw BIP-39 export is advanced-only. Storage is **encrypt-then-store**: seed/secret encrypted by a Secure Enclave (iOS) / StrongBox (Android) hardware key, **biometric-gated**, `…ThisDeviceOnly`, no auto-backup, `biometryCurrentSet`-bound. Never log/serialize the plaintext seed.
- **Erasure-via-salt-destruction (crypto-shredding) + SBT burn.** The right-to-erasure flow destroys **every reachable copy** of every per-field salt (crypto-shred: per-record DEK destroyed → all ciphertext copies undecryptable), deletes the off-chain record, and **burns the SBT** (the registry's token-existence gate then fails the tag closed at every future verification; there is no on-chain owner link to drop, and the burned id stays retired forever because `profileRoot` survives the burn - §4.2). The 128-bit salt is the hiding term (low value-entropy is fine); copy-proliferation is the real risk, so unreachable copies (holder device, third-party importers) are DPIA residual risk. Wired to both GDPR Art. 17 and CCPA §1798.105 (45-day) request paths.
- **Mandatory DPIA + CCPA/GDPR 45-day delete endpoint** on the crypto-shredding flow above. Legal posture is **evidentiary, not authoritative** (trust tiers per §0).

### 13.6 v3 normative items (granular SBT lifecycle, recovery, auth, funds) — extend §13.1–§13.5

Source: [`research/09-sbt-lifecycle.md`](./research/09-sbt-lifecycle.md) + audit-04/05/06. Code in `implementation.md §11.7`.

- **Granular SBT roles + originator + authority override** (your decision). `ISSUER_ROLE` (create, custodially), `AUTHORITY_ROLE` (cross-issuer status), `DEFAULT_ADMIN_ROLE`. Record **immutable `issuerOf[tokenId]`** at mint; status mutations require `msg.sender == issuerOf || hasRole(AUTHORITY_ROLE)`. **Reject ERC-5484** frozen burn-auth (can't express "issuer OR *current* authority"). The custodial contract additionally removes `RECOVERY_ROLE` and any profile-root setter entirely: `profileRoot` is write-once (§4.2), so there is no update capability for a role to hold. (The audit-era `UPDATER_ROLE`/`setProfileRoot` design applied to the retired owner-owned SBT.)
- **Status, not burn.** `DogTagStatus {Active, Lost, TransferPending, Deceased, Revoked}`; `Active↔Lost`/`Active↔TransferPending` reversible, `Deceased`/`Revoked` terminal. **`Deceased` is set by `AUTHORITY_ROLE` or the original issuer — never the owner** (death is reported by an accredited party, often a different vet than the minter). **Never burn for lifecycle** (would orphan referencing credentials); `burn` is admin **GDPR-erasure only**.
- **Recovery = re-issue (D3).** The custodial `DogTagSBTConsent` has NO `recover()`/`RECOVERY_ROLE`: a signature-authorized rebind names the new owner on-chain, which is exactly the linkage owner-unlinkability removes. Recovery is a fresh custodial issuance under a new `dogTagId` + new `R` (`ProfileTreeStore.reissue`), and referencing credentials do **not** survive (accept-the-break, 2026-07-16); see §4.2. (Historical: the retired owner-owned SBT resolved the audit's unspecified-transfer Critical with a consensual two-signature `recover()` re-bind gated by `RECOVERY_ROLE`; that mechanism is deleted with the contract.)
- **Hardened `confirm`.** Derive `signer` from the **transaction** (never the request body); require `tx.to`/`tx.input`/`tx.value:0`/`tx.chainId:135` to equal the prepared draft; pin the emitting contract address for the `RootIssued` log; require `isWhitelistedFor(recordType, signer)` at confirm; wait **N confirmations** (reorg-safe); idempotent on `txHash`.
- **`dogTagId` is non-personal** (random/sequential) - **forbidden** to be any hash of the microchip (neither `keccak256(microchip)` nor `Poseidon(microchip)` - would anchor a brute-forceable chip hash). Cross-pet enumeration by owner address is structurally impossible: no owner address appears on-chain (§4.2).
- **Operator-session auth** guards every issuance/settings/signer route (`prepare`, `confirm`, `/records/*`, `settings/signing-mode`, `issuer/signers`, `import/*`, `calendar/*`); only `GET /records/{id}` (record-JWT) and HMAC cross-backend routes are unauthenticated. Legacy `/records` is retired or operator-gated.
- **Cross-backend erasure propagation.** A delete-request propagates **central → every business backend** (the vet is the GDPR controller and holds copies); each runs the same crypto-shred. Consent withdrawal wires to retention re-eval → erase.
- **Funds custody minimized.** Prefer **gas sponsorship / account abstraction (ERC-4337/7702)** so pet owners **never hold PLASMA**; native send/receive omitted from v1. If funds custody is ever added, obtain a money-transmission legal read (parallel to the privacy DPIA).

### 13.7 v3 normative items (privacy of on-chain verification events) — extend §11.1 / §13.5

Source: [`research/11-consent-attestation.md`](./research/11-consent-attestation.md) + [`research/12-verification-integration.md`](./research/12-verification-integration.md). Code in `implementation.md §11.8`.

- **Proof-of-verification publishes a permanent behavioral linkage.** A recorded `Verified` event is **owner-blind** (no subject field), but still ties `dogTagId` ↔ verifier (relayer) ↔ time - i.e. *which business verified which pet, when*. This pet-level trail is **pseudonymous personal data, in DPIA scope** (NOT exempt). The **mandatory DPIA (§11.1) must be refreshed** to cover the verification-event linkage.
- **Mitigations (normative):**
  1. **No owner on-chain, ever.** What becomes public per verification is only `(dogTagId, purpose, relayer, nullifier, R, recordType, deadline)` - no owner address, no key, no credential data. The privacy-endgame variant the audit era called "publish the nullifier *instead of* `subject`" is **achieved**: the live event carries the nullifier and has no subject field at all. (The retired owner-revealing path that emitted `subject` - and, in its ECDSA variant, disclosed the record to the verifier - is deleted.)
  2. **Owner-unlinkability is identity-level, not event-level.** `dogTagId` and `R` are stable per tag, so one tag's verifications remain linkable to each other; state this in the DPIA rather than overclaiming.
  3. **Consent receipts kept off-chain and deletable**, inside the existing **crypto-shred erasure scope** (`verification_records`, off-chain consent receipts - §9, §13.5).
  4. **Prefer a permissioned chain / no public block explorer**, so the linkage is not openly enumerable.

### 13.8 v3.1 normative items (verification-subsystem audit remediations) — code in impl §11.9

Three audits (07 ZK, 08 contracts, 09 systems) found the ZK path **unsound as first specified**. The fixes (full code in `implementation.md §11.9`):

- **Bind `purpose` end-to-end - STILL NORMATIVE, realized in-circuit.** `purpose` (distinct from `recordType`) is signed inside the EdDSA consent message `M`, is in the nullifier, is a circuit public signal, keys the `VERIFY:` whitelist, and is a `Verified` event field. Without it the `VERIFY:` gate collapsed to one global boolean (re-introducing v1's C-2 flaw) and the taxonomy never reached chain.
- **audit-07 C-1 (zkCommit keccak↔Poseidon binding) — RESOLVED-by-unification.** v4 unifies the credential commitment on Poseidon: there is **ONE root `R`** anchored by `issue(R)` and proven directly by the circuit, so **there is no off-chain keccak↔Poseidon binding left to be unsound**. The originator-gated `zkCommit`, the dual `rKec`/`rZk` leaf computation, and any in-circuit keccak are deleted (CHANGESPEC-v4 §2).
- **audit-08 C-2 (unbound `issuerForAny`/`kecOf`) - RESOLVED-by-unification.** With a single public root `R`, the registry calls `DogTagIssuer.isValid(R)` **directly**; the `zkIndex`/`cloneOf`/`kecOf`/`issuerForAny` lookups are deleted (clone resolution is the write-once `rootIssuer[R]` - §13.9).
- **Subject binding - SUPERSEDED by the owner-hidden redesign.** The audit-era fix bound `subject` into the EdDSA message and required an on-chain `keyOf[subject]==keyHash` + `ownerOf(dogTagId)==subject` so a relayer couldn't attribute a verification to a victim. The live circuit removes `subject` entirely; the equivalent soundness now comes from the **in-tree consent key** (the `owner.consentKey` leaf proven ∈ `R`), the on-chain `R == profileRoot(dogTagId)` binding, and the relayer-bound message + nullifier (§4.7).
- **Pinned Poseidon - STILL NORMATIVE** (circom == TS == Rust, CI cross-vectors against `poseidon([1,2])=0x115cc0f5...189a`) so the device, the server prover, and the circuit agree bit-for-bit on leaves, roots, and the nullifier. This covers the **credential commitment itself** (§3.3/§3.4), not just the nullifier; no on-chain Poseidon remains (the nullifier is a circuit output the chain only consumes).
- **Generalized hardened confirm** asserts the `Verified` event + `consumed[nf]` for verify submissions.
- **Submit gate + fail-fast.** The phone posts the proof directly to the verifier host with the one-time export token - a **gas-spend gate only** (the proof names its own relayer, so a stolen token cannot redirect it); the backend preflight mirrors the registry's requires (`pub[relayer] == activeSigner`, ranges, deadline margin, Art. 9, unconditional `VERIFY:` whitelist) to fail fast before paying gas. (The audit-era HMAC-authenticated central relay hop is retired with the owner-revealing path; there is no central relay in the verify loop.)
- **Art. 9:** `SERVICE_ATTESTATION` has no on-chain root and is **NOT verifiable via on-chain proof-of-verification** (rejected at registry + backend); `purpose` labels are non-sensitive (no cleartext Art. 9 leak in `Verified.purpose`).
- **ZK privacy scope (on-device proving is CANONICAL):** the **phone** generates the Groth16 proof locally and POSTs only `{proof, pubSignals}` - the groomer **never receives the raw record or the witness**, so verification minimizes exposure **both on-chain AND to the groomer** (true zero-knowledge against the verifier, not just the chain). Server-side proving is the `/prove-consent` trusted fallback (§10.3) or the `dogtag-prover-rs` test oracle, never the canonical path - any earlier wording calling on-device a "v2 upgrade" or claiming "the verifier receives the witness/disclosed doc" is **superseded**.
- **Per-tag BabyJubjub consent key - STILL NORMATIVE, now in-tree.** Derived from `(wallet seed, dogTagId)` and committed as the `owner.consentKey` leaf, so per-tag verifications never re-link through a shared wallet-level key, and no `keyOf` registry remains in DPIA scope. Rotation is the recovery re-issue (new tree, new key, new id - §13.6).
- **Deploy/ops:** clone resolution for `isValid(R)` via the write-once `rootIssuer[R]` index (§13.9), a real propose/execute timelock on the verifier swap, and Phase 2.5 gated on ROAX supporting the **BN254 pairing precompiles**.

### 13.9 v4.1 normative items (Poseidon-unification audit remediations) — code in impl §11.10

Audits 10 (Poseidon), 11 (contracts), 12 (systems) confirmed the unification eliminates the C-1/C-2 binding Criticals and is structurally sound, with these required fixes:

- **Clone resolution via write-once `rootIssuer[R]`** (audit-11 V4-C1, Critical). A single Poseidon root `R` is issued in exactly one per-business clone, but `recordType→clone` is one-to-many - so the registry MUST resolve the clone **from the root itself**: `issue(R)` writes a protocol-global write-once `rootIssuer[R]=clone` (the factory doubles as the index), and `VerificationRegistryConsent` does `clone = rootIssuer[R]; require(clone!=0); isValid(R)`. This supersedes the `purposeToRecordType`/`issuerFor[recordType]` resolution (which couldn't pick the right per-business clone → wrong-issuer / revocation-evasion).
- **Per-arity Poseidon determinism** (audit-10 P-C1, Critical): CI anchors at **t=2, t=3, t=6, t=7** (not just `poseidon([1,2])`), bit-identical across the pinned circom / TS / Rust libraries, since `R_P`/constants are per-`t`.
- **Field-reduction parity** (audit-10 P-C2, Critical): all reductions pinned to the BN254 **scalar field `r`** (`purpose = keccak256(label) mod r` identical in circom + Rust + TS); the registry range-checks every public signal `< r` before use, so congruent values can't collide in the `consumed` set. (The retired ECDSA path's on-chain `PoseidonT7` nullifier - and its pre-hash range-checks - are gone with it.)
- **In-circuit Merkle matches the SDK** (odd-promotion, integer `[0,p)` comparator, single-leaf), `bytesToField` edge vectors, Rust limb-decode discipline, and a real propose/execute verifier-swap timelock.
- **Confirmed eliminated:** audit-07 C-1 / audit-08 C-2 (no off-chain keccak↔Poseidon binding remains). **Still normative:** purpose binding, hardened confirm, Art. 9 exclusion, per-tag consent key, nullifier-as-public-signal, full-vector range checks. The subject↔key (`keyOf[subject]==keyHash`) and `ownerOf==subject` gates are **superseded by the owner-hidden redesign** (in-tree consent key + `profileRoot` binding - §13.8). All v1/v2/v3.1 remediations otherwise intact.

## 12. Open items / future
- Government/airline issuer stacks (USDA APHIS endorsement via VEHCS, EU competent authority, DOT/airline verification).
- Batched anchoring (contracts already support it).
- On-chain Merkle proof verification lib (off-chain suffices for v1).
- ROAX EIP-1559 support confirmation; `evm_version = paris` until PUSH0 confirmed.
- **Multisig for `DEFAULT_ADMIN_ROLE` — execution pending (code shipped).** The two-phase EOA→multisig hand-off is implemented and tested (§13.1 H-3; `script/MigrateGovernance.s.sol` + `GovernanceMigrationTest` + [`GOVERNANCE_MIGRATION.md`](./GOVERNANCE_MIGRATION.md)); the remaining step is the captain selecting the multisig signers and executing the (irreversible) hand-off on-chain.
- Titer-test and EU recodification field updates as standards evolve.
- **Delegation (non-owner scoped consent) - architecture decided 2026-07-20, implementation deferred post-v1.** It ships as a **separate delegate circuit** with its own ceremony, so it gates nothing on the owner path; see [`DELEGATION.md`](./DELEGATION.md). The one item that decision leaves open is `DEPTH` - now the **only** remaining ceremony-gated decision before the mainnet trusted-setup re-run.
