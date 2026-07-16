// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @notice The M3 Level-B consent verifier (`Groth16VerifierConsent`), pub = 7 signals.
interface IGroth16VerifierConsent {
    function verifyProof(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256[7] calldata pub
    ) external view returns (bool);
}

interface IIssuerRegistry {
    function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
}

interface IDogTagIssuer {
    function isValid(bytes32 root) external view returns (bool);
}

/// @notice Level-B reads `profileRoot` for the tag<->root binding and `ownerOf` for token EXISTENCE
/// only - never for owner IDENTITY (D1: the tag is minted to a neutral custodian, so `ownerOf` carries
/// no owner meaning and must never gate verification by comparison).
interface IDogTagSBT {
    function profileRoot(uint256 tokenId) external view returns (bytes32);
    function ownerOf(uint256 tokenId) external view returns (address);
}

interface IRootIndex {
    function rootIssuer(bytes32 root) external view returns (address);
}

/// @title VerificationRegistryConsent — Level-B owner-blind proof-of-verification (M4).
/// @notice Records a verification from a `DogTagConsent` (circuits/consent.circom) Groth16 proof that a
/// HIDDEN owner consented to a DISCLOSED relayer for a DISCLOSED purpose. The owner never appears —
/// not in the public signals, not in storage, not in the event.
///
/// Spec: `dogtag-zkverify-z2/level-b-spec.md` §"Verify (M2 circuit + M4 registry)". This contract is
/// the M4 half: verify vs the new VK, bind `R == profileRoot(dogTagId)`, consume the nullifier, emit an
/// owner-blind `Verified`, and carry NO owner-IDENTITY checks - no `ownerOf == subject`, no `keyOf`.
/// (`ownerOf` IS called, but purely as a token-existence gate whose return value is discarded; see the
/// burn note in `recordVerificationZK`.)
///
/// @dev DELIBERATELY SEPARATE from the Level-A `VerificationRegistry` (still live on ROAX at
/// 0x4E2f0996..., still serving the ECDSA + Level-A ZK flow until M7 cuts the apps over). Level-A is
/// FROZEN, not edited — the same pattern M2 used for `verification.circom` and M3 for
/// `Groth16Verifier`. Differences from Level-A, all forced by the owner-blind model:
///   - NO ECDSA `recordVerification` path: it COMPARES `ownerOf(dogTagId) == subject`, and under D1
///     (custodial mint) `ownerOf` resolves to the custodian and can never match a real owner. It is the
///     owner-identity COMPARISON that is structurally dead here, not the `ownerOf` read itself (which
///     this registry still makes, value discarded, as an existence gate). Level-B consent is proven
///     in-ZK or not at all. Legacy ECDSA verification stays on the Level-A registry.
///   - NO `ConsentKeyRegistry` (D2: the consent key moved INTO the tree, so `keyOf` is retired).
///   - NO on-chain Poseidon6: Level-A derived the nullifier on-chain from `subject`; here the nullifier
///     is a public signal the circuit binds to the hidden `ownerSecret` (D5).
contract VerificationRegistryConsent is AccessControlDefaultAdminRules {
    uint256 internal constant SNARK_SCALAR_FIELD =
        21888242871839275222246405745257275088548364400416034343698204186575808495617; // BN254 r
    // 2^160 — the address space. A field element `< r` can still exceed 2^160, so `uint160(pub[2])`
    // would truncate the high bits. Requiring the address-typed signal `< TWO_POW_160` keeps the
    // on-chain truncation bit-identical to the full field element the proof witnesses (audit L1).
    uint256 internal constant TWO_POW_160 = 1 << 160;
    uint256 internal constant ZK_TIMELOCK = 2 days;

    // Art. 9: SERVICE_ATTESTATION has no on-chain root → NOT verifiable on-chain (§11.9(h)).
    // REDUCED mod r, unlike Level-A's raw `keccak256("SERVICE_ATTESTATION")`. `recordType` is a public
    // SIGNAL here, so it is always < r, while the raw keccak (0xa757...ed43) EXCEEDS r — comparing
    // against the raw constant could never match and would silently make this guard dead code.
    // = keccak256("SERVICE_ATTESTATION") % r; asserted against the raw keccak in the test suite.
    uint256 internal constant SERVICE_ATTESTATION_FIELD =
        10025591956217394737855806998434905929145386518960477508456501950324730293568;

    // Public-signal indices. ORDER IS LOAD-BEARING and fixed by the circuit's output declaration order
    // (circuits/README.consent.md §"Public-signal ordering"); it is frozen with the M3 VK.
    // pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
    uint256 internal constant P_DOGTAGID = 0;
    uint256 internal constant P_PURPOSE = 1;
    uint256 internal constant P_RELAYER = 2;
    uint256 internal constant P_NULLIFIER = 3;
    uint256 internal constant P_ROOT = 4;
    uint256 internal constant P_RECORDTYPE = 5;
    uint256 internal constant P_DEADLINE = 6;

    IIssuerRegistry public immutable issuerRegistry;
    IDogTagSBT public immutable sbt;
    IRootIndex public immutable rootIndex;

    IGroth16VerifierConsent public zkVerifier;
    IGroth16VerifierConsent public pendingZkVerifier;
    uint256 public zkVerifierEta;

    mapping(bytes32 => bool) public consumed;
    bool public restrictToWhitelistedRelayers = true;

    /// @notice Owner-blind (spec §"Owner-blind events"): `subject` is GONE — emitting it would undo the
    /// circuit's whole purpose. `deadline` replaces it so a consumer can still bound the consent window.
    /// @dev Same NAME but a different signature from the Level-A `Verified` (which carried `subject`),
    /// hence a different topic0. The two never collide on one address: this event only ever comes from a
    /// Level-B registry. The oversight indexer learned this topic0 in M8 (additive dual-decode).
    event Verified(
        uint256 indexed dogTagId,
        address indexed relayer,
        bytes32 purpose,
        bytes32 nullifier,
        uint256 deadline,
        uint256 ts
    );
    event ZkVerifierProposed(address indexed verifier, uint256 eta);
    event ZkVerifierUpdated(address indexed verifier);

    constructor(address ir, address sbt_, address zk, address ridx, address admin)
        AccessControlDefaultAdminRules(2 days, admin)
    {
        require(ir != address(0) && sbt_ != address(0) && zk != address(0) && ridx != address(0), "zero");
        issuerRegistry = IIssuerRegistry(ir);
        sbt = IDogTagSBT(sbt_);
        zkVerifier = IGroth16VerifierConsent(zk);
        rootIndex = IRootIndex(ridx);
    }

    function _verifyKey(bytes32 purpose) internal pure returns (bytes32) {
        return keccak256(abi.encode("VERIFY:", purpose));
    }

    /// @notice Level-B ZK verify: pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline].
    /// @dev `recordType`/`deadline` are PUBLIC SIGNALS now (they were unbound calldata args in Level-A),
    /// so both are cryptographically bound to the proof rather than relayer-supplied. `recordType` is
    /// nonetheless PROVER-asserted, not consent-signed (it is not in the EdDSA message `M` nor the
    /// nullifier): only the owner's app can build this proof, so the app — not the relayer — chooses it.
    /// Treat it as a label bound to this proof, NOT as an owner attestation
    /// (circuits/README.consent.md §"`recordType` is prover-asserted").
    function recordVerificationZK(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256[7] calldata pub
    ) external {
        // Range-check EVERY signal before any of them is narrowed or compared (#358).
        for (uint256 i; i < 7; i++) {
            require(pub[i] < SNARK_SCALAR_FIELD, "!field");
        }
        // L1: `relayer` is the only address-typed signal in Level-B (`subject` is gone). It must fit in
        // 160 bits so the `uint160` narrowing below cannot alias msg.sender via `pub = addr + k·2^160`.
        require(pub[P_RELAYER] < TWO_POW_160, "addr range");

        require(block.timestamp <= pub[P_DEADLINE], "expired");
        require(pub[P_RECORDTYPE] != SERVICE_ATTESTATION_FIELD, "art9"); // §11.9(h)
        require(address(uint160(pub[P_RELAYER])) == msg.sender, "not relayer");
        if (restrictToWhitelistedRelayers) {
            require(
                issuerRegistry.isWhitelistedFor(_verifyKey(bytes32(pub[P_PURPOSE])), msg.sender), "!verify-wl"
            );
        }

        // THE Level-B binding (spec §"Verify"): the proof's root must be THIS tag's on-chain root. The
        // circuit deliberately does NOT bind dogTagId <-> R, so this is the ONLY place it is checked.
        // Without it a prover could fold a tree they fully control and consent as any dogTagId.
        require(bytes32(pub[P_ROOT]) == sbt.profileRoot(pub[P_DOGTAGID]), "R !profileRoot");

        // Token-existence gate - fails closed on burn/GDPR-erasure. `DogTagSBT.burn` (DEFAULT_ADMIN,
        // erasure-only) calls `_burn` but never clears `profileRoot[id]`, and `profileRoot` is a plain
        // mapping read that survives the burn - so the binding above still passes for an erased tag.
        // Without this call an erased tag keeps emitting `Verified` until someone SEPARATELY calls
        // `revoke(R)`. Level-A failed closed here only as a side effect of its `ownerOf == subject`
        // check; dropping that comparison (correctly, per D1) dropped the erasure gate with it.
        // OWNER-BLIND: existence ONLY. OZ `ownerOf` is `_requireOwned`, so it REVERTS
        // (ERC721NonexistentToken) on a burned token - that revert IS the check. The return value is
        // DISCARDED and MUST NEVER be compared to anything: under D1 it is the neutral custodian, so it
        // says nothing about the owner. Comparing it would reintroduce exactly what Level-B removes.
        sbt.ownerOf(pub[P_DOGTAGID]);

        require(zkVerifier.verifyProof(a, b, c, pub), "bad proof");

        // D5: the nullifier is a PUBLIC SIGNAL bound to the hidden ownerSecret + consentNonce — never
        // derived on-chain (Level-A hashed it from `subject`, which Level-B does not have). Replaying one
        // signed consent repeats its nullifier and is rejected; a fresh nonce mints a new one and passes.
        bytes32 nf = bytes32(pub[P_NULLIFIER]);
        require(!consumed[nf], "replayed");
        consumed[nf] = true;

        // Revocation (§11.10(a)): resolve the issuing clone FROM the root and re-check validity, so a
        // revoked tag stops verifying. Level-B keeps this Level-A protection unchanged.
        // NOTE (M5): issuance MUST still `issue(R)` the per-tag root into a DogTagIssuer clone, not only
        // `setProfileRoot`, or every Level-B verify reverts here as "unknown root".
        address clone = rootIndex.rootIssuer(bytes32(pub[P_ROOT]));
        require(clone != address(0), "unknown root");
        require(IDogTagIssuer(clone).isValid(bytes32(pub[P_ROOT])), "cred !valid");

        emit Verified(
            pub[P_DOGTAGID], msg.sender, bytes32(pub[P_PURPOSE]), nf, pub[P_DEADLINE], block.timestamp
        );
    }

    // ---- admin ----
    function setRelayerRestriction(bool on) external onlyRole(DEFAULT_ADMIN_ROLE) {
        restrictToWhitelistedRelayers = on;
    }

    /// @notice Real timelock on the verifier swap (§11.10(g)): propose, then execute after ZK_TIMELOCK.
    function proposeZkVerifier(address v) external onlyRole(DEFAULT_ADMIN_ROLE) {
        pendingZkVerifier = IGroth16VerifierConsent(v);
        zkVerifierEta = block.timestamp + ZK_TIMELOCK;
        emit ZkVerifierProposed(v, zkVerifierEta);
    }

    function executeZkVerifier() external onlyRole(DEFAULT_ADMIN_ROLE) {
        require(address(pendingZkVerifier) != address(0), "none");
        require(block.timestamp >= zkVerifierEta, "timelock");
        zkVerifier = pendingZkVerifier;
        pendingZkVerifier = IGroth16VerifierConsent(address(0));
        emit ZkVerifierUpdated(address(zkVerifier));
    }
}
