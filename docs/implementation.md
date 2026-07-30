# DogTag Ecosystem — Implementation Guide

> Companion to [`architecture.md`](./architecture.md). This document specifies **what each function does, with pseudocode**, the API surface of every service, the contract bodies, the Docker topology, and a deploy/test plan. Language-agnostic pseudocode; real code lives in the listed paths.

## 0. Monorepo layout

```
dogtag-mono-repo/
├── apps/
│   ├── android/                 # Kotlin + Jetpack Compose
│   └── ios/                     # Swift + SwiftUI
├── stacks/
│   ├── admin/   { web/ api/ docker-compose.yml .env.example }   # central, we host
│   ├── vet/     { web/ api/ docker-compose.yml .env.example }   # self-hosted
│   └── groomer/ { web/ api/ docker-compose.yml .env.example }   # self-hosted
├── circuits/                    # consent.circom (circom 2.x) + Groth16 trusted-setup + snarkjs-generated consent verifier
├── contracts/                   # Foundry (src/ script/ test/ foundry.toml)
├── crates/dogtag-standard-rs/   # Rust SDK (canonicalize, merkle, verify, custody, consent) + UniFFI
├── crates/dogtag-prover-rs/     # Groth16 proving service (ark-circom + ark-groth16; integrated witness-gen)
├── packages/
│   ├── dogtag-standard-ts/      # TS SDK (mirror of the Rust SDK)
│   └── ui/                      # shared React components + theme tokens
├── docs/  { architecture.md implementation.md research/ }
└── references/
```

Workspace tooling: **pnpm** workspace (TS packages + web apps), **Cargo** workspace (Rust crate + 3 API binaries can share it), **Foundry** for contracts. Root `Makefile`/`justfile` with `dev`, `build`, `test`, `deploy-contracts`, `up:<stack>`.

> **Hash unification (CHANGESPEC-v4 §0).** The credential commitment (leaf hash + Merkle + the
> consent nullifier) is a **single Poseidon root `R`** - `circuits/` and the SDKs use the **pinned
> circomlib BN254 Poseidon** (one parameter set, pinned libs, CI anchor vector - §11.2). keccak is
> retained ONLY where the EVM/ECDSA standards mandate it (ECDSA/tx signing at the EVM boundary, address
> derivation, and the `recordType`/`VERIFY:`/clone-`salt` namespacing keys - §7-keep-list); a keccak
> label that must enter the circuit (`purpose`, `recordType`) is reduced `mod r` at the field boundary.
> Everything that enters the Groth16 circuit or is part of the credential commitment is Poseidon.

---

## 1. Shared standard SDK (`dogtag-standard-ts` & `dogtag-standard-rs`)

The two SDKs are **byte-for-byte equivalent**. Spec is normative; both have a shared test-vector file (`testvectors.json`) asserted in CI.

### 1.1 Canonical value encoding

```
fn encodeValue(typeTag, value) -> bytes:
    match typeTag:
      0 NULL    -> []                                  // empty
      1 BOOL    -> [0x00] if !value else [0x01]
      2 STRING  -> utf8(NFC_normalize(value))
      3 INTEGER -> ascii(decimalString(value))         // big-int; no leading zeros; no "-0"
      4 DECIMAL -> ascii(canonicalDecimal(value))      // fixed-point string; no trailing zeros; single form
      5 BYTES   -> value                               // raw

fn assertNotFloat(value): if value is f32/f64 -> ERROR("floats forbidden; use INTEGER or DECIMAL string")
```

### 1.2 Leaf hashing — Poseidon  (architecture §3.3; CHANGESPEC-v4 §1)

> **Poseidon over the same canonical bytes.** `encodeValue` (§1.1) is **REUSED VERBATIM** — only the
> final hash changes from keccak to the pinned circomlib BN254 Poseidon (§11.2). Poseidon hashes BN254
> field elements (< 254 bits), so each byte-string component is first reduced to one field by `fieldOf`
> (length-prefixed 31-byte limbs, domain-separated Poseidon fold), giving a fixed-arity leaf call.

```
const DS_LEAF=1; const DS_NODE=2; const DS_BYTES=3; const DS_NULLIFIER=4   // domain tags (CHANGESPEC §1)

fn u64be(n) -> 8 bytes big-endian
fn fieldFromLimb(limb: bytes<=31) -> field: be_decode(limb)        // < 2^248 < p, no modular reduction

fn bytesToField(x: bytes) -> field:                                // injective, length-bound, multi-limb
    b     = u64be(len(x)) ++ x                                      // 8-byte big-endian length prefix
    limbs = split b into 31-byte big-endian limbs                   // last limb right-zero-padded to 31
    acc   = DS_BYTES
    for L in limbs: acc = Poseidon(acc, fieldFromLimb(L))           // DS_BYTES domain-separated fold (arity 2)
    return acc

fn fieldOf(scalar uint) -> field: scalar reduced into [0,p)        // 15-digit chip, timestamps, typeTag, uint160 addresses all fit one field
fn fieldOf(bytes x)     -> field: bytesToField(x)
fn fieldOfKeyPath(kp)   -> field: bytesToField(utf8(NFC_normalize(kp)))
fn fieldOfValue(tag,v)  -> field: bytesToField(encodeValue(tag, v))   // encodeValue == §1.1, UNCHANGED

fn hashLeaf(keyPath: string, salt: bytes16, typeTag: u8, value) -> field:
    assert len(salt) == 16
    return Poseidon(DS_LEAF, fieldOfKeyPath(keyPath), fieldOf(salt), fieldOf(typeTag), fieldOfValue(typeTag, value))
    // arity-5 (circomlib t=6). Serialized as bytes32 big-endian (always < p < 2^254). (§11.2)
```

### 1.3 Merkle tree + inclusion proofs — Poseidon  (architecture §3.4; CHANGESPEC-v4 §1; DSDP plan §2.3)

