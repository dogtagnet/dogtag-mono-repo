# DogTag Ecosystem — Architecture

> Status: v1 design. Source research: [`docs/research/`](./research). Reference UI/data: [`references/`](../references).
> Chain: **ROAX** (EVM, chainId `0x87` = **135**, native gas token **PLASMA**, RPC `https://devrpc.roax.net`, explorer `https://explorer.roax.net` — Blockscout-style). RPC was returning `502` at design time; treat liveness as a deploy-time pre-check.

---

## 1. Vision & scope

DogTag is a **pet-credentialing ecosystem**. Pet owners hold their pets' identity, health, service, and travel records in a mobile app. Veterinarians and groomers (and later governments/airlines) run software that **issues and consumes verifiable credentials** about pets. Credentials are **anchored on-chain** (Merkle roots) and **verifiable three ways**: cryptographic integrity, on-chain issuance/revocation status, and **DNS-bound issuer identity** — the OpenAttestation trust triangle, implemented here **from scratch** with a **language-agnostic, JSON-free canonicalization**. A contextual fourth fragment, **on-chain ownership** (`DogTagSBT.ownerOf` == the user's self-custodial wallet), gates the owner's own self-import but is informational for third-party verifiers (§5).

### 1.1 Products in this monorepo

| Product | Tech | Who runs it | Folder |
|---|---|---|---|
| Pet-owner app (Android) | Kotlin + Jetpack Compose | End users | `apps/android` |
| Pet-owner app (iOS) | Swift + SwiftUI | End users | `apps/ios` |
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
                         │  DogTagSBT · IssuerRegistry · Issuers      │
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

This is the **open-sourced, library-agnostic** core. Identical results in TypeScript, Rust, and Solidity. Full rationale: [`research/02-attestation.md`](./research/02-attestation.md), [`research/01-data-standards.md`](./research/01-data-standards.md).

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
  "privacy": { "obfuscated": [] }            // hashes of redacted leaves (selective disclosure)
}
```

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

### 3.3 Canonical leaf hashing (the algorithm)

```
leafHash = keccak256( 0x00                       // domain separator: LEAF
                    ‖ len(keyPathBytes)  ‖ keyPathBytes
                    ‖ len(salt=16)       ‖ salt
                    ‖ uint8(typeTag)
                    ‖ len(valueBytes)    ‖ valueBytes )
```

- All `len(...)` are `uint32` big-endian. **Length-prefixing every component** kills intra-leaf second-preimage ambiguity.
- Value encoding rules (deterministic across languages):
  - `null` → empty bytes.
  - `bool` → `0x00`/`0x01`.
  - `string` → **NFC-normalized** UTF-8 bytes.
  - `integer` → decimal ASCII, **no leading zeros, no `-0`** (arbitrary precision; covers microchip IDs and lot numbers as exact integers — never floats).
  - `decimal` → fixed decimal **string** (e.g. weight `"22.7"`), normalized (no trailing zeros beyond significant, single canonical form). **Native floats are forbidden.**
  - `bytes` → raw bytes.
- **keccak256** = Ethereum Keccak (padding `0x01`), **not** NIST SHA3-256 (`0x06`). Confirmed in research; this is what Solidity's `keccak256` and OZ `MerkleProof` use.

> Implementations: TS uses `@noble/hashes` `keccak_256`; Rust uses `alloy_primitives::keccak256` (or `tiny-keccak::Keccak::v256`); Solidity uses native `keccak256`. The byte layout above is a hand-rolled length-prefixed concatenation (not `abi.encode`) so it is trivially identical in all three without an ABI codec. (research/02 suggested `abi.encode`; we pin the explicit concat for zero ABI dependency — documented as the normative spec.)

### 3.4 Merkle tree build

```
1. Compute leafHash for every field.
2. Sort leaf hashes ascending, bytewise. (deterministic order, ignores field order)
3. Build bottom-up:
     parent = keccak256( 0x01 ‖ sortPair(left, right) )   // 0x01 = NODE domain sep
     sortPair(a,b) = a <= b ? a‖b : b‖a                    // commutative
   - A lone odd node is PROMOTED unchanged to the next level (never duplicated).
