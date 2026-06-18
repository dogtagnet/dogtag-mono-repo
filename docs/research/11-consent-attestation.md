# 11 — Consent Artifact & On-Chain Proof-of-Verification Attestation

**Status:** Research + design, 2026-06-17.
**Scope:** The EIP-712 consent a pet owner signs when a groomer/vet (verifier/relayer) validates a record, and the on-chain "proof of verification" attestation the relayer submits. Two on-chain paths: a **normal (non-ZK)** path and a **Groth16 ZK** path (public signals `[dogTagId, relayer, subject]`). Designed against `architecture.md` §4 (DogTagSBT / IssuerRegistry / DogTagIssuer), §5 (verification pillars), §13.6 (EIP-712 `recover` precedent), and `research/07-legal-privacy.md` (on-chain personal-data constraints).

> Chain: ROAX, EVM, `chainId = 135`, native gas `PLASMA`. EIP-712 domain therefore pins `chainId:135`. This mirrors the existing `Claim` typed-data used by `DogTagSBT.recover()` (implementation.md §11.7(a)) — we follow the same `EIP712` + `_hashTypedDataV4` + `ECDSA.recover` shape so there is one canonical signing convention across contracts.

---

## 0. The flow being designed

```
1. Groomer scans the pet owner's QR (user→business JWT, audience dogtag-business, §7)
   → groomer backend fetches the wrapped doc / DogTag SBT reference (dogTagId, recordType, merkleRoot).
2. Groomer backend runs the 3 authenticity pillars (integrity + DogTagIssuer.isValid + DNS) — §5.
3. The OWNER signs an EIP-712 VerificationConsent in their mobile wallet (MPC/BIP-39, §10),
   binding the groomer's wallet (relayer) + a nonce + deadline. Owner pays NO gas.
4. Groomer backend EITHER
     (a) forwards {consent, userSig} as-is (normal path), OR
     (b) builds a Groth16 proof whose public signals are [dogTagId, relayer, subject].
5. Groomer (the relayer) submits the tx and pays PLASMA gas:
     (a) VerificationRegistry.recordVerification(consent, userSig), OR
     (b) VerificationRegistry.recordVerificationZK(proof, publicSignals).
6. Contract verifies, consumes a one-time nullifier, emits Verified(...).
```

The consent is what makes this **owner-authorized**: a groomer cannot record "I verified this user" without the user's fresh, relayer-bound signature. The relayer is bound **into** the signature so the groomer who collected consent is the only party who can spend it.

---

## 1. EIP-712 consent typed-data — `VerificationConsent`

### 1.1 Domain

```solidity
EIP712("DogTag", "1")        // name="DogTag", version="1"
// domainSeparator = keccak256(abi.encode(
//   keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
//   keccak256("DogTag"), keccak256("1"),
//   135,                       // chainId — ROAX
//   address(VerificationRegistry)   // verifyingContract
// ))
```

- `name:"DogTag"`, `version:"1"`, `chainId:135`, `verifyingContract = VerificationRegistry`.
- **`verifyingContract` MUST be the `VerificationRegistry`** (not `DogTagSBT`). A consent is meaningful only for *this* registry; a different contract that re-used `name/version` would compute a different `domainSeparator` and the signature would not validate there. This is the first anti-cross-contract guard.
- `chainId:135` pins the signature to ROAX; a fork or a different EVM chain produces a different separator → no cross-chain replay (EIP-712 §domain; ERC-7964 only matters if we *want* cross-chain, which we do not).
- Use the **distinct verb in the type string** (`VerificationConsent`) so this signature can never be confused with the SBT `Claim` signature even though both share the `DogTag` domain name (different `verifyingContract` already separates them; the distinct typehash is defence-in-depth).

### 1.2 Struct

```solidity
struct VerificationConsent {
    uint256 dogTagId;        // the pet's SBT tokenId being verified
    bytes32 recordType;      // keccak256("VACCINATION") | keccak256("DOG_PROFILE") (architecture §3.6/§13.4)
    bytes32 credentialRoot;  // the wrapped-doc merkleRoot being attested (commitment to WHAT was verified)
    address relayer;         // groomer/vet wallet that will submit the tx — MANDATORY, msg.sender must equal this
    address subject;         // the owner's per-pet wallet address — must equal ecrecover(consent)
    uint256 nonce;           // per-subject monotonic nonce (replay scope)
    uint256 deadline;        // unix seconds; tx reverts after this
}

bytes32 constant VERIFICATION_CONSENT_TYPEHASH = keccak256(
  "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 credentialRoot,address relayer,address subject,uint256 nonce,uint256 deadline)"
);
```

