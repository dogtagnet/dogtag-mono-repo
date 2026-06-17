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
├── contracts/                   # Foundry (src/ script/ test/ foundry.toml)
├── crates/dogtag-standard-rs/   # Rust SDK (canonicalize, merkle, verify, custody) + UniFFI
├── packages/
│   ├── dogtag-standard-ts/      # TS SDK (mirror of the Rust SDK)
│   └── ui/                      # shared React components + theme tokens
├── docs/  { architecture.md implementation.md research/ }
└── references/
```

Workspace tooling: **pnpm** workspace (TS packages + web apps), **Cargo** workspace (Rust crate + 3 API binaries can share it), **Foundry** for contracts. Root `Makefile`/`justfile` with `dev`, `build`, `test`, `deploy-contracts`, `up:<stack>`.

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

### 1.2 Leaf hashing  (architecture §3.3)

```
const DOMAIN_LEAF = 0x00
fn u32be(n) -> 4 bytes big-endian

fn hashLeaf(keyPath: string, salt: bytes16, typeTag: u8, value) -> bytes32:
    kp  = utf8(NFC_normalize(keyPath))
    val = encodeValue(typeTag, value)
    assert len(salt) == 16
    buf = [DOMAIN_LEAF]
        ++ u32be(len(kp))  ++ kp
        ++ u32be(16)       ++ salt
        ++ [typeTag]
        ++ u32be(len(val)) ++ val
    return keccak256(buf)            // Ethereum Keccak (tiny-keccak v256 / noble keccak_256)
```

### 1.3 Merkle tree  (architecture §3.4)

```
const DOMAIN_NODE = 0x01
fn sortPair(a: bytes32, b: bytes32) -> bytes64: return a<=b ? a++b : b++a   // bytewise compare
fn hashNode(a, b) -> bytes32: return keccak256([DOMAIN_NODE] ++ sortPair(a,b))

fn buildMerkle(leafHashes: bytes32[]) -> { root, layers }:
    if leafHashes.empty: ERROR
    level = sort_ascending_bytewise(leafHashes)        // dedupe identical leaves? NO — salts make them unique
    layers = [level]
    while len(level) > 1:
        next = []
        i = 0
        while i < len(level):
            if i+1 < len(level): next.push(hashNode(level[i], level[i+1])); i += 2
            else:                next.push(level[i]);            i += 1   // promote odd, no duplicate
        level = next; layers.push(level)
    return { root: level[0], layers }

fn merkleProof(layers, leafHash) -> bytes32[]:           // sibling set (unordered ok: commutative)
    idx = indexOf(layers[0], leafHash); proof = []
    for L in 0 .. len(layers)-2:
        sib = (idx ^ 1)
        if sib < len(layers[L]): proof.push(layers[L][sib])  // skip when promoted (no sibling)
        idx = idx >> 1
    return proof

fn processProof(proof, leaf) -> bytes32:                 // recompute root from leaf + siblings
    h = leaf; for s in proof: h = hashNode(h, s); return h
```

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
        leaves.push(hashLeaf(keyPath, salt, typeTag, rawValue))
    { root, _ } = buildMerkle(leaves)
    return WrappedDoc {
      version: "dogtag/1.0",
      data: unflatten(data),
      signature: { type:"DogTagMerkleProof", targetHash: root, proof: [], merkleRoot: root },
      issuer: issuerMeta,        // {name, domain, documentStore, recordType}
      privacy: { obfuscated: [] }
    }
```

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
The crate exposes `wrap_document`, `verify`, `build_merkle`, `hash_leaf` (and value encoders) over **UniFFI** so Android (Kotlin) and iOS (Swift) call the *same* verification code. `custody`/RPC stay server-side only.

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