4. Root of the tree = targetHash for this document.
   - Single-leaf document: targetHash = that one leaf hash.
```

- **Commutative `sortPair`** ⇒ proofs are unordered sibling sets; on-chain verification needs no left/right bits and is compatible with OZ `MerkleProof.processProof` semantics (with our domain separators baked into a tiny custom verifier — see §5.4).
- One comparator everywhere; domain separators (`0x00` leaf / `0x01` node) prevent leaf/node confusion attacks (a hardening OA omits).

### 3.5 Selective disclosure / obfuscation

To redact a field while keeping the same root: move the field's **leaf hash** into `privacy.obfuscated[]`, delete its cleartext from `data`. Verifier recomputes the leaf set as `(hashes of remaining fields) ∪ privacy.obfuscated`, rebuilds the tree, and gets the **same `targetHash`**. Lets a pet owner share, e.g., rabies status without revealing owner address.

### 3.6 Credential schemas (W3C VC 2.0 envelope)

All credentials wrap in **W3C Verifiable Credentials Data Model 2.0**. The envelope is canonicalized exactly per §3.2–§3.4 (keccak256 salted leaves; **we do NOT adopt JSON-LD/RDF canonicalization** — SMART Health Cards / EU DCC lesson: anchor only a hash/root, never RDF-canonicalize). Envelope fields (canonical, per CHANGESPEC §0):

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

**Dog identity** (`DOG_PROFILE`, mints the SBT): `dogTagId` (SBT tokenId), `name`, `species` (top-level), `breedVbo` (Vertebrate Breed Ontology id, e.g. `VBO:0200798`) + `breedLabel`, `sex` (`male`|`female`) **separate from** `neuterStatus` (`intact`|`neutered`|`spayed`), `dateOfBirth` (derive age — drop free-text age), `colour`, `distinctiveFeatures`, `weightHistory[]{value, unit:"kg"|"lb", measuredOn}` (unit-bearing + dated), `microchip`, `photoHashes[]` (off-chain blobs, hash only).

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

---

## 4. Smart-contract architecture

Solidity + Foundry, OpenZeppelin v5. Full snippets/signatures: [`research/03-chain-contracts.md`](./research/03-chain-contracts.md). Deploy/verify: §8.

### 4.1 Contract set

```
DogTagSBT          — ERC-721 + ERC-5192 soulbound. One non-transferable token per pet ("chip").
IssuerRegistry     — central AccessControl whitelist of issuer signing addresses (the gate).
DogTagIssuer       — per-record-type anchoring contract (implementation, cloned). Issues/revokes
                     bytes32 merkle roots; every write gated by IssuerRegistry.