Field rationale:

| Field | Binds | Why |
|---|---|---|
| `dogTagId` | which pet | Anchors the consent to one SBT; the event keys on it. |
| `recordType` | what kind of check | `keccak256(label)` per §13.4; lets a verifier prove "vaccination status checked" vs "identity checked" without leaking detail. |
| `credentialRoot` | which credential | A `bytes32` Merkle root = a **commitment** to the exact wrapped doc verified (not the cleartext). Lets the contract optionally cross-check `DogTagIssuer.isValid(credentialRoot)` (§4.4) and lets a later auditor prove *this* root was the thing attested. For `DOG_PROFILE` this is the SBT `profileRoot`. |
| `relayer` | **who may submit** | **Mandatory.** `require(msg.sender == consent.relayer)`. The owner is consenting to *this specific groomer* recording the verification — nobody else. |
| `subject` | who consents | `require(ecrecover(consent) == consent.subject)`. Also lets the contract optionally check `DogTagSBT.ownerOf(dogTagId) == subject` (the §5 ownership pillar, here used as a real gate because the subject claims to be the owner). |
| `nonce` | one-time-use scope | Per-`subject` counter; the contract increments/consumes. |
| `deadline` | freshness | Short-lived (suggest 5–15 min, matching the QR JWT's 180 s spirit — generous enough for the owner to tap "approve" and the groomer to build a proof + submit). |

### 1.3 How `relayer + nonce + deadline` defeat replay and cross-verifier reuse

- **Cross-verifier reuse (the key threat).** Without `relayer` in the signed payload, a consent collected by groomer A could be replayed by groomer B (or an eavesdropper) to fabricate "B verified this user." Because `relayer` is *inside* the signed struct **and** the contract enforces `msg.sender == consent.relayer`, only the wallet the owner named can spend the consent. Groomer B's submission reverts (`msg.sender != relayer`), and B cannot edit `relayer` because that would invalidate `userSig`. This is the same "bind the executor into the signed message" hardening already used by `DogTagSBT.recover()` (binds `newOwner`).
- **Replay (same verifier, twice).** `nonce` is consumed (per-`subject`) and `deadline` expires. Even within the deadline window, the consumed nonce makes the second submission revert. So groomer A cannot re-bill / re-record the same consent, and cannot record it N times to inflate a "verifications" count.
- **Stale-credential replay.** `credentialRoot` + optional `DogTagIssuer.isValid(credentialRoot)` mean a consent over a *since-revoked* credential can be rejected at submit time even if signed earlier.
- **Cross-contract / cross-chain.** Handled by the EIP-712 domain (`verifyingContract` + `chainId:135`) — a consent cannot be replayed against another contract or chain.

EIP-712 itself provides **no** replay protection — the application must add nonce + deadline (per the EIP-712 spec and OZ guidance). We do.

---

## 2. `VerificationRegistry` contract design

A **new, dedicated contract** (not folded into `DogTagSBT` or `DogTagIssuer`, which are intentionally immutable anchoring contracts). It is `EIP712` + reads `IssuerRegistry` / `DogTagSBT` / `DogTagIssuer` for optional cross-checks, and holds the Groth16 verifier address for the ZK path.

### 2.1 Interface (both paths)

```solidity
// SPDX-License-Identifier: MIT
pragma solidity 0.8.x; // evm_version = paris (architecture §13.1 M-4)

import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {ECDSA}  from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {AccessControlDefaultAdminRules} from
    "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

interface IGroth16Verifier {
    // snarkjs-generated verifier; pubSignals order = [dogTagId, relayer, subject, nullifier]
    function verifyProof(
        uint[2] calldata a, uint[2][2] calldata b, uint[2] calldata c,
        uint[4] calldata pubSignals
    ) external view returns (bool);
}

interface IIssuerRegistry { function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool); }
interface IDogTagIssuer  { function isValid(bytes32 root) external view returns (bool); }
interface IDogTagSBT     { function ownerOf(uint256 id) external view returns (address);
                           function status(uint256 id) external view returns (uint8); }

contract VerificationRegistry is EIP712, AccessControlDefaultAdminRules {

    struct VerificationConsent {
        uint256 dogTagId;
        bytes32 recordType;
        bytes32 credentialRoot;
        address relayer;
        address subject;
        uint256 nonce;
        uint256 deadline;
    }

    bytes32 public constant VERIFICATION_CONSENT_TYPEHASH = keccak256(
      "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 credentialRoot,address relayer,address subject,uint256 nonce,uint256 deadline)"
    );

    IIssuerRegistry public immutable issuerRegistry;
    IDogTagSBT      public immutable sbt;
    IGroth16Verifier public immutable zkVerifier;
    // recordType => DogTagIssuer clone (for optional isValid cross-check); 0 = skip
    mapping(bytes32 => address) public issuerFor;

    // ---- replay / nullifier state ----
    mapping(address => uint256) public nonces;        // normal path: per-subject monotonic nonce
    mapping(bytes32 => bool)    public consumed;      // BOTH paths: one-time nullifier set

    // ---- config: who may submit ----
    bool public restrictToWhitelistedRelayers;        // if true, relayer must be a whitelisted issuer signer

    event Verified(
        uint256 indexed dogTagId,
        address indexed relayer,
        address indexed subject,
        bytes32 recordType,
        bytes32 credentialRoot,   // 0x0 on ZK path (private)
        bytes32 nullifier,
        uint256 timestamp,
        bool    zk                // false = normal path, true = ZK path
    );

    constructor(address issuerRegistry_, address sbt_, address zkVerifier_, address admin_)
        EIP712("DogTag", "1")
        AccessControlDefaultAdminRules(2 days, admin_)
    { issuerRegistry = IIssuerRegistry(issuerRegistry_); sbt = IDogTagSBT(sbt_); zkVerifier = IGroth16Verifier(zkVerifier_); }

    // =========================================================================
    // (a) NORMAL (non-ZK) PATH
    // =========================================================================
    function recordVerification(VerificationConsent calldata c, bytes calldata userSig) external {
        // freshness + relayer binding
        require(block.timestamp <= c.deadline,        "expired");
        require(msg.sender == c.relayer,              "not relayer");           // relayer bound INTO consent
        require(c.nonce == nonces[c.subject],         "bad nonce");

        // recover signer over the EIP-712 digest; must be the consenting subject
        bytes32 digest = _hashTypedDataV4(keccak256(abi.encode(
            VERIFICATION_CONSENT_TYPEHASH,
            c.dogTagId, c.recordType, c.credentialRoot, c.relayer, c.subject, c.nonce, c.deadline
        )));
        require(ECDSA.recover(digest, userSig) == c.subject, "bad sig");

        // optional but recommended cross-checks
        if (restrictToWhitelistedRelayers)
            require(issuerRegistry.isWhitelistedFor(c.recordType, msg.sender), "relayer not issuer");
        require(sbt.ownerOf(c.dogTagId) == c.subject,        "subject !owner");  // §5 ownership pillar as a real gate
        address iss = issuerFor[c.recordType];
        if (iss != address(0)) require(IDogTagIssuer(iss).isValid(c.credentialRoot), "cred !valid");

        // one-time nullifier (per consent) — see §5
        bytes32 nf = keccak256(abi.encode(c.subject, c.dogTagId, c.recordType, c.nonce));
        require(!consumed[nf], "replayed");
        consumed[nf] = true;
        nonces[c.subject] = c.nonce + 1;

        emit Verified(c.dogTagId, c.relayer, c.subject, c.recordType, c.credentialRoot, nf, block.timestamp, false);
    }

    // =========================================================================
    // (b) ZK PATH — public signals = [dogTagId, relayer, subject, nullifier]
    // =========================================================================
    function recordVerificationZK(
        uint[2] calldata a, uint[2][2] calldata b, uint[2] calldata c_,
        uint[4] calldata pub                 // [dogTagId, relayer, subject, nullifier]
    ) external {
        uint256 dogTagId = pub[0];
        address relayer  = address(uint160(pub[1]));
        address subject  = address(uint160(pub[2]));
        bytes32 nf       = bytes32(pub[3]);

        require(relayer == msg.sender, "not relayer");          // relayer is a PUBLIC SIGNAL, bound to msg.sender
        require(!consumed[nf],         "replayed");
        require(zkVerifier.verifyProof(a, b, c_, pub), "bad proof");

        if (restrictToWhitelistedRelayers)
            require(issuerRegistry.isWhitelistedFor(bytes32(0), msg.sender), "relayer not issuer"); // recordType private in ZK
        consumed[nf] = true;

        // credentialRoot is NOT published on the ZK path (privacy); recordType folded into the circuit/nullifier
        emit Verified(dogTagId, relayer, subject, bytes32(0), bytes32(0), nf, block.timestamp, true);
    }

    // ---- admin ----
    function setIssuerFor(bytes32 recordType, address issuer) external onlyRole(DEFAULT_ADMIN_ROLE) { issuerFor[recordType] = issuer; }
    function setRelayerRestriction(bool on)                   external onlyRole(DEFAULT_ADMIN_ROLE) { restrictToWhitelistedRelayers = on; }
    function eip712Domain() external view returns (...) { /* OZ EIP712 exposes for client tooling */ }
}
```

### 2.2 Who can call

Three plausible policies; **recommendation = "any address, but it must be the consent's `relayer`/public-signal relayer, with an optional whitelist toggle":**

- **`msg.sender == relayer` is always enforced** on both paths — so the caller is whoever the owner named (normal) or whoever the proof committed to (ZK). This alone prevents arbitrary third parties from recording verifications on a user's behalf.
- **`restrictToWhitelistedRelayers` toggle** (admin-set, default `true` for production). When on, the relayer must be a whitelisted issuer signer (`IssuerRegistry.isWhitelistedFor`). This means **only accredited vets/groomers** can write verification records — keeping the registry meaningful and preventing spam/griefing from random wallets. When off (e.g. an open ecosystem of verifiers), any wallet can be a relayer but still only with a valid owner consent.
- We deliberately **do not** require the relayer to be the issuer of the credential — a groomer verifying a *vet-issued* vaccination is the whole point. The whitelist (when on) gates "is this a real DogTag business," not "did you issue this credential."

Gas is paid by `msg.sender` = the relayer (the groomer), satisfying the "groomer pays gas, owner doesn't" requirement.

### 2.3 Design notes / hardening

- **Mutable vs immutable:** unlike `DogTagIssuer` (immutable clones), `VerificationRegistry` carries admin config (`issuerFor`, toggle, possibly a new `zkVerifier` if the circuit is upgraded). Keep it a single deployed contract under `AccessControlDefaultAdminRules` (two-step admin + delay, per §13.3 H-3). If the Groth16 verifier must be swappable, make `zkVerifier` settable by admin (timelocked) rather than `immutable`.
- **Groth16 footguns (must address in the circuit + verifier, per snarkjs issues #358/#383):**
  1. **Public-signal range checks** — the verifier MUST reject any public signal ≥ the BN254 scalar field modulus; missing this check is a known double-spend vector. Use a current snarkjs verifier that includes the `r` range check.
  2. **Proof malleability** — Groth16 proofs are malleable (a second valid `(a,b,c)` exists for the same public signals). **Therefore the nullifier MUST live in the *public signals*, never be derived from the proof bytes** — our design does this (`pub[3]`), so malleating the proof cannot bypass `consumed[nf]`.
  3. **`recordType`** is private on the ZK path; if a verifier needs per-recordType whitelisting it must either make `recordType` public or fold a fixed recordType into the verification-key (one circuit per recordType). The interface above passes `bytes32(0)` — pick one explicitly at integration.

---

## 3. EAS (Ethereum Attestation Service) — precedent & recommendation

**What EAS is** (per [docs.attest.org](https://docs.attest.org/) and [eas-contracts](https://github.com/ethereum-attestation-service/eas-contracts)): a public-good pair of singleton contracts — a **SchemaRegistry** (register a typed schema once → get a `bytes32` schema UID) and **EAS** (make on-chain or off-chain attestations referencing a schema). Key shapes:

```solidity
struct AttestationRequestData { address recipient; uint64 expirationTime; bool revocable; bytes32 refUID; bytes data; uint256 value; }
struct AttestationRequest      { bytes32 schema; AttestationRequestData data; }
struct DelegatedAttestationRequest { bytes32 schema; AttestationRequestData data; Signature signature; address attester; uint64 deadline; }
function attest(AttestationRequest) external payable returns (bytes32 uid);
function attestByDelegation(DelegatedAttestationRequest) external payable returns (bytes32 uid); // EIP-712, relayer submits, attester signs
function revoke(RevocationRequest) external payable;       // marks revoked, does NOT delete (data stays on-chain forever)
function getAttestation(bytes32 uid) external view returns (Attestation);
```

EAS's **`attestByDelegation`** is *exactly* our relayer pattern: the attester signs an EIP-712 payload (with a `deadline`), and a relayer submits + pays gas. EAS supports **off-chain attestations** (zero gas; the attestation lives only in a URL fragment / off-chain store) and **on-chain** ones, with optional schema **resolvers** (hook contracts).

### 3.1 EAS vs custom `VerificationRegistry`

| Dimension | EAS | Custom `VerificationRegistry` |
|---|---|---|
| **Tooling / portability** | EASScan explorer, SDK, indexers; portable schema UID; ecosystem reuse | None out-of-box; we build indexing |
| **Delegated/relayer** | Built-in `attestByDelegation` (EIP-712 + deadline) | We implement it (done above) — but identical effort |
| **Gas** | Extra indirection (schema lookup, generic `bytes data` ABI-decode, resolver call) → **higher gas** than a purpose-built function | Tight, single-purpose calldata → cheaper |
| **Control** | Schema/revocation semantics fixed by EAS; **the `attester`/`recipient` model doesn't natively express "relayer ≠ subject, relayer bound into the signature, owner is recipient"** | Full control: relayer/subject/nonce/deadline exactly as we need |
| **ZK integration** | EAS has **no native Groth16 path**; you'd wrap a proof inside `bytes data` and verify in a resolver — awkward, and the nullifier/`consumed` logic still has to be custom | First-class: `recordVerificationZK` is the contract |
| **Deployment on ROAX** | EAS is **not deployed on ROAX** (it's on Ethereum L1 + major L2s). We'd have to **deploy EAS ourselves** → we lose the "shared public good / cross-chain portability" benefit entirely, while keeping the generic-overhead cost | We deploy one contract anyway |
| **GDPR** | Same on-chain permanence problem; **revoke ≠ delete** (data stays); plus EASScan **publicly indexes recipient addresses** → *worse* default exposure | We control exactly which fields are emitted (and can prefer the ZK path) |

### 3.2 Recommendation — **custom `VerificationRegistry`, EAS-*informed***

Build the custom contract. Reasons:

1. **EAS isn't on ROAX**, so the headline EAS benefits (shared registry, EASScan, ecosystem indexers, cross-chain schema portability) **do not exist for us** — we'd self-deploy EAS and get the generic-attestation *overhead* with none of the network-effect *upside*.
2. Our requirements (relayer **bound into** the signature + enforced as `msg.sender`, per-subject nonce, one-time nullifier shared between a normal and a **Groth16** path) are **more specific** than EAS's generic `attester/recipient/data` model. Forcing them through EAS means custom resolvers + `bytes data` packing anyway.
3. The **ZK path** is a non-starter on stock EAS.
4. Privacy: we want minimal, deliberate fields emitted and a clear ZK option — easier to reason about in a contract we own.

**Borrow from EAS** (so we don't reinvent good ideas): (a) the `attestByDelegation` EIP-712-with-`deadline` relayer pattern (we already match it); (b) the `revocable`/`refUID` notion — add an optional `revokeVerification(nullifier)` if a verification needs retracting; (c) emit a stable, schema-like event so external indexers can treat `Verified` as a typed attestation. If DogTag ever multi-chains onto a network where EAS *is* deployed and portability matters more than the ZK path, revisit.

---

## 4. Relayer / meta-transaction pattern

The groomer is the relayer: it submits the owner's signed consent and pays PLASMA gas. Options:

| Pattern | What it is | Fit here |
|---|---|---|
| **Plain "relayer submits a signed message"** | The owner signs an EIP-712 struct; the relayer calls a normal function passing `{struct, sig}`; the contract `ecrecover`s and acts on the *recovered* signer, not `msg.sender`. | **✅ Recommended.** Simplest sound pattern. No extra infra, no trusted forwarder, no bundler/EntryPoint. This is exactly `recordVerification(consent, userSig)` and matches EAS `attestByDelegation` and `DogTagSBT.recover()`. |
| **EIP-2771 trusted forwarder** | A forwarder appends the real sender to calldata; the recipient trusts it via `_msgSender()`. | **✗ Overkill / risky.** Adds a trusted forwarder (a malicious/buggy forwarder can spoof `msg.sender`); the standard is for making *arbitrary* calls gasless. We don't need a generic forwarder — we have one specific function and we *want* `msg.sender == relayer` to be meaningful (it's the relayer, not the subject). 2771 would actually fight our design. |
| **ERC-4337 account abstraction** | UserOps via a bundler + EntryPoint + smart-account wallets. | **✗ For this attestation.** Heavy infra (bundler, EntryPoint, paymaster). Reserve 4337/7702 for the **owner's gas-sponsored wallet** story (§13.6(f)) — *issuance/recover* gas — not for this groomer-submitted attestation, where the groomer already has a funded EOA and naturally pays gas. |

### 4.1 Recommendation

**Plain signed-message relay.** The owner signs `VerificationConsent`; the groomer's backend (its own funded ROAX key — the same key custody as backend signing mode, §6.1) submits `recordVerification`/`recordVerificationZK` and pays gas. No forwarder, no EntryPoint.

**The relayer is bound INTO the signed consent** (`consent.relayer`, enforced `== msg.sender`) so it cannot be swapped — this is the security property EIP-2771 *lacks* (2771 lets the forwarder name any sender). On the ZK path the relayer is a **public signal** committed by the proof and likewise checked `== msg.sender`. Either way, the party that paid for and built the submission is provably the party the owner authorized.

---

## 5. Replay & nullifier design

### 5.1 Normal path

- **Per-subject nonce.** `nonces[subject]` is monotonic; `recordVerification` requires `c.nonce == nonces[subject]` and then sets `nonces[subject] = c.nonce + 1`. A consent is single-use; re-submitting reverts on the nonce.
- **Per-consent nullifier.** `nf = keccak256(subject, dogTagId, recordType, nonce)` is recorded in `consumed[nf]`. Belt-and-suspenders with the nonce, and — crucially — it is the **same nullifier mapping the ZK path writes**, so the two paths share one anti-double-record set. A verification recorded once (either path) cannot be recorded again (either path) under the same nullifier.
- **Deadline.** `block.timestamp <= c.deadline` (suggest 5–15 min). Bounds how long a captured consent is live, limiting the window for any mischief and forcing fresh owner approval per visit.

### 5.2 ZK path — alignment

- The circuit **outputs the nullifier as a public signal** (`pub[3]`), computed *inside* the circuit from private inputs (e.g. `nullifier = Poseidon(subjectSecret, dogTagId, recordType, sessionSalt)`), so the relayer cannot forge a nullifier that collides with or dodges an existing one.
- The contract treats `consumed[nf]` identically for both paths. To make a verification fungible across paths (so a groomer can't record it once normally and once via ZK), the **nullifier derivation should be defined to coincide** for the same logical event (same subject/pet/recordType/session). If perfect coincidence is impractical (the ZK nullifier hides `subject`), accept that the two paths produce *different* nullifiers but each path is still independently one-time; document the residual that a single logical visit *could* be recorded once per path, and have the indexer dedupe on `(dogTagId, relayer, day)` if business logic needs strict once-per-visit.
- **Groth16 malleability** (snarkjs #383): because the nullifier is a *public signal*, malleating the proof bytes yields the **same** public signals → the same `nf` → still blocked by `consumed`. This is why the nullifier must never be derived from `(a,b,c)`.

### 5.3 What stops abuse

- **Groomer re-recording the same verification:** nonce consumed + `consumed[nf]` set → second call reverts.
- **A *different* groomer reusing the consent:** `msg.sender == consent.relayer` (normal) / `relayer == pub[1] == msg.sender` (ZK) → the other groomer's `msg.sender` mismatches → revert. They also can't alter `relayer` without breaking `userSig`/the proof.
- **Eavesdropper replay:** same as above — they are not the bound relayer.
- **Expired consent:** `deadline`.
- **Cross-contract/chain:** EIP-712 domain.

---

## 6. Privacy / GDPR of the on-chain verification record

Recording `subject` (userWallet) + `dogTagId` + `relayer` + `recordType` + timestamp per verification creates a **permanent behavioural linkage**: *which user's pet was checked by which groomer/vet, when, and for what*. Under the analysis in `research/07-legal-privacy.md` §4 this is **pseudonymous personal data** (a wallet address reasonably linkable to a person; Recital 26 / ICO: pseudonymisation is still personal data) on an immutable, globally-replicated ledger — squarely in **DPIA scope**, with the Art. 17 erasure and Chapter V transfer tensions of §4.2–§4.3. EASScan-style public indexing would make this *worse* by default (publicly browsable recipient histories). The normal path is **more exposing** than the ZK path because it can also emit `credentialRoot` (a commitment to the exact credential).

### 6.1 Severity ranking of what each path leaks on-chain

| Field | Normal path | ZK path | Note |
|---|---|---|---|
| `dogTagId` | emitted | emitted (public signal) | non-personal id (never `keccak256(microchip)`, §13.6) — but *linkable* to the pet+owner via off-chain data |
| `subject` (userWallet) | emitted | emitted (public signal) | the behavioural-linkage core |
| `relayer` | emitted | emitted | reveals *which business*; usually less sensitive (businesses are public) but builds the "who visited whom" graph |
| `recordType` | emitted | **private** (folded into circuit) | ZK hides whether it was vaccination vs profile |
| `credentialRoot` | **emitted** (optional) | **not emitted** | normal path can pin the exact credential; ZK doesn't |

**Both paths still publish `subject` + `dogTagId`** — so the ZK path, as specified (public signals `[dogTagId, relayer, subject]`), reduces *what* is revealed (no recordType, no credentialRoot) but **does not by itself break the user↔groomer↔pet linkage**. That linkage is the irreducible core of "proof that *this user* was verified by *this groomer*."

### 6.2 Mitigations (recommended)

1. **Fresh per-pet wallet address (already mandated, §11.1/§13.6).** `subject` is the per-pet derived address `m/44'/60'/0'/0/{petIndex}`, not a person's reusable address. This **prevents cross-pet enumeration** (an observer can't roll up one person's *whole* pet history from one address) and is the single most effective on-chain mitigation. **It does help the ZK path too** — but note it does *not* hide that *this pet's* address was verified by *this groomer*; it limits the blast radius to one pet, not the person's portfolio.
2. **Prefer the ZK path for routine third-party verification.** It drops `recordType` and `credentialRoot` from chain. Recommend the ZK path be the default for groomer/airline checks; reserve the normal path for cases where an auditor genuinely needs the on-chain `credentialRoot` commitment.
3. **Make `subject` a commitment/nullifier instead of the raw address where the use-case allows.** This is the strongest mitigation and the natural ZK evolution: instead of publishing `subject = userWallet`, publish only the **nullifier** (a Semaphore-style `Poseidon(secret, scope)`), proving in-circuit that the consenter controls the SBT owner *without revealing which address*. Then on-chain you have `[dogTagId, relayer, nullifier]` — no raw user address. **Trade-off:** you lose the ability to cheaply check `ownerOf(dogTagId) == subject` on-chain (it moves into the circuit as a private constraint), and any business logic that needs the address must get it off-chain. Recommend offering this as **ZK v2** (public signals `[dogTagId, relayer, nullifier]`, `subject` proven privately) once the basic ZK path ships. The `recordVerificationZK` signature above already passes a nullifier as `pub[3]`, so the migration is "stop publishing `pub[2]=subject`, keep `pub[3]=nullifier`."
4. **Minimize emitted fields / don't over-index.** Do not run a public EASScan-style explorer over `Verified`; keep indexing internal. Emit `credentialRoot` on the normal path **only when required** (make it optional/zeroable).
5. **Consent receipt off-chain (GDPR record-keeping).** The richer detail (cleartext recordType, credential, lawful basis, withdrawal) lives in the off-chain `consents`/`consent_receipts` collections (§9), which are deletable. On erasure: crypto-shred those + the per-pet key material; the on-chain `Verified` event remains but, with the per-pet address unlinked from the person off-chain (and `recordType`/`credentialRoot` absent on ZK), the residual on-chain artefact is a `(dogTagId, relayer, nullifier, ts)` tuple that is **far harder to attribute** — consistent with §11.1's "close to erasure, not safe harbour; document residual in the DPIA."
6. **DPIA is mandatory and must explicitly cover this verification-event linkage** — it is *new* on-chain personal data beyond issuance/ownership, and §07 / §11.1 require the DPIA to be refreshed on any change to on-chain fields. Flag it.

### 6.3 Net privacy recommendation

- **Default to the ZK path** for third-party (groomer/vet/airline) verification; it removes `recordType` + `credentialRoot` from chain.
- **Always use the fresh per-pet address** as `subject` (limits enumeration to one pet).
- **Plan ZK v2 = publish a nullifier, not `subject`** — the only design that actually severs the user-address linkage while still proving "the SBT owner consented."
- **The normal path is the more-exposing fallback** for when an on-chain credential commitment is genuinely needed; gate `credentialRoot` emission behind necessity.

---

## 7. Open items

- **ROAX EIP-712 client support** in the mobile MPC/BIP-39 wallets — confirm `eth_signTypedData_v4` works through Privy/MetaMask-Embedded and the BIP-39 path; the UX is a single "Approve verification" tap.
- **Groth16 circuit spec** — define private inputs, the exact nullifier derivation (`Poseidon(...)`), and whether `recordType` is public or per-circuit; pin a snarkjs version whose verifier includes the field-modulus range check (snarkjs #358).
- **Nullifier coincidence across paths** — decide whether the two paths must produce identical nullifiers for the same logical event, or whether indexer-side dedupe is acceptable.
- **`revokeVerification`** — decide if a recorded verification ever needs retraction (EAS `revoke` precedent; marks-not-deletes).
- **DPIA update** — add the verification-event behavioural-linkage to the mandatory DPIA (§07 §8, §11.1).

---

## 8. Sources

- EIP-712 — Typed structured data hashing and signing: https://eips.ethereum.org/EIPS/eip-712
- ERC-7964 — Crosschain EIP-712 Signatures (why we pin chainId): https://eips.ethereum.org/EIPS/eip-7964
- ERC-2612 — Permit (nonce+deadline precedent): https://eips.ethereum.org/EIPS/eip-2612
- OpenZeppelin EIP712 / `_hashTypedDataV4`: https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/utils/cryptography/EIP712.sol
- OpenZeppelin ECDSA: https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/utils/cryptography/ECDSA.sol
- OpenZeppelin Cryptography utils (5.x): https://docs.openzeppelin.com/contracts/5.x/api/utils/cryptography
- Ethereum Attestation Service — docs: https://docs.attest.org/
- EAS — attestations / delegated attestations: https://docs.attest.org/docs/core--concepts/attestations
- EAS contracts (`IEAS.sol`, struct shapes): https://github.com/ethereum-attestation-service/eas-contracts
- EAS FAQ (on-chain immutability, revoke ≠ delete): https://docs.attest.org/docs/quick--start/faqs
- EASScan privacy policy (public recipient indexing): https://easscan.org/privacy
- EIP-2771 vs ERC-4337 (Alchemy): https://www.alchemy.com/overviews/4337-vs-2771
- Meta transactions ERC-2771 (Alchemy): https://www.alchemy.com/overviews/meta-transactions
- snarkjs double-spend / public-signal range check (issue #358): https://github.com/iden3/snarkjs/issues/358
- snarkjs Groth16 malleability → double-spend (issue #383): https://github.com/iden3/snarkjs/issues/383
- Circom proving circuits (Solidity verifier generation): https://docs.circom.io/getting-started/proving-circuits/
- EDPB Guidelines 02/2025 on blockchain & personal data: https://www.edpb.europa.eu/system/files/2025-04/edpb_guidelines_202502_blockchain_en.pdf
- ICO pseudonymisation (salted hash = personal data): https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/data-sharing/anonymisation/pseudonymisation/