### 2.2 `DogTagIssuer.sol` (clone implementation — no constructor)
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
```solidity
contract DogTagSBT is ERC721, IERC5192 {
    IssuerRegistry public registry;
    mapping(uint256=>bytes32) public profileRoot;
    error Soulbound();
    modifier onlyWhitelisted(){ require(registry.isWhitelisted(msg.sender)); _; }
    constructor(address reg) ERC721("DogTag","DTAG"){ registry=IssuerRegistry(reg); }

    function mint(address to,uint256 dogTagId,bytes32 root) external onlyWhitelisted {
        _safeMint(to,dogTagId); profileRoot[dogTagId]=root; emit Locked(dogTagId);
    }
    function setProfileRoot(uint256 id,bytes32 root) external onlyWhitelisted { profileRoot[id]=root; }
    function locked(uint256) external pure returns(bool){ return true; }
    function _update(address to,uint256 id,address auth) internal override returns(address){
        address from=_ownerOf(id);
        if(from!=address(0) && to!=address(0)) revert Soulbound();   // block transfer; allow mint+burn
        return super._update(to,id,auth);
    }
    function supportsInterface(bytes4 i) public view override returns(bool){ return i==0xb45a3c0e || super.supportsInterface(i); }
}
```

### 2.5 Deploy script `script/Deploy.s.sol`
```
1. deploy IssuerRegistry(adminMultisig)
2. deploy DogTagIssuer impl (uninitialized)
3. deploy DogTagIssuerFactory(impl, registry)
4. deploy DogTagSBT(registry)
5. factory.createIssuer("Vaccination", keccak("VACCINATION"), saltVacc)   // protocol-wide issuers
   factory.createIssuer("DogProfile", keccak("DOG_PROFILE"), saltProf)
6. registry.whitelistIssuer(protocolSignerForProfiles)
7. write addresses -> deployments/roax.json
```

---

## 3. Business backend (vet & groomer) — Rust API

Axum + MongoDB + `dogtag-standard-rs`. Vet and groomer share most of this (separate folders, diverge later). Port: vet `41874`, groomer `43618`.

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

---

## 4. Central / admin backend — Rust API (port `39742`)

Powers mobile apps + admin portal. Axum + MongoDB + Alloy (admin signer for whitelisting).

### 4.1 Mobile-user API
```
POST /v1/auth/...                         // signup/login, push token
GET  /v1/pets , POST /v1/pets { microchip:{code,standard,implantDate,bodyLocation}, ... }
POST /v1/pets/{id}/mint                   // mint DogTag SBT
    require microchip.code unique; build profile VC -> wrap -> root
    // SBT minted to the USER'S self-custodial (or embedded-MPC) wallet address (CHANGESPEC §4)
    central protocol signer: DogTagSBT.mint(userWalletAddress, dogTagId, root)
    save pets{dogTagId,...}   // verifier later reads DogTagSBT.ownerOf(dogTagId) == userWalletAddress
GET  /v1/credentials , POST /v1/credentials/import { wrappedDoc }
    verdict=verify(...); require valid; store reference
POST /v1/share/{credentialId}             // user->business: mint one-time JWT (aud dogtag-business)
GET  /share/{ref}  Bearer<jwt>            // business pulls shared doc
```

### 4.2 Business registry & discovery
```
GET  /v1/businesses?type=&near=lat,lng&radius=
    geo query -> [{businessId,type,name,geo,services,apiBaseUrl,domain,documentStores,hmacKeyId}]  // non-personal
POST /v1/businesses (admin)               // register a deployment + issue HMAC key
```

### 4.3 Issuer whitelisting (admin)
```
POST /v1/issuer-applications              // from business backend §3.2 (status pending)
    // accepts MULTIPLE addresses per issuer entity: {issuerEntityId, addresses[], recordTypes[], ...}
GET  /v1/issuer-applications (admin)
POST /v1/issuer-applications/{id}/approve (admin)
    verify accreditation off-chain (usdaNan 6-digit, license{number,jurisdiction,expiry})
    // one issuer ENTITY -> many whitelisted signer addresses (wallet EOA + backend) (CHANGESPEC §3)
    for (address, recordType) in application.addresses x application.recordTypes:
        adminSigner: IssuerRegistry.whitelistFor(recordType, address)
    mark approved; notify business
POST /v1/issuer-applications/{id}/reject (admin)
POST /v1/issuer-applications/{id}/delist (admin)   // delist inactive-mode / rotated addresses
    for (address, recordType): adminSigner: IssuerRegistry.delistFor(recordType, address)
```

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
- **Issue credential**: pick recordType → form (schema-driven, validates §1.6) → "Sign & Issue" (POST `/records`) → show txHash + "Show QR" (`/records/{id}/share`, render QR).
- **Records list**: status (issued/revoked), re-generate QR anytime, revoke.
- **Import from user**: "Import Profile / Vaccination" → show scan prompt → `/import/pull`.
- **Calendar + Appointments**: connect Google, calendar grid, approve/decline/reschedule (mirrors reference groomer UI).