DogTagIssuerFactory— deploys DogTagIssuer EIP-1167 clones (one per record type / per business).
```

### 4.2 `DogTagSBT` — the pet identity (granular-role lifecycle)

The "DogTag" factory: **issues an on-chain identity per chip/pet** that everyone references. The lifecycle (create / update / revoke / status, recovery) is split into **least-privilege roles + originator binding + authority override**, per [`research/09-sbt-lifecycle.md`](./research/09-sbt-lifecycle.md). (Normative refinements in §13.6.)

**Standards posture:** ERC-721 + ERC-5192 (permanently `locked`); `issuerOf` + issuer/verifier separation borrowed from ERC-5727 (vocabulary only); **ERC-5484's frozen mint-time `BurnAuth` is rejected** — it cannot express "the original issuer **OR** a *current* authority," and authority legitimately changes (a clinic closes, a regulator steps in). Status semantics follow W3C Bitstring Status List (status is *about* the credential; never destroys it).

**Granular action roles** (OZ v5 `AccessControlEnumerable`, so the accredited set is publicly auditable):
- `ISSUER_ROLE` — **create/mint** a DogTag.
- `UPDATER_ROLE` — **update** the profile root.
- `AUTHORITY_ROLE` — **cross-issuer revoke + status transitions** (incl. `Deceased`); any current authority may act on any token.
- `RECOVERY_ROLE` — execute **lost-key re-bind** (owner-address recovery).
- `DEFAULT_ADMIN_ROLE` — protocol multisig (`AccessControlDefaultAdminRules`, two-step + delay).

**Originator binding + authority override** (resolves your deceased question): record `issuerOf[tokenId]` at mint, **immutable**. Mutations are gated by `msg.sender == issuerOf[tokenId] || hasRole(AUTHORITY_ROLE, msg.sender)`. So the **original issuer can always update/revoke its own tokens**, and **any *current* authority can act on any token** — which is exactly why marking a pet **`Deceased`** is an `AUTHORITY_ROLE`-or-original-issuer action (a death is often reported by a *different* accredited vet than the minter), **never** the owner. Because authority membership is mutable, authority evolves without re-issuing tokens (impossible under ERC-5484's frozen value).

**Status model — soft status, NEVER burn** (`DogTagStatus` enum): `Active`, `Lost`, `TransferPending`, `Deceased`, `Revoked`. `Active↔Lost` and `Active↔TransferPending` are reversible; **`Deceased` and `Revoked` are terminal/irreversible**. We do **not** burn on death/revocation — burning would orphan every credential that references `tokenId` and break historical verifiability. `burn` is reserved for the **admin GDPR-erasure path only** (§13). Every transition emits `StatusChanged(tokenId, from, to, by, reason)`.

- `tokenId` (`dogTagId`) = the canonical pet identity; **allocated as a random/sequential non-personal id** — **never** `keccak256(microchip)` (that would anchor a brute-forceable chip hash on-chain — §11.1). All other credentials reference it.
- **Owned by the USER's self-custodial wallet** (mobile embedded-MPC/BIP-39 address — §10); soulbound, so ownership can't be silently moved. `ownerOf(dogTagId)` is read at the owner's *self-import* only (§5 contextual ownership). To break cross-pet enumeration, mint each pet to a **fresh per-pet derived address** (§11.1, §13.6).
- One SBT per microchip (uniqueness enforced off-chain by the central backend before mint).

**Lost-key recovery — signature-authorized re-bind, NOT burn-and-remint** (resolves the audit's Critical on the unspecified transfer scheme): `recover()` **preserves `tokenId` and `issuerOf`** (only the owner address changes), so referencing credentials survive. It is exempt from the soulbound lock and gated by `RECOVERY_ROLE` **plus an EIP-712 signature from the *destination* owner** binding `{dogTagId, newOwner, nonce, deadline, chainId:135, verifyingContract}`. True lost-key (no key at all) → `RECOVERY_ROLE` executes after an **off-chain identity proof to the protocol** (does not require the lost key); sale/transfer → also requires the current owner's authorization (EIP-712 or `ownershipHistory`). ERC-6147 guard is an opt-in social-recovery layer, off by default.

Key functions:
```solidity
function mint(address to, uint256 dogTagId, bytes32 profileRoot) external onlyRole(ISSUER_ROLE); // to == user's fresh per-pet address; records issuerOf[dogTagId]=msg.sender; emits Locked + Issued
function setProfileRoot(uint256 dogTagId, bytes32 newRoot) external; // require msg.sender==issuerOf[id] || AUTHORITY_ROLE; only if status==Active
function setStatus(uint256 dogTagId, DogTagStatus s, string reason) external; // require msg.sender==issuerOf[id] || AUTHORITY_ROLE; Deceased/Revoked terminal; never owner; emits StatusChanged
function recover(uint256 dogTagId, address newOwner, uint256 nonce, uint256 deadline, bytes ownerSig) external onlyRole(RECOVERY_ROLE); // EIP-712 by newOwner; preserves tokenId+issuerOf; emits Recovered
function ownerOf(uint256 tokenId) external view returns (address); // read only at owner self-import (contextual ownership pillar)
function status(uint256 dogTagId) external view returns (DogTagStatus);
function locked(uint256 tokenId) external pure returns (bool); // always true
function burn(uint256 tokenId) external onlyRole(DEFAULT_ADMIN_ROLE); // GDPR-erasure ONLY; emits Burned
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

Onboarding flow (off-chain → on-chain, also triggered on a signing-mode switch — see §6): a new signer address → vet submits `{issuerEntityId, address, mode, recordTypes, USDA#, license#}` to the **central/admin backend** → admin verifies accreditation off-chain → admin calls `whitelistFor(recordType, addr)` per record type → app polls `isWhitelistedFor` until live. Only then can that address issue. Delist inactive-mode addresses to avoid a stale, over-broad whitelist; backend key rotation = a new address to whitelist.

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

