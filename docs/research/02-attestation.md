# 02 — Blockchain Document Attestation: OpenAttestation Research & Our Variant

> Research basis for building a from-scratch document-attestation system inspired by
> [OpenAttestation (OA)](https://github.com/Open-Attestation/open-attestation). We are **not** using
> their library; we reimplement the model with our own deterministic byte-canonicalization.
>
> Date: 2026-06-17. All source URLs are inline. Code snippets are quoted verbatim from the OA repos
> (commit at `master`) unless marked as pseudocode.

---

## 1. OpenAttestation data model

OA's core idea: a JSON document is transformed into a flat set of independently-hashable **leaves**,
each leaf salted so the values are unguessable, then those leaf hashes are combined into a **merkle
root**. The root (and per-document proof) are committed on-chain. This lets you verify integrity of
the whole document, and selectively redact individual fields without breaking the root.

### 1.1 Wrapping pipeline

`wrapDocument(data)` does three things
([`src/2.0/wrap.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/wrap.ts)):

1. `saltData(data)` — recursively replace every primitive with a salted, type-tagged string.
2. `digestDocument(document)` — flatten + hash every leaf, combine into the `targetHash`.
3. Attach a `signature` block: `{ type: "SHA3MerkleProof", targetHash, proof: [], merkleRoot }`.

```ts
// src/2.0/wrap.ts (verbatim)
const createDocument = (data, option) => ({
  version: SchemaId.v2,
  data: saltData(data),
});

export const wrapDocument = (data, options) => {
  const document = createDocument(data, options);
  // ...schema validation...
  const digest = digestDocument(document);
  const signature = {
    type: "SHA3MerkleProof",
    targetHash: digest,
    proof: [],
    merkleRoot: digest,
  };
  return { ...document, signature };
};
```

A wrapped v2 document looks like:

```json
{
  "version": "open-attestation/2.0",
  "data": {
    "issuers": [{
      "name": "5b4a...:string:Example University",
      "documentStore": "9f2c...:string:0x1234...",
      "identityProof": {
        "type": "8e1d...:string:DNS-TXT",
        "location": "a0b3...:string:example.edu"
      }
    }],
    "recipient": { "name": "c7d8...:string:Alice" },
    "score": "ee7f3323-1634-4dea-8c12-f0bb83aff874:number:5"
  },
  "signature": {
    "type": "SHA3MerkleProof",
    "targetHash": "<hex>",
    "proof": [],
    "merkleRoot": "<hex>"
  }
}
```

### 1.2 Salting — the leaf string format `uuid:type:value`

Source: [`src/2.0/salt.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/salt.ts).

Each primitive value is replaced by a string `"<uuidv4>:<type>:<stringified-value>"`:

```ts
// src/2.0/salt.ts (verbatim)
export function primitiveToTypedString(value: any) {
  switch (typeof value) {
    case "number":
    case "string":
    case "boolean":
    case "undefined":
      return `${typeof value}:${String(value)}`;
    default:
      if (value === null) return "null:null"; // typeof null === "object"
      throw new Error(`Parsing error, value is not of primitive type: ${value}`);
  }
}

export function uuidSalt(value: string) {
  const salt = uuid();                                  // uuid v4
  return `${salt}:${primitiveToTypedString(value)}`;    // "uuid:type:value"
}

export const saltData = (data: any) => deepMap(data, uuidSalt);
```

Concrete examples (note salt length is the 36-char UUID + 1 colon = 37 chars, `UUIDV4_LENGTH = 37`):

| Original value | Salted leaf string |
|---|---|
| `5` (number) | `ee7f3323-1634-4dea-8c12-f0bb83aff874:number:5` |
| `"Alice"` (string) | `c7d8...:string:Alice` |
| `true` (boolean) | `a1b2...:boolean:true` |
| `null` | `f0e1...:null:null` |
| `undefined` | `9988...:undefined:undefined` |

`deepMap` recurses into objects and arrays, salting only leaves; container keys/structure are
**not** salted. The salt is **per-value random** (a fresh uuid v4 each time), which is what makes
redacted leaf hashes unguessable (defeats brute-forcing of low-entropy values like a yes/no field).

> **Important subtlety:** OA's type tag is derived from JavaScript `typeof`. `String(value)` is used
> for the value. This is a JS-specific encoding (e.g. `number:5`, `number:5.5`, `number:1e21`) and is
> a primary thing we will replace with a language-agnostic scheme (§7).

### 1.3 Digest — flatten, hash each leaf, combine

Source: [`src/2.0/digest.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/digest.ts).

```ts
// src/2.0/digest.ts (verbatim)
export const flattenHashArray = (data: any) => {
  const flattenedData = omitBy(flatten(data), isKeyOrValueUndefined);
  return Object.keys(flattenedData).map((k) => {
    const obj: any = {};
    obj[k] = flattenedData[k];
    return keccak256(JSON.stringify(obj));   // hash of {"path.to.field": "uuid:type:value"}
  });
};

export const digestDocument = (document) => {
  const hashedDataArray = get(document, "privacy.obfuscatedData", []);   // already-hashed redactions
  const unhashedData = get(document, "data");
  const hashedUnhashedDataArray = flattenHashArray(unhashedData);

  const combinedHashes = hashedDataArray.concat(hashedUnhashedDataArray);
  const sortedHashes = sortBy(combinedHashes);                          // deterministic order
  return keccak256(JSON.stringify(sortedHashes));                       // == targetHash
};
```

Key facts:

- **One leaf per scalar field.** `flatten` turns nested data into dot/bracket paths, e.g.
  `issuers[0].name`, `recipient.name`, `score`. Each becomes a single-key object
  `{ "issuers[0].name": "uuid:string:..." }`.
- **Leaf hash** = `keccak256(JSON.stringify({ path: saltedValue }))`. So the exact bytes hashed are
  the UTF-8 of a JSON string like `{"score":"ee7f...:number:5"}`.
- **targetHash** = `keccak256(JSON.stringify(sortBy(allLeafHashes)))`. Note: in OA v2 the targetHash
  is **not** itself a binary merkle tree over leaves — it is a hash of the *sorted JSON array of leaf
  hash hex strings*. (The binary merkle tree only appears when batching multiple documents; see
  §1.4.) The leaf hashes are sorted lexicographically (`sortBy` over hex strings) for determinism.

### 1.4 targetHash vs merkleRoot vs proof (the signature block)

Source: [`src/shared/merkle/merkle.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/shared/merkle/merkle.ts)
and [`src/2.0/wrap.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/wrap.ts).

- **`targetHash`** — the digest of *this single document* (§1.3). Uniquely identifies the document.
- **`merkleRoot`** — when many documents are wrapped together as a batch (`wrapDocuments`), each
  document's `targetHash` is a leaf of a **binary merkle tree**; `merkleRoot` is that tree's root.
  This is the value committed on-chain. **For a single document, `merkleRoot === targetHash` and
  `proof === []`.**
- **`proof`** — the list of sibling ("uncle") hashes needed to walk from this document's `targetHash`
  up to the `merkleRoot`. Empty for single-document wraps.

```ts
// src/2.0/wrap.ts (batch) (verbatim)
const merkleTree = new MerkleTree(documents.map(d => d.signature.targetHash).map(hashToBuffer));
const merkleRoot = merkleTree.getRoot().toString("hex");
// per document: proof = merkleTree.getProof(targetHash), plus merkleRoot
```

#### Binary merkle tree construction (OA's exact rules)

```ts
// src/shared/merkle/merkle.ts + src/shared/utils/utils.ts (verbatim)
function getNextLayer(elements) {
  return elements.reduce((layer, element, index, arr) => {
    if (index % 2 === 0) layer.push(combineHashBuffers(element, arr[index + 1]));
    return layer;
  }, []);
}
// combineHashBuffers: if one side missing, pass the other up unchanged (lone odd node promoted)
export function combineHashBuffers(first?, second?) {
  if (!second) return first;
  if (!first)  return second;
  return hashToBuffer(keccak256(bufSortJoin(first, second)));  // SORT the pair, then hash
}
export function bufSortJoin(...args) {
  return Buffer.concat([...args].sort(Buffer.compare));        // lexicographic by bytes
}
// leaves themselves are sorted before tree build:
export function hashArray(arr) { return arr.map(toBuffer).sort(Buffer.compare); }
```

Rules that matter for cross-language agreement:

1. **Leaves are sorted** (`Buffer.compare`, i.e. lexicographic over raw bytes) before building.
2. **Each pair is sorted before hashing** (`bufSortJoin`) — so the tree is *commutative per node*;
   verification does not need to track left/right. The proof is just a set of siblings combined the
   same sorted way (`checkProof` reduces `combineHashBuffers(hash, pair)`).
3. **Odd node promotion**: a lone node at the end of an odd-length layer is carried up unchanged
   (no duplication of the last node, unlike Bitcoin's merkle tree).
4. Parent = `keccak256(sort(left, right))`.

`checkProof` (verbatim): `proof.reduce((h, pair) => combineHashBuffers(h, pair), element)` and compare
to root. Because pairs are sorted, this is order-independent.

---

## 2. Selective disclosure / obfuscation

Source: [`src/2.0/obfuscate.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/obfuscate.ts).

Because each field is its own leaf and `targetHash` is a hash of the *sorted set of leaf hashes*, you
can remove a field's cleartext as long as you keep its **leaf hash** in the set.

```ts
// src/2.0/obfuscate.ts (verbatim)
export const obfuscateData = (_data, fields) => {
  const data = cloneDeep(_data);
  const fieldsToRemove = Array.isArray(fields) ? fields : [fields];

  const dataToObfuscate = flatten(pick(data, fieldsToRemove));
  const obfuscatedData = Object.keys(dataToObfuscate).map((k) => {
    const obj = {}; obj[k] = dataToObfuscate[k];
    return toBuffer(obj).toString("hex");        // == keccak256(JSON.stringify({path: saltedValue}))
  });

  fieldsToRemove.forEach((path) => unset(data, path));   // delete cleartext
  return { data, obfuscatedData };
};

export const obfuscateDocument = (document, fields) => {
  const { data, obfuscatedData } = obfuscateData(document.data, fields);
  const newObfuscatedData = (document?.privacy?.obfuscatedData ?? []).concat(obfuscatedData);
  return { ...document, data, privacy: { ...document.privacy, obfuscatedData: newObfuscatedData } };
};
```

Mechanism:

- The removed field's salted leaf hash is moved into `privacy.obfuscatedData: string[]`.
- When `digestDocument` recomputes, it concatenates `privacy.obfuscatedData` with the live leaf
  hashes, sorts, and hashes — yielding the **same `targetHash`** as before redaction.
- Redaction is **idempotent and composable**: obfuscated hashes are appended to the existing array,
  so you can redact in successive steps and the root never changes.
- The random per-value salt is what stops a verifier from confirming a guessed value of a redacted
  field by recomputing its hash. (v3 uses `proof.privacy.obfuscated` instead of
  `privacy.obfuscatedData`; same idea — [`utils.ts:isObfuscated/getObfuscatedData`](https://github.com/Open-Attestation/open-attestation/blob/master/src/shared/utils/utils.ts).)

> Limitation: obfuscation reveals *which paths* were present (the count grows `obfuscatedData`), and
> in v2 the removed key path is still implicit. It does not hide document *shape* perfectly.

---

## 3. Identity proof via DNS

### 3.1 DNS-TXT method

ADR: [decentralized_identity_proof_DNS-TXT.md](https://github.com/Open-Attestation/adr/blob/master/decentralized_identity_proof_DNS-TXT.md).
Docs: [Configuring DNS](https://www.openattestation.com/docs/integrator-section/verifiable-document/ethereum/dns-proof/).

A DNS **TXT** record on the issuer's domain binds that domain to the on-chain contract address that
issued the document. Exact record format:

```
TXT   openatts net=ethereum netId=1  addr=0x007d40224f6562461633ccfbaffd359ebb2fc9ba   (Mainnet)
TXT   openatts net=ethereum netId=3  addr=0x0c9d5E6C766030cc6f0f49951D275Ad0701F81EC   (Ropsten)
```

Fields:

- `openatts` — fixed prefix identifying the record as an OpenAttestation binding.
- `net` — blockchain family, e.g. `ethereum`.
- `netId` — EVM chain id (1 = mainnet, etc.).
- `addr` — the **document store** (or token registry) contract address.

The document declares which domain to check, in each issuer entry:

```json
"identityProof": { "type": "DNS-TXT", "location": "example.edu" }
```

**Verification flow:**

1. Read `issuer.documentStore` (the on-chain contract address) and `issuer.identityProof.location`
   (the claimed domain) from the document.
2. Resolve TXT records for `location` via a trusted DNS-over-HTTPS resolver (OA uses Google/Cloudflare
   DoH for tamper-resistant transport).
3. Find an `openatts` record whose `net`/`netId` matches the document's network and whose `addr`
   **equals** the document's `documentStore` address (case-insensitive hex compare).
4. If found → the domain owner has endorsed this contract → identity = the domain. Render
   "Issued by example.edu".

This is a **bi-directional binding**: only the domain admin can create the TXT record, and the record
names the exact contract — so it ties "who owns the domain" to "who controls the issuing contract".
It replaces a centralized issuer registry with DNS's existing decentralized trust + admin control.

### 3.2 DNS-DID method (brief)

ADR: [issuing_using_did.md](https://github.com/Open-Attestation/adr/blob/master/issuing_using_did.md).

Instead of an on-chain document store, the issuer **signs the merkleRoot** with the private key of a
`did:ethr:0x...` DID. The document's `proof` block carries `verificationMethod` (the DID key id),
`signature`, `created`, and `proofPurpose`. Identity is bound to DNS via a TXT record that publishes
the DID/public key for the domain (record type `dns-did`); verification:

1. Recompute/verify the signature over the merkleRoot using the DID's public key.
2. Resolve the domain's TXT record and confirm it lists that DID/public key.

This issues documents **without an on-chain transaction** (cheaper), at the cost of no on-chain
issuance status (revocation needs a separate OCSP responder / revocation store).

---

## 4. Smart contract architecture

### 4.1 Legacy DocumentStore (the classic, simplest model)

The original OA `DocumentStore` (`Ownable`) is the mental model the task references:

```solidity
// legacy DocumentStore (paraphrased from OA v2 / OpenCerts era)
contract DocumentStore is Ownable {
  string public name;
  mapping(bytes32 => uint256) public documentIssued;   // merkleRoot => block number
  mapping(bytes32 => uint256) public documentRevoked;   // hash => block number

  event DocumentIssued(bytes32 indexed document);
  event DocumentRevoked(bytes32 indexed document);

  function issue(bytes32 document) public onlyOwner {
    require(documentIssued[document] == 0, "Already issued");
    documentIssued[document] = block.number;
    emit DocumentIssued(document);
  }
  function revoke(bytes32 document) public onlyOwner returns (bool) {
    require(documentRevoked[document] == 0, "Already revoked");
    documentRevoked[document] = block.number;
    emit DocumentRevoked(document);
    return true;
  }
  function isIssued(bytes32 document) public view returns (bool) { return documentIssued[document] != 0; }
  function isRevoked(bytes32 document) public view returns (bool) { return documentRevoked[document] != 0; }
  function bulkIssue(bytes32[] memory documents) public onlyOwner;   // gas-batched
  function bulkRevoke(bytes32[] memory documents) public onlyOwner;
}
```

- You issue a **merkleRoot** (`bytes32`). Issuance status = "is this root in the issued mapping".
- `revoke` operates on a root (or an individual document targetHash, for batch revocation of one item).
- Access control: `onlyOwner` (single owner = the issuer).

### 4.2 Current DocumentStore (v2 of the contracts repo)

Source (verbatim) from [Open-Attestation/document-store](https://github.com/Open-Attestation/document-store):
[`IDocumentStore.sol`](https://github.com/Open-Attestation/document-store/blob/master/src/interfaces/IDocumentStore.sol),
[`BaseDocumentStore.sol`](https://github.com/Open-Attestation/document-store/blob/master/src/base/BaseDocumentStore.sol),
[`DocumentStoreAccessControl.sol`](https://github.com/Open-Attestation/document-store/blob/master/src/base/DocumentStoreAccessControl.sol).

The modern contract adds **on-chain merkle proof verification** and **OpenZeppelin AccessControl
roles** (replacing single-owner `Ownable`), and is upgradeable.

```solidity
interface IDocumentStore {
  event DocumentIssued(bytes32 indexed document);
  event DocumentRevoked(bytes32 indexed documentRoot, bytes32 indexed document);
  function name() external view returns (string memory);
  function revoke(bytes32 documentRoot) external;
  function isIssued(bytes32 documentRoot) external view returns (bool);
  function isRevoked(bytes32 documentRoot) external view returns (bool);
  function isActive(bytes32 documentRoot) external view returns (bool);
}
```

```solidity
// BaseDocumentStore.sol (verbatim signatures)
function issue(bytes32 documentRoot)
  external onlyValidDocument(documentRoot, documentRoot, new bytes32[](0)) onlyRole(ISSUER_ROLE);

function revoke(bytes32 documentRoot, bytes32 document, bytes32[] memory proof) external onlyRole(REVOKER_ROLE);
function revoke(bytes32 documentRoot) external onlyRole(REVOKER_ROLE);

function isIssued(bytes32 documentRoot, bytes32 document, bytes32[] memory proof) public view returns (bool);
function isIssued(bytes32 documentRoot) public view returns (bool);
function isRevoked(bytes32 documentRoot, bytes32 document, bytes32[] memory proof) public view returns (bool);
function isRevoked(bytes32 documentRoot) public view returns (bool);
function isActive(bytes32 documentRoot, bytes32 document, bytes32[] memory proof) public view returns (bool);
function isActive(bytes32 documentRoot) public view returns (bool);
```

```solidity
// DocumentStoreAccessControl.sol (verbatim)
bytes32 public constant ISSUER_ROLE  = keccak256("ISSUER_ROLE");
bytes32 public constant REVOKER_ROLE = keccak256("REVOKER_ROLE");
function __DocumentStoreAccessControl_init(address owner) internal onlyInitializing {
  _grantRole(DEFAULT_ADMIN_ROLE, owner);
  _grantRole(ISSUER_ROLE, owner);
  _grantRole(REVOKER_ROLE, owner);
}
```

Storage uses ERC-7201 namespaced slots with two mappings: `documentIssued` and `documentRevoked`
(`mapping(bytes32 => bool)`). The `onlyValidDocument` modifier runs an OpenZeppelin `MerkleProof.verify`
so you can issue a single root and later prove/revoke individual member documents under it. Single-doc
calls pass `documentRoot == document` and an empty proof.

### 4.3 TokenRegistry (ERC-721) approach

For **transferable** documents (e.g. title deeds, bills of lading), OA uses an **ERC-721** token
registry instead of a document store. The document's `targetHash` is the **tokenId** (`uint256`).
"Issuing" = minting the token to a beneficiary; ownership/holdership transfers move the
document's title on-chain. The same DNS-TXT identity binding applies, with `addr` pointing at the
token-registry contract. (See `isTransferableAsset`/`getAssetId` in
[`utils.ts`](https://github.com/Open-Attestation/open-attestation/blob/master/src/shared/utils/utils.ts).)

### 4.4 Deployment / factory

OA historically used a **DocumentStoreCreator** factory (and later a CREATE2-based deployer /
`DeployUtils` library + Foundry deploy scripts in the current repo) so issuers can deploy a named
store cheaply and get a deterministic address. The factory emits the new store address and sets the
caller as owner/admin. (See `script/` and `src/libraries/DeployUtils.sol` in the document-store repo.)

---

## 5. Verification pipeline

OA's verifier (`oa-verify`) runs independent **verification fragments** and combines them. The three
pillars:

1. **Document integrity** (`OpenAttestationHash`): recompute the digest from `data` (+ obfuscated
   hashes), confirm it equals `signature.targetHash`; then run `checkProof(proof, merkleRoot, targetHash)`
   to confirm the document belongs to the committed `merkleRoot`. No network needed.
   ([Document integrity](https://www.openattestation.com/docs/verify-section/document-integrity)).
2. **Issuance status** (`OpenAttestationEthereumDocumentStoreStatus` /
   `...TokenRegistryMinted`): call the on-chain contract — `isIssued(merkleRoot)` true and
   `isRevoked(...)` false (or, for token registry, the token is minted/owned).
   ([Issuance status](https://www.openattestation.com/docs/verify-section/issuance-status)).
3. **Issuance identity** (`OpenAttestationDnsTxtIdentityProof` / `...DnsDidIdentityProof`):
   resolve DNS TXT for `identityProof.location` and confirm an `openatts` record lists the
   contract `addr` (DNS-TXT), or the DID signature + DNS DID record (DNS-DID).
   ([Issuance identity](https://www.openattestation.com/docs/verify-section/issuance-identity)).

**Combining into a verdict.** Each fragment yields `VALID` / `INVALID` / `ERROR` / `SKIPPED`. OA
groups fragments by **type** (`DOCUMENT_INTEGRITY`, `DOCUMENT_STATUS`, `ISSUER_IDENTITY`). The overall
verdict is `VALID` **only if every type group has at least one `VALID` and no `INVALID`**. Any
`INVALID` fragment → overall `INVALID`. A type with only `SKIPPED` → not verified. The verifier is
extensible: you can add custom fragments.

---

## 6. keccak256 confirmation

- **keccak256 is a cryptographic hash function** — the original Keccak (SHA-3 family) submission, the
  variant Ethereum adopted. **It differs from NIST FIPS-202 SHA3-256** only in the domain-separation
  padding byte (Keccak uses `0x01`, FIPS SHA3 uses `0x06`); same sponge/permutation otherwise. EVM's
  `keccak256` opcode and Solidity's `keccak256(bytes)` are this Keccak variant. OA's JS uses
  `js-sha3`'s `keccak256` (NOT `sha3_256`) — matching Ethereum.
- **Over UTF-8 bytes:** OA hashes `keccak256(JSON.stringify(obj))`, where the JS string is encoded as
  **UTF-8 bytes** before hashing. So the actual preimage is the UTF-8 byte sequence of a JSON string
  like `{"score":"ee7f...:number:5"}`. Output is 32 bytes (256 bits), represented as 64 hex chars.
- It is collision- and (second-)preimage-resistant under standard assumptions; security of the whole
  scheme reduces to keccak256's properties **plus** a fully deterministic, unambiguous preimage
  encoding (the part we must get right — §7).

---

## 7. Our variant — drop JSON.stringify, use deterministic byte-canonicalization

### 7.1 Critique of OA's approach

OA hashes `keccak256(JSON.stringify({ path: "uuid:type:value" }))`. Problems for a multi-language
(TS / Rust / Solidity) system:

- **`JSON.stringify` is not a cross-language canonical form.** Key ordering, whitespace, unicode
  escaping (`\uXXXX` vs raw), and number formatting differ between languages and even JSON libs.
  Solidity has no JSON at all. Reproducing JS's exact stringify in Rust/Solidity is fragile.
- **JS number semantics leak in.** `String(value)` for numbers uses JS float formatting
  (`1e21`, `-0`, `5.0` → `5`, precision limits beyond 2^53). Not portable.
- **`typeof`-derived type tags** are JS-specific (`undefined`, `object`→`null` special-case).
- **Sorting**: OA sorts hex strings (`sortBy`) at digest level but raw bytes (`Buffer.compare`) at
  tree level — two different orderings. We must pick **one** and apply it consistently.

The good parts to keep: one leaf per field; per-value random salt; commutative (sorted-pair) merkle
tree so verification needn't track left/right; obfuscation = keep leaf hash, drop cleartext.

### 7.2 Recommended canonical leaf encoding (language-agnostic)

Define a leaf as the tuple `(keyPath, salt, typeTag, value)`. Serialize to a byte string with an
unambiguous, length-prefixed (TLV-style) framing so no field boundary is ambiguous, then keccak256.
**Avoid any text delimiter that can also appear inside a field** (the reason OA's `:`-split has to
`join(":")` back the value).

**Algorithm `encodeLeaf` (produces the 32-byte leaf hash):**

```
encodeLeaf(keyPath: string, salt: 16 bytes, type: u8, valueBytes: bytes) -> bytes32:
    let kp   = utf8(NFC(keyPath))                 # normalize key path (see pitfalls)
    let out  = concat(
        u32_be(len(kp)),  kp,                     # length-prefixed key path
        salt,                                     # exactly 16 bytes (raw, not hex)
        [type],                                   # 1 byte type tag (enum below)
        u32_be(len(valueBytes)), valueBytes       # length-prefixed value
    )
    return keccak256(out)
```

This is exactly reproducible in:
- **Solidity:** `keccak256(abi.encodePacked(uint32(kp.length), kp, salt, type, uint32(v.length), v))`
  — or better, `keccak256(abi.encode(keyPath, salt, type, value))` which is itself canonical and
  collision-free by ABI rules (preferred if you can use the same `abi.encode` shape off-chain).
- **TS:** build a `Uint8Array` and `keccak256` it (e.g. `@noble/hashes/sha3` `keccak_256`).
- **Rust:** `tiny-keccak`/`sha3` Keccak256 over the same byte vector.

> Recommendation: **standardize on `abi.encode(keyPath, salt, typeTag, value)`** as the canonical
> preimage. It is the one encoding all three ecosystems can produce identically with audited libs
> (`ethers.AbiCoder` in TS, `alloy`/`ethabi` in Rust, native in Solidity), and it is length-aware so
> there is no second-preimage ambiguity between fields. Use this instead of any string concatenation.

**Type tag enum (1 byte), with canonical value encodings:**

| tag | type | value bytes (canonical) |
|---|---|---|
| `0x00` | null | empty |
| `0x01` | bool | single byte `0x00`/`0x01` |
| `0x02` | string | UTF-8 of **NFC-normalized** string |
| `0x03` | integer | decimal ASCII of a big-integer, no leading zeros, leading `-` for negatives, `0` is `"0"` (no `-0`) |
| `0x04` | decimal/number | **forbid raw floats**; require a decimal string in a fixed normal form (see pitfalls) or store as integer + scale |
| `0x05` | bytes | raw bytes |

### 7.3 Salt

- Use **16 random bytes** from a CSPRNG per value (128-bit; equal to a UUID's entropy, more than
  enough to defeat guessing of redacted low-entropy fields). Store/transport as lowercase hex or
  base64url. Hash the **raw 16 bytes**, not the hex text, so encoding choices don't affect the hash.
- Do **not** reuse salts across fields or documents.

### 7.4 Merkle build rules (pick one ordering, document it)

```
buildRoot(leafHashes: bytes32[]) -> bytes32:
    if leafHashes is empty: return 0x00..00 (define + reject empty docs)
    sort leafHashes ascending by raw 32-byte value (bytewise/lexicographic)   # ONE ordering everywhere
    layer = leafHashes
    while layer.length > 1:
        next = []
        for i in 0..layer.length step 2:
            if i+1 < layer.length:
                next.push( keccak256( sortPair(layer[i], layer[i+1]) ) )      # sorted pair -> commutative
            else:
                next.push( layer[i] )                                         # promote lone odd node
        layer = next
    return layer[0]

sortPair(a,b): return a <= b ? concat(a,b) : concat(b,a)                       # bytewise compare
```

- **Sort leaves once, bytewise**, before building (matches OA's `hashArray`/`Buffer.compare`). Do not
  also use a different ordering for the digest — there is no separate "digest of sorted hex array"
  stage in our design; the merkle root *is* the document hash.
- **Sorted-pair hashing** makes nodes commutative → a proof is just an unordered sibling set and
  `checkProof` reduces with the same `sortPair` (no left/right bit needed). This is simpler to verify
  in Solidity (`OpenZeppelin MerkleProof` already assumes sorted pairs).
- **Odd layer**: promote the lone node unchanged (do **not** duplicate it — duplication enables a
  known forgery where a tree with a duplicated last leaf has the same root).
- For a single document we can either (a) define root = the single leaf hash, or (b) keep targetHash
  = root of the per-field tree and an outer batch tree like OA. Recommended: **the merkle root over
  the document's field-leaves is the on-chain `bytes32`**; batching multiple documents is an optional
  outer tree using the same rules.

### 7.5 Pitfalls to nail down (and our decisions)

1. **Unicode normalization.** Different sources may produce NFC vs NFD for the same visual string,
   yielding different bytes/hashes. **Decision: NFC-normalize every string (and key path) before
   encoding.** Reject strings that are not already NFC at ingest, or normalize and record that we did.
   (Solidity cannot normalize — so normalization must happen off-chain and the on-chain side simply
   hashes the bytes it is given; the *issuer* is responsible for NFC.)
2. **Number encoding.** JS floats are not portable. **Decision: no native floats in leaves.** Encode
   integers as decimal-ASCII big-integers (no leading zeros, no `-0`); encode fractional values as a
   fixed-form decimal string (no exponent, no trailing zeros, single leading zero `0.x`) **or** as
   `{ value: bigint, scale: u8 }`. Forbid `NaN`/`Infinity`.
3. **String vs number ambiguity.** The type tag byte prevents `"5"` (string) and `5` (int) from
   colliding — they have different tags and the length-prefix differs. Keep tags mandatory.
4. **Field-boundary / second-preimage within a leaf.** Plain concatenation (`uuid:type:value`) is
   ambiguous if delimiters appear in data. **Length-prefix or `abi.encode` every field** so no two
   distinct `(key,salt,value)` tuples produce the same bytes.
5. **Second-preimage across leaf vs internal node (the classic merkle attack).** An attacker could
   try to present an internal node hash as if it were a leaf. **Decision: domain-separate** leaf and
   node hashing, e.g. prefix leaf preimages with `0x00` and node preimages with `0x01`
   (`keccak256(0x00 ‖ leafPreimage)` vs `keccak256(0x01 ‖ sortPair(l,r))`). OA does not do this; we
   should, because we control all three implementations.
6. **Sort order consistency.** Use **one** comparator (unsigned bytewise over the 32-byte hash)
   everywhere: leaf pre-sort, pair sort, and proof reduction. Document it as "ascending bytewise".
7. **Empty document / empty layer.** Define behavior: reject zero-leaf documents; never let root be a
   predictable constant that could be issued accidentally. The contract already rejects `0x0`.
8. **Key path format.** Fix the path grammar (e.g. dot for object keys, `[i]` for arrays) and forbid
   those metacharacters inside keys (OA already rejects `.`, `[`, `]` in keys). Normalize array
   indices as base-10 with no leading zeros.
9. **Hex case.** When comparing contract addresses (DNS-TXT) and when transporting hashes, compare
   case-insensitively / store lowercase; never let hex case affect a hash (we hash raw bytes, so it
   won't, as long as we don't hash hex text).

### 7.6 DNS-TXT format we should adopt

Keep OA's proven shape, namespaced to our project, and include the chain id explicitly:

```
TXT   dogtag net=ethereum chainId=1 addr=0x<documentStoreAddress>
# optional: version=1 to allow format evolution
```

Verification: read `(addr, chainId)` from the document's issuer block + the claimed domain; resolve
TXT over DoH; require a `dogtag` record where `addr` (case-insensitive) and `chainId` match the
contract that issued the root.

### 7.7 Contract interface we should adopt (DocumentStore / issuer)

Start from the legacy simple model, add AccessControl roles, keep on-chain merkle verification
optional. Recommended Solidity surface:

```solidity
interface IDogtagDocumentStore {
  event DocumentIssued(bytes32 indexed root);
  event DocumentRevoked(bytes32 indexed root);

  // roles
  function ISSUER_ROLE()  external view returns (bytes32);   // keccak256("ISSUER_ROLE")
  function REVOKER_ROLE() external view returns (bytes32);   // keccak256("REVOKER_ROLE")

  function name() external view returns (string memory);

  // issuance (root = our merkle root, bytes32)
  function issue(bytes32 root) external;                     // onlyRole(ISSUER_ROLE); revert if already issued
  function bulkIssue(bytes32[] calldata roots) external;     // gas-batched

  // revocation
  function revoke(bytes32 root) external;                    // onlyRole(REVOKER_ROLE)
  function bulkRevoke(bytes32[] calldata roots) external;

  // status (view)
  function isIssued(bytes32 root) external view returns (bool);
  function isRevoked(bytes32 root) external view returns (bool);
  function isActive(bytes32 root) external view returns (bool);   // issued && !revoked
}
```

- Storage: `mapping(bytes32 => bool) issued;` `mapping(bytes32 => bool) revoked;`
- Access control: OpenZeppelin `AccessControl`; `DEFAULT_ADMIN_ROLE` + `ISSUER_ROLE` + `REVOKER_ROLE`
  granted to the deployer/owner at init.
- Deployment: a `DocumentStoreFactory` using CREATE2 for deterministic addresses, emitting the new
  store address and granting roles to the requested owner.
- For transferable documents later: an ERC-721 `TokenRegistry` where `tokenId = uint256(root)`.

---

## Source index

- OA monorepo: <https://github.com/Open-Attestation/open-attestation>
  - salt: <https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/salt.ts>
  - digest: <https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/digest.ts>
  - obfuscate: <https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/obfuscate.ts>
  - wrap: <https://github.com/Open-Attestation/open-attestation/blob/master/src/2.0/wrap.ts>
  - merkle: <https://github.com/Open-Attestation/open-attestation/blob/master/src/shared/merkle/merkle.ts>
  - utils: <https://github.com/Open-Attestation/open-attestation/blob/master/src/shared/utils/utils.ts>
- document-store contracts: <https://github.com/Open-Attestation/document-store>
- oa-verify: <https://github.com/Open-Attestation/oa-verify>
- dnsprove: <https://github.com/Open-Attestation/dnsprove>
- ADR DNS-TXT: <https://github.com/Open-Attestation/adr/blob/master/decentralized_identity_proof_DNS-TXT.md>
- ADR DID issuance: <https://github.com/Open-Attestation/adr/blob/master/issuing_using_did.md>
- Docs — integrity: <https://www.openattestation.com/docs/verify-section/document-integrity>
- Docs — issuance status: <https://www.openattestation.com/docs/verify-section/issuance-status>
- Docs — issuance identity: <https://www.openattestation.com/docs/verify-section/issuance-identity>
- Docs — DNS config: <https://www.openattestation.com/docs/integrator-section/verifiable-document/ethereum/dns-proof/>