```
fn cmpField(a, b) -> bool: a <= b                          // integer compare in [0, p) (canonical)
fn hashNode(a: field, b: field) -> field:                  // commutative: sort the pair
    (lo, hi) = cmpField(a,b) ? (a,b) : (b,a)
    return Poseidon(DS_NODE, min(a,b)=lo, max(a,b)=hi)      // arity-3 (t=3); DS_NODE prevents leaf/node confusion

fn buildMerkle(leafHashes: field[]) -> { root, layers }:
    if leafHashes.empty: ERROR
    level = sort_ascending_by_integer_value(leafHashes)     // canonical leaf order; salts make leaves unique
    layers = [level]
    while len(level) > 1:
        next = []
        i = 0
        while i < len(level):
            if i+1 < len(level): next.push(hashNode(level[i], level[i+1])); i += 2
            else:                next.push(level[i]);            i += 1   // promote odd, no duplicate
        level = next; layers.push(level)
    return { root: level[0], layers }                       // single leaf -> root == that leaf

// Inclusion proof = an ORDERED, root-ward list of Sibling(field) | Promote steps (one per level).
// Promotion is EXPLICIT (a Promote step), not represented by omission, so the proof depth == the
// tree depth and the shape is reconstructable from the proof alone (DSDP plan §2.3).
fn merkleProof(layers, leafHash) -> step[]:                // step = Sibling(field) | Promote
    idx = indexOf(layers[0], leafHash); proof = []
    for L in 0 .. len(layers)-2:
        sib = (idx ^ 1)
        proof.push(sib < len(layers[L]) ? Sibling(layers[L][sib]) : Promote)  // lone odd node -> explicit Promote
        idx = idx >> 1
    return proof

fn processProof(steps, leafHash) -> field:                 // FOLD PRIMITIVE — NOT a membership check (C1/E2)
    h = leafHash                                           // trusts leafHash: an internal node folds to R just as happily
    for s in steps: if s is Sibling(x): h = hashNode(h, x)  // Promote: pass-through, no hashing
    return h

fn verifyInclusion(keyPath, salt, tag, value, steps, R) -> bool:   // NORMATIVE disclosed-leaf check (DSDP §2.3)
    leaf = hashLeaf(keyPath, salt, tag, value)             // RECOMPUTE under DS_LEAF — never trust a caller-supplied hash
    return processProof(steps, leaf) == R                  // sound via leaf=Poseidon5 vs node=Poseidon3 arity/domain split
```
> The in-circuit ordered tree applies the **same** `sortPair`+`DS_NODE` (via comparator+mux over the
> SDK's sorted leaf order) so the proven root == the SDK's `R` bit-for-bit (§11.8(d)). One tree
> definition; the circuit just proves it.

### 1.4 Wrap a credential

```
fn wrapDocument(credential: VC, issuerMeta) -> WrappedDoc:
    validateSchema(credential)                            // §1.6 invariants
    flat = flatten(credential)                            // [(keyPath, jsType, rawValue)]
    data = {}; leaves = []
    for (keyPath, jsType, rawValue) in flat:
        assertNotFloat(rawValue)
        typeTag = mapType(jsType, rawValue)
        salt    = random16()
        data[keyPath] = hex(salt) + ":" + typeTag + ":" + asString(rawValue)   // self-describing
        leaves.push(hashLeaf(keyPath, salt, typeTag, rawValue))                 // Poseidon leaf (§1.2)
    { root: R, _ } = buildMerkle(leaves)                                        // single Poseidon root R (§1.3)
    return WrappedDoc {
      version: "dogtag/1.0",
      data: unflatten(data),
      signature: { type:"DogTagMerkleProof", targetHash: R, proof: [], merkleRoot: R },   // R serialized as bytes32 BE
      privacy: { obfuscated: [] },
      issuer: issuerMeta         // {name, domain, documentStore, recordType}
    }
```
> **Single root `R` (CHANGESPEC-v4 §0/§2).** There is **one** Poseidon root per tree - the value the SDK
> computes and the value `DogTagIssuer.issue(R)` anchors; for the tag's tree it is also the root the
> Groth16 consent circuit proves inclusion against (§11.8). The parallel `hashLeafZk`/`poseidonMerkle`/`rZk`
> machinery and the keccak `rKec` credential root are **removed** - `hashLeaf`/`buildMerkle` (§1.2/§1.3)
> are now Poseidon and are the *only* tree.
> `testvectors.json` asserts `R` across TS/Rust/circom (§9). keccak survives only for the
> §7-keep-list uses (ECDSA/addresses/namespacing), never for the credential commitment.

### 1.4a `dogTagId` encoding — operator handle vs. on-chain id (the field-hash)

A `dogTagId` has **two forms**, and the boundary between them is load-bearing for the SBT, the
consent circuit, and the on-chain binding:

- **HANDLE** (the numeric, operator-facing id): what the vet operator types into the issuance form,
  the value stored as a record's `credentialSubject.dogTagId` **Integer leaf**, and the
  off-chain pet/session key. Just a decimal number.
- **ON-CHAIN id** = `field_of_value(Integer(handle))` — the **leaf value** of that Integer (`leaf.rs::field_of_value` = `bytes_to_field(encode_value(..))`, §1.2). This single field element is the id used **everywhere on-chain and in-circuit**:
  the `DogTagSBTConsent` mint / `profileRoot` key, the consent circuit's public `pub[0]` (`dogTagId`),
  the EdDSA consent message `M`, the Poseidon nullifier, and the
  device's post-mint anchor poll (`profileRoot`).

The reason they must match is the on-chain binding `R == profileRoot(dogTagId)`
(`VerificationRegistryConsent`): `consent_assemble` field-hashes the handle **exactly once** and uses
that identical field for both the circuit's `dogTagId` input and the `build_profile_tree` KDF binding
that produces `R` (§1.10). If the SBT were minted under the raw handle, every later consent proof
would fail `R != profileRoot(pub[0])`. Minting under `field_of_value(handle)` keeps
`profileRoot(dogTagId)` aligned with `pub[0]`.

```
fn onchainDogTagId(handle) -> field:                       // the CANONICAL on-chain id
    return field_of_value(Integer(handle))                 // == the credential's dogTagId leaf value (§1.2)
```

This one transform has a **single implementation reused everywhere**:
- Rust SDK FFI **`dog_tag_id_field_hex(dec)`** (`crates/dogtag-standard-rs/src/ffi.rs`) — UniFFI-exported
  (`dogTagIdFieldHex` on mobile); the `field-hash` bin (`crates/dogtag-standard-rs/src/bin/field-hash.rs`)
  is the CLI mirror.
- Backend helper **`onchain_dog_tag_id(handle)`** (`stacks/vet/api/src/routes.rs`) — used by the
  DOG_PROFILE id-allocation collision check (`profileRoot(field_of_value(handle))` set?) and by the
  async anchor + mint + read-back (§3.11).
- SDK **`consent_assemble`** (`crates/dogtag-standard-rs/src/consent_assemble.rs`) - the one place the
  handle is field-hashed for a consent proof.
- Mobile: the scan/consent flow (`dogTagIdFieldHex(dogTagIdDec)`) and both platforms' anchor reads -
  `RoaxRpc.profileRoot` is queried at the field-hashed id, not the raw handle.

### 1.5 Selective disclosure

```
fn obfuscate(doc, keyPaths[]) -> doc':
    for kp in keyPaths:
        (salt, typeTag, value) = parse(doc.data[kp])
        h = hashLeaf(kp, salt, typeTag, value)
        doc.privacy.obfuscated.push(h)
        delete doc.data[kp]
    return doc                                            // root unchanged (proof in 1.7)
```

### 1.6 Schema validation (finalized fields + coded values — CHANGESPEC §0/§1)

The validator operates on the **finalized v2 field set** (CHANGESPEC §0). VC 2.0 envelope =
arrays for `@context`/`type`, human prose in `description` (never `@context`); identity is
**referenced by `dogTagId` only** — vaccine/service records do NOT copy name/breed/etc.

```
fn validateSchema(c):
    # --- VC 2.0 envelope (CHANGESPEC §0/§1.6) ---
    require isArray(c["@context"]) && c["@context"][0]=="https://www.w3.org/ns/credentials/v2"
                                   && includes(c["@context"], DOGTAG_CONTEXT_URI)
    require isArray(c.type) && includes(c.type, "VerifiableCredential")
    require present: c.id, c.issuer, c.validFrom, c.credentialSubject, c.credentialSchema
    require present: c.credentialStatus            # revocation, first-class; mirrors on-chain isValid
    if c.description present: require isString(c.description)   # prose lives here, NOT in @context
    require c.credentialSubject.dogTagId           # reference identity; do NOT duplicate name/breed

    # --- legal/trust meta (every credential, CHANGESPEC §0) ---
    require c.attestationType
    require c.signatureTrustTier in {accredited_authority, licensed_vet, self_attested}
    require c.legalEffect == "evidentiary"
    require present: c.legalBasisVersion, c.jurisdiction

    # --- microchip = OBJECT, never float/bare number (CHANGESPEC §0/§1.2) ---
    needsChip = includes(c.type,"RabiesVaccinationCertificate")
             || c.recordType in {EU_HEALTH_CERT} || c.cdcPath=="standard"
    if c.credentialSubject.microchip present || needsChip:
        m = c.credentialSubject.microchip; require isObject(m)
        require regex(m.code, /^[0-9]{15}$/) && len(m.code)==15
        require m.standard in {"ISO_11784_11785","OTHER"}
        require present: m.implantDate           # MANDATORY (EU/VEHCS: vaccinationDate >= implantDate)

    # --- DOG_PROFILE: normalized identity (CHANGESPEC §0/§1.8) ---
    if c.recordType==DOG_PROFILE:
        require present: c.credentialSubject.species          # top-level
        require c.credentialSubject.breedVbo                  # VBO id, e.g. VBO:0200798
        require c.credentialSubject.breedLabel                # coded + human label
        require c.credentialSubject.sex in {male, female}     # sex != neuterStatus
        require c.credentialSubject.neuterStatus in {intact, neutered, spayed}
        require c.credentialSubject.dateOfBirth               # derive age; no free-text age
        for w in c.credentialSubject.weightHistory:           # unit-bearing + dated
            require w.unit in {"kg","lb"} && isDecimalString(w.value) && present(w.measuredOn)
        # --- ownerIdentity: the human behind the device (VET-entered at issue, §3.11) ---
        # NOTE: applies to a wrapDocument-built DOG_PROFILE VC. The owner-hidden TAG tree itself is
        # built on-device by build_profile_tree (§1.9) and is NOT schema-validated by the issuer, who
        # only ever sees the opaque root R; ownerIdentity stays off-chain with the issuing vet (§3.11).
        require isObject(c.credentialSubject.ownerIdentity)
        require present: c.credentialSubject.ownerIdentity.countryOfIdentification   # ISO country (e.g. GB)
        require present: c.credentialSubject.ownerIdentity.identification            # gov ID / passport number
        require present: c.credentialSubject.ownerIdentity.name                      # official name AS ON the ID
        # NO ownerAddress field: the owner's wallet enters the TAG tree only as the hidden
        # owner.address leaf (§1.9), never as a cleartext credential field
        # photoHashes[] are hashes of off-chain blobs only

    # --- VACCINATION: coded vaccine + nextDueDate (CHANGESPEC §0/§1.3-1.4) ---
    if includes(c.type,"RabiesVaccinationCertificate"):
        require present: vaccineProductCode,        # USDA APHIS Vet Biologics PCN
                         vaccineProductName, vaccineManufacturer, batchLotNumber,
                         vaccinationDate, validFrom, validUntil, nextDueDate, authorizedVet
        require c.series in {primary, booster}
        require c.credentialSubject.microchip.implantDate <= vaccinationDate
        require ageWeeksAt(vaccinationDate) >= 12
        if c.series=="primary": require validFrom == vaccinationDate + 21d
        if c.titer present: require c.titer.resultIUml >= 0.5         # titer{labId,sampledAt,resultIUml}
                         && c.titer.sampledAt >= vaccinationDate + 30d

    # --- SERVICE_ATTESTATION: trust-tiered, NOT a boolean; Art.9, OFF-CHAIN (CHANGESPEC §0/§1.5) ---
    if c.recordType==SERVICE_ATTESTATION:
        require c.assistanceType in {service_dog, emotional_support, none}
        require c.issuerTrustTier in {adi_accredited, licensed_pro,
                                      handler_self_attestation, unverified_registry}
        require present: c.taskDescription
        for ctx in c.legalContext: require ctx in {ADA, ACAA, FHA}
        require c.storage=="off_chain"   # special-category; NEVER hashed on-chain

    # --- jurisdiction-specific (unchanged from §11.5) ---
    if c.recordType==EU_HEALTH_CERT:
        require validUntilEntry == validFrom + 10d && onwardValid <= entry + 4mo
    if c.recordType==CDC_IMPORT_FORM: require ageMonthsAtEntry >= 6; keep OFF-CHAIN
    if includes(c.type,"DOT"): c.trustLevel = SELF_ATTESTED   # handler attestation, not vet
```

### 1.7 Verify — ⚠️ SUPERSEDED by §11.3 (do not code this version)

> **SUPERSEDED.** This early sketch made `ownership` a universally-required pillar, which breaks
> third-party/import verification. **Code §11.3** instead: three authenticity pillars gate validity;
> `ownership` is a **contextual** fragment (gates only owner self-import; `NOT_APPLICABLE` for third
> parties); fragments are 4-state `VALID|INVALID|ERROR|NOT_APPLICABLE`.

```
async fn verify(doc, {rpc, dnsResolver, userWalletAddress}) -> Verdict:
    # pillar 1: integrity (offline)
    leaves = []
    for (kp, packed) in flatten(doc.data):
        (salt, typeTag, value) = parse(packed)
        leaves.push(hashLeaf(kp, salt, typeTag, value))
    leaves = leaves ++ doc.privacy.obfuscated
    { root } = buildMerkle(leaves)
    integrity = (root == doc.signature.targetHash)
             && (processProof(doc.signature.proof, doc.signature.targetHash) == doc.signature.merkleRoot)

    # pillar 2: issuance status (on-chain read)
    issuance = await rpc.call(doc.issuer.documentStore, "isValid(bytes32)", doc.signature.merkleRoot)

    # pillar 3: identity (DNS-TXT over DoH)
    txts = await dnsResolver.txt(doc.issuer.domain)
    identity = any(t matches "dogtag net=ethereum chainId=135 addr=<documentStore>" for t in txts)

    # pillar 4: ownership (on-chain read) — the SBT owner is the address the user controls
    onchainOwner = await rpc.call(DOGTAG_SBT_ADDR, "ownerOf(uint256)", doc.dogTagId)
    ownership = (onchainOwner == userWalletAddress)

    valid = integrity && issuance && identity && ownership
    return { valid, fragments: { integrity, issuance, identity, ownership } }
```

### 1.8 Rust-only: custody module (`crates/dogtag-standard-rs/src/custody.rs`)

Uses Alloy. (research/04)

```
fn genesis_generate() -> Mnemonic:                     // 24 words, 256-bit OsRng
    Mnemonic::<English>::new_with_count(OsRng, 24)

fn derive_account(seed_phrase, index) -> LocalSigner:
    MnemonicBuilder::<English>::default().phrase(seed_phrase).index(index).build()
    // path defaults to m/44'/60'/0'/0/{index}

fn encrypt_seed(phrase, passphrase) -> bytes:          // age scrypt + ChaCha20-Poly1305
fn decrypt_seed(blob, passphrase) -> Zeroizing<String>

async fn sign_and_send(signer, rpc, to, calldata) -> TxHash:
    provider = ProviderBuilder::new().wallet(signer).connect(rpc)
    tx = TransactionRequest::default().to(to).input(calldata)
        .with_eip1559_or_legacy(provider)              // probe fee data; fall back to gas_price
    provider.send_transaction(tx).await.watch()
```

### 1.9 UniFFI export
The crate exposes `wrap_document`, `verify`, `build_merkle`, `hash_leaf`, `obfuscate` (§1.5 selective
disclosure — surfaced as `obfuscateDocumentJson`, so the holder can redact leaves locally and produce
a PII-free presentation copy whose root still equals the on-chain `R`), `dogTagIdFieldHex` (§1.4a),
and the value encoders over **UniFFI** so Android (Kotlin) and iOS (Swift) call the *same*
verification code. `custody`/RPC stay server-side only.

The owner-hidden core is device-side by design:
- `profile_tree` - `build_profile_tree` / `derive_owner_secret`, surfaced as `buildProfileTreeHex` /
  `deriveOwnerSecretHex`.
  The owner's app builds the per-tag Merkle tree locally and hands the issuer **only** the root `R` (§3.11).
  The owner-secret (the nullifier's secret leaf) derives from the wallet seed and must never be transmitted, or a server could recompute every nullifier and link it back to the owner.
  See `docs/MOBILE_OWNER_SECRET.md`.
- `consent_assemble` (+ the `prover` feature's on-device `prove_consent`) - assembles the consent
  witness (per-tag consent key, EdDSA over `M`, the three reserved-leaf inclusion paths) as
  decimal-string circuit inputs (§1.10, §3.10).
  The identical assembly feeds the `/prove-consent` server-prove fallback (§3.10b).

> **Retired FFI surface.** The owner-revealing consent module (`verification_consent_typehash`,
> `hash_typed_consent`, `sign_consent_ecdsa`, `sign_consent_eddsa`, `derive_babyjub_consent_key`,
> `signConsentEddsa`, `bindConsentKeyDigestHex` and friends) is deleted with the EIP-712 consent path
> (§1.10); the UniFFI bindings are regenerated without it.

M7 P4 adds the `discovery` module - `validate`, surfaced as **`validateDiscovery`** - the pure client
TRUST gate that checks a platform's `unverifiedClaims` (the resolve-GET convenience tier) against the
dogtag-owned discovery anchor before the app trusts any platform-supplied version/registry (§3.10d). It
lives in this crate rather than `dogtag-prover-rs` precisely because the app links this one and not the
ark-heavy prover; it does string/int/semver compares only. Regenerating **both** committed bindings
(`apps/ios/DogTag/dogtag_standard.swift`, `apps/android/.../dogtag_standard.kt`) is mandatory after any
FFI change - Android CI bundles the committed `.kt` as-is, so a stale binding silently makes the
validator uncallable.

### 1.10 Consent module — `VerificationConsent` EIP-712 typed-data (CHANGESPEC §0/§1; research 11)

> **Retired (owner-revealing path removed; see `consent.circom` / `consent_assemble`).** The EIP-712
> `VerificationConsent` typed-data struct, its typehash/digest (`hash_typed_consent`), the ECDSA
> (`sign_consent_ecdsa`) and subject-bearing EdDSA signing paths, the wallet-scoped
> `derive_babyjub_consent_key`, and the subject-bearing nullifier are all deleted. Consent is no
> longer a signed message a contract recovers; it is a Groth16 proof over the frozen
> `DogTagConsent(6)` circuit.

The LIVE consent crypto (SDK: `crates/dogtag-standard-rs/src/eddsa.rs` + `consent_assemble.rs`,
mirrored on-device via UniFFI):

```
# --- per-tag consent key (NO on-chain registry; the key lives INSIDE the tag's tree) ---
fn deriveBabyjubConsentKeyPerTag(seed, dogTagId) -> BabyJubKeypair
    # BLAKE-512 KDF over (domain "DogTag/consent-key/babyjubjub/v2", dogTagId, seed) -> prv2pub
    # (the one per-tag KDF preimage builder - crates/dogtag-standard-rs/src/kdf.rs - also derives
    #  the owner-secret and the reserved-leaf salts, so the (seed, dogTagId) binding is uniform)
fn keyHash(Ax, Ay) -> field: Poseidon(Ax, Ay)        # == the owner.consentKey LEAF VALUE inside R

# --- the consent signature (EdDSA-BabyJubjub over Poseidon; 5 inputs, NO DS tag) ---
M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)
sig = eddsa_poseidon_sign(M, perTagKey)              # (R8x, R8y, S); verified IN-CIRCUIT, never on-chain

# --- the consent nullifier (computed BY THE CIRCUIT as a public output; relayer-bound, subject-less) ---
nullifier = Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)   # DS_NULLIFIER=4
# ownerSecret is the hidden owner.secret leaf value proven ∈ R, so the nullifier is bound to the
# genuine owner leaf; replaying one signed consent repeats it (rejected on-chain), a fresh nonce
# mints a new one.

# --- public-signal order (frozen with the ceremony VK; ALWAYS index via public_signals::level_b) ---
pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
```
> `dogTagId`, `purpose`, `recordType` enter as field elements: the id is
> `field_of_value(Integer(handle))` (§1.4a); keccak labels are reduced `mod r` once at the field
> boundary. `deadline` is signed inside `M`, so the relayer cannot widen the consent window.
> `recordType` is prover-asserted (it is in neither `M` nor the nullifier): only the owner's app can
> build the proof, so the app - not the relayer - chooses it. There is no `subject`, no `challenge`,
> no EIP-712 domain, and no consent-key bind digest. `R` here is the TAG's tree root - the value
> `mintCustodial(dogTagId, R)` sealed and `issue(R)` anchored - and the registry binds it on-chain via
> `R == profileRoot(dogTagId)` (§11.9).

---

## 2. Smart contracts (`contracts/`)

Solidity ^0.8.24, OZ v5, `evm_version = paris`. (research/03)

### 2.1 `IssuerRegistry.sol`
```solidity
contract IssuerRegistry is AccessControl {
    mapping(address => bool) private _whitelisted;
    event IssuerWhitelisted(address signer); event IssuerDelisted(address signer);
    constructor(address admin){ _grantRole(DEFAULT_ADMIN_ROLE, admin); }
    function whitelistIssuer(address s) external onlyRole(DEFAULT_ADMIN_ROLE){ _whitelisted[s]=true; emit IssuerWhitelisted(s);}    
    function delistIssuer(address s)  external onlyRole(DEFAULT_ADMIN_ROLE){ _whitelisted[s]=false; emit IssuerDelisted(s);}    
    function isWhitelisted(address s) external view returns(bool){ return _whitelisted[s]; }
}
```

### 2.2 `DogTagIssuer.sol` (clone implementation — no constructor) — ⚠️ SUPERSEDED by §11.1

> **⚠️ SUPERSEDED — code §11.1, not this body.** The dual-root binding machinery this section's note
> once described — `zkCommit(rKec, rZk)`, the `ZkCommitment` event, and the `kecOf[rZk] → rKec`
> mapping — is **REMOVED** (CHANGESPEC-v4 §0/§2; resolves audit-07 C-1 / audit-08 C-2). There is **one**
> Poseidon root `R`: the SDK computes it, `issue(R)` anchors it (a plain `bytes32` SSTORE — zero
> on-chain hashing), the Groth16 consent circuit proves inclusion against the tag's exact `R`, and `VerificationRegistryConsent` calls
> `isValid(R)` **directly** on the public root (no `kecOf`/`zkIndex`). Code the **single-root**
> `DogTagIssuer` in **§11.1** (per-recordType `isWhitelistedFor`, `issuedBy`, `_disableInitializers`),
> with clone resolution for `isValid(R)` via the write-once `rootIssuer[R]` index (§11.10(a)). This
> §2.2 body (single-boolean whitelist, no `issuedBy`, uninitialized impl) is retained for diff context
> only — see also the §2.1–§2.4 supersession note at the end of §11.9.

```solidity
contract DogTagIssuer is Initializable {
    IssuerRegistry public registry; bytes32 public recordType; string public name;
    mapping(bytes32=>uint256) public issuedAt; mapping(bytes32=>uint256) public revokedAt;
    event RootIssued(bytes32 root,address by,uint256 ts); event RootRevoked(bytes32 root,address by,uint256 ts);
    modifier onlyWhitelisted(){ require(registry.isWhitelisted(msg.sender),"not whitelisted"); _; }

    function initialize(string calldata n, bytes32 rt, address reg) external initializer {
        name=n; recordType=rt; registry=IssuerRegistry(reg);
    }
    function issue(bytes32 r) public onlyWhitelisted {
        require(issuedAt[r]==0,"issued"); issuedAt[r]=block.timestamp; emit RootIssued(r,msg.sender,block.timestamp);
    }
    function revoke(bytes32 r) public onlyWhitelisted {
        require(issuedAt[r]!=0 && revokedAt[r]==0,"bad"); revokedAt[r]=block.timestamp; emit RootRevoked(r,msg.sender,block.timestamp);
    }
    function bulkIssue(bytes32[] calldata rs)  external onlyWhitelisted { for(uint i;i<rs.length;i++) issue(rs[i]); }   // batch-ready
    function bulkRevoke(bytes32[] calldata rs) external onlyWhitelisted { for(uint i;i<rs.length;i++) revoke(rs[i]); }
    function isIssued(bytes32 r) external view returns(bool){ return issuedAt[r]!=0; }
    function isRevoked(bytes32 r) external view returns(bool){ return revokedAt[r]!=0; }
    function isValid(bytes32 r) external view returns(bool){ return issuedAt[r]!=0 && revokedAt[r]==0; }
}
```
> **Single Poseidon root `R` (CHANGESPEC-v4 §0/§2).** `DogTagIssuer.issue(R)` stores the **one** Poseidon
> root (still just a `bytes32` SSTORE — zero on-chain hashing). The dual-root binding machinery —
> `zkCommit(rKec, rZk)`, the `ZkCommitment` event, and the `kecOf[rZk] → rKec` mapping — is **removed**:
> the Groth16 consent circuit proves inclusion against the same anchored `R`, so `VerificationRegistryConsent` calls
> `isValid(R)` **directly** on the public root (§2.6, §11.8). The corrected `DogTagIssuer` (no `kecOf`,
> no `zkCommit`) is in §11.1; code that, not §2.2.

### 2.3 `DogTagIssuerFactory.sol`
```solidity
contract DogTagIssuerFactory {
    using Clones for address; address public immutable impl; address public immutable registry;
    event IssuerCreated(address clone, bytes32 recordType, string name);
    constructor(address _impl,address _registry){ impl=_impl; registry=_registry; }
    function createIssuer(string calldata name, bytes32 recordType, bytes32 salt) external returns(address c){
        c = impl.cloneDeterministic(salt); DogTagIssuer(c).initialize(name, recordType, registry);
        emit IssuerCreated(c, recordType, name);
    }
    function predictIssuer(bytes32 salt) external view returns(address){ return impl.predictDeterministicAddress(salt, address(this)); }
}
```

### 2.4 `DogTagSBT.sol` (ERC-721 + ERC-5192 soulbound)

> **Retired (owner-revealing path removed; see `DogTagSBTConsent.sol`).** The recipient-taking
> `mint(to, ...)` and the mutable `setProfileRoot` specified here made `ownerOf(dogTagId)` a public,
> permanent pet↔owner link; the contract is deleted. The live SBT is
> `contracts/src/DogTagSBTConsent.sol`: `mintCustodial(id, root)` takes **no recipient** (every tag is
> minted to one neutral, immutable custodian), `profileRoot` is **write-once** with no setter (and a
> burned id can never be re-minted), the soulbound lock is absolute (mint + burn only), and roles are
> `ISSUER_ROLE` / `AUTHORITY_ROLE` / `DEFAULT_ADMIN_ROLE` under `AccessControlDefaultAdminRules`
> (architecture §4.2, impl §11.7(a)).

### 2.6 Verification contracts (CHANGESPEC §0/§2 - live registry behavior in §11.9)

Two contracts for the on-chain proof-of-verification leg. **NOT** EAS (EAS isn't on ROAX and has no
Groth16 path).

- **`Groth16VerifierConsent`** (`contracts/src/Groth16VerifierConsent.sol`) - snarkjs
  `zkey export solidityverifier` output from the frozen consent-ceremony `.zkey`; BN254/alt_bn128;
  `verifyProof(uint[2] a, uint[2][2] b, uint[2] c, uint[7] pub) view returns(bool)` where
  `pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` (§11.9(d); `R` is the single
  Poseidon root). Built from `circuits/consent.circom`. Do not hand-edit.
- **`VerificationRegistryConsent`** (`contracts/src/VerificationRegistryConsent.sol`) -
  `AccessControlDefaultAdminRules`. ONE entrypoint, `recordVerificationZK(a,b,c,pub[7])`: owner-blind
  by construction (no ECDSA path, no subject, no `keyOf`). Verifier capability gated by
  `IssuerRegistry.isWhitelistedFor(keccak256("VERIFY:"||purpose), relayer)` - **separate from issuer
  roles**. Binds `R == profileRoot(dogTagId)`, reads `ownerOf` purely as a token-existence gate
  (return value discarded), consumes the proof-bound nullifier, resolves the issuing clone from the
  write-once `rootIssuer[R]` and re-checks `isValid(R)` directly on the public root, and swaps its
  verifier only through a propose/execute 2-day timelock. Behavior spec in §11.9(e); the contract
  source is authoritative.

> **Retired.** The two-path `VerificationRegistry` (ECDSA `recordVerification` + subject-bearing
> `recordVerificationZK`), the non-consent `Groth16Verifier`, and the `ConsentKeyRegistry` (with its
> gasless `bindConsentKeyFor` EIP-712 bind and `keyOf` linkage) are deleted with the owner-revealing
> path. The consent key lives inside the tag's tree (§1.10), so there is nothing to bind on-chain.

### 2.5 Deploy scripts (`script/Deploy.s.sol` + `script/DeployCustodialIssuance.s.sol` + `script/DeployProtocolRegistry.s.sol`)
```
# script/Deploy.s.sol - the shared base
1. deploy IssuerRegistry(admin)
2. deploy DogTagIssuer impl (uninitialized; constructor _disableInitializers)
3. deploy DogTagIssuerFactory(impl, registry, admin)   // doubles as the write-once rootIssuer[R] index
4. factory.createIssuer(...) per record type / business (issue(R) calls rootIndex.registerRoot(R))
5. write addresses -> deployments/roax.json

# script/DeployCustodialIssuance.s.sol - the owner-hidden stack
6. read CUSTODIAN (no default; require custodian != admin, custodian is an EOA-style neutral sink)
7. deploy Groth16VerifierConsent()                     // snarkjs-generated from the frozen ceremony zkey
8. deploy DogTagSBTConsent(admin, custodian)
9. deploy VerificationRegistryConsent(issuerRegistry, sbtConsent, verifierConsent, rootIndex, admin)
   registry.whitelistFor(keccak("VERIFY:"||purpose), relayer)  per verifier (groomer/vet/airline)
10. write addresses -> deployments/roax.json

# script/DeployProtocolRegistry.s.sol + script/PublishProtocolVersions.s.sol - the discovery anchor
11. deploy ProtocolRegistry; publish the version records (both axes) keyed by the internal
    protocol version key
```

---

## 3. Business backend (vet & groomer) — Rust API

Axum + MongoDB + `dogtag-standard-rs`. Vet and groomer are the **same binary**, separated by the deployment role `BUSINESS_TYPE`: a `groomer` verifies and does not issue, so `public_router` does **not** mount the issuance surfaces (`/credentials/*`, `/records/*`, `/r/{token}`, `/profiles/issue/*`, `/p/{token}`) for it — they do not exist rather than existing-and-refusing. The role fails open: anything that is not the literal `groomer` keeps the full issuing surface. So the issuance sections below (§3.3, §3.4, §3.11) describe the issuing role only; every other section, including the shop CRM of §3.12, is mounted for every role. Port: vet `41874`, groomer `43618`.

### 3.1 Genesis & custody endpoints
```
POST /genesis/start
   if state != UNINITIALIZED: 409
   m = genesis_generate(); STASH_IN_MEMORY(m); state=PENDING_BACKUP
   return { words: m.words(), challengeIndices: random 3 positions }

POST /genesis/confirm { words[challengeIndices] , passphrase }
   verify typed words match stash; signer = derive_account(stash, 0)
   blob = encrypt_seed(stash, passphrase); persist blob + keystore_meta{account0:addr}
   zeroize stash; state=INITIALIZED
   return { address: signer.address }

POST /unlock { passphrase }            // on every boot
   seed = decrypt_seed(blob, passphrase); hold in SecretBox (mlock); cache signers
   return { unlocked:true, accounts:[...] }

POST /accounts { label }               // derive next index from frontend
   n = next_index(); a = derive_account(seed, n).address; store {n,a,label}
   return { index:n, address:a }
```

### 3.2 Whitelist application (relays to central)
```
POST /issuer/apply { accreditationNumber, licenseNumber, accountIndex }
   addr = account(accountIndex); call CENTRAL POST /v1/issuer-applications {addr, accred, license, domain}
   return { applicationId, status:"pending" }
   # admin approves out-of-band -> registry.whitelistIssuer(addr) on-chain
```

### 3.3 Issue a record (the core flow)
```
POST /records { recordType, fields, dogTagId }
   require unlocked && account whitelisted (cache from registry.isWhitelistedFor(recordType, signer))
   vc = buildVC(recordType, fields, dogTagId, issuerMeta)
   doc = wrapDocument(vc, issuerMeta{name,domain,documentStore=issuerAddrFor(recordType),recordType})
   calldata = encode("issue(bytes32)", doc.signature.merkleRoot)
   txHash = sign_and_send(signer, ROAX_RPC, issuerAddr, calldata)
   recordId = uuid(); save records{recordId, recordType, dogTagId, wrappedDoc:doc, root, txHash, status:"issued"}
   return { recordId, root: doc.signature.merkleRoot, txHash }

POST /records/{id}/revoke
   calldata = encode("revoke(bytes32)", record.root); sign_and_send(...); mark revoked
```

> NOTE: `/records` is the legacy single-mode (backend-only) path. v2 issuance flows through the
> dual-signing `prepare`/`confirm` endpoints in §3.8 (canonical version in §11.6). `/records`
> remains as the `mode:"backend"` convenience shortcut.

### 3.8 Dual switchable signing (CHANGESPEC §3 — research 08 A)

Two **mutually-exclusive, switchable** signing modes behind one `SigningStrategy` abstraction.
The **merkle-root / wrapped-document build is ALWAYS server-side** (shared SDK) — identical in
both modes. Only the final "sign + broadcast" step differs.

```
# SigningStrategy interface (mirrors packages/dogtag-standard-ts/src/signing/strategy.ts)
interface SigningStrategy:
    mode: "wallet" | "backend"
    activeSignerAddress() -> address          # MUST be isWhitelistedFor(recordType, signer)
    submit(prepared) -> { recordId, txHash, signerAddress, mode }
    status() -> { connected, detail }

#   WalletStrategy  — wagmi v2 + viem 2 + Reown AppKit (MetaMask + WalletConnect v2).
#                     Browser wallet signs the backend's unsignedTx; user pays PLASMA gas.
#   BackendStrategy — Alloy backend HD custody (§1.8/§3.1) signs+broadcasts; clinic key pays gas.

# --- per-issuer signing-mode setting (persisted SERVER-SIDE so it follows the user) ---
PUT /settings/signing-mode { mode: "wallet" | "backend" }
   require operator session
   persist issuer_settings{ signingMode: mode }     # mutually exclusive radio
   return { signingMode: mode }
GET /settings/signing-mode -> { signingMode }
```

```
# --- PREPARE: build (always server-side) + branch on mode ---
POST /credentials/prepare { recordType, dogTagId, fields }
   require unlocked && operator session
   vc  = buildVC(recordType, fields, dogTagId, issuerMeta)        # identity referenced by dogTagId
   doc = wrapDocument(vc, issuerMeta{...,documentStore=issuerAddrFor(recordType),recordType})
   recordId = uuid(); save records{recordId, ..., wrappedDoc:doc, root:doc.signature.merkleRoot,
                                   status:"prepared"}
   calldata = encode("issue(bytes32)", doc.signature.merkleRoot)
   issuerAddr = issuerAddrFor(recordType)
   if issuer_settings.signingMode == "wallet":
       # return UNSIGNED tx; frontend wallet signs+broadcasts (A1.5)
       return { recordId, merkleRoot, targetHash, proof:[],
                unsignedTx: { to: issuerAddr, data: calldata, value: 0, chainId: 135 } }
   else:  # backend mode: sign + broadcast right here
       signer = activeBackendSigner()
       preflightWhitelist(recordType, signer.address)            # §3.8 below; fail fast
       txHash = sign_and_send(signer, ROAX_RPC, issuerAddr, calldata)
       confirmRecord(recordId, txHash, signer.address)           # same path as wallet confirm
       return { recordId, merkleRoot, txHash, signerAddress: signer.address, mode:"backend" }

# --- CONFIRM: backend RE-VERIFIES on-chain before marking issued (a lying frontend can't fake it) ---
POST /credentials/confirm { recordId, txHash, signer }
   r = records[recordId]; require r.status=="prepared"
   receipt = rpc.getTransactionReceipt(txHash); require receipt.status==success
   # re-verify: RootIssued(root,by,ts) event present AND issuedAt[root] != 0 on the issuer
   ev = findEvent(receipt.logs, issuerAddrFor(r.recordType), "RootIssued")
   require ev.root == r.root && ev.by == signer
   require rpc.call(issuerAddrFor(r.recordType), "issuedAt(bytes32)", r.root) != 0
   r.status = "issued"; r.txHash = txHash
   r.audit = { signingMode: issuer_settings.signingMode, signerAddress: signer }   # audit only
   save r; return { recordId, status:"issued" }
```

```
# --- viem chain-add calldata surfaced to the wallet frontend (A1.4) ---
# wallet_switchEthereumChain { chainId:'0x87' }; on error 4902 fall back to:
wallet_addEthereumChain params = {
    chainId: '0x87',                                   # 135 / PLASMA chain ROAX
    chainName: 'ROAX',
    nativeCurrency: { name:'Plasma', symbol:'PLASMA', decimals:18 },
    rpcUrls: ['https://devrpc.roax.net'],
    blockExplorerUrls: ['https://explorer.roax.net'],
}

# --- multi-address whitelist preflight (CHANGESPEC §3 — A3) ---
# One issuer ENTITY maps to MANY whitelisted signer addresses (wallet EOA + backend address).
# Invariant: the ACTIVE signer must be isWhitelistedFor(recordType, signer).
fn preflightWhitelist(recordType, signer):
    ok = rpc.eth_call(ISSUER_REGISTRY_ADDR, "isWhitelistedFor(bytes32,address)", recordType, signer)
    if !ok: ERROR("address not approved for this recordType yet")   # fail fast — wallet mode = user pays gas

GET  /issuer/signers                      # per-(address x recordType) whitelist matrix for the status UI
POST /issuer/signers { address, mode, recordTypes[] }   # new-address onboarding -> central approval queue
   # central admin calls IssuerRegistry.whitelistFor(recordType, address); poll isWhitelistedFor until live.
   # Switching modes is an onboarding event; delist inactive-mode addresses to avoid stale over-broad whitelist.
```

### 3.9 EXPORT session — on-chain proof-of-verification (`/verify/*`) — CHANGESPEC §3; research 10/11/12

The **groomer's** on-chain attestation leg: the owner **exports** an on-device consent proof to the
groomer (symmetric counterpart of IMPORT, §3.4). **DECOUPLED from `/import/pull`** (§3.5):
`/import/pull` is off-chain operational data; `/verify/*` is the on-chain attestation and imports
**no data at all**. The owner pays no gas; the groomer (relayer) pays PLASMA. **Proving is
ON-DEVICE** - the phone POSTs only `{proof: {a,b,c,pubSignals[7]}}`; the groomer never receives the
witness, the raw record, or any owner identifier.

```
# (1) groomer starts an EXPORT session -> low-density QR carrying {host, one-time token, groomerAddr}
POST /verify/session/start { purpose, recordType }
   require operator session && unlocked
   relayer = activeSignerAddress()                              # groomer's funded wallet
   require isWhitelistedFor(keccak256("VERIFY:"||purpose), relayer)    # else 403
   require valid VERIFICATION_REGISTRY_CONSENT_ADDR             # else 503 - fail closed at START, before
                                                                #   the owner scans/consents/proves
   challenge = random(); sessionId = uuid()
   token = hex(16 random bytes)                                 # one-time token (NOT a JWT); 600s TTL
   save verify_sessions{ sessionId, token, relayer, purpose, recordType, challenge, status:"pending" }
   return { qrUrl: DEPLOYMENT_URL+"/x/"+token+"?a="+relayer, sessionId }   # frontend renders QR (§5)

# (1b) phone resolves the export session WITHOUT consuming the token (PEEK; token survives a failure)
GET /x/{token}
   s = verify_sessions[token]; require s.status=="pending"
   claims = { protocolVersion, chainId, verificationRegistry, issuerClone, purpose:s.purpose }   # §3.10d CONVENIENCE tier
   return { sessionId, relayer, purpose, recordType, challenge,
            unverifiedClaims: claims }                          # phone: assert groomerAddr(QR)==relayer,
                                                                #        isWhitelistedFor(VERIFY:purpose, relayer),
                                                                #        DNS-verify groomer (prod/remote; skip local),
                                                                #        validate unverifiedClaims vs the dogtag anchor (§3.10d)

# (2) the owner's phone proves consent ON-DEVICE (consent_assemble + the frozen consent circuit,
#     §1.10) and posts the proof. ONE owner-hidden handler (verify.rs::consent_submit_levelb - the fn
#     name mirrors the internal version key); the same route also accepts an operator-session "cold"
#     submit (no session bound; a fresh owner-blind audit row is minted).
POST /v1/verify/consent { proof, sessionId?, exportToken? }     # proof = {a,b,c,pubSignals[7]} vs consent.circom
   # pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline] - ALWAYS index via
   #   dogtag_standard::public_signals::level_b (named constants; the module name mirrors the internal
   #   version key). A bare literal keys the wrong slot and fails silently downstream - e.g. polling
   #   consumed(R) for a nullifier that is never set.
   # NO consent/sig object, NO bind leg: there is no subject and no consent-key registry (the consent
   #   key lives INSIDE the tree), so there is nothing to bind and no owner slot a caller could fill.
   # recordType + deadline are READ OUT of pub[5]/pub[6] - proof-bound, never relayer-invented (the
   #   relayer cannot widen the device's deadline).
   the gate (operator session OR one-time export token) is a GAS-SPEND gate only. The proof names its own
     submitter (relayer is bound into both the EdDSA message M and the nullifier), so a caller cannot direct
     it anywhere the owner did not already consent to; the gate exists so an open endpoint cannot drain the
     relayer on reverting proofs. The token is PEEKED, never consumed by this handler, so a FAILED verify
     does not burn the owner's QR (retry same QR). A named session that fails to load FAILS CLOSED
     (never demoted to the cold path, which would drop every session guard).
   session binding (phone path, when a session resolved - cold operator calls skip it): before spending
     gas, require pub[purpose]==purpose_key(s.purpose), pub[relayer]==s.relayer, and
     pub[recordType]==purpose_key(s.record_type) - the REDUCED keccak (pub[5] is a circuit output, so < r;
     the raw rt_key may exceed r, the art9 trap). recordType matters most: it is prover-asserted, so nothing
     else pins it. The export token is a capability to spend the relayer's gas, so it must not fund a
     submission unrelated to the session its QR named.
   preflight (mirrors the registry's requires; NOT the security boundary - the on-chain gates re-run):
     every pub[i] < r; pub[relayer] < 2^160 checked on the FULL element BEFORE narrowing (audit L1);
     pub[relayer] == custody active address; deadline > now + 120s (MIN_DEADLINE_MARGIN_SECS - a device
       inside that window must RE-PROVE with a further deadline; the relayer has no way to widen it);
     pub[recordType] != SERVICE_ATTESTATION_FIELD (art9); isWhitelistedFor(VERIFY:pub[purpose], relayer)
       - checked UNCONDITIONALLY, deliberately stricter than the registry's toggleable
       restrictToWhitelistedRelayers, so a mis-set on-chain flag can never turn this into an open relay.
   drive a VerifySession audit row to status:"recording" - SESSION-SCOPED (phone): drive the session's
     EXISTING row (keeps its human-readable purpose/recordType labels, already proved to reduce to
     pub[1]/pub[5]) rather than minting a second, so the trail holds ONE row per verification and the phone
     polls the sessionId its QR named. COLD (operator): MINT a fresh row (purpose/recordType stored as the
     bytes32 WORDS they arrive as - cold, only the words exist). Replay protection: the proof-bound
     nullifier (consumed on-chain) for the cold path, and the SESSION STATUS GUARD for the phone path (the
     row leaves "pending", so a second submit against the settled session is refused - this handler never
     consumes the token; sound because a session OUTLIVES its 600s token). The trail (GET /verify/history)
     is owner-blind by construction: VerifySession has no subject field and the proof has no signal that
     could fill one.
   RESPOND IMMEDIATELY { status:"recording", protocolVersion, sessionId, registry, nullifier }
   THEN async (detached - Axum cancels a handler's future on client disconnect, which would strand the
     row at "recording" while the tx mines anyway): recordVerificationZK(a,b,c,pub) against
     VerificationRegistryConsent, broadcast from custody::ACTIVE_SIGNER_INDEX (the SAME signer the
     preflight validated, never cfg.vet_signer_index) → row to "recorded"+txHash, or "error" with the
     on-chain revert reason in tx_hash. BOTH arms terminal; never left at "recording". The phone polls
     GET /verify/session/{id} and the chain consumed(nullifier).
```

### 3.10 Prover integration (`dogtag-prover-rs`) - server-side prover; CHANGESPEC §0/§3; research 10

In production the **phone** generates the Groth16 consent proof on-device (the SDK's
`consent_assemble` + the `prover`-feature FFI). The `dogtag-prover-rs` crate is the server-side
prover behind the `/prove-consent` fallback (§3.10b) and the test-oracle re-prove for the e2e
scripts. It serves ONLY the consent version: `resolve(None)` returns the consent artifact set (the
sole registered entry), and an unknown version fails closed.

> **On-device witness calc = the `circom-witnesscalc` GRAPH** (`crates/dogtag-standard-rs/src/prover_ffi.rs`,
> behind the `prover` feature). 64-bit devices (arm64 Android, iOS) prove **on-device**. The graph
> calculator (`WitnessFn::CircomWitnessCalc`, asset `consent.graph`) **replaced** the `rust-witness`
> wasm2c path, which miscompiled the circuit's i64 BN254 field arithmetic on 32-bit ARM (armv7) —
> zeroing the last-computed output wires. The graph interpreter is integer-width-correct on any
> target; but a 32-bit phone still can't produce a *valid* Groth16 with the Arkworks prover, so it
> falls back to the §3.10b server prove.

```
# crates/dogtag-prover-rs — ark-circom + ark-groth16 (pure Rust, integrated witness-gen, no native deps)
load:  resolve the protocol version -> an ArtifactDescriptor (src/artifact.rs); WHICH files to load and
       WHICH hashes to pin them to come from it, not from hard-coded filenames. Unnamed version => the
       consent set (keyed by the internal version key "dogtag-levelb/1" - an internal identifier, and
       the sole registered entry); an unknown one fails closed. Load that version's artifacts ONCE;
       ENFORCE each pinned hash BEFORE the parse - mismatched/corrupt fails closed (M4)
prove(circuitInput):                                           # the §3.10b PRE-ASSEMBLED consent input
   witness = build_witness(circuitInput)                       # nullifier + R are circuit OUTPUTS
   proof   = ark_groth16::prove(zkey, witness)
   return serialize(proof) -> (a:uint[2], b:uint[2][2], c:uint[2],
                               pub:uint[7]=[dogTagId,purpose,relayer,nullifier,R,recordType,deadline])
# rapidsnark = documented escape hatch only if the circuit balloons past a few hundred k constraints.
```

### 3.10a Server proving API (`POST /prove-verification`) — 32-bit-Android fallback (Workstream A)

> **Retired (owner-revealing path removed; see `consent.circom` / `consent_assemble`).** The
> `/prove-verification` route, its `prover_assemble::assemble_circuit_input` server-side assembly, and
> the `verification.circom` artifacts it proved are deleted. The replacement concept is the consent
> server-prove fallback, `POST /prove-consent` (§3.10b): the device assembles the consent input
> locally (the seed never leaves the device) and only the heavy Groth16 prove runs on the trusted
> prover-service; the device then submits the returned proof itself. The backend route exists; the
> mobile wiring lands in a later slice.

### 3.10b Level-B consent proving API (`POST /prove-consent`) - M7 P0

The consent proving route on the prover-service: it proves the frozen `consent.circom`
(`DogTagConsent(6)`). It touches no circuit/VK/ceremony/contract (all frozen) - only a prover code
path. (The section title keeps its original name; "Level-B" here survives only as part of the
internal version key `dogtag-levelb/1`, an internal identifier, not a product label.)

```
POST /prove-consent { circuitInput, version? }        # stacks/vet/api/src/routes.rs (prove_consent)
   # circuitInput = the PRE-ASSEMBLED DogTagConsent(6) input the DEVICE built locally
   #   (dogtag_standard::consent_assemble / the prove_consent FFI): scalars as decimal strings, the
   #   three *Siblings signals as length-6 arrays. The server does NOT assemble - the consent witness
   #   needs the owner's wallet SEED, which must never leave the device, so only the heavy Groth16
   #   prove runs here. TRUST BOUNDARY: the seed stays on the device, but the assembled input DOES
   #   carry ownerSecret + ownerAddress - so owner-unlinkability holds against a chain observer and
   #   the relayer, and NOT against this service's operator (which can name the owner and recompute
   #   the nullifier). See the trust-boundary note on `ConsentProver::prove` in prover.rs.
   # version = OPTIONAL; must be the internal consent version key ("dogtag-levelb/1"), else 400.
   proof = ConsentProver.prove(circuitInput)           # ark-0.6; verifies against the frozen consent VK
   return { a, b, c, pub }                              # pub=[dogTagId,purpose,relayer,nullifier,R,
                                                        #      recordType,deadline] (7, frozen OUTPUT order;
                                                        #      no subject/keyHash)
```

- **Lazy, fail-closed PER REQUEST - not at boot.** The `ConsentProver`
  (`stacks/vet/api/src/prover.rs`) loads on the FIRST `/prove-consent` request (from
  `CIRCUITS_BUILD_DIR`); a missing/hash-mismatched artifact set 503s THAT request rather than failing
  the whole instance's boot (M7 §3.5). A malformed `circuitInput` is a 400 (client error), never a
  5xx the caller would retry.

### 3.10c Signed-manifest discovery API (`GET /protocol/manifest`) - M7 P3

The OFFLINE fallback for the on-chain `ProtocolRegistry` discovery anchor (§5.1 lock B): dogtag serves the
SAME version content - BOTH axes (R-5): the contract set's trio + verifier and the bound artifact set's
fetch-pins, each carrying its own identity (`version_id` / `artifact_set_id`) - as a **dogtag-key-signed JSON** an app can
verify with a pinned dogtag key (no RPC, no server liveness). It is a CACHE/FALLBACK, never a second
authority - on any conflict the on-chain record wins (`dogtag_prover::manifest::reconcile`). A NEW route
that does **not** touch the resolve GET (`/p/`, `/x/`); that resolve-GET extension landed in P4 (§3.10d). Unlike §3.10a/b
it needs **no** prover feature - it lives on the main `public_router`. Full brick: AGENTS.md
"ProtocolRegistry discovery anchor + signed-manifest fallback (M7 P3)".

```
GET /protocol/manifest?version=dogtag-levelb/1        # stacks/vet/api/src/protocol.rs (get_manifest)
   # version = the internal protocol version key (an internal identifier, not a product label);
   #   a QUERY param, not a path segment (it contains `/`)
   key = env DOGTAG_MANIFEST_SIGNING_KEY              # 32-byte ed25519 seed, 64 hex chars
      #   UNSET -> 503 (feature disabled, fail-closed); SET-but-malformed -> 503 (logged, secret NOT logged)
   content = manifest::build(version)                 # unknown version -> None -> 404
      #   DRY from the file-verified dogtag-prover-rs artifact descriptor + the version's on-chain axis
      #   (VersionDeployment) + its bound artifact axis (ArtifactRelease == the activeArtifactSetOf mirror)
   return SignedManifest { content, alg:"ed25519", signature, public_key }   # 200; app verifies vs a PINNED key
```

- **ed25519, offline-verifiable, on-chain-wins.** The content + signature + offline `verify`/`reconcile`
  helpers live in `crates/dogtag-prover-rs/src/manifest.rs`; this route (`protocol.rs`) is only the HTTP
  surface + key loading. `verify` checks against the app's PINNED dogtag key, not the envelope's advertised
  `public_key`, so a wrong-signer or tampered manifest fails; `reconcile` takes the two on-chain axes as
  SEPARATE params (`&OnchainContractSet`, `&OnchainArtifactSet` - they are read separately on-chain, R-5),
  reports every field that disagrees with on-chain and always returns the on-chain value as authoritative
  (the deprecation lever `min_app_version` and `circuit_id` included). The TWO on-chain `active` lifecycle
  bits - one per axis - ride through `reconcile` into `Reconciliation::contract_set`/`::artifact_set`
  UNCOMPARED (the signed manifest carries no lifecycle state, so a
  disagreement is impossible by construction) - that pass-through is what wires `deprecateContractSet`/`deprecateArtifactSet` into the
  §3.10d anti-downgrade check.

### 3.10d Discovery API + app anchor-validation (resolve GET convenience tier) - M7 P4

The client TRUST gate on top of §3.10c: the two NON-consuming resolve GETs (`GET /x/{token}` §3.9, `GET
/p/{token}` §3.11) now ALSO return a nested `unverifiedClaims` block - the CONVENIENCE tier (§5.2), the
platform's own CLAIMS about which protocol version / chain / registry / issuer clone / purpose this flow
belongs to. It is **additive**: every pre-existing top-level field is unchanged, so an older app that
ignores the block keeps working. Built from this deployment's config (`app::convenience_claims`): both
resolve GETs advertise the unified version (the `LEVEL_B_VERSION` constant - the internal version key,
an internal identifier) + this deployment's verification registry. Full brick: AGENTS.md "Discovery API
+ app anchor-validation (M7 P4)".

```
# the block both resolve GETs add (wire key is `unverifiedClaims`, NOT `claims` — it is deliberately
# labelled as unverified, because to a consuming app these are claims, not authority)
"unverifiedClaims": { protocolVersion, chainId, verificationRegistry, issuerClone, purpose }
   #  /x/ (verify)   -> issuerClone = the clone for the session recordType; purpose = VerifySession.purpose
   #  /p/ (issuance) -> issuerClone = PROFILE_DOCUMENT_STORE;  purpose = "DOG_PROFILE" (no verify-purpose
   #                    exists for issuance, so the record type is the namespace the app already knows)

# the app then RESOLVES the dogtag-owned anchor (§3.10c / on-chain ProtocolRegistry.getContractSet +
# getActiveArtifactSet - the two independent version axes, R-5) and gates:
dogtag_standard::discovery::validate(claims, anchor, ClientContext{ app_version, expected_purpose })
   -> Ok(ValidatedVersion{ version, circuit_id,     # feeds artifact selection (dogtag_prover::artifact::resolve)
                           artifact_set })          # the validated ARTIFACT axis (R-5) - returned separately
                                                    # because it rotates independently of `version`
   -> Err(..)                                       # ABORT — never prove against an unvalidated claim
```

- **The validator is PURE and lives in the STANDARD crate** (`crates/dogtag-standard-rs/src/discovery.rs`),
  not the prover crate, because the mobile app links `dogtag-standard-rs` over UniFFI but NOT the ark-heavy
  `dogtag-prover-rs`. It does string/int/semver compares only - no ZK, no chain I/O, no signature check.
  RESOLVING the anchor (the `getContractSet`+`getActiveArtifactSet` eth-calls, or the §3.10c manifest `verify` + `reconcile`) is the
  CALLER's job. The FFI export is `validateDiscovery` (committed Swift + Kotlin bindings regenerated).
- **Fail-closed checks**, each aborting the flow: version-coherence, artifact-axis coherence
  (`ArtifactSetIncoherent` - the anchor's `artifact_set` must hash to its `artifact_set_id`; the platform
  never claims the artifact axis, so this is the same caller-integrity guard applied independently to the
  second axis), the TWO independent lifecycle bits
  `contract_set_active` and `artifact_set_active` (**both** required - deprecating EITHER axis refuses the
  version, the anti-downgrade lever §8.4; the error names which one via `DeprecatedAxis`, rendered into the
  message so it survives the FFI's flattening to a string), `chainId`, `verificationRegistry` (**the anti-redirect trip** - a lying
  platform cannot steer a proof onto an attacker registry), `purpose`, and `minAppVersion`. The two `0x`-hex
  FIELDS (`versionId`, `verificationRegistry`) compare case-insensitively; `minAppVersion` compares
  NUMERICALLY (`1.10.0` > `1.9.0`), tolerating a `-rc1`/`+build` suffix and failing closed on a malformed core.
- **`purpose` is checked against the app's OUT-OF-BAND intent** (`ClientContext::expected_purpose`), not a
  chain field: neither on-chain axis (`ContractSet`/`ArtifactSet`) carries a purpose (purpose is per-verification,
  not per-version), and comparing the platform's claim against the platform's own session would be vacuous.
- **Server-side trust-tier mapping** is `stacks/vet/api/src/discovery.rs` (`anchor_from_manifest` /
  `anchor_from_reconciliation`) - the only place linking both crates. `anchor_from_reconciliation` enforces
  on-chain precedence (it returns the conflicts if the signed manifest disagrees) and sources BOTH lifecycle
  bits from the reconciled ON-CHAIN records, passing them through SEPARATELY (it does not pre-AND them -
  `validate` owns that enforcement, and keeping them distinct is what lets the abort name the axis), so a
  chain-deprecated version fails closed as `DeprecatedVersion`. A native app builds the same pair from
  `getContractSet` + `getActiveArtifactSet` and MUST wire both: collapsing them into one field would
  silently discard whichever kill switch was dropped.
- **Coherence trap:** `convenience_claims` hardcodes the `LEVEL_B_VERSION` constant as
  `protocolVersion` while `verificationRegistry` comes from `VERIFICATION_REGISTRY_CONSENT_ADDR`.
  Pointing that env at a registry that does not belong to the advertised version emits an internally
  incoherent claim pair and every validating client trips `RegistryMismatch` - safe (fail-closed) but
  broken; move both in the same change.

### 3.4 QR / JWT sharing
```
POST /records/{id}/share -> { qrUrl }
   jti = uuid(); jwt = sign_eddsa({ iss:DEPLOYMENT_URL, sub:id, aud:"dogtag-mobile",
                                    scope:"read:record", iat, nbf, exp: now+180s, jti })
   store jti in jwt_jti (TTL=exp)
   return { qrUrl: DEPLOYMENT_URL + "/r?t=" + jwt + "&i=" + id }   // frontend renders QR

GET /records/{id}   Authorization: Bearer <jwt>
   claims = verify_eddsa(jwt, leeway=30s)
   require claims.sub==id && claims.scope=="read:record"
   require consume_jti(claims.jti)   // one-time: SETNX/delete; 401 if already used
   return records[id].wrappedDoc

# Low-density VARIANT (server-side one-time token; preferred for QR scanning):
POST /records/{id}/share -> { qrUrl, recordId }
   token = hex(16 random bytes)                     # 32 hex chars — tiny, low-density QR
   put_share_token(token -> { record_id:id, exp: now+180s })   # one-time
   return { qrUrl: DEPLOYMENT_URL + "/r/" + token, recordId: id }   # NO JWT, NO query string

GET /r/{token}   (unauthenticated, like the record-JWT GET)
   record_id = take_share_token(token)   # atomic remove == ONE-TIME; missing/expired -> 404/410
   return records[record_id].wrappedDoc  # same body as GET /records/{id}
   # SAME one-time-use guarantee as the embedded record-JWT, but a far lower-density QR.
   # The legacy /r?t= JWT path above remains for back-compat.
```

### 3.5 Import FROM user (user→business QR)
```
POST /import/start { kind: "profile" | "vaccination" } -> { scanInstruction }
   # business shows "scan user QR"; user app shows QR carrying a JWT for CENTRAL API
POST /import/pull { userApiBase, userJwt, recordRef }
   require operator session
   doc = GET userApiBase + "/share/" + recordRef  (Bearer userJwt)
   verdict = verify(doc, {rpc:ROAX_RPC, dns, mode:"third-party"})   // business is NOT the owner — §11.3
   require verdict.valid                                            // = 3 authenticity pillars (ownership N/A)
   upsert clients/pets_cache from doc.credentialSubject
   return { imported:true, verdict }
```

### 3.6 Calendar sync (research/05)
```
GET  /calendar/google/connect           -> OAuth consent URL (offline+consent, scope calendar.events)
GET  /calendar/google/callback?code     -> exchange -> store refresh token
POST /calendar/sync                      -> incremental:
    resp = gcal.events.list(syncToken)   // 410 -> wipe map, full resync
    for ev in resp.items:
        if ev.extendedProperties.private.dogtag.owned and etag matches stored: skip   // echo
        elif untagged external: upsert busy-block (read-only)
        else: reconcile mapping
    save nextSyncToken
WATCH renewal cron: every 6 days re-create events.watch channel
```

### 3.7 Appointment replica (business side)
```
PUT  /v1/appointments/{id}        // from central; Idempotency-Key + HMAC verify
    if incoming.rev <= local.rev: 200 (noop)         // apply-if-newer
    upsert replica; mirror to Google (create/update tagged event, store etag+rev)
    return { rev: local.rev }
POST /v1/appointments/{id}/cancel // terminal wins
POST staff action (confirm/decline/complete/no_show):
    bump nothing locally; POST CENTRAL /v1/businesses/{bid}/appointment-events {id,rev,event,occurredAt}
GET  /v1/appointments?updatedSince=  // catch-up pull
```

### 3.11 Dog-tag issuance — the VET mints the DOG_PROFILE SBT (`/profiles/issue/*`)

**Vets issue dog tags**, mirroring import/export: a session + a one-time QR. The device creates its
self-custodial wallet on-device (§6.4), the vet operator enters the `ownerIdentity` + pet fields, and
the device folds its profile tree **locally** and posts only the root `R`. The vet signer - which must
hold `DogTagSBTConsent.ISSUER_ROLE` (granted by the protocol admin; a trust escalation → accredited
vets only) - anchors + mints on-chain. **Gasless for the device** (the vet pays PLASMA). No owner
wallet address ever crosses the wire or reaches the chain.

```
# (1) operator starts an ISSUE session -> low-density QR carrying {vetHost, one-time token}
POST /profiles/issue/session/start { ownerIdentity, ...petFields }   # operator session (demo: prefilled)
   require operator session && whitelistedFor(keccak256("DOG_PROFILE"), vetSigner)
   require hasRole(DogTagSBTConsent.ISSUER_ROLE, vetSigner)  # else the mint below would revert
   require trim(pet.name) != ""                              # else 400 - both mobile parsers refuse a session whose
                                                             #   pet name resolves blank, so it would resolve fine in
                                                             #   (1b) yet always fail on-device, wasting the one-time QR
   dogTagId = allocate(); sessionId = uuid()
   // allocate() skips ids already taken on the SBT - the counter resets on restart and the SBT is
   //   shared across issuers. The taken-marker is the write-once profileRoot[id] (set by mintCustodial
   //   and never cleared, so it SURVIVES a burn - a retired id can never be re-allocated).
   token = hex(16 random bytes)                              # 32 hex chars — one-time, NOT a JWT (reuse share-token store)
   save issue_sessions{ sessionId, token, dogTagId, ownerIdentity, petFields, status:"pending", exp: now+180s }
   return { qrUrl: DEPLOYMENT_URL+"/p/"+token, sessionId, dogTagId }   # frontend renders QR (§5)

# (1b) phone resolves the issue session WITHOUT consuming the token (consume on bind)
GET /p/{token}
   s = issue_sessions[token]                                 # a token that still resolves IMPLIES the pre-bind state -
                                                             #   the bind consumes it atomically before the session can
                                                             #   leave "pending" - so the metadata needs no status gate
   claims = { protocolVersion, chainId, verificationRegistry,       # §3.10d CONVENIENCE tier; issuance has no verify
              issuerClone: profile_document_store, purpose:"DOG_PROFILE" }   #   purpose, so the record type is the namespace
   return { sessionId, dogTagId, status,
            pet: { name,                                     # the session's pet record - the attributes the device
                   profile: { species, breedVbo, breedLabel, #   folds into its device-built profile root R
                              sex, neuterStatus, dateOfBirth,
                              weightHistory: [{unit, value, measuredOn}] },   # weight `value` = decimal STRING, never a float
                   microchip: { code, standard, implantDate, bodyLocation } },
            ownerIdentity: { countryOfIdentification, identification, name },
                                                             # BOTH containers emitted UNCONDITIONALLY: the mobile
                                                             #   parsers (iOS Net.swift resolveDogTagIssue, Android
                                                             #   CentralApi.parseProfileIssueSession) fail closed on an
                                                             #   ABSENT container but tolerate empty fields, so a
                                                             #   no-identity session degrades to empty strings, never a
                                                             #   missing key. ownerIdentity rides BESIDE the pet data
                                                             #   (the later D1 hidden-leaf source; NOT folded into R here)
            unverifiedClaims: claims }                       # phone shows what it's about to receive, and
                                                             #   validates unverifiedClaims vs the dogtag anchor (§3.10d)

# (2) the device folds its tree locally and posts ONLY the root -> vet anchors + mints (ASYNC; below)
POST /profiles/issue/custodial-bind { token, root }          # token-authenticated; NO wallet, NO signature
   // NO signature BY DESIGN: mintCustodial takes no recipient (the tag goes to the contract's immutable
   //   custodian), so there is no wallet to prove control of, and accepting one would hand the server the
   //   very owner link this design removes. The one-time 180s token IS the authorization.
   // `root` is the DEVICE-built profile root R (§1.9 profile_tree), folded from the owner's wallet seed:
   //   the three hidden owner leaves (owner.address / owner.consentKey / owner.secret) + the pet
   //   attributes. The server CANNOT recompute it (it has no seed) and builds NO VC - R is opaque, only
   //   shape-checked as a non-zero 0x + 64 hex word.
   require valid_root_hex(root) && unlocked
   require SBT_CONSENT_ADDR && PROFILE_ISSUER_ADDR well-formed   # BEFORE consuming the token, so a
                                                                 #   half-wired stack never burns the QR
   session_id = take_bind_token(token)                      # CONSUME atomically (2nd call -> 410)
   s = profile_sessions[session_id]; require s.status=="pending"
   onchainId = onchain_dog_tag_id(s.dogTagId)               # = field_of_value(Integer(handle)) — the SAME field
                                                            #   the device folded into R as a KDF binding input;
                                                            #   a raw handle here yields R != profileRoot(id)
   require profileRoot(onchainId) is unset                  # re-checked HERE, causally before the irreversible
                                                            #   issue(R); a sealed id is retired FOREVER -> 409.
                                                            #   An INCONCLUSIVE read fails closed -> 503 (proceeding
                                                            #   is what spends the irreversible write). Narrows but
                                                            #   does not close the cross-instance TOCTOU; the
                                                            #   contract's write-once mint is the real backstop.
   s.root = root; s.protocolVersion = LEVEL_B_VERSION; save s   # stamped with the internal version key
   # RESPOND IMMEDIATELY, mint in the BACKGROUND - the ROAX mint receipt is ~12-24s, far past the
   #   phone's HTTP read timeout:
   spawn:
       DogTagIssuer(PROFILE_ISSUER_ADDR).issue(root)         # (a) ANCHOR FIRST, so rootIssuer[R] resolves
                                                             #     — if this fails nothing is minted and the id is
                                                             #     still free; ordering is prescribed by the contract
       sent = DogTagSBTConsent.mintCustodial(onchainId, root)  # (b) THEN SEAL — irreversible past this point
       # VERIFY BOTH on-chain conditions a later verify checks. NOTE: deliberately NO ownerOf comparison -
       #   the owner is the neutral custodian and comparing it would reintroduce the linkage.
       ok = profileRoot(onchainId)==root && DogTagIssuer.isValid(root)
       s.status = ok ? "bound" : "error"; s.txHash = sent.txHash; save s
   return { dogTagId, onchainDogTagId, root, protocolVersion, status:"minting" }
   // The phone POLLS the chain at the FIELD-HASHED id (§1.4a) until profileRoot(onchainId)==R, then
   //   imports its dog tag (pet appears). The tag's tree + salts live in the device's ProfileTreeStore.

# (3) portal polls the session for the mint result
GET /profiles/issue/session/{id}                            # operator-session gated
   s = profile_sessions[id]
   return { status, dogTagId, root?, txHash?, protocolVersion? }   # status: pending -> bound (or error)
   // There is NO walletAddress field: an owner-hidden issuance never learns one.
```

**`ownerIdentity` stays with the issuing vet, off-chain.** The operator-entered identity block
(`{countryOfIdentification, identification, name}`) lives on the session row: the issuing vet
legitimately holds the identity of the person it issues to (the record-custodian role).
It is NOT committed into `R` today; committing it as a hidden, selectively-disclosable leaf is planned but not implemented - see `docs/DPIA.md` §2.1.
The owner's wallet enters the tree only as the hidden `owner.address` leaf the device folds locally (§1.9); it is never a cleartext credential field and never appears in calldata.

> **ZK note (attribute leaves do NOT change the consent circuit).** The consent proof (§3.9/§11.8)
> proves only the three reserved owner leaves' inclusion in `R` (tree depth 6, up to 64 leaves), so
> adding or changing disclosable pet-attribute leaves never changes the circuit's public signals or
> the proof shape.

### 3.12 Shop CRM — the business's own clients, appointments & verification history

The business's OWN customer book (`stacks/vet/api/src/crm.rs`), operator-session gated and mounted
for **every** role (a booking book is not role-specific; the groomer portal is its first consumer).
These paths are **unversioned** on purpose: `/v1/*` is for cross-service callers (central HMAC, the
owner's phone), unversioned paths are operator-facing — and `/v1/appointments` is already the
Phase-7 central-owned appointment **replica** (§3.7), which is a different entity from this
system-of-record.

```
GET|POST        /clients                # list (q/limit/offset) | create
GET|PUT|DELETE  /clients/{id}           # a client + its embedded pets (each pet may carry a dogTagId)
GET|POST        /appointments           # list (q/clientId/petId/status/from/to/limit/offset) | create
GET|PUT|DELETE  /appointments/{id}      # the single read also returns this appointment's verifications
GET             /verifications          # list (q/clientId/appointmentId/status/purpose/from/to/…)
GET             /verifications/{id}
```

- **Search + paging are the SERVER's job.** Each row carries a denormalized lowercased `searchKey`
  (an unanchored substring scan, narrowed not indexed) alongside indexed `startAt`/`createdAt`;
  `from`/`to` are a half-open `[from, to)` window in **unix seconds**, and `limit` is clamped. The
  browser never pulls a collection to filter it.
- **Verification linkage.** `POST /verify/session/start` (§3.9) takes an optional `appointmentId`;
  supplying it resolves the appointment's client + pet and files the resulting verification against
  both. An id that does not resolve is a **400**, never a silent downgrade to an unlinked
  verification. An ad-hoc (or cold) verification still lands in the history as an unlinked row.
- **Privacy boundary of a verification row.** It holds only the public on-chain facts (purpose,
  recordType, status, txHash, the consumed nullifier, the opaque `dogTagId`) plus the keyPaths the
  owner chose to disclose — never their values, and never the owner's `subject` wallet: though
  derivable from the tx, persisting it would create a client→wallet linkage the protocol withholds
  from a verifier. On an owner-hidden verification the disclosed list is **empty**, and that
  emptiness is the guarantee, not a gap.
- **`GET /x/{token}` is unchanged.** It is unauthenticated (anyone who scans the QR reads it), so the
  appointment/client linkage stays server-side and never enters that response.

---

## 4. Central / admin backend — Rust API (port `39742`)

Powers mobile apps + admin portal. Axum + MongoDB + Alloy (admin signer for whitelisting).

### 4.1 Mobile-user API
```
# --- NO central device registration ---
// There is NO POST /v1/register and NO "Central API URL" on the phone. The device creates its
// self-custodial wallet on-device (§6.4: 24-word seed -> secp256k1 EVM address) and obtains its
// dog tag from a VET via the vet-stack issue session (§3.11) — every host it talks to comes from a
// scanned QR. The device sends the vet ONLY its device-built root R (custodial-bind); no wallet
// address is ever transmitted, to the vet or to central.
POST /v1/auth/...                         // (legacy) signup/login, push token — not used for dog-tag onboarding
GET  /v1/pets                             // owner's pets (cache; the dog tag itself is held by the device)
    // NOTE: the DOG_PROFILE SBT mint happens at the VET, not central — see §3.11
    //   (POST /profiles/issue/session/start + /profiles/issue/custodial-bind). The VET signer
    //   (ISSUER_ROLE) calls DogTagSBTConsent.mintCustodial(dogTagId, root); central never mints the
    //   dog tag and no party ever learns the owner's wallet.
GET  /v1/credentials , POST /v1/credentials/import { wrappedDoc }
    verdict=verify(...); require valid; store reference
POST /v1/share/{credentialId}             // user->business: mint one-time JWT (aud dogtag-business)
GET  /share/{ref}  Bearer<jwt>            // business pulls shared doc

# --- on-chain proof-of-verification: NO central leg ---
# Retired (owner-revealing path removed; see consent.circom / consent_assemble): the central
# `/v1/verify/consent` relay that forwarded a signed `VerificationConsent` (and its `/verify/consent/submit`
# HMAC hop + `/v1/verify/receipts`) is deleted. The phone posts its consent PROOF directly to the
# verifier host's `/v1/verify/consent` (§3.9), authenticated by the one-time export token; central is
# not in the verify loop and never sees a consent. Owner-side consent receipts, where kept, are
# off-chain and deletable (§4.5).
```

### 4.2 Business registry & discovery
```
GET  /v1/businesses?type=
    -> {"businesses":[{businessId,type,name,geo,contact,services,apiBaseUrl,domain,
                       documentStores,hmacKeyId}]}  // non-personal
    // near=<lat>,<lng> & radius=<km> are still ACCEPTED and still filter server-side, and both are
    // DEPRECATED. Do not add a caller. Current-position discovery uses the indexer's separate
    // body-only POST /v1/businesses/nearest contract below, after device-side coarsening and explicit
    // disclosure. Nothing in this repo sends a position through this legacy URL query.
POST /v1/businesses (admin)               // register a deployment + issue HMAC key

GET  indexer /v1/businesses?name=&kind=&kind=&limit=&offset=
    -> source-order page + {total,limit,offset,hasMore}; no caller position
POST indexer /v1/businesses/nearest?name=&kind=&kind=&limit=&offset=
    body {"approximateLat":1.352,"approximateLng":103.820}
    -> nearest-first located page; every row includes server-computed distanceKm
    // approximate position is rounded to 3 decimals ON DEVICE, body-only, no-store, and is neither
    // logged nor persisted by indexer-api. There is no radius/map/place/autocomplete/geocoder query.
```

### 4.3 Issuer whitelisting (admin)
```
POST /v1/issuer-applications              // from business backend §3.2 (status pending)
    // accepts MULTIPLE addresses per issuer entity: {issuerEntityId, addresses[], recordTypes[], ...}
GET  /v1/issuer-applications (admin)
POST /v1/issuer-applications/{id}/approve (admin)         // == approve_application; whitelists ISSUE and VERIFY
    verify accreditation off-chain (usdaNan 6-digit, license{number,jurisdiction,expiry})
    // one issuer ENTITY -> many whitelisted signer addresses (wallet EOA + backend) (CHANGESPEC §3)
    for (address, recordType) in application.addresses x application.recordTypes:
        adminSigner: IssuerRegistry.whitelistFor(recordType, address)            // issuance record-types
    // VERIFIER onboarding via the SAME apply->approve flow (no demo-bootstrap/script cast):
    // the application carries application.verifyPurposes[] (e.g. grooming_intake/boarding_intake/daycare_access);
    // approval whitelists VERIFY:<purpose> per address ON-CHAIN, alongside the issuance record-types.
    for (address, purpose) in application.addresses x application.verifyPurposes:
        adminSigner: IssuerRegistry.whitelistFor(verify_key(purpose), address)   // verify_key per §11.8 (below)
    mark approved; notify business
    // verify_key(purpose) = keccak256(abi.encode("VERIFY:", keccak256(purpose) mod r))  -- purpose reduced
    //   mod BN254 r so the registry stores/nullifies the SAME reduced value (see purpose_key / §11.8(e)).
POST /v1/issuer-applications/{id}/reject (admin)
POST /v1/issuer-applications/{id}/delist (admin)   // delist inactive-mode / rotated addresses
    for (address, recordType): adminSigner: IssuerRegistry.delistFor(recordType, address)
```

### 4.3a Admin portal scope — approve + whitelist ONLY (no device registration, no minting)

The admin/central portal does **NOT** register devices or mint dog tags. There is **no**
`GET/POST /v1/admin/owners`, **no** "Registered devices" / "Mint dog-tag" page, and **no**
`POST /v1/admin/owners/{ownerId}/dogtag`. The admin portal's only on-chain power over onboarding is the
**apply→approve** whitelisting of vet/groomer signer addresses (§4.3): issuer record-types via
`IssuerRegistry.whitelistFor(recordType, addr)` and verifier capability via
`whitelistFor(VERIFY:<purpose>, addr)`.

**The dog tag is issued by the VET** (§3.11 `POST /profiles/issue/session/start` +
`/profiles/issue/custodial-bind`): the vet signer - which must hold `DogTagSBTConsent.ISSUER_ROLE`
(granted once by the protocol admin) - anchors the device-built root (`issue(R)`) and calls
`DogTagSBTConsent.mintCustodial(dogTagId, R)`. The mint is **gasless for the device** (the vet pays).
See §3.11 for the vet-entered `ownerIdentity` (off-chain, session-only) and the device-folded hidden
owner leaves.

> **`ISSUER_ROLE` is a trust escalation.** A holder can mint **any** `dogTagId` (and seal its
> write-once root), so in production the protocol admin grants it **only to accredited vets** (gated by
> accreditation review). In the demo, `scripts/demo-bootstrap.sh` does
> `grantRole(keccak256("ISSUER"), vetSigner)`.

### 4.5 Consent, retention & right-to-erasure (CHANGESPEC §2 — research 07)

> **Nothing personal on-chain — ever.** On-chain = salted commitments (salts off-chain),
> revocation status, non-personal DIDs, timestamps, accreditation refs. **Even a salted hash is
> personal data**, and an *unsalted* hash of a low-entropy microchip number is brute-forceable —
> hence per-field random 16-byte salts (§1.2) are the **privacy mechanism**, not just anti-forgery.

```
# --- per-purpose consent + receipts (lawful basis) ---
POST /v1/consents { purpose, lawfulBasis }      // -> Consent record
    create consents{ id, ownerId, purpose, lawfulBasis, grantedAt }
    receipt = ConsentReceipt{ consentId, hash, issuedAt }   // tamper-evident receipt
    return { consentId, receipt }
GET  /v1/consents                               // list owner consents + receipts
POST /v1/consents/{id}/withdraw                 // withdrawal; stops processing for that purpose

# --- retention metadata on credentials ---
# every credential carries retention{ basis, clock }; a retention sweep purges expired off-chain
# records via the SAME erasure flow below (delete record + destroy salt/key).

# --- CCPA/GDPR delete endpoint (45-day) — wired to the erasure flow ---
POST /v1/privacy/delete-request { ownerId, scope }      // CCPA/GDPR data-subject request
    create deletion{ id, ownerId, scope, dueBy: now + 45d, status:"pending" }
    return { requestId, dueBy }
# fulfilled within 45 days (manual or cron), executing erase():
fn erase(ownerId, scope):
    # ERASURE = delete off-chain record + DESTROY salt/key  -> unlinks the on-chain commitment
    for rec in offchain_records(ownerId, scope):
        destroy_salts(rec)            # per-field 16-byte salts -> commitment becomes unlinkable
        destroy_encryption_keys(rec)  # off-chain blob keys
        delete rec                    # off-chain PII (Owner{...}, photos, service attestations)
    # verification-event scope (CHANGESPEC §4/§5): off-chain consent receipts + audit copies are deletable
    for v in verification_records(ownerId, scope):   # consents/consent_receipts + verification audit copies
        destroy_encryption_keys(v); delete v         # the on-chain Verified(...) tuple+nullifier persists but
                                                     # is owner-blind (no subject field); the residual linkage
                                                     # is pet-scoped (dogTagId <-> relayer <-> time)
    # NB: on-chain verification events carry NO owner, but the dogTagId<->relayer<->time trail is still
    #     pseudonymous personal data -> DPIA MUST be refreshed to cover it (CHANGESPEC §5).
    # NB: the on-chain salted commitment stays but is now UNLINKABLE; this is a documented
    # mitigation, NOT a regulator-blessed safe harbour. A DPIA is MANDATORY (CHANGESPEC §2).
    mark deletion.status = "completed"
```

### 4.4 Appointments — source of truth
```
POST /v1/appointments { businessId, dogTagId, slot }
    biz = businesses[businessId]; create {id, rev:1, state:REQUESTED, ...}
    PUT biz.apiBaseUrl + /v1/appointments/{id}  (Idempotency-Key, HMAC sign)
    return appointment
POST /v1/businesses/{bid}/appointment-events { id, rev, event, occurredAt }  // HMAC verify
    apply state machine (terminal wins; apply-if-newer); bump rev; push notify user
GET  /v1/appointments?updatedSince=       // mobile + business catch-up
GET  /v1/businesses/{bid}/availability?day=  // proxy to biz or cache: workinghours − appts − freebusy − capacity
```

---

## 5. Frontends (React + Vite + TS, Tailwind + shadcn, `packages/ui`)

### 5.0 Light/dark theme + wallet-connect + signing toggle (CHANGESPEC §3/§5)

Shared across vet, groomer, and admin portals (lives in `packages/ui`):

- **Light/dark theme toggle.** `packages/ui` semantic tokens gain **light + dark** palettes; a
  persisted theme toggle in each portal. (Matches the groomer reference aesthetic — dark sidebar /
  light content — but as a real user-switchable light/dark mode.) Portals are light/dark only,
  **not** the mobile app's 7 colorways (§6.3). Components reference semantic tokens only.
- **Wallet-connect UI.** Reown AppKit `<appkit-button />` (wagmi v2 + viem 2): connect MetaMask /
  WalletConnect v2; "Switch to ROAX" using the §3.8 chain-add calldata (`wallet_switchEthereumChain`
  → on 4902 `wallet_addEthereumChain` 0x87/PLASMA).
- **Signing-mode toggle.** A single per-issuer mutually-exclusive radio — *Browser wallet* ⟷
  *Server-managed key* — under **Settings**, persisted server-side via `PUT /settings/signing-mode`
  (§3.8). Helper text: "Browser wallet: you pay PLASMA gas. Server key: the clinic's wallet pays."
- **Status panel.** Wallet mode → connected address + ROAX-chain check + per-recordType whitelist
  badge (`isWhitelistedFor` preflight). Backend mode → genesis state (`INITIALIZED`/`LOCKED`) +
  backend signer address + **PLASMA balance** (gas-funding health). Switching affects only future
  signing; in-flight prepared drafts are re-validated; switching is blocked while a submit is pending.

### 5.1 Vet portal (`stacks/vet/web`, port 41873)
- **Setup wizard**: genesis (show 24 words → confirm challenge → set passphrase), derive accounts, apply for whitelist (enter USDA#/license#), set DNS-TXT instructions for their domain.
- **Issue a record**: pick recordType → form (schema-driven, validates §1.6) → "Sign & Issue" (POST `/records`) → show txHash + "Show QR" (`/records/{id}/share`, render QR).
- **Records list**: backed by the backend's own DB (`GET /records`, operator-gated) — status (issued/revoked/expired), the immutable on-chain proof (tx, block, contract) + explorer link, edit off-chain label/notes (`PATCH /records/{id}` — on-chain-derived fields rejected), mark expired, re-generate QR anytime, revoke (soft — row + proof kept).
- **Import from user**: "Import Profile / Vaccination" → show scan prompt → `/import/pull` (off-chain; **decoupled** from Verify below).
- **Export (on-chain proof-of-verification)** - CHANGESPEC §5: pick purpose → `POST /verify/session/start` → render the one-time **export QR** (`/x/<token>?a=<relayer>`; owner scans, approves consent in-app) → poll session: the owner's phone generates the Groth16 consent proof **on-device** and POSTs `{proof}` (auth via the one-time `exportToken`) → the relayer submits on-chain → show **on-chain verification status** (pending → `Verified` txHash + explorer link). The UI notes "private - no credential data and no owner on chain"; a verification imports no data.
- **Calendar + Appointments**: connect Google, calendar grid, approve/decline/reschedule (mirrors reference groomer UI).

### 5.2 Groomer portal (`stacks/groomer/web`, port 43617)
A grooming business's working application, not a bare verification tool. **A groomer verifies and does not issue**, so there is no "Issue a record" entry and no Records page - and the backend agrees, since `BUSINESS_TYPE=groomer` does not mount the issuance routes at all (§3).
- **Dashboard / Calendar / Appointments / Clients / All verifications** - the shop's own surfaces over the §3.12 CRM: today's bookings, day+week calendar grids, the booking book (client + pet, service, slot, notes, status), the customer directory (owner particulars + their pets, each optionally carrying a `dogTagId`), and the complete verification history - all searched, filtered and paged **server-side**.
- **Verification is started FROM an appointment** (appointment detail → `VerifyFlow` with `appointmentId`), so the session, its result and its evidence are filed against both that visit and its client and stay searchable in **All verifications** (filterable by client, appointment, purpose, status, date). The **ad-hoc** `/verify` page is deliberately kept for walk-ins with no booking: identical machinery, no business context, lands as an unlinked row.
- Import pet **profile** + **vaccination status** via QR (`/import/*`), verify on chain+DNS before accepting.
- **Export (on-chain proof-of-verification)**: the same `@dogtag/ui` `VerifyFlow` as §5.1 - pick purpose, show the export QR, on-chain verification status, with no disclosure-mode choice. A groomer can verify a vet-issued vaccination **without being an issuer** (`VERIFY:` whitelist namespace, distinct from issuer roles). Decoupled from `/import/*`.
- Same genesis/custody setup as §5.1 - the shop still needs its own signer, the relayer that pays gas for the on-chain verification - and it applies for whitelisting with `verifyPurposes`.
- **Groomers / Reports / Marketing** remain placeholders mirroring the reference UI.

### 5.3 Admin portal (`stacks/admin/web`, port 39741)
- Business registry CRUD + table (no in-app map).
- Issuer applications queue → approve (triggers on-chain `whitelistIssuer`) / reject.
- Whitelist viewer (on-chain state), appointment/observability dashboards.

---

## 6. Mobile apps (Android + iOS)

### 6.1 Shared
- **Verification** via `dogtag-standard-rs` UniFFI bindings (`verify`, `wrapDocument`, `obfuscate`) — identical to server.
- **API base**: central API (`https://api.dogtag.io`) for accounts/discovery/booking; per-business URLs come from discovery responses & QR origins.
- **Travel receipt (holder-presented)** — a held `TRAVEL_CLEARANCE` credential renders as the same CDC-modeled receipt the government web portal shows (ROLE_APPS §3.3), built LOCALLY from the stored wrapped doc: `TravelReceiptView.swift` / `TravelReceiptScreen.kt`, reached from the credential detail screen when `group == travel`. Section A/B/C + validity + the public `receiptId` + a derived `effectiveStatus` banner (live `isValid` → REVOKED, lapsed `validUntil` → EXPIRED, else VALID). Selective disclosure is **holder-controlled**: Section-A person PII defaults to withheld, per-field reveal toggles flip it, `dogTagId` stays locked visible, and **Share redacted** runs the `obfuscate` binding (§1.5) over the withheld leaves so the shared copy hides them while still rebuilding to the on-chain root `R`. The receipt shows a **PII-free QR** to the public `<protocol.statusBaseUrl>/r/<receiptId>` status page — the only QR encode retained on-device (a status-page URL leaks nothing). The base comes from the `protocol` block (architecture §3.1) and **never** from `issuer.domain`, which is a `did:web` identity rather than a host; a credential carrying no base has no status page, and the receipt renders no QR and says so.

### 6.2 Screens (from references)
- Onboarding ("Welcome to Dog Tags") → tabs **Verify · Travel · Home · Documents · Profile**.
- Home: pet card + Credentials grouped (Health / Service Dog / Travel Docs).
- Add health/travel record wizards with type pickers (Vaccine/Checkup/Surgery/Lab/Prescription/Dental; CDC/DOT/Other travel).
- **Scan QR** (Verify tab): parse `https://<host>/r?t=&i=` → fetch wrapped doc → `verify()` → import under pet, show 3-pillar verdict.
- **Share** (user→business): show QR (one-time JWT against central).
- **Find vet/groomer**: after the owner taps the current-location action, the app plainly says their
  approximate location is sent to DogTag to find nearby vets/groomers and is not stored. It requests
  coarse location, rounds latitude/longitude to three decimals on-device, then POSTs that approximation
  to the indexer's body-only nearest endpoint with `kind=vet&kind=groomer`, `limit`, and `offset`.
  The service computes distance/order once and returns paged rows carrying `distanceKm`; the device
  preserves that order rather than scanning the whole directory. Provider-name search is a server
  filter using the same owner kind set. The service itself does not hardcode those kinds and can serve
  admin/government to later callers. Results are a list only: no map, Directions handoff, chosen-place
  input, location autocomplete, place hints, or third-party geocoding.

### 6.3 Theming (7 themes)
```
ThemeTokens = { primary, secondary, surface, background, onPrimary, onSurface, success, danger, ... }
themes = { black, white, blue, red, pink, green, yellow }   // each: light + dark palette
```
- **Android**: `ColorScheme` per theme via `MaterialTheme`; `ThemeController` persists choice (DataStore); components use `MaterialTheme.colorScheme.*` only.
- **iOS**: `ThemeManager: ObservableObject` in `@Environment`; `Color.primaryToken` etc. resolve from active theme; persisted in `UserDefaults`.
- Components reference **semantic tokens only** → switching theme recolors everything; layout/components unchanged.

### 6.4 Wallet module (Settings) — self-custodial EVM wallet (CHANGESPEC §4 — research 08 B)

A Telegram-style in-app wallet **under Settings**. The DogTag SBT is **NOT owned by this wallet
on-chain** (custodial mint - §3.11): the wallet seed is instead the derivation root of the tag's
hidden owner leaves - the owner-secret, the reserved-leaf salts, and the per-tag consent key all
derive from `(seed, dogTagId)` - so holding the seed IS holding the pet.
Verification reads `profileRoot`, never an owner.

```
WalletModule (Settings -> Wallet):
  # --- DEFAULT: embedded MPC wallet (no seed-phrase UX for non-crypto owners) ---
  default = EmbeddedMpcWallet              # MetaMask Embedded Wallets (ex-Web3Auth) / Privy
                                           #   real TSS; social/passkey login; provider can't sign alone
  # --- ADVANCED: raw BIP-39 self-custody export ---
  advanced = RawBip39Wallet                # Android web3j 4.12.x / iOS web3swift 3.3.2
                                           #   m/44'/60'/0'/0/0 -> secp256k1 -> EVM address

  # --- storage: encrypt-then-store (HW key encrypts the seed; ciphertext in normal storage) ---
  storeSecret(seed):
      hwKey = SecureEnclave.P256 (iOS, kSecAttrTokenIDSecureEnclave, biometryCurrentSet)
           OR Keystore.AES-GCM (Android, setIsStrongBoxBacked(true), setUserAuthenticationRequired(true))
      ciphertext = hwKey.encrypt(seed)     # Enclave/StrongBox can't store arbitrary secrets directly
      persist ciphertext (Keychain ...ThisDeviceOnly / EncryptedSharedPreferences)
      # decryption is biometric-gated; zeroize plaintext after use; never log the seed

  show: address (+ balance only if funds custody is enabled). v1 PREFERS gas sponsorship / AA so the
        owner never holds PLASMA -> OMIT native send/receive in v1 (see §11.7(f)).
  dappConnect: Reown WalletKit (Android com.reown:walletkit, iOS reown-swift) — OFF by default for
        non-crypto owners; DogTag defines NO dApp-signable protocol message (the retired EIP-712
        Claim/RecoverConsent went with the rebind path).

  # --- recovery / transfer: RE-ISSUE, not rebind (D3) - §11.7(a)/(f) ---
  # There is NO on-chain recover(): a rebind would name the new owner on-chain, which the owner-hidden
  # model removes. Losing or rotating the seed means a fresh custodial issuance under a NEW dogTagId +
  # NEW R (ProfileTreeStore.reissue); referencing credentials do not survive and are re-obtained fresh
  # from their issuers (accept-the-break, 2026-07-16).
```

### 6.5 Import verification — 4 checks (CHANGESPEC §4 — research 08 B)

A record imports under a pet; the tag itself binds to the device by **seed possession**, never by an
on-chain owner read (`ownerOf` is the neutral custodian and carries no owner meaning - §3.11).

```
fn importRecord(doc, {rpc, dnsResolver}):
    # (1) offline integrity: recompute targetHash (single-doc: proof MUST be empty, so it IS R) — no network trust
    require recompute(doc) == doc.signature.targetHash
    require doc.signature.proof.empty && doc.signature.merkleRoot == doc.signature.targetHash  # non-empty proof REJECTED (C1)
    # (2) on-chain anchoring (RPC eth_call)
    require rpc.call(doc.issuer.documentStore, "isValid(bytes32)", doc.signature.merkleRoot)
    # (3) identity: DNS-TXT + central registry cross-check
    require dnsResolver.txtMatches(doc.issuer.domain, doc.issuer.documentStore, chainId=135)
         && registry.knows(doc.issuer.domain, doc.issuer.documentStore)
    # (4) tag binding (the owner's own DOG-TAG import only): the sealed on-chain root equals the tree
    #     this device folded from its own wallet seed (poll at the FIELD-HASHED id, §1.4a)
    require rpc.call(DOGTAG_SBT_ADDR, "profileRoot(uint256)", onchainDogTagIdOf(doc)) == R_local
    #     "is this mine" is DEVICE-LOCAL (the hidden owner leaves fold from this device's seed);
    #     ownerOf MUST NOT be compared - it would reintroduce the owner link (§11.3 note).
    # 3 authenticity pillars + tag binding -> import as MINE.
    # (Third-party/business import drops check 4 and uses mode:"third-party" — §3.5.)
```

### 6.6 Consent signing for on-chain proof-of-verification (CHANGESPEC §6; research 10/11)

When a verifier (groomer/vet/airline) records an on-chain proof-of-verification, the owner approves a
consent in-app and the phone proves it in zero knowledge (§1.10). Owner pays **no gas**; the verifier
submits + pays PLASMA.

```
# --- keys on the device ---
secp256k1Key  = wallet key (§6.4)                              # EVM wallet; NOT used for consent
babyJubKey    = deriveBabyjubConsentKeyPerTag(seed, dogTagId)  # per-tag consent key (§1.10); lives INSIDE
                                                               #   the tree as the owner.consentKey leaf -
                                                               #   there is NO on-chain bind step

# --- per-verification: scan the verifier's QR -> validate -> review -> prove on-device -> submit ---
fn approveVerification(scan):
    s = GET https://<host>/x/<token>       # {sessionId, relayer, purpose, recordType, challenge, unverifiedClaims}
    # M7 P4 (§3.10d): resolve the dogtag-owned anchor (ProtocolRegistry.getContractSet +
    # getActiveArtifactSet / signed manifest) and gate BEFORE proving; a failure ABORTS (never fall
    # back to the platform's claim).
    v = validateDiscovery(s.unverifiedClaims, anchor,
                          {appVersion, expectedPurpose: <the app/user's OWN out-of-band intent for this scan>})
    #   ^ expectedPurpose must NOT be s.purpose / unverifiedClaims.purpose: both are platform-supplied,
    #     so checking one against the other is vacuous (§3.10d). It is the purpose the owner is
    #     knowingly consenting to, sourced independently of the scanned session.
    assert groomerAddr(QR) == s.relayer
    assert isWhitelistedFor(VERIFY:s.purpose, s.relayer)      # on-chain read
    dns-verify the host (prod/remote; skip local)             # dogtag-verify=<relayer> TXT
    show "Approve {purpose} by {relayer}?"                    # single tap; owner sees pet + verifier + purpose
    input = consentAssemble(seed, dogTagIdHandle, purpose, relayer, recordType,
                            deadline, consentNonce: fresh)    # §1.10; field-hashes the handle ONCE (§1.4a)
    proof = proveConsent(input)                               # on-device Groth16 (§3.10); or the
                                                              #   /prove-consent trusted fallback (§3.10b)
    POST https://<host>/v1/verify/consent { proof, sessionId, exportToken }   # §3.9; then poll the
                                                              #   session + consumed(nullifier) on-chain
```
- Consent assembly + proving reuse the **same UniFFI modules** (§1.9/§1.10) as the server prover, so
  every prover proves byte-identical statements.
- There is **no on-chain key-registration step**: the consent key was committed into the tag's tree at
  issuance. (The retired flow pre-bound a wallet-scoped key through an on-chain consent-key registry;
  that leg is deleted.)

---

## 7. Docker & ports

Each stack = `web` (nginx serving Vite build) + `api` (Rust) + `mongo` (internal). Example `stacks/vet/docker-compose.yml`:
```yaml
services:
  web:   { build: ./web, ports: ["41873:80"], depends_on: [api] }
  api:   { build: ./api, ports: ["41874:8080"], env_file: .env, depends_on: [mongo] }
  mongo: { image: mongo:7, volumes: ["vetdata:/data/db"] }   # NO host port — network-internal
networks: { default: { name: dogtag-vet } }
volumes:  { vetdata: {} }
```
Ports: admin 39741/39742, vet 41873/41874, groomer 43617/43618. `.env.example` per stack:
```
ROAX_RPC=https://devrpc.roax.net
ROAX_CHAIN_ID=135
MONGO_URI=mongodb://mongo:27017/dogtag
DEPLOYMENT_URL=https://vet.example.com
DEPLOYMENT_DOMAIN=vet.example.com
ISSUER_REGISTRY_ADDR=0x...
ISSUER_ADDR_VACCINATION=0x...
JWT_ED25519_PRIVATE=...           # per-deployment, separate from chain keys
KEYSTORE_PATH=/data/seed.age
CENTRAL_API=https://api.dogtag.io
HMAC_SHARED_SECRET=...
GOOGLE_CLIENT_ID=...  GOOGLE_CLIENT_SECRET=...
```

---

## 8. Contract deploy & verify (Foundry → ROAX)

> The deployment ledger is `contracts/deployments/roax.json`; runbook in `docs/DEPLOY.md`. With the
> owner-revealing path retired, the disposable testnet is wiped and redeployed fresh (decision D5) -
> no pre-unification records survive. ROAX requires **legacy gas** (use `--legacy`).

`contracts/foundry.toml`: `evm_version = "paris"`, pinned `solc`. (research/03)
```bash
# liveness pre-check (RPC was 502 at design time)
cast chain-id --rpc-url https://devrpc.roax.net    # expect 135

forge script script/Deploy.s.sol:Deploy --rpc-url https://devrpc.roax.net \
  --chain 135 --private-key $PRIVATE_KEY --broadcast -vvvv --legacy   # ROAX needs legacy gas

forge verify-contract --rpc-url https://devrpc.roax.net \
  --verifier blockscout --verifier-url https://explorer.roax.net/api/ \
  <ADDRESS> src/DogTagIssuer.sol:DogTagIssuer
```

---

## 9. Testing strategy

- **SDK parity**: shared `testvectors.json` (inputs → expected leaf hashes, roots, proofs) asserted in **both** TS and Rust CI → guarantees cross-language determinism. Include Solidity test that recomputes a node hash to confirm on-chain agreement.
- **Poseidon 4-language parity (NORMATIVE — CHANGESPEC-v4 §0/§1/§9)**: a single `poseidon-vectors.json` run through **circom** (witness + tiny test circuit), **poseidon-lite** (TS), **light-poseidon** `new_circom` (Rust), and a deployed **`poseidon-solidity` PoseidonT3..T7** (Foundry); CI asserts **bit-identical** field outputs in all four — any lib failing at its pinned version is rejected at the lockfile/CI gate. Required vectors:
  - **anchor**: `poseidon([1,2]) = 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a` in all four languages.
  - **leaf**: `hashLeaf` per typeTag (null/bool/string-NFC/integer/decimal `22.7`,`0.5`/bytes) + `bytesToField` edges (empty, 1 byte, exactly 31, exactly 32 → 2 limbs, multi-hundred-byte string, NFC-combining = its NFC image); assert `tag 2 "5" != tag 3 5`.
  - **Merkle**: single-leaf (root == leaf), two leaves (commutativity: swap → same `R`), three leaves (odd promotion), selective-disclosure (drop cleartext, keep Field in `obfuscated[]` → same `R`); circom in-circuit recomputed root == SDK `R`.
  - **inclusion** (DSDP §2.3): `merkleProof`/`verifyInclusion` `Sibling | Promote` conformance — every leaf of trees of leaf counts `{1,2,3,5,6,7,13,24,34}` (so multi-level promotion is exercised), a mixed-leaf-type tree, and negatives (tampered value, corrupted sibling, wrong root) that MUST reject; asserted Rust↔TS↔Swift (`sdk_parity`/`ffi_parity`, `sdk.test`, the iOS Verify-tab panel).
  - **nullifier**: a fixed `(ownerSecret,dogTagId,purpose,relayer,consentNonce)` with `purpose`'s keccak label > r (forces the mod-r reduction), asserted identical in **circom output signal == Rust** - the parity gate protecting the `consumed` set (the nullifier is circuit-computed; no on-chain Poseidon remains).
- **Contracts**: Foundry tests - soulbound revert on transfer, whitelist gating (only whitelisted can issue/revoke), issue/revoke/isValid lifecycle, clone init, factory determinism, custodial mint invariants (no recipient, non-zero root, write-once `profileRoot`, burned id never re-mintable).
- **Circuit** (`circuits/consent.circom`): witness/proof round-trip; the five statements (the three reserved owner leaves' inclusion in ONE root `R` under their PINNED keyPath+typeTag constants; consent-key leaf value `== Poseidon(Ax,Ay)`; EdDSA-BabyJubjub consent over `M = Poseidon(dogTagId,purpose,relayer,deadline,consentNonce)`; `nullifier == Poseidon(DS_NULLIFIER,ownerSecret,dogTagId,purpose,relayer,consentNonce)`; `relayer < 2^160`); negative tests (wrong leaf, bad sig, tampered nullifier, keyPath substitution); pin the `.zkey` hash; `snarkjs zkey verify` against the reused `.ptau`.
- **VerificationRegistryConsent** (Foundry): `recordVerificationZK` - Groth16 over `pub[7]`; range-check every signal (+ `relayer < 2^160` on the FULL element); `R == profileRoot(dogTagId)` binding; the `ownerOf` token-existence gate (a burned tag fails closed; the return value is never compared); proof-bound `deadline`/`recordType` (Art. 9 reject); nullifier consume/replay; `VERIFY:` whitelist gating distinct from issuer roles; `relayer==msg.sender` (reject a different submitter); `rootIssuer[R]` resolution + `isValid(R)` revocation; the propose/execute verifier timelock. (No ECDSA path, no `keyOf`, no subject - owner-blind by construction.)
- **Nullifier double-spend**: one signed consent cannot be recorded twice - replaying the proof (or a re-proved copy of the same consent) repeats the nullifier and is rejected by the `consumed` set, while a fresh `consentNonce` mints a new nullifier and passes; Groth16 **proof-malleability** test - a malleated `(a,b,c)` yields the same public-signal nullifier → still blocked.
- **Public-signal range-checks**: `recordVerificationZK` rejects any public signal `>= SNARK_SCALAR_FIELD` (snarkjs #358); nullifier is a **public signal** (`pub[3]`), never derived from proof bytes (snarkjs #383).
- **Backend**: integration tests for genesis state machine, JWT one-time-use, issue→fetch→verify round-trip against a local anvil (chainId 135 fork), calendar echo-loop, appointment rev ordering.
- **E2E**: vet issues → mobile scans → verifies VALID; revoke → mobile re-verify shows issuance INVALID; obfuscate field → still VALID.
- **Mobile**: UniFFI binding tests assert mobile `verify()` == server `verify()` on the same vectors.

---

## 11. Audit remediations (NORMATIVE — corrected code; overrides §1–§9 on conflict)

Resolves the findings in `docs/research/audit-01/02/03`. Use these versions when coding. Cross-ref: `architecture.md §13`.

### 11.1 Corrected contracts

```solidity
// IssuerRegistry: per-record-type scoping + hardened admin (fixes C-2, H-3, M-registry)
contract IssuerRegistry is AccessControlDefaultAdminRules {
    bytes32 public constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");
    // PROFILE_ISSUER_ROLE removed: SBT mint capability is enforced on DogTagSBTConsent (ISSUER_ROLE +
    // originator binding, §11.7), never read here. The early sketch below is superseded by §11.7(a).
    mapping(bytes32 => mapping(address => bool)) private _wl;  // recordType => signer => ok
    event Whitelisted(bytes32 indexed recordType, address indexed signer);
    event Delisted(bytes32 indexed recordType, address indexed signer);
    constructor(address adminMultisig)
        AccessControlDefaultAdminRules(3 days, adminMultisig) {           // two-step + delay
        _grantRole(WHITELIST_ADMIN, adminMultisig);
    }
    function whitelistFor(bytes32 rt, address s) external onlyRole(WHITELIST_ADMIN){ _wl[rt][s]=true; emit Whitelisted(rt,s);}    
    function delistFor(bytes32 rt, address s)    external onlyRole(WHITELIST_ADMIN){ _wl[rt][s]=false; emit Delisted(rt,s);}    
    function isWhitelistedFor(bytes32 rt, address s) external view returns(bool){ return _wl[rt][s]; }
}

// DogTagIssuer clone (fixes C-1, H-1, M-2)
contract DogTagIssuer is Initializable {
    IssuerRegistry public registry; bytes32 public recordType; string public name;
    mapping(bytes32=>uint256) public issuedAt; mapping(bytes32=>uint256) public revokedAt;
    mapping(bytes32=>address) public issuedBy;                            // H-1 originator
    event RootIssued(bytes32 indexed root,address indexed by,uint256 ts);
    event RootRevoked(bytes32 indexed root,address indexed by,uint256 ts);
    constructor(){ _disableInitializers(); }                             // C-1: lock impl
    modifier onlyWhitelisted(){ require(registry.isWhitelistedFor(recordType, msg.sender),"!wl"); _; }
    function initialize(string calldata n, bytes32 rt, address reg) external initializer {
        require(reg!=address(0)); name=n; recordType=rt; registry=IssuerRegistry(reg);
    }
    function issue(bytes32 r) public onlyWhitelisted {
        require(r!=bytes32(0) && issuedAt[r]==0,"bad");
        issuedAt[r]=block.timestamp; issuedBy[r]=msg.sender; emit RootIssued(r,msg.sender,block.timestamp);
    }
    function revoke(bytes32 r) public onlyWhitelisted {
        require(issuedAt[r]!=0 && revokedAt[r]==0,"bad");
        require(msg.sender==issuedBy[r] || registry.hasRole(0x00,msg.sender),"!owner"); // H-1
        revokedAt[r]=block.timestamp; emit RootRevoked(r,msg.sender,block.timestamp);
    }
    function bulkIssue(bytes32[] calldata rs)  external onlyWhitelisted { for(uint i;i<rs.length;i++) issue(rs[i]); }
    function bulkRevoke(bytes32[] calldata rs) external onlyWhitelisted { for(uint i;i<rs.length;i++) revoke(rs[i]); }
    function isValid(bytes32 r) external view returns(bool){ return issuedAt[r]!=0 && revokedAt[r]==0; }
    // adminRevoke(bytes32[]) — protocol admin mass-revoke for compromised signers (delisting is forward-only)
}

// DogTagSBT (fixes C-2 dedicated role, H-2 admin-only burn)
// RETIRED SKETCH - an early draft of the owner-revealing SBT, retained for audit-diff context only.
// The live contract is the custodial DogTagSBTConsent (no recipient, write-once root - §11.7(a)).
contract DogTagSBT is ERC721, IERC5192 {
    IssuerRegistry public registry; mapping(uint256=>bytes32) public profileRoot; error Soulbound();
    constructor(address reg) ERC721("DogTag","DTAG"){ registry=IssuerRegistry(reg); }
    modifier onlyProfileIssuer(){ require(registry.hasRole(registry.PROFILE_ISSUER_ROLE(), msg.sender)); _; }
    function mint(address to,uint256 id,bytes32 root) external onlyProfileIssuer { _safeMint(to,id); profileRoot[id]=root; emit Locked(id);}    
    function setProfileRoot(uint256 id,bytes32 root) external onlyProfileIssuer { profileRoot[id]=root; }
    function burn(uint256 id) external { require(registry.hasRole(0x00,msg.sender),"admin"); _burn(id); emit Burned(id);} // H-2
    function locked(uint256) external pure returns(bool){ return true; }
    function _update(address to,uint256 id,address auth) internal override returns(address){
        address from=_ownerOf(id); if(from!=address(0) && to!=address(0)) revert Soulbound(); return super._update(to,id,auth);
    }
    function supportsInterface(bytes4 i) public view override returns(bool){ return i==0xb45a3c0e || super.supportsInterface(i); }
}

// Factory: permissioned + deterministic salt (fixes M-1)
function createIssuer(string name, bytes32 recordType, address business) external onlyRole(ADMIN) returns(address c){
    bytes32 salt = keccak256(abi.encode(recordType, business));
    c = impl.cloneDeterministic(salt); DogTagIssuer(c).initialize(name, recordType, registry); emit IssuerCreated(c,recordType,name);
}
```

**v2 contract notes (CHANGESPEC §3/§4):**
- The live SBT mint is **custodial**: `DogTagSBTConsent.mintCustodial(dogTagId, root)` - no recipient
  parameter - called by the **vet** signer (which holds `ISSUER_ROLE`) at issue time (§3.11);
  verification reads `profileRoot(dogTagId)`, never an owner. (The `DogTagSBT` sketch above, with
  `PROFILE_ISSUER_ROLE` and `mint(to,...)`, is an early draft of the retired owner-owned contract,
  retained for diff context only - see §11.7(a).)
- The per-`recordType` `isWhitelistedFor(recordType, signer)` above already supports **multi-address
  whitelisting**: **one issuer entity maps to many whitelisted signer addresses** (e.g. a vet's
  MetaMask EOA *and* their backend-derived address), each `whitelistFor`'d per record type. The
  contract has no concept of "the same vet" — the issuer↔signers mapping is an off-chain view.

### 11.2 Corrected canonicalization — Poseidon commitment (fixes A1, A2, A3, F2a, F2b; CHANGESPEC-v4 §1)

The canonical-byte grammar below feeds `encodeValue` (§1.1), which is **REUSED VERBATIM** — only the
final hash over those canonical bytes is Poseidon (§1.2/§1.3), via `fieldOf`/`bytesToField` packing.

```
DECIMAL grammar (operate on the input STRING, never a float):
  valid  = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?$/
  canon  = strip fractional trailing zeros; drop trailing "."; "-0" -> "0"; reject exponent/space/"+"
INTEGER: /^-?(0|[1-9][0-9]*)$/ ; no leading zeros; "-0"->"0"
mapType: types come from the SCHEMA (typed input), NOT typeof. wrapDocument signature becomes
         wrapDocument(typedCredential)  where each scalar is {tag, value:string|bool|null|bytes}
assertNotFloat(v): if v is f32/f64 -> ERROR   // hard guard, always on

NFC: pin Unicode version U in SDK; reject unpaired surrogates; store NFC form in data.
     Solidity NEVER normalizes — issuance stores R; the chain recomputes no leaves.

flatten(obj) -> [(keyPath,...)]  PINNED grammar:
  object key  -> ".key"  (key NFC, reserved chars [.[]] rejected)
  array elem  -> "[i]"   (i base-10, no leading zeros)
  root has no leading "."; empty object/array -> a null-typed leaf at that path
parse(packed): split on FIRST TWO ":" only -> (saltHex, tag, valueRest)  // value may contain ":"
```

**(a) Byte→field packing (`fieldOf`/`bytesToField`).** Poseidon hashes BN254 field elements < 254 bits
(≈31.7 bytes), so each component is reduced to one field by the **length-prefixed, 31-byte-chunked,
domain-separated fold** of §1.2: `bytesToField(x)` prepends `u64be(len(x))`, splits into 31-byte
big-endian limbs (each `< 2^248 < p`, no modular reduction → injective), and folds them with
`acc = Poseidon(acc, limb)` seeded `acc = DS_BYTES`. `salt`(16 B)/`typeTag`(1 B)/addresses(uint160) fit
one field directly. `keyPath` images are circuit constants; only `value` needs the in-circuit chunked
absorb, bounded by the schema's max field length. The leaf is one fixed-arity call
`Poseidon(DS_LEAF, kpField, saltField, tagField, valField)`.

**(b) Pinned circomlib BN254 instantiation (ONE parameter set, all languages).** `x^5` S-box; `R_F=8`;
per-`t` `R_P` from circomlib's table (`t=2→56, t=3→57, t=5→60, t=6→60, t=7→…`); round constants from
circomlib's `poseidon_constants.circom` (Grain LFSR, seed string `"poseidon"`); circomlib per-`t` MDS;
capacity lane 0 with **domain separation as a first input slot** (NOT a capacity IV) to stay on the exact
circomlib API in all four libs. 128-bit security target. Domain tags: **`DS_LEAF=1`, `DS_NODE=2`,
`DS_BYTES=3`, `DS_NULLIFIER=4`** — distinct first-slot constants + distinct arity make leaf/node/byte-fold/
nullifier non-confusable.

**(c) The four pinned libraries (pin versions; CI-gated).**
- **circom** → **circomlib** `Poseidon(nInputs)` (the reference; t∈[2,16]).
- **TS** → **`poseidon-lite`** (`poseidon2`,`poseidon5`,`poseidon6`,`poseidon7`; pure JS, no WASM; pin version).
- **Rust** → **`light-poseidon`** via **`Poseidon::<Fr>::new_circom(n)`** (circom-compatible constructor —
  NOT a generic one, or constants diverge; Veridise-audited; over `ark_bn254::Fr`; build each `Fr` from a
  ≤31-byte limb provably `< p`, never `from_be_bytes_mod_order`).
- **Solidity** → **`poseidon-solidity`** `PoseidonT3`..`PoseidonT7` (gas-optimized circomlib-compatible;
  deterministic-deploy at a fixed address; only the nullifier and any future on-chain Merkle verifier need it
  — issuance computes no on-chain Poseidon).

**(d) CI anchor vector (NORMATIVE — highest-risk item; circomlibjs has historically drifted, so pin + test).**
A single `poseidon-vectors.json`; CI runs the **same inputs** through circom (witness + tiny test circuit),
poseidon-lite, light-poseidon (`new_circom`), and a deployed `PoseidonT*` (Foundry) and asserts
**bit-identical** field outputs against the anchor:
```
poseidon([1, 2]) = 7853200120776062878684798364095072458815029376092732009249414926327459813530
                 = 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a
```
Any library failing this vector at its pinned version is **rejected at the lockfile/CI gate** — no library
is compatible by reputation, only after the vector passes (§9). Full leaf/Merkle/nullifier vector set in §9.

> `microchip.code` is **string (tag 2)**, 15 ASCII digits → one 31-byte limb; `tag 2 "5" != tag 3 5` is a
> required negative vector (§9). `dogTagId`/`purpose` Poseidon inputs MUST be `< p` (reduce `purpose`'s
> keccak label mod p once at the field boundary; allocate `dogTagId < p` and range-check — §11.9(d)).

### 11.3 Corrected verify (CONTEXTUAL ownership — fixes audit-05 V8 / audit-06 §6.1 Critical)

The **three authenticity pillars** (integrity + issuance + identity) define credential validity for **everyone**. `ownership` is a **contextual fourth fragment** that gates *only the owner's own self-import* — for third-party verifiers (groomer importing a customer's record, airline, border officer, another vet) it is `NOT_APPLICABLE` and **must not** force INVALID. Fragments are 4-state: `VALID | INVALID | ERROR | NOT_APPLICABLE` (ERROR = transient RPC failure of an *in-scope* check; NOT_APPLICABLE = out of scope for this verification).

```
async fn verify(doc,{rpc,dns,userWalletAddress?,mode}) -> Verdict:   // mode: "self-import" | "third-party"
   // integrity: ALWAYS rebuild whole tree; never trust processProof alone (C1)
   for h in doc.privacy.obfuscated: require is32bytes(h)            // D1
   require requiredPathsPresent(doc)   // @context[*], type[*], credentialStatus.*, dogTagId, rabies mandatory — NON-obfuscatable (audit-05 V3/V6)
   leaves = [hashLeaf(parse(packed)) for (kp,packed) in flatten(doc.data)]
   require no overlap(leaves, doc.privacy.obfuscated)               // D1
   {root} = buildMerkle(leaves ++ doc.privacy.obfuscated)
   integrity = root==doc.signature.targetHash
            && doc.signature.proof.empty                    // single-doc: proof MUST be empty, so targetHash IS R
            && doc.signature.merkleRoot==doc.signature.targetHash   // a non-empty proof is REJECTED, never folded (C1)
   issuance = try rpc.isValid(doc.issuer.documentStore, doc.signature.merkleRoot, confirmations=5) else ERROR
   identity = dns.txtMatches(doc.issuer.domain, doc.issuer.documentStore, chainId=135)
           && registry.knows(doc.issuer.domain, doc.issuer.documentStore)
   credentialValid = integrity==VALID && issuance==VALID && identity==VALID   // the 3 authenticity pillars

   // ownership: CONTEXTUAL. dogTagId at credentialSubject.dogTagId (audit-05 V10), present+non-obfuscated.
   if mode=="self-import":                       // mobile owner claiming a record as "mine" (§6.5)
       require userWalletAddress present
       ownership = try (rpc.call(DOGTAG_SBT_ADDR,"ownerOf(uint256)",dogTagIdOf(doc))==userWalletAddress ? VALID:INVALID) else ERROR
       valid = credentialValid && ownership==VALID
   else:                                          // third-party (groomer/airline/vet) — ownership informational only
       ownership = userWalletAddress present
                   ? (try (rpc.call(...)==userWalletAddress ? VALID:INVALID) else ERROR)
                   : NOT_APPLICABLE
       valid = credentialValid                    // ownership does NOT gate cross-party validity
   return {valid, fragments:{integrity,issuance,identity,ownership}}
```
> `§1.7` is **SUPERSEDED** by this. `§3.5 /import/pull` (business importing a customer record) MUST call `verify(doc,{rpc,dns,mode:"third-party"})` — never the self-import path — or every legitimate business import is rejected.

> **Custodial-tag note.** Under the custodial SBT (§3.11) `ownerOf` returns the neutral custodian, so `mode:"self-import"` cannot be satisfied for a live tag and the mobile app does not use it: the owner's own dog-tag import gates on `profileRoot(dogTagId) == R` + device-local seed possession instead (§6.5). The fragment stays in the SDK for generality, but comparing `ownerOf` to identify an owner MUST NOT be reintroduced - it is exactly the linkage the owner-hidden model removes.

### 11.4 Corrected auth & endpoints (fixes audit-03 C-1, C-2, H-auth, H-rev)

```
# Custody under /admin, localhost/admin-session only, /unlock rate-limited:
POST /admin/genesis/start | /admin/genesis/confirm | /admin/unlock | /admin/accounts

# Operator session guards all issuance/import/calendar routes (portal login).

# Central user->business share MUST mirror business-side asserts (C-1):
GET /share/{ref}  Bearer<jwt>:
   claims=verify_eddsa(jwt, leeway=30s)
   require claims.sub==ref && claims.aud=="dogtag-business" && claims.scope=="read:record"
   require consume_jti(claims.jti)        # atomic SET NX / unique index
   return credentials[ref].wrappedDoc

# appointment-events ownership binding (C-2):
POST /v1/businesses/{businessId}/appointment-events {appointmentId, lastRev, event, occurredAt}:
   key = hmacKeyFor(businessId); verify_hmac(req, key)               # key resolved BY path businessId
   appt = appointments[appointmentId]; require appt.businessId==businessId   # ownership
   newRev = appt.rev + 1                                             # central is SOLE rev allocator (H-rev)
   apply_state_machine(appt, event, newRev); push_notify(appt.userId)

# jti consume is atomic:
fn consume_jti(jti): db.jti.insertUnique(jti, ttl=exp)  // throws if exists -> 401
```

### 11.5 Corrected schema validator (fixes audit-03 H-schema)

```
fn validateSchema(c):
    m = c.credentialSubject.microchip                                    # OBJECT, not flat (audit-06 §1.5)
    needsChip = c.recordType in {EU_HEALTH_CERT} || c.cdcPath=="standard"
             || c.type includes "RabiesVaccinationCertificate"
    if needsChip || m present:
        require isObject(m) && regex(m.code, /^[0-9]{15}$/) && typeOf(m.code)==STRING  # tag 2, leading zeros preserved (audit-05 V4)
        require m.standard in {"ISO_11784_11785","OTHER"} && present(m.implantDate)
    if c.type includes "RabiesVaccinationCertificate":
        require present: vaccineProductCode, vaccineProductName, vaccineManufacturer, batchLotNumber,
                         vaccinationDate, validFrom, validUntil, nextDueDate, authorizedVet   # +Code +nextDueDate (audit-06 §1.6)
        require m.implantDate <= vaccinationDate
        require ageWeeksAt(vaccinationDate) >= 12
        if c.series=="primary": require validFrom == vaccinationDate + 21d   # booster-aware
        if c.titer is present: require decimalGte(c.titer.resultIUml, "0.5")  # STRING compare, not float (audit-05 V2)
                            && c.titer.sampledAt >= vaccinationDate + 30d
                            && c.titer.sampledAt <= issueDate - 90d
    if c.recordType==EU_HEALTH_CERT:
        require validUntilEntry == validFrom + 10d && onwardValid <= entry + 4mo
        if echinococcus required: require 24h <= treatmentBeforeEntry <= 120h
    if c.recordType==CDC_IMPORT_FORM: require ageMonthsAtEntry >= 6; keep OFF-CHAIN
    if c.type includes "DOT": c.trustLevel = SELF_ATTESTED                   # handler, not vet
```

> The finalized v2 field set (coded vaccine PCN, VBO breed, microchip object with mandatory
> `implantDate`, trust-tiered service attestation, VC 2.0 envelope arrays + `credentialStatus`,
> `nextDueDate`, normalized `sex`/`neuterStatus`/`weightHistory`/`species`, identity by reference)
> is in §1.6. Apply both: §1.6 = full field set, §11.5 = corrected conditional/jurisdiction rules.

### 11.6 Dual-signing prepare/confirm, ownership preflight & erasure (NORMATIVE — CHANGESPEC §3/§4)

```
# --- prepare/confirm: build is ALWAYS server-side; only sign+broadcast differs by mode ---
POST /credentials/prepare { recordType, dogTagId, fields }:
   require unlocked && operator session
   doc = wrapDocument(buildVC(recordType, fields, dogTagId, issuerMeta), issuerMeta)  // identical both modes
   recordId = uuid(); save records{recordId, ..., root:doc.signature.merkleRoot, status:"prepared"}
   calldata = encode("issue(bytes32)", doc.signature.merkleRoot); issuerAddr = issuerAddrFor(recordType)
   if signingMode=="wallet":
      return { recordId, merkleRoot, targetHash, proof:[],
               unsignedTx:{ to:issuerAddr, data:calldata, value:0, chainId:135 } }   // frontend wallet signs
   else:  // backend mode
      signer = activeBackendSigner(); preflightWhitelist(recordType, signer.address)
      txHash = sign_and_send(signer, ROAX_RPC, issuerAddr, calldata)
      return confirm(recordId, txHash, signer.address)   // same path as wallet confirm

POST /credentials/confirm { recordId, txHash }:        // NO client-supplied `signer` (audit-04 V2-C1/L1)
   require operator session                            // audit-06 §2.4
   r = records[recordId]; require r.status=="prepared" && r.confirmedTxHash==null   // idempotency (audit-04 M)
   issuerAddr = issuerAddrFor(r.recordType)            // resolved ONLY from trusted central registry (audit-04 V2-H3)
   tx = rpc.getTransaction(txHash); receipt = rpc.getTransactionReceipt(txHash)
   require receipt.status==success
   // bind the tx to THIS prepared draft (audit-04 V2-C1/M3): exact calldata, target, value, chain
   require tx.to==issuerAddr && tx.input==r.prepared.calldata && tx.value==0 && tx.chainId==135
   signer = tx.from                                    // DERIVE signer from the tx, never the body
   require registry.isWhitelistedFor(r.recordType, signer)        // authorized at confirm time
   ev = findEvent(receipt.logs, where: log.address==issuerAddr && log.transactionHash==txHash, "RootIssued")
   require ev.root==r.root && ev.by==signer            // emitting contract pinned (no spoofed RootIssued)
   // finality: wait N confirmations; reorg-safe (audit-04 V2-H1)
   require rpc.call(issuerAddr,"issuedAt(bytes32)",r.root, confirmations=5) != 0
   r.status="issued"; r.confirmedTxHash=txHash
   r.audit={ signerAddress:signer, signingMode: modeForAddress(signer) }   // mode derived from signer, not live setting (audit-06 §2.2)
   save r; return { recordId, status:"issued" }
   // before N confirmations -> status="confirming"; if tx reorged out -> revert to "prepared", re-submit.

# --- whitelist preflight: ACTIVE signer must be isWhitelistedFor(recordType, signer); fail fast ---
fn preflightWhitelist(recordType, signer):
   if !rpc.eth_call(ISSUER_REGISTRY_ADDR, "isWhitelistedFor(bytes32,address)", recordType, signer):
      ERROR("address not approved for this recordType")   // wallet mode = user pays gas; revert wastes it

# --- right-to-erasure (CCPA/GDPR delete, 45-day) — CRYPTO-SHREDDING (audit-05 V11/V14) ---
# On wrap: per-record DEK; salts + packed `data` stored ENCRYPTED under DEK; DEK wrapped by owner KEK.
# "Destroy the salt" across replicas/oplog/WAL/backups is only tractable as KEY destruction.
POST /v1/privacy/delete-request { ownerId, scope } -> { requestId, dueBy: now+45d, status:"pending" }
fn erase(ownerId, scope):                              // fulfilled within 45 days; overdue -> escalate
   for rec in offchain_records(ownerId, scope):
      destroy_DEK(rec)              // crypto-shred: all ciphertext copies (DB, oplog, backups, importer caches) become undecryptable == salts gone
      delete rec                    // best-effort plaintext/ciphertext delete (Owner PII, photos, Art.9 service attestations, obfuscated[] copies)
   propagate_erasure(ownerId, scope)         // central -> EVERY business backend holding copies (HMAC-signed, like appt sync) — the vet is the GDPR controller (audit-06 §4.3)
   for dogTagId in owner_pets(ownerId):      // drop the live ownerOf<->pet pseudonymous link (audit-05 V13 / audit-06 §4.1)
      DogTagSBTConsent.burn(dogTagId)         // admin GDPR-erasure burn (NOT the status path); the
                                              //   token-existence gate then fails the tag closed at
                                              //   verify, and the id stays retired (write-once root)
   # Residual (DPIA-recorded): 128-bit salt unlinks ANY value once ALL copies destroyed — copy-proliferation
   # (holder device, unreachable third-party importers) is the real risk, not entropy; immutable event-log
   # history (RootIssued/Locked/Transfer) persists. MITIGATION, not a safe harbour. DPIA MANDATORY.

# central -> business erasure propagation endpoint (controller's copy MUST be deleted too):
POST /v1/businesses/{businessId}/erase {ownerRef, scope}  (HMAC):  business runs the same crypto-shred locally.
# consent withdrawal wires to retention re-eval -> erase:
POST /v1/consents/{id}/withdraw -> stop processing for purpose; trigger retention re-evaluation -> erase() if no other basis.
```

### 11.7 v3 — granular SBT lifecycle, coded-value/array determinism, auth & wallet (NORMATIVE)

Resolves `research/09-sbt-lifecycle.md` + audit-04/05/06 v2 items.

**(a) DogTagSBT with granular roles + issuerOf + soft status + recover (replaces §11.1 burn-and-remint):**

> **Retired (owner-revealing path removed; see `DogTagSBTConsent.sol`).** The owner-owned `DogTagSBT`
> this item specified - `mint(to,...)`, `setProfileRoot`, `RECOVERY_ROLE`, the two-signature
> `recover()` rebind with its EIP-712 `Claim`/`RecoverConsent` typehashes and `recoverNonce`, and the
> `_inRecovery` soulbound bypass - is deleted. The live custodial contract
> (`contracts/src/DogTagSBTConsent.sol`) keeps the parts of this spec that survive: granular roles
> (`ISSUER_ROLE`, `AUTHORITY_ROLE`, `DEFAULT_ADMIN_ROLE` under `AccessControlDefaultAdminRules` +
> `AccessControlEnumerable`), immutable `issuerOf[id]` set at mint, the soft `Status` enum with
> terminal `Deceased`/`Revoked` (issuer-or-authority gated, never an owner), and admin-only GDPR
> `burn`. It replaces the rest structurally: `mintCustodial(id, root)` takes **no recipient** (every
> tag goes to the immutable neutral custodian), `profileRoot` is **write-once** with no setter (and a
> burned id can never be re-minted), the soulbound lock is absolute (mint + burn only), and recovery
> is a **fresh custodial issuance under a new `dogTagId` + new `R`** (D3; `ProfileTreeStore.reissue` -
> referencing credentials do not survive; accept-the-break, 2026-07-16).
>
> **Do not confuse recovery with DELEGATION.** The recovery re-issue **replaces** the principal. Delegation - an owner authorizing a **non-owner** (a caretaker at the groomer) for **scoped consent while staying the owner** - is a separate mechanism and must never be implemented as a partial owner rebind. It is decided and deferred: a **separate delegate circuit**, delegate authorized by an owner-signed message and therefore **never committed in `R`**, routed as its own protocol version; the owner consent circuit stays frozen. See [`DELEGATION.md`](./DELEGATION.md) (decided 2026-07-20).
- `markDeceased` = `setStatus(id, Deceased, reason)` by `AUTHORITY_ROLE` **or the original `issuerOf`** — never the owner (a death needs an accredited party, often a *different* vet than the minter). Terminal. **No burn** — historical vaccination/travel creds referencing `dogTagId` stay verifiable.
- `dogTagId` is a **random/sequential non-personal id** — Foundry/CI test MUST assert it is **never any hash of the microchip** (neither `keccak256(microchip)` nor `Poseidon(microchip)`; any hash of a low-entropy chip is brute-forceable) (audit-06 §4.2, audit-12 M-2).

**(b) mapType for array-element decimals (fixes audit-05 V1 — reopened A2 float trap):**
```
mapType(keyPath): template = replace_all(keyPath, /\[[0-9]+\]/, "[]")   // weightHistory[0].value -> weightHistory[].value
                  return SCHEMA_TYPES[template]                          // decimal ; NEVER typeof / f64
// weightHistory[].value, titer.resultIUml enter wrapDocument as TYPED DECIMAL STRINGS; assertNotFloat covers array elements.
```

**(c) canonicalCode normalization for coded identifiers (fixes audit-05 V15 — NFC ≠ case/whitespace):**
```
canonicalCode(s, system):   // vaccineProductCode(APHIS PCN), breedVbo, usdaNan, ...
   s=NFC(s); s=trim(s); reject if internal whitespace
   if system in {VBO, APHIS_PCN}: s=uppercase(s)        // "vbo:0200798" -> "VBO:0200798"
   require s matches systemRegex(system)                // VBO:/^VBO:[0-9]{7}$/ ; usdaNan:/^[0-9]{6}$/
   return s                                             // store canonical form in `data` (stored==hashed)
// batchLotNumber is CASE-PRESERVING (trim+NFC only); enum strings (sex/unit/standard) validated case-STRICT, no silent lowercasing (V16).
```

**(d) empty-container + microchip.code pins (audit-05 V4/V5):** empty `{}`/`[]` → one **null (tag 0) leaf** at the path (reconciles arch §13 ↔ §11.2). `microchip.code` is always **string (tag 2)** (15 digits would silently survive an f64 round-trip and leading zeros would be stripped as int). `requiredPaths` per recordType (non-obfuscatable): `@context[*]`, `type[*]`, `credentialStatus.*`, `credentialSubject.dogTagId`, rabies product/manufacturer/batch. "This chip is vaccinated" flows MUST join the vaccine cred with the `DOG_PROFILE` cred (chip↔dogTagId binds only there — audit-05 V6).

**(e) operator-auth on ALL issuance/settings/signer routes (audit-06 §2.4):**
```
require operator session for: /credentials/prepare, /credentials/confirm, /records/*,
                              GET|PUT /settings/signing-mode, GET /issuer/signers, /import/*, /calendar/*
unauthenticated ONLY: GET /records/{id} (record-JWT) and HMAC cross-backend routes.
LEGACY POST /records: either RETIRE in v2 or gate with `operator session && unlocked && whitelisted`
                      (else: remote unauthenticated issuance + gas-drain on the self-hosted box).
PUT /settings/signing-mode: 409 if any status=="prepared" record outstanding (no mid-flight split — audit-06 §2.3).
```

**(f) mobile wallet: funds-custody acknowledgment + recovery (audit-06 §3.2/§3.5):**
- **Default to gas sponsorship / account abstraction (ERC-4337/7702)** so pet owners **never hold PLASMA**: issuance gas is the issuer-backend's; the only user-side on-chain action is read-only import + occasional `recover`. **Omit native send/receive from v1** → removes most wallet attack surface + the money-transmission question (get a legal read if funds custody is ever added).
- **Seed-loss recovery (normative):** primary = the embedded-MPC provider's passkey/email-share recovery (Privy/MetaMask Embedded), which **restores the seed** - and with it every per-tag derivation (owner-secret, reserved-leaf salts, consent key), so the existing tag keeps working. A seed that is truly lost (or must be rotated) means the tag's hidden owner leaves can never be proven again: the remedy is the **recovery re-issue** - a fresh custodial issuance under a new `dogTagId` + new `R` (§11.7(a)); the abandoned tag can additionally be erased via admin `burn` (GDPR). There is no on-chain rebind of any kind. dApp-connect (Reown WalletKit) is **off by default** for non-crypto owners; DogTag defines no dApp-signable protocol message (the retired EIP-712 `Claim` went with the rebind path).

### 11.8 On-chain proof-of-verification — consent + Groth16 (NORMATIVE — CHANGESPEC §0-§5; research 10/11/12)

The verification leg. Canonical names per CHANGESPEC-v4 §0/§2. **Single Poseidon root `R`**
(§1.2–§1.4): the device computes the tag's `R`, `issue(R)` anchors it, `mintCustodial(dogTagId, R)`
seals it, the consent circuit proves the hidden owner leaves' inclusion in it, and the registry checks
`isValid(R)` **directly** on the public root - no `rKec`/`rZk` duality, no `zkCommit`, no
`kecOf`/`zkIndex`/`issuerForAny`. Public-signal order (§11.9(d)): **`[dogTagId, purpose, relayer,
nullifier, R, recordType, deadline]`**. The nullifier is `Poseidon(DS_NULLIFIER, ownerSecret,
dogTagId, purpose, relayer, consentNonce)` (pinned circomlib BN254 - §11.2) - a **public signal**
computed by the circuit and bound to the hidden `owner.secret` leaf, so **one consent = one
attestation** with no on-chain Poseidon at all.

> **Items (a), (b), and (d) below specified retired owner-revealing surface and are replaced by
> retirement notes; (c) and (e)-(g) carry the live surface. The live registry behavior is §11.9(e)
> and the authoritative source is `contracts/src/VerificationRegistryConsent.sol`.**

**(a) `VerificationRegistry.sol` (normal + ZK; shared nullifier; range-check ALL public signals):**

> **Retired (owner-revealing path removed; see `consent.circom` / `consent_assemble`).** The two-path
> registry - the ECDSA `recordVerification(consent, userSig)` entrypoint (EIP-712
> `VerificationConsent` with a `subject` field, `ownerOf(dogTagId) == subject`, an on-chain
> `PoseidonT7` nullifier) and the subject-bearing `recordVerificationZK` (pub incl.
> `subject`/`keyHash`, `keyOf[subject] == keyHash`) - is deleted. The live contract is
> `VerificationRegistryConsent` (§11.9(e)): ONE Groth16 entrypoint, owner-blind by construction. What
> survives from this spec: the relayer pattern (plain relay - **no EIP-2771**, no ERC-4337; the
> relayer is a public signal enforced `== msg.sender`), the Groth16 footguns (nullifier is a
> **public signal**, never derived from the malleable `(a,b,c)` - snarkjs #383; **range-check ALL
> public signals** `< SNARK_SCALAR_FIELD` - snarkjs #358), the `VERIFY:` purpose-scoped whitelist, and
> the direct `isValid(R)` re-check.

**(b) `ConsentKeyRegistry.sol` (one-time BabyJubjub↔secp256k1 binding):**

> **Retired (owner-revealing path removed).** The on-chain consent-key registry - `bindConsentKey`,
> the gasless `bindConsentKeyFor` relay, the EIP-712 `BindConsentKey` digest, `bindNonce`, and the
> `keyOf[wallet]` linkage - is deleted. The consent key is per-tag, derived from
> `(wallet seed, dogTagId)`, and committed INSIDE the tag's tree as the `owner.consentKey` leaf
> (`keyHash = Poseidon(Ax, Ay)`, §1.10); the circuit proves that leaf's inclusion in `R`, so there is
> nothing to bind on-chain and no wallet address to bind it to.

**(c) `Groth16VerifierConsent.sol`** - generated verbatim by `snarkjs zkey export solidityverifier`
from the frozen consent-ceremony `.zkey` (`circuits/Groth16Verifier.consent.sol` →
`contracts/src/Groth16VerifierConsent.sol`); BN254/alt_bn128;
`verifyProof(uint[2] a, uint[2][2] b, uint[2] c, uint[7] pub)` (the 7 consent public signals,
§11.9(d)). Do not hand-edit. Address-pinned in config; `circuits/`-built. (The non-consent
`Groth16Verifier` for the retired circuit is deleted.)

**(d) circom circuit - signals + what it proves:**

> **Retired (owner-revealing path removed; see `circuits/consent.circom`).** The
> `DogTagVerification(24, 5)` template of `circuits/verification.circom` - the variable-arity
> credential-leaf witness (`leafKeyPathHashes/leafTypeTags/leafSalts/leafValues[N]`,
> `dogTagIdLeafIndex`, `pathElements/pathIndices`), the public `subject` input, the `keyHash` output,
> and the subject-bearing message/nullifier - is deleted, and its zkey/graph are no longer app assets.
> The LIVE circuit is `DogTagConsent(6)` (`circuits/consent.circom`, frozen with the ceremony VK):
>
> - **Public (all declared as OUTPUTS, in frozen order):** `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` - no subject, no keyHash.
> - **Private:** the three reserved owner leaves' values + salts (`ownerAddress`; the consent-key leaf value `keyHash = Poseidon(Ax,Ay)`; `ownerSecret`), their front-packed inclusion paths (`*Siblings[6]` + `*PathLen`), the per-tag BabyJubjub pubkey `(Ax, Ay)`, the EdDSA signature `(R8x, R8y, S)`, and `consentNonce`.
> - **Proves:** (1..3) each reserved leaf ∈ `R` under its PINNED keyPath+typeTag constants (pinning is load-bearing against keyPath substitution; "exactly one reserved triple per `R`" is a normative issuance precondition - `DELEGATION.md` §5); (4) `EdDSAPoseidonVerifier` over `M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)`; (5) `nullifier == Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)`; + `Num2Bits(160)` on `relayer`.
> - The circuit does NOT bind `dogTagId <-> R` (the registry's `R == profileRoot(dogTagId)` is the only place that is checked) and does NOT prove `isValid` (re-checked on-chain).

**(e) prover flow.** In production the **phone proves on-device**: `consent_assemble` builds the named
inputs from the wallet seed + the disclosed consent parameters, the `circom-witnesscalc` GRAPH
(`consent.graph`) computes the witness, and circom-prover/Arkworks runs Groth16 against the frozen
`consent_final.zkey`; the phone serializes `(a,b,c,pub[7])` and POSTs it for the
`recordVerificationZK` call - the witness never leaves the device. `dogtag-prover-rs` (ark-circom +
ark-groth16, pure Rust, integrated witness-gen, no native deps) runs the **identical** proving flow
server-side: the `/prove-consent` trusted fallback (§3.10b) and the test-oracle re-prove for the e2e
scripts. Sub-second either way. `rapidsnark` is a documented escape hatch only if the circuit
balloons past a few hundred k constraints.

**(f) trusted setup (NORMATIVE):** reuse the **Hermez / Perpetual Powers of Tau** phase-1 `.ptau`
(do NOT run phase 1) + run a **multi-party phase-2 (≥3 independent contributors) ending in a public
random beacon**; publish the transcript (anyone can `zkey verify`), pin the final `.zkey` hash in CI,
ship it in the prover image. The consent ceremony is **done** (testnet-grade):
`docs/CEREMONY_TRANSCRIPT.consent.md` is the transcript, and the committed
`consent_final.zkey`/`consent_verification_key.json` are its output. The pin is **ENFORCED, not just
asserted in CI** (audit M4): each version's artifact set + hashes live in an `ArtifactDescriptor`
(`crates/dogtag-prover-rs/src/artifact.rs`) and `Prover::load_versioned` **rejects** any artifact whose hash
differs (fail-closed) - a swapped or corrupt proving key never silently produces proofs against the
wrong key. A zkey hash is NOT a VK hash: the descriptor carries the VK its proofs verify against as a
separate identity, so the two can never be conflated. A deployment shipping a **different** zkey (a
production-ceremony output) overrides the pin via the **`CONSENT_EXPECTED_ZKEY_SHA256`** env var on
the prover-service - a production-ceremony swap is a pure config change; leave it unset to enforce
the bundled hash. A compromised phase-2 lets a party **forge consents, not leak data** (Groth16 ZK
holds regardless), and the **core three-pillar trust model (§11.3) does not depend on the ZK setup at
all** - a forged consent is still constrained by the nullifier, the `R == profileRoot(dogTagId)`
binding, and the on-chain `isValid(R)` re-check.

**(g) EXPORT `/verify/*` endpoint pseudocode:** the canonical flow is **§3.9** (session start → `/x/`
resolve → the owner-hidden `POST /v1/verify/consent` proof submit with peek-not-consume token gating,
session binding, registry-mirroring preflight, and the async record). The retired two-path submit
(`/verify/consent/submit` with `consent`/`sig`/`bind` and its mode switch) and the transitional
`/verify/consent/levelb` twin routes are gone; there is one owner-hidden flow.

> **`/import/pull` (off-chain data) stays DECOUPLED from `/verify/*` (on-chain attestation).**
> Verification imports no data at all; disclosure, when the owner wants it, goes through the
> import/share flow.

### 11.9 v3.1 — verification-subsystem audit remediations (NORMATIVE; overrides §4.7/§11.8 on conflict)

Resolves audit-07 (ZK), audit-08 (contracts), audit-09 (systems). **The consent path ships with the (d)/(e) items below realized in the frozen circuit + live registry.** The single-Poseidon-root issuance + 3-pillar verify are unaffected.

> **RESOLVED-by-unification (CHANGESPEC-v4 §0/§2/§4).** Poseidon unification eliminates two Criticals
> outright: **audit-07 C-1** (the keccak↔Poseidon `rKec`/`rZk` binding trusted off-chain, not proven
> in-circuit) and **audit-08 C-2** (forgeable `zkCommit` / undefined `issuerForAny` / the binding as the
> trust gap) — there is **no off-chain binding left to be unsound**. The circuit proves leaves → the
> single root `R`; the registry re-checks `isValid(R)` **directly** (strictly simpler and safer than the
> old mapping). Accordingly **(c) `zkCommit` is DELETED** along with `kecOf`/`zkIndex`/`cloneOf`/
> `issuerForAny` and the `0x02` binding leaf. Of the remaining audit-era ZK-soundness gates, purpose
> binding, range-checks, and nullifier-as-public-signal stay NORMATIVE; the subject↔key and `ownerOf`
> gates were later superseded by the owner-hidden redesign - see (a)/(e) below.

**(a) Corrected `VerificationConsent` (adds `purpose` + `challenge`).**

> **Retired (owner-revealing path removed; see `consent.circom` / `consent_assemble`).** The EIP-712
> `VerificationConsent` struct (with its `subject` field and `challenge` session bind) is deleted with
> the signed-message consent. What this item fixed lives on in the circuit: `purpose` (DISTINCT from
> `recordType`) is signed inside the EdDSA message `M` and is in the nullifier; the consent window is
> the proof-bound `deadline` (`pub[6]`, signed inside `M`); session binding is enforced by the submit
> handler against the resolved session (§3.9), not by a signed challenge field.

**(b) Canonical nullifier (pinned Poseidon, includes `purpose`).** `nullifier = Poseidon(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)` (`DS_NULLIFIER=4`; 6 inputs → circomlib t=7). The **one** pinned circomlib BN254 instantiation (§11.2): the circom circuit emits it as a **public-signal output** (never derived from proof bytes - snarkjs #383), bound to the hidden `owner.secret` leaf proven ∈ `R` - **CI asserts circom == Rust** on shared vectors. It is **relayer-bound and subject-less**: scope is per `(dogTagId, purpose, relayer)` + nonce, so replaying one signed consent repeats it (rejected) and a fresh nonce mints a new one. `purpose`'s keccak label is reduced mod r once at the field boundary (§11.2(d)); addresses are `uint160` → one field. There is no on-chain Poseidon recompute - the chain only consumes the signal.

**(c) `zkCommit` — DELETED by unification (resolves audit-07 C-1 / audit-08 C-2).** There is no second root to bind: the circuit proves leaves → the single Poseidon root `R`, and `DogTagIssuer.issue(R)` anchors that exact root. `zkCommit`, the `ZkCommitment` event, the `kecOf[rZk]→rKec` mapping, `zkIndex`/`cloneOf`, the undefined `issuerForAny()`, and the `keccak(0x02‖rKec‖rZk)` binding leaf are all **removed** (CHANGESPEC-v4 §0/§2). The registry resolves the clone via the existing per-`recordType` `issuerFor[recordType]` and calls `isValid(R)` directly on the public root.

**(d) Circuit public signals (live).** Public: `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` - all seven declared as circuit OUTPUTS in exactly this order (frozen with the ceremony VK; mirrored by the registry's `P_*` constants and `public_signals::level_b`). The circuit MUST: prove the three reserved owner leaves' inclusion in one root `R` under their PINNED keyPath+typeTag constants (the anti-substitution fix that generalizes audit-07 H-1); verify the EdDSA-BabyJubjub consent signature over `M = Poseidon(dogTagId, purpose, relayer, deadline, consentNonce)` (binds `purpose` + `relayer` + `deadline` - the audit-07 C-2 purpose bind, realized without a subject); output `nullifier` per (b); range-check `relayer` to 160 bits. It deliberately does NOT bind `dogTagId <-> R` (the registry's `profileRoot` check owns that) and has no `subject`/`keyHash` signals - the consent key is proven in-tree.

**(e) `recordVerificationZK` (live: profileRoot binding + existence gate + `isValid(R)` direct + purpose-scoped whitelist).**
```solidity
function recordVerificationZK(uint[2] a,uint[2][2] b,uint[2] c, uint[7] pub) external {
   // pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
   for (uint i; i<7; i++) require(pub[i] < SNARK_SCALAR_FIELD);                        // range-check ALL (#358)
   require(pub[2] < 2**160, "addr range");                                             // full-element check BEFORE narrowing (L1)
   require(block.timestamp <= pub[6], "expired");                                      // deadline is proof-bound
   require(pub[5] != SERVICE_ATTESTATION_FIELD, "art9");                               // (h): reduced-mod-r constant
   require(address(uint160(pub[2])) == msg.sender, "not relayer");                     // relayer == caller
   if (restrictToWhitelistedRelayers)
       require(registry.isWhitelistedFor(keccak256(abi.encode("VERIFY:", bytes32(pub[1]))), msg.sender)); // purpose-specific
   require(bytes32(pub[4]) == sbt.profileRoot(pub[0]), "R !profileRoot");              // THE owner-hidden tag<->root binding
   sbt.ownerOf(pub[0]);                                                                // EXISTENCE gate only (reverts on burn);
                                                                                       //   return value DISCARDED - never compared
   require(zkVerifier.verifyProof(a,b,c,pub), "bad proof");
   bytes32 nf = bytes32(pub[3]); require(!consumed[nf]); consumed[nf]=true;            // nullifier = PUBLIC SIGNAL (#383)
   address clone = rootIssuer[bytes32(pub[4])]; require(clone != address(0));          // §11.10(a): resolve clone FROM the root R (write-once)
   require(DogTagIssuer(clone).isValid(bytes32(pub[4])));                              // isValid(R) DIRECTLY on the public root - no kecOf
   emit Verified(pub[0], msg.sender, bytes32(pub[1]), nf, pub[6], block.timestamp);    // owner-blind: NO subject
}
```
(Abridged; the authoritative body - including the propose/execute verifier timelock - is `contracts/src/VerificationRegistryConsent.sol`.)
> The audit-era subject↔key (`keyOf[subject]==keyHash`) and `ownerOf(dogTagId)==subject` gates are **superseded by the owner-hidden redesign**: their soundness role (a relayer cannot attribute a verification to a victim) is carried by the in-tree consent key + the `R == profileRoot(dogTagId)` binding. Clone resolution is from the **root** via the write-once `rootIssuer[R]` index (written at `issue(R)`), NOT via `purposeToRecordType`/`issuerFor[recordType]` - a `recordType→clone` map is one-to-many across businesses and cannot pick the clone that actually issued `R` (audit-11 V4-C1).

**(f) Generalized hardened confirm (audit-08).** For verify submissions, §11.6 `confirm` asserts the **`Verified`** event (emitted by the registry address) + `consumed[nf]==true` at N confirmations — not just `RootIssued`. Else confirm degrades to receipt-status-only.

**(g) Submit auth + fail-fast (audit-09 F-2/F-3, restated for the owner-hidden flow).** There is no central relay hop left to HMAC-sign: the phone POSTs the proof directly to the verifier host, gated by the one-time export token (a gas-spend gate - §3.9). The off-chain fail-fast survives as the registry-mirroring preflight (`pub[relayer]==activeSigner`, ranges, deadline margin, Art. 9, unconditional `VERIFY:` whitelist) plus the session binding of `purpose`/`relayer`/`recordType`; the token is peeked, never consumed by a failed submit, and replay is blocked by the session status guard + the on-chain nullifier.

**(h) Art. 9 enforcement (audit-09 P-3 Critical).** `SERVICE_ATTESTATION` is off-chain-only with **no on-chain root** → it is **NOT verifiable via on-chain proof-of-verification** (state explicitly; reject at registry + backend). The mechanism applies to `VACCINATION`, `DOG_PROFILE`, `TRAVEL_CLEARANCE`, `EU_HEALTH_CERT`. `purpose` labels MUST be non-sensitive (no Art. 9 leakage in cleartext `Verified.purpose`).

**(i) ZK privacy scope - on-device proving is CANONICAL (audit-09 B-4, resolved).** The **phone generates the Groth16 proof on-device** and POSTs only `{proof, pubSignals}`; the verifier backend submits `(a,b,c,pub)` and **never receives the witness or the raw record**. Verification therefore minimizes exposure **both on-chain AND to the groomer** (true ZK against the verifier, not merely the chain) - "the groomer never holds the cert" is TRUE. Server-side proving is the `/prove-consent` **trusted fallback** (§3.10b - the owner's own/owner-trusted prover, which does see the witness) or the `dogtag-prover-rs` test oracle; neither is the canonical path. Any earlier wording calling on-device a "v2 upgrade" or claiming "the verifier receives the witness/disclosed doc" is **superseded**.

**(j) Per-tag consent key (audit-09 P-5 / audit-08 M-3) - realized in-tree.** The BabyJubjub consent key derives **per tag** from `(wallet seed, dogTagId)` (§1.10) and is committed as the `owner.consentKey` leaf, so verifications of different tags never re-link through a shared key and **no `keyOf` registry remains in DPIA scope**. Rotation is the recovery re-issue (new tree, new key, new `dogTagId` - §11.7(a)); there is no lost-key lockout because the whole tree re-derives from the seed.

**(k) Deploy + ops (audit-08 M-4/M-5; superseded re: clone resolution by §11.10(a)).** Clone resolution for `isValid(R)` is via the write-once `rootIssuer[R]` index (§11.10(a)) written at `issue(R)` - **not** `setIssuerFor`/`zkIndex` (both deleted by unification). `Deploy.s.sol` wires the `rootIndex` and authorizes factory clones to call `registerRoot`; the verifier swap MUST have a **real timelock** - realized as `proposeZkVerifier`/`executeZkVerifier` (2 days). Gate Phase 2.5 on the ROAX chain supporting the **BN254 pairing precompiles**. Buildability specs (audit-09 B-3): relayer-address→businessId resolution and `purpose` validation are in scope; the consent key needs no delivery to any server (it derives on-device).

> **Superseded bodies:** `§2.1–§2.4` (single-boolean `IssuerRegistry`, `whitelistIssuer`, pre-remediation `createIssuer`/deploy) are **superseded** by the per-recordType `isWhitelistedFor` model in `§11.1` — code `§11.1`/`§11.8`/`§11.9`, never `§2.x`.

### 11.10 v4.1 — Poseidon-unification audit remediations (NORMATIVE; overrides §11.8/§11.9 on conflict)

Resolves audit-10 (Poseidon determinism), audit-11 (contracts), audit-12 (systems). **C-items are deploy-blocking.**

**(a) Issuer-clone resolution — write-once `rootIssuer[R]` (fixes audit-11 V4-C1 Critical; SUPERSEDES the `purposeToRecordType`/`issuerFor[recordType]` resolution in §11.9).** A single root `R` is issued in exactly one per-business clone, but `recordType→clone` is one-to-many, so it cannot resolve the issuing clone (false-negative DoS for all but one business; or revocation-evasion/wrong-issuer pass). Maintain a **protocol-global write-once index**:
```solidity
mapping(bytes32 => address) public rootIssuer;     // R -> the clone that issued it (write-once)
function registerRoot(bytes32 R) external { require(isFactoryClone(msg.sender) && rootIssuer[R]==address(0)); rootIssuer[R]=msg.sender; }
// DogTagIssuer.issue(R): after storing issuedAt[R], call rootIndex.registerRoot(R);
// VerificationRegistryConsent resolves the clone FROM the root, never from recordType/purpose:
address clone = rootIssuer[R]; require(clone != address(0), "unknown root"); require(DogTagIssuer(clone).isValid(R));
```
Drop `purposeToRecordType` for `isValid` resolution. Defense-in-depth: leaf-bind `(dogTagId, recordType, issuerEntityId)` into the Poseidon leaves.

**(b) Per-arity Poseidon CI anchors (fixes audit-10 P-C1 Critical).** `poseidon([1,2])` exercises only t=3; the system uses **t=2** (bytesToField fold), **t=3** (Merkle node), **t=6** (leaf), **t=7** (nullifier), and `R_P`/constants/MDS are per-`t`. CI MUST assert **pinned anchor vectors at t=2, t=3, t=6, t=7** bit-identical across circom / poseidon-lite / light-poseidon / poseidon-solidity (t=7 against deployed `PoseidonT7`). **circomlib is the reference-of-record** — the anchor vectors are generated from circomlib and the other three libs are conformance-tested against circomlib's outputs (on disagreement, circomlib wins; repin/replace the offending lib).

**(c) Field-reduction parity + range-check discipline (fixes audit-10 P-C2 Critical).** Pin ALL reductions to the **BN254 scalar field `r`** (not base `q` - modulus confusion = silent divergence). `purpose = keccak256(label) mod r` identical in circom + Rust + Solidity comparison constants (e.g. the registry's reduced `SERVICE_ATTESTATION_FIELD`). The discipline survives the retired ECDSA path's removal as the registry's `< r` range check on **every** public signal plus the SDK's single field-boundary reduction - else values congruent mod r collide in the `consumed` set. CI negative vector: `id` vs `id+r` MUST be rejected, not silently equal.

**(d) `bytesToField` edge vectors + limb range-check (audit-10 P-H1).** Vectors `""`, `"a"`, `"a\x00"`, 31B, 32B, length-extension-negative; in-circuit range-check the limb count. (Packing confirmed injective + length-extension-safe via the 8-byte length prefix in limb 0.)

**(e) In-circuit Merkle == SDK Merkle (audit-10 P-H2).** The circuit MUST replicate the integer-`[0,p)` `min/max` comparator, **odd-promotion** (NOT power-of-two padding), and single-leaf passthrough; a stock index-bit template diverges on non-power-of-2 counts. Root-equality vectors for leaf counts {1,2,3,5,6,7}.

**(f) Rust limb decode (audit-10 P-H4).** Decode ≤31-byte limbs directly; **forbid `from_be_bytes_mod_order`/32-byte widening** (wraps mod r, diverges from circom). Unit-test Rust field-encoding vs a circom witness.

**(g) `setZkVerifier` real timelock + `rootIssuer` write-once (audit-11 V4-M1).** Verifier-setter behind an actual timelock; `rootIssuer[R]` strictly write-once.

## 10. Build order (maps to the build-out prompt)
1. `dogtag-standard-rs` + `dogtag-standard-ts` + test vectors (the trust core).
2. `contracts/` + Foundry tests + deploy to ROAX.
3. Business backend (vet) — genesis, issue, QR/JWT, verify.
4. Central/admin backend — registry, whitelisting, mobile API, appointments.
5. Vet & groomer portals; admin portal.
6. Mobile apps (Android then iOS) with UniFFI verify + theming.
7. Calendar sync + cross-backend appointments.
8. E2E hardening + audits.