```
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
    integrity: recompute leaves+merkle from data → == targetHash; proof→merkleRoot
    issuance:  issuer.isValid(merkleRoot) via ROAX RPC (read-only)
    identity:  DNS TXT of issuer.domain lists issuer.documentStore + chainId
  + ownership (CONTEXTUAL — only in the owner's self-import context):
    ownership: DogTagSBT.ownerOf(dogTagId) == userWalletAddress (record imports as "yours" only if you control the on-chain owner)
    NOT_APPLICABLE for third-party verifiers (a groomer/airline is not ownerOf)
  mobile stores credential under the pet, grouped by recordType
```

---

## 5. Verification pipeline

A credential's **authenticity** rests on **three pillars** — it is VALID only if all three return VALID and none returns INVALID (OA-style fragment model; each fragment is tri-state `VALID | INVALID | ERROR` — a network/RPC error ≠ forged ≠ valid):

1. **Integrity** — recompute every leaf hash from `data`, union with `privacy.obfuscated`, rebuild the Merkle tree → must equal `signature.targetHash`. Then `processProof(proof, targetHash)` → must equal `signature.merkleRoot`. (Pure, offline, in the SDK.)
2. **Issuance status** — read `DogTagIssuer(issuer.documentStore).isValid(merkleRoot)` over ROAX RPC. Must be `true` (issued, not revoked).
3. **Identity (DNS)** — resolve `issuer.domain` TXT records over DNS-over-HTTPS; one must read `dogtag net=ethereum chainId=135 addr=<documentStore>` (case-insensitive addr, matching chainId). Binds the human-trusted domain to the contract.

A fourth fragment, **ownership**, is **contextual — not a universal validity gate**:

4. **Ownership** — read `DogTagSBT.ownerOf(dogTagId) == userWalletAddress` over ROAX RPC. **Gating only in the mobile owner's self-import context** (a record imports as "yours" only if you control the on-chain owner — a forged/stolen record for a pet you don't own won't bind to you). For **third-party verifiers** (a groomer/airline/vet is *not* `ownerOf`), ownership is `NOT_APPLICABLE`/informational and the three authenticity pillars decide validity. Tri-state.

The SDK exposes `verify(wrappedDoc, {rpc, dnsResolver, userWalletAddress?}) → { valid, fragments: {integrity, issuance, identity, ownership} }`. When `userWalletAddress` is absent (third-party verification), `ownership` resolves to `NOT_APPLICABLE` and never blocks `valid`. Both TS and Rust implement it identically; mobile apps call the Rust crate (via FFI/UniFFI) or a thin native port.

### 5.4 On-chain verifier note

`DogTagIssuer.isValid` only checks the **anchored root**. Merkle-proof checking happens **off-chain** in the SDK (cheaper, and the chain only needs the root). A `MerkleVerifierLib` mirrors the §3.4 domain-separated commutative hash so any contract that wants on-chain proof verification can, but v1 does not require it.

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

## 7. QR + JWT record sharing

- **JWT alg:** EdDSA (Ed25519), a **per-deployment keypair separate from blockchain keys** (ES256 fallback). Lib `jsonwebtoken` 10.x.
- **Claims:** `iss`=deployment URL, `sub`=recordId (scoping anchor), `aud`=`dogtag-mobile`, `scope`=`read:record`, `iat`/`nbf`, `exp` ~2–5 min, `jti`.
- **Enforcement:** server checks `sub == path recordId`, scope, and a `jti` store (Redis/Mongo `SETNX … EX exp`) for **one-time use**; `leeway = 30s` for clock skew.
- **QR payload:** HTTPS deep link `https://<deployment-host>/r?t=<jwt>&i=<recordId>` — the **origin is the API base**, so the per-deployment URL requirement is satisfied by construction. ECC level M, byte mode, ~QR v6–10. `qrcode` crate.
- **Two QR directions:**
  - **Issuer → user** (vet/groomer shows QR; mobile pulls the record to import).
  - **User → business** (mobile shows QR carrying a JWT against the **central** API; groomer/vet pulls the pet profile / vaccination status the user is sharing). Same one-time-JWT pattern, audience `dogtag-business`.

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