### 5.2 Groomer portal (`stacks/groomer/web`, port 43617)
- Mirrors the reference dashboard (Dashboard/Calendar/Appointments/Clients/Groomers/Reports/Marketing/Settings).
- Import pet **profile** + **vaccination status** via QR (`/import/*`), verify on chain+DNS before accepting.
- Same genesis/custody setup (groomers can issue their own records too).

### 5.3 Admin portal (`stacks/admin/web`, port 39741)
- Business registry CRUD + map.
- Issuer applications queue → approve (triggers on-chain `whitelistIssuer`) / reject.
- Whitelist viewer (on-chain state), appointment/observability dashboards.

---

## 6. Mobile apps (Android + iOS)

### 6.1 Shared
- **Verification** via `dogtag-standard-rs` UniFFI bindings (`verify`, `wrapDocument`) — identical to server.
- **API base**: central API (`https://api.dogtag.io`) for accounts/discovery/booking; per-business URLs come from discovery responses & QR origins.

### 6.2 Screens (from references)
- Onboarding ("Welcome to Dog Tags") → tabs **Verify · Travel · Home · Documents · Profile**.
- Home: pet card + Credentials grouped (Health / Service Dog / Travel Docs).
- Add health/travel record wizards with type pickers (Vaccine/Checkup/Surgery/Lab/Prescription/Dental; CDC/DOT/Other travel).
- **Scan QR** (Verify tab): parse `https://<host>/r?t=&i=` → fetch wrapped doc → `verify()` → import under pet, show 3-pillar verdict.
- **Share** (user→business): show QR (one-time JWT against central).
- **Find vet/groomer**: Maps (Maps Compose / MapKit) → discovery API → book appointment.

### 6.3 Theming (7 themes)
```
ThemeTokens = { primary, secondary, surface, background, onPrimary, onSurface, success, danger, ... }
themes = { black, white, blue, red, pink, green, yellow }   // each: light + dark palette
```
- **Android**: `ColorScheme` per theme via `MaterialTheme`; `ThemeController` persists choice (DataStore); components use `MaterialTheme.colorScheme.*` only.
- **iOS**: `ThemeManager: ObservableObject` in `@Environment`; `Color.primaryToken` etc. resolve from active theme; persisted in `UserDefaults`.
- Components reference **semantic tokens only** → switching theme recolors everything; layout/components unchanged.

### 6.4 Wallet module (Settings) — self-custodial EVM wallet (CHANGESPEC §4 — research 08 B)

A Telegram-style in-app wallet **under Settings**. The **DogTag SBT is minted to and owned by the
user's wallet address** (`DogTagSBT.mint(userWalletAddress, dogTagId, root)` in §4.1); verification
reads `ownerOf`.

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
        non-crypto owners; DogTag's EIP-712 Claim is signed ONLY via the in-app recover() flow, never a dApp.

  # --- recovery / transfer: recover() preserves tokenId + issuerOf (NOT burn-and-remint) — §11.7(a)/(f) ---
  # RECOVERY_ROLE + EIP-712 destination signature {dogTagId,newOwner,nonce,deadline,chainId:135}.
  # Lost-key (no key): RECOVERY_ROLE after off-chain identity proof to the protocol — does not need the lost key.
```

### 6.5 Import verification — 4 checks (CHANGESPEC §4 — research 08 B)

A record imports as **"yours"** only when the on-chain SBT owner is the address you control.

```
fn importRecord(doc, myWalletAddress, {rpc, dnsResolver}):
    # (1) offline integrity: recompute targetHash + merkle membership (no network trust)
    require recompute(doc) == doc.signature.targetHash
    require processProof(doc.signature.proof, doc.signature.targetHash) == doc.signature.merkleRoot
    # (2) on-chain anchoring (RPC eth_call)
    require rpc.call(doc.issuer.documentStore, "isValid(bytes32)", doc.signature.merkleRoot)
    # (3) identity: DNS-TXT + central registry cross-check
    require dnsResolver.txtMatches(doc.issuer.domain, doc.issuer.documentStore, chainId=135)
         && registry.knows(doc.issuer.domain, doc.issuer.documentStore)
    # (4) ownership (self-import context ONLY): SBT owner == the address I control
    require rpc.call(DOGTAG_SBT_ADDR, "ownerOf(uint256)", dogTagIdOf(doc)) == myWalletAddress
    # 3 authenticity pillars + ownership -> import as MINE. Equivalent to verify(..., mode:"self-import") §11.3.
    # (Third-party/business import drops check 4 and uses mode:"third-party" — §3.5.)
```

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

`contracts/foundry.toml`: `evm_version = "paris"`, pinned `solc`. (research/03)
```bash
# liveness pre-check (RPC was 502 at design time)
cast chain-id --rpc-url https://devrpc.roax.net    # expect 135

forge script script/Deploy.s.sol:Deploy --rpc-url https://devrpc.roax.net \
  --chain 135 --private-key $PRIVATE_KEY --broadcast -vvvv   # add --legacy if no EIP-1559

forge verify-contract --rpc-url https://devrpc.roax.net \
  --verifier blockscout --verifier-url https://explorer.roax.net/api/ \
  <ADDRESS> src/DogTagIssuer.sol:DogTagIssuer
```

---

## 9. Testing strategy

- **SDK parity**: shared `testvectors.json` (inputs → expected leaf hashes, roots, proofs) asserted in **both** TS and Rust CI → guarantees cross-language determinism. Include Solidity test that recomputes a node hash to confirm on-chain agreement.
- **Contracts**: Foundry tests — soulbound revert on transfer, whitelist gating (only whitelisted can issue/revoke), issue/revoke/isValid lifecycle, clone init, factory determinism.
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
    bytes32 public constant PROFILE_ISSUER_ROLE = keccak256("PROFILE_ISSUER_ROLE");
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
- `DogTagSBT.mint(to,...)` mints to the **user's wallet address** (`to = userWalletAddress`); the
  verifier reads `DogTagSBT.ownerOf(dogTagId)` (the `ownership` fragment, §11.3).
- The per-`recordType` `isWhitelistedFor(recordType, signer)` above already supports **multi-address
  whitelisting**: **one issuer entity maps to many whitelisted signer addresses** (e.g. a vet's
  MetaMask EOA *and* their backend-derived address), each `whitelistFor`'d per record type. The
  contract has no concept of "the same vet" — the issuer↔signers mapping is an off-chain view.

### 11.2 Corrected canonicalization (fixes A1, A2, A3, F2a, F2b)

```
DECIMAL grammar (operate on the input STRING, never a float):
  valid  = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?$/
  canon  = strip fractional trailing zeros; drop trailing "."; "-0" -> "0"; reject exponent/space/"+"
INTEGER: /^-?(0|[1-9][0-9]*)$/ ; no leading zeros; "-0"->"0"
mapType: types come from the SCHEMA (typed input), NOT typeof. wrapDocument signature becomes
         wrapDocument(typedCredential)  where each scalar is {tag, value:string|bool|null|bytes}
assertNotFloat(v): if v is f32/f64 -> ERROR   // hard guard, always on

NFC: pin Unicode version U in SDK; reject unpaired surrogates; store NFC form in data.
     Solidity NEVER normalizes — it only hashes nodes (0x01) from existing bytes32.

flatten(obj) -> [(keyPath,...)]  PINNED grammar:
  object key  -> ".key"  (key NFC, reserved chars [.[]] rejected)
  array elem  -> "[i]"   (i base-10, no leading zeros)
  root has no leading "."; empty object/array -> a null-typed leaf at that path