### 9.2 Business (vet/groomer) DB
- `keystore_meta` — genesis state, encrypted-seed location, derived accounts (addresses+labels only) — backend signing mode.
- `records` — issued wrapped documents `{recordId, recordType, dogTagId, wrappedDoc, root, txHash, signingMode, signerAddress, custodian, retention{basis, clock}, status}`. **`custodian` (the practice = legal record owner) is distinct from the pet-`Owner`.**
- `issuer_signers` — `{issuerEntityId, address, mode('wallet'|'backend'), recordTypes[], whitelistedTxHash, status}` — one issuer entity, many signing addresses (§4.3).
- `consents` / `consent_receipts` — per-purpose lawful-basis records (mirror of §9.1 for issuer-side processing).
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
- **Storage = encrypt-then-store:** a hardware key in the **Secure Enclave (iOS) / StrongBox (Android)** encrypts the seed/secret; the ciphertext is stored in normal storage; decryption is **biometric-gated** (the Enclave/Keystore can't hold an arbitrary 256-bit seed directly). Require `biometryCurrentSet`/`setUserAuthenticationRequired` so re-enrolling biometrics invalidates the key; `…ThisDeviceOnly`/no auto-backup.
- Shows **address + PLASMA balance**, send/receive; connects to external dApps (scan `wc:` URI) via **Reown WalletKit** (both platforms).
- **The pet's `DogTagSBT` is owned by this wallet's address** (a **fresh per-pet derived address** — §4.2/§13.6). It supplies the **contextual `ownership` fragment** used only at the owner's own self-import (`ownerOf(dogTagId) == myWalletAddress`); third-party verifiers don't use it (§5). Pet claim/transfer and lost-key recovery use **`recover()` (signature-authorized re-bind preserving `tokenId`), not burn-and-remint** (§13.6). v1 prefers **gas sponsorship / AA so owners hold no PLASMA**.

### 10.2 Portal theming — light/dark

The **vet, groomer, and admin web portals** get a real user-switchable **light/dark theme toggle** (persisted), via `packages/ui` semantic tokens gaining light + dark palettes. Matches the groomer reference aesthetic (dark sidebar / light content) but as switchable light/dark — **portals are light/dark only, not the 7 mobile colorways**.

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

- **No personal data in cleartext or recoverable form on-chain.** On-chain holds only: salted commitments (salts off-chain), revocation/status, non-personal DIDs/keys, timestamps, schema/version, accreditation refs. **A salted hash is itself personal data** (pseudonymisation, not anonymisation — ICO/EDPB), and an **unsalted hash of a low-entropy microchip number (15 digits) is brute-forceable → effectively reversible → personal data on an immutable ledger.** Hence per-field **16-byte random salts are the privacy mechanism, not just anti-forgery.** A globally-replicated ledger is also an independent GDPR Chapter V (cross-border transfer) problem — minimising on-chain personal data minimises on-chain transfer.
- **The owner's wallet address ↔ pet SBT link IS pseudonymous personal data** and must be treated as such (it is on-chain by design — §4.2/§5). It is in DPIA scope, NOT exempt. Mitigations: **derive a fresh address per pet** (`m/44'/60'/0'/0/{petIndex}`) so an observer can't enumerate one person's whole pet history from a reused address; SBT burn is part of the erasure flow (below); the v2+ upgrade path is account-abstraction (ERC-4337/7702) with sponsored gas. Do **not** claim "nothing personal on-chain" without this qualification — it would not survive a DPIA. `dogTagId` (SBT tokenId) is allocated as a **non-personal** id — never `keccak256(microchip)` (that would anchor a brute-forceable low-entropy hash of the chip).
- **Never on-chain (enumerated):** any owner PII (name/address/email/phone), document scans, **service-animal / disability indicators** (GDPR Art. 9 special category; CPRA sensitive PI) — service/assistance attestation data is off-chain only; and unsalted/low-entropy hashes of the microchip code or cert serials.
- **Right-to-erasure = destroy every copy of every per-field salt + delete the off-chain record + burn the SBT** so the on-chain commitment becomes **unlinkable** and the live `ownerOf → wallet` binding is dropped. The salt (16-byte CSPRNG, 128-bit) is the hiding term — even for low-entropy values an adversary must brute-force 2^128 salts — so destroying the salt unlinks **provided all copies are destroyed**. The weak link is **copy-proliferation**: the salt sits in cleartext in every distributed wrapped-doc `data` (issuer DB, holder device, importer caches, QR copies, backups/oplog). Implement erasure as **crypto-shredding**: encrypt salts/`data` under a per-record DEK at rest and destroy the DEK so all reachable ciphertext copies become undecryptable; copies the protocol can't reach (holder device, third-party importers) are DPIA residual risk. This is **risk-mitigation, NOT a regulator-blessed safe harbour** (CNIL: "close to" erasure; EDPB does not bless key-destruction as automatically satisfying Art. 17). A **DPIA is mandatory**, refreshed on any change to on-chain fields or chain topology; prefer a permissioned network where possible.
- **CCPA/GDPR delete endpoint (45-day SLA)** wired to the *same* off-chain delete + salt/key-destruction flow.
- **Consent + retention:** per-purpose `Consent`/`ConsentReceipt` records (lawful basis, withdrawable, timestamped — §9); `retention{basis, clock}` on credentials (default ≥5 yrs US / ≥3 yrs EU where silent).
- **Evidentiary legal posture + trust tiers:** a DNS-bound, chain-anchored W3C VC proves integrity/timing but carries **no eIDAS Art. 35 / ESIGN presumption** — authority is **extrinsic**, flowing from the accredited issuer (USDA-accredited vet / APHIS / competent authority). Encode `attestationType`, `signatureTrustTier`, `legalEffect` (`evidentiary`), `legalBasisVersion`, `jurisdiction`. The DOT form records that a **self-attestation under 18 U.S.C. §1001 exists** — never "verified disability". Never market the baseline as "legally binding / government-grade".
- **Record-custodian distinct from owner:** the practice/clinic owns the *record* (legal custodian); the pet-`Owner` has information-access rights — do not conflate (§9).

---

## 13. Audit remediations (v1.1 — NORMATIVE; overrides §1–§12 on conflict)

Three independent audits ([`research/audit-01-contracts.md`](./research/audit-01-contracts.md), [`audit-02-crypto.md`](./research/audit-02-crypto.md), [`audit-03-systems.md`](./research/audit-03-systems.md)) found issues that **must** be resolved before any deploy. This section is the corrected design; `implementation.md §11` carries the corrected code/pseudocode.

### 13.1 Smart contracts (audit-01)
- **C-1 — lock the clone implementation.** `DogTagIssuer` gets `constructor(){ _disableInitializers(); }`. The implementation is the only Blockscout-verified address; leaving it initializable lets an attacker point `registry` at a malicious contract.
- **C-2 — per-record-type, per-address scoping (not one global boolean).** The single global `isWhitelisted` is replaced by `IssuerRegistry.isWhitelistedFor(bytes32 recordType, address signer)`. Each issuer clone checks `registry.isWhitelistedFor(recordType, msg.sender)`. SBT mint/profile uses a **dedicated** `PROFILE_ISSUER_ROLE` distinct from record issuers. A groomer key can never touch vaccination roots or pet profiles.
- **H-1 — originator binding.** `DogTagIssuer` records `issuedBy[root]=msg.sender` on `issue`; only the original issuer **or** protocol admin may `revoke` it. `DogTagSBT.setProfileRoot` is restricted to `PROFILE_ISSUER_ROLE` and records the writer.
- **H-2 — `burn` is protocol-admin-only**, emits `Burned`, owners cannot self-burn (prevents orphaning referencing credentials).
- **H-3 — admin hardening.** `IssuerRegistry` uses `AccessControlDefaultAdminRules` (two-step admin transfer + delay); `DEFAULT_ADMIN_ROLE` is a **multisig** at deploy (not an open item). Whitelist duty and role-admin duty are split.
- **M-1 — `createIssuer` is permissioned** (`onlyRole(DEFAULT_ADMIN_ROLE)`), salt = `keccak256(recordType, business)` to stop front-running/squatting.
- **M-4 — chain settings.** `evm_version = paris` everywhere (consistent); verify-reads wait **N block confirmations** (configurable; default 5) to tolerate reorgs — issuance status is only trusted past finality.
- **M (registry desync)** — `IssuerRegistry` is the single source of truth; no parallel bespoke mapping.

### 13.2 Canonicalization & Merkle (audit-02) — determinism is mandatory
- **A1 — `canonicalDecimal` is pinned** to a closed ASCII grammar over the *input string*: `^-?(0|[1-9][0-9]*)(\.[0-9]+)?$`; strip fractional trailing zeros; drop a trailing dot; map `-0→0`; reject exponents/whitespace/`+`. (Covers weight `22.7`, titer `0.5`.)
- **A2 — typed input at the wrap boundary.** Numbers are **never** taken from a native float. Integers and decimals enter `wrapDocument` as **typed strings** (schema-driven), are carried as strings end-to-end, and `assertNotFloat` is a hard guard. `verify` never re-infers types — it reads the tag from the packed leaf.
- **A3 — Unicode pinned.** NFC normalization against a **pinned Unicode version** (stated in the SDK), unpaired surrogates rejected, NFC form stored in `data`. **Solidity participates at the node level only** — it never builds a leaf from a raw string.
- **C1 — invariant:** single-document verification **MUST rebuild the whole tree** and compare to `targetHash`; it must **never** trust `processProof` alone. (`processProof` is inclusion-only; position/shape unbound under commutative+odd-promotion.)
- **E2 — before enabling batching:** bind subtree size in the node hash — `hashNode = keccak256(0x01 ‖ u32be(subtreeLeafCount) ‖ sortPair(a,b))` — or use ordered proofs for the batch layer. Not needed for single-doc v1, but the format reservation is documented now.
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
- **`ownerOf` import check.** A record imports as the user's own only if `DogTagSBT.ownerOf(dogTagId) == userWalletAddress`. `ownership` is a **contextual, tri-state fragment** (§5): it gates the owner's self-import but is `NOT_APPLICABLE`/informational for third-party verification (a groomer/airline/vet is not `ownerOf`) — validity for them rests on the three authenticity pillars only. The SBT is **minted to and owned by the user's self-custodial wallet address** (§4.2); ownership changes use **`recover()` (signature-authorized re-bind), not burn-and-remint** — see §13.6.
- **PII-off-chain rule (qualified).** No recoverable personal data on-chain. Even a salted hash is personal data; an unsalted hash of a low-entropy microchip number is brute-forceable. Per-field 16-byte random **salts are the privacy mechanism**. Service/disability (Art. 9) data is off-chain only. **The wallet-address↔pet SBT link is pseudonymous personal data in DPIA scope** (§11.1) — mitigate with a fresh per-pet address and (v2+) account abstraction; `dogTagId` is non-personal (never `keccak256(microchip)`). Do not ship the unqualified "nothing personal on-chain" wording.
- **Multi-address whitelist.** `IssuerRegistry` supports **multiple signing addresses per issuer entity** (one-to-many). Invariant: the active signer MUST be `isWhitelistedFor(recordType, signer)`. A mode switch / second device / backend key rotation introduces a new address → admin approval queue → `whitelistFor` → poll until live; pre-flight `eth_call` to fail fast; delist inactive-mode addresses to avoid stale over-broad whitelisting.
- **MPC wallet storage.** Mobile default is an **embedded MPC wallet** (TSS — provider can't sign alone); raw BIP-39 export is advanced-only. Storage is **encrypt-then-store**: seed/secret encrypted by a Secure Enclave (iOS) / StrongBox (Android) hardware key, **biometric-gated**, `…ThisDeviceOnly`, no auto-backup, `biometryCurrentSet`-bound. Never log/serialize the plaintext seed.
- **Erasure-via-salt-destruction (crypto-shredding) + SBT burn.** The right-to-erasure flow destroys **every reachable copy** of every per-field salt (crypto-shred: per-record DEK destroyed → all ciphertext copies undecryptable), deletes the off-chain record, and **burns the SBT** (drops the live `ownerOf → wallet` link) so the on-chain commitment and ownership binding can no longer be reconstructed. The 128-bit salt is the hiding term (low value-entropy is fine); copy-proliferation is the real risk, so unreachable copies (holder device, third-party importers) are DPIA residual risk. Wired to both GDPR Art. 17 and CCPA §1798.105 (45-day) request paths.
- **Mandatory DPIA + CCPA/GDPR 45-day delete endpoint** on the crypto-shredding flow above. Legal posture is **evidentiary, not authoritative** (trust tiers per §0).

### 13.6 v3 normative items (granular SBT lifecycle, recovery, auth, funds) — extend §13.1–§13.5

Source: [`research/09-sbt-lifecycle.md`](./research/09-sbt-lifecycle.md) + audit-04/05/06. Code in `implementation.md §11.7`.

- **Granular SBT roles + originator + authority override** (your decision). Replace the single profile role with `ISSUER_ROLE` (create), `UPDATER_ROLE` (update), `AUTHORITY_ROLE` (cross-issuer revoke + status), `RECOVERY_ROLE` (re-bind), `DEFAULT_ADMIN_ROLE`. Record **immutable `issuerOf[tokenId]`** at mint; mutations require `msg.sender == issuerOf || hasRole(AUTHORITY_ROLE)`. **Reject ERC-5484** frozen burn-auth (can't express "issuer OR *current* authority").
- **Status, not burn.** `DogTagStatus {Active, Lost, TransferPending, Deceased, Revoked}`; `Active↔Lost`/`Active↔TransferPending` reversible, `Deceased`/`Revoked` terminal. **`Deceased` is set by `AUTHORITY_ROLE` or the original issuer — never the owner** (death is reported by an accredited party, often a different vet than the minter). **Never burn for lifecycle** (would orphan referencing credentials); `burn` is admin **GDPR-erasure only**.
- **Recovery = signature-authorized re-bind, not burn-and-remint** (resolves the audit's Critical unspecified-transfer). `recover()` preserves `tokenId` + `issuerOf` (referencing creds survive), gated by `RECOVERY_ROLE` **+ EIP-712 signature from the destination owner** binding `{dogTagId, newOwner, nonce, deadline, chainId:135, verifyingContract}`. Catastrophic lost-key (no key) → `RECOVERY_ROLE` after off-chain identity proof (does not need the lost key). ERC-6147 guard opt-in only.
- **Hardened `confirm`.** Derive `signer` from the **transaction** (never the request body); require `tx.to`/`tx.input`/`tx.value:0`/`tx.chainId:135` to equal the prepared draft; pin the emitting contract address for the `RootIssued` log; require `isWhitelistedFor(recordType, signer)` at confirm; wait **N confirmations** (reorg-safe); idempotent on `txHash`.
- **`dogTagId` is non-personal** (random/sequential) — **forbidden** to be `keccak256(microchip)` (would anchor a brute-forceable chip hash). **Fresh per-pet owner address** to break cross-pet enumeration.
- **Operator-session auth** guards every issuance/settings/signer route (`prepare`, `confirm`, `/records/*`, `settings/signing-mode`, `issuer/signers`, `import/*`, `calendar/*`); only `GET /records/{id}` (record-JWT) and HMAC cross-backend routes are unauthenticated. Legacy `/records` is retired or operator-gated.
- **Cross-backend erasure propagation.** A delete-request propagates **central → every business backend** (the vet is the GDPR controller and holds copies); each runs the same crypto-shred. Consent withdrawal wires to retention re-eval → erase.
- **Funds custody minimized.** Prefer **gas sponsorship / account abstraction (ERC-4337/7702)** so pet owners **never hold PLASMA**; native send/receive omitted from v1. If funds custody is ever added, obtain a money-transmission legal read (parallel to the privacy DPIA).

## 12. Open items / future
- Government/airline issuer stacks (USDA APHIS endorsement via VEHCS, EU competent authority, DOT/airline verification).
- Batched anchoring (contracts already support it).
- On-chain Merkle proof verification lib (off-chain suffices for v1).
- ROAX EIP-1559 support confirmation; `evm_version = paris` until PUSH0 confirmed.
- Multisig for `DEFAULT_ADMIN_ROLE`.
- Titer-test and EU recodification field updates as standards evolve.