parse(packed): split on FIRST TWO ":" only -> (saltHex, tag, valueRest)  // value may contain ":"
```

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
            && (doc.signature.proof.empty
                 ? doc.signature.merkleRoot==doc.signature.targetHash
                 : processProof(doc.signature.proof, doc.signature.targetHash)==doc.signature.merkleRoot)
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
      DogTagSBT.burn(dogTagId)                // admin GDPR-erasure burn (NOT the status path)
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
```solidity
contract DogTagSBT is ERC721, IERC5192, AccessControlEnumerable, EIP712 {
    enum Status { Active, Lost, TransferPending, Deceased, Revoked }
    bytes32 constant ISSUER_ROLE=keccak256("ISSUER"); bytes32 constant UPDATER_ROLE=keccak256("UPDATER");
    bytes32 constant AUTHORITY_ROLE=keccak256("AUTHORITY"); bytes32 constant RECOVERY_ROLE=keccak256("RECOVERY");
    mapping(uint256=>address) public issuerOf;     // immutable, set at mint
    mapping(uint256=>Status)  public status;
    mapping(uint256=>uint256) public recoverNonce;
    error Soulbound(); error NotIssuerOrAuthority(); error Terminal();
    bytes32 constant CLAIM=keccak256("Claim(uint256 dogTagId,address newOwner,uint256 nonce,uint256 deadline)");
    modifier issuerOrAuthority(uint256 id){ if(msg.sender!=issuerOf[id] && !hasRole(AUTHORITY_ROLE,msg.sender)) revert NotIssuerOrAuthority(); _; }

    function mint(address to,uint256 id,bytes32 root) external onlyRole(ISSUER_ROLE){
        _safeMint(to,id); issuerOf[id]=msg.sender; status[id]=Status.Active; profileRoot[id]=root;
        emit Locked(id); emit Issued(id,msg.sender);
    }
    function setProfileRoot(uint256 id,bytes32 r) external issuerOrAuthority(id){ require(status[id]==Status.Active); profileRoot[id]=r; }
    function setStatus(uint256 id,Status s,string calldata reason) external issuerOrAuthority(id){
        Status f=status[id]; if(f==Status.Deceased||f==Status.Revoked) revert Terminal();   // terminal, irreversible
        status[id]=s; emit StatusChanged(id,f,s,msg.sender,reason);                          // owner can NEVER call this
    }
    // lost-key / sale recovery: PRESERVES tokenId + issuerOf (referencing creds survive). EIP-712 by destination.
    function recover(uint256 id,address newOwner,uint256 nonce,uint256 deadline,bytes calldata ownerSig) external onlyRole(RECOVERY_ROLE){
        require(block.timestamp<=deadline && nonce==recoverNonce[id]++);
        bytes32 d=_hashTypedDataV4(keccak256(abi.encode(CLAIM,id,newOwner,nonce,deadline)));
        require(ECDSA.recover(d,ownerSig)==newOwner);    // proves control of DESTINATION (binds chainId 135 + this contract via EIP712 domain)
        status[id]=Status.TransferPending; _recoveryRebind(_ownerOf(id),newOwner,id); status[id]=Status.Active; emit Recovered(id,newOwner);
    }
    function burn(uint256 id) external onlyRole(DEFAULT_ADMIN_ROLE){ _burn(id); emit Burned(id); } // GDPR erasure ONLY
    function locked(uint256) external pure returns(bool){ return true; }
    function _update(address to,uint256 id,address auth) internal override returns(address){
        address from=_ownerOf(id); if(from!=address(0)&&to!=address(0)&&!_inRecovery) revert Soulbound(); return super._update(to,id,auth);
    }
}
```
- `markDeceased` = `setStatus(id, Deceased, reason)` by `AUTHORITY_ROLE` **or the original `issuerOf`** — never the owner (a death needs an accredited party, often a *different* vet than the minter). Terminal. **No burn** — historical vaccination/travel creds referencing `dogTagId` stay verifiable.
- `dogTagId` is a **random/sequential non-personal id** — Foundry/CI test MUST assert it is **never** `keccak256(microchip)` (audit-06 §4.2).

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
- **MPC key-loss recovery (normative):** primary = the embedded-MPC provider's passkey/email-share recovery (Privy/MetaMask Embedded). Catastrophic loss (no key at all) = `RECOVERY_ROLE` executes `recover()` after an **off-chain identity proof to the protocol** (central knows `userId↔dogTagId↔ownerAddress`) — does **not** require the lost key. dApp-connect (Reown WalletKit) is **off by default** for non-crypto owners; DogTag's own EIP-712 `Claim` is only ever signed via the in-app recovery flow (distinct domain), never a connected dApp.

## 10. Build order (maps to the build-out prompt)
1. `dogtag-standard-rs` + `dogtag-standard-ts` + test vectors (the trust core).
2. `contracts/` + Foundry tests + deploy to ROAX.
3. Business backend (vet) — genesis, issue, QR/JWT, verify.
4. Central/admin backend — registry, whitelisting, mobile API, appointments.
5. Vet & groomer portals; admin portal.
6. Mobile apps (Android then iOS) with UniFFI verify + theming.
7. Calendar sync + cross-backend appointments.
8. E2E hardening + audits.
