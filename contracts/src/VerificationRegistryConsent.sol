// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {
    AccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

/// @notice The frozen-ceremony consent verifier (`Groth16VerifierConsent`), pub = 7 signals.
interface IGroth16VerifierConsent {
    function verifyProof(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256[7] calldata pub
    ) external view returns (bool);
}

/// @notice The `ProviderRegistry` authority core, asked one question: may this relayer submit a
/// verification for this purpose? `canVerify` reads the VERIFY axis directly and takes the purpose
/// label rather than a pre-hashed key, so this registry never re-derives the key itself — a
/// hand-mirrored `keccak256(abi.encode("VERIFY:", purpose))` here would be a second source of truth
/// that silently refuses every relayer the moment it drifts from the core's own derivation.
interface IProviderRegistry {
    function canVerify(bytes32 purpose, address relayer) external view returns (bool);
}

interface IDogTagIssuer {
    function isValid(bytes32 root) external view returns (bool);
}

/// @notice Owner-hidden verification reads `profileRoot` for the tag<->root binding and `ownerOf` for
/// token EXISTENCE only - never for owner IDENTITY (D1: the tag is minted to a neutral custodian, so
/// `ownerOf` carries no owner meaning and must never gate verification by comparison).
interface IDogTagSBT {
    function profileRoot(uint256 tokenId) external view returns (bytes32);
    function ownerOf(uint256 tokenId) external view returns (address);
}

/// @notice The `DogTagIssuerFactory` root index — `rootIssuer[R]` is write-once and writable only from
/// inside one of that factory's own clones, which is what makes it usable as provenance.
/// @dev This reference is `immutable`, so the factory it names is the one this registry resolves roots
/// through for its whole life. Replacing the factory means replacing this registry too; that is the
/// deliberate price of the pointer not being rewritable by anyone, ever.
interface IRootIndex {
    function rootIssuer(bytes32 root) external view returns (address);
}

/// @title VerificationRegistryConsent — owner-blind proof-of-verification.
/// @notice Records a verification from a `DogTagConsent` (circuits/consent.circom) Groth16 proof that a
/// HIDDEN owner consented to a DISCLOSED relayer for a DISCLOSED purpose. The owner never appears —
/// not in the public signals, not in storage, not in the event.
///
/// The registry verifies against the frozen VK, binds `R == profileRoot(dogTagId)`, consumes the
/// nullifier, emits an owner-blind `Verified`, and carries NO owner-IDENTITY checks - no
/// `ownerOf == subject`, no `keyOf`.
/// (`ownerOf` IS called, but purely as a token-existence gate whose return value is discarded; see the
/// burn note in `recordVerificationZK`.)
///
/// @dev The owner-blind model structurally removes three surfaces an owner-revealing one would need,
/// and each absence is load-bearing rather than incidental:
///   - NO ECDSA `recordVerification` path: such a path COMPARES `ownerOf(dogTagId) == subject`, and under D1
///     (custodial mint) `ownerOf` resolves to the custodian and can never match a real owner. It is the
///     owner-identity COMPARISON that is structurally dead here, not the `ownerOf` read itself (which
///     this registry still makes, value discarded, as an existence gate). Consent is proven in-ZK or
///     not at all.
///   - NO consent-key registry (D2: the consent key lives INSIDE the tree, so there is no `keyOf` to
///     ask and no per-owner mapping to leak).
///   - NO on-chain Poseidon6: deriving the nullifier on-chain would need `subject`; here it
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
    // REDUCED mod r, NOT the raw `keccak256("SERVICE_ATTESTATION")`. `recordType` is
    // a public SIGNAL here, so it is always < r, while the raw keccak (0xa757...ed43) EXCEEDS r — comparing
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

    IProviderRegistry public immutable providerRegistry;
    IDogTagSBT public immutable sbt;
    IRootIndex public immutable rootIndex;

    IGroth16VerifierConsent public zkVerifier;
    IGroth16VerifierConsent public pendingZkVerifier;
    uint256 public zkVerifierEta;

    mapping(bytes32 => bool) public consumed;
    bool public restrictToApprovedRelayers = true;

    /// @notice Owner-blind (spec §"Owner-blind events"): `subject` is GONE — emitting it would undo the
    /// circuit's whole purpose. `deadline` replaces it so a consumer can still bound the consent window.
    /// @dev The signature contains no owner/subject field; its topic0 therefore commits to the
    /// owner-hidden event shape expected by consumers.
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

    constructor(address core, address sbt_, address zk, address ridx, address admin)
        AccessControlDefaultAdminRules(2 days, admin)
    {
        require(
            core != address(0) && sbt_ != address(0) && zk != address(0) && ridx != address(0), "zero"
        );
        providerRegistry = IProviderRegistry(core);
        sbt = IDogTagSBT(sbt_);
        zkVerifier = IGroth16VerifierConsent(zk);
        rootIndex = IRootIndex(ridx);
    }

    /// @notice Owner-hidden ZK verify:
    /// pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline].
    /// @dev `recordType`/`deadline` are PUBLIC SIGNALS, rather than unbound calldata arguments,
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
        // L1: `relayer` is the only address-typed signal (`subject` is absent). It must fit in
        // 160 bits so the `uint160` narrowing below cannot alias msg.sender via `pub = addr + k·2^160`.
        require(pub[P_RELAYER] < TWO_POW_160, "addr range");

        require(block.timestamp <= pub[P_DEADLINE], "expired");
        require(pub[P_RECORDTYPE] != SERVICE_ATTESTATION_FIELD, "art9"); // §11.9(h)
        require(address(uint160(pub[P_RELAYER])) == msg.sender, "not relayer");
        if (restrictToApprovedRelayers) {
            require(providerRegistry.canVerify(bytes32(pub[P_PURPOSE]), msg.sender), "!verify-wl");
        }

        // The owner-hidden binding: the proof's root must be THIS tag's on-chain root. The
        // circuit deliberately does NOT bind dogTagId <-> R, so this is the ONLY place it is checked.
        // Without it a prover could fold a tree they fully control and consent as any dogTagId.
        require(bytes32(pub[P_ROOT]) == sbt.profileRoot(pub[P_DOGTAGID]), "R !profileRoot");

        // Token-existence gate - fails closed on burn/GDPR-erasure. `DogTagSBTConsent.burn` (DEFAULT_ADMIN,
        // erasure-only) calls `_burn` but never clears `profileRoot[id]`, and `profileRoot` is a plain
        // mapping read that survives the burn - so the binding above still passes for an erased tag.
        // Without this call an erased tag keeps emitting `Verified` until someone SEPARATELY calls
        // `revoke(R)`. The explicit existence read preserves this erasure gate without comparing an owner.
        // OWNER-BLIND: existence ONLY. OZ `ownerOf` is `_requireOwned`, so it REVERTS
        // (ERC721NonexistentToken) on a burned token - that revert IS the check. The return value is
        // DISCARDED and MUST NEVER be compared to anything: under D1 it is the neutral custodian, so it
        // says nothing about the owner. Comparing it would reintroduce exactly what this model removes.
        sbt.ownerOf(pub[P_DOGTAGID]);

        require(zkVerifier.verifyProof(a, b, c, pub), "bad proof");

        // D5: the nullifier is a PUBLIC SIGNAL bound to the hidden ownerSecret + consentNonce — never
        // derived on-chain from `subject`, which this model does not expose. Replaying one
        // signed consent repeats its nullifier and is rejected; a fresh nonce mints a new one and passes.
        bytes32 nf = bytes32(pub[P_NULLIFIER]);
        require(!consumed[nf], "replayed");
        consumed[nf] = true;

        // Revocation (§11.10(a)): resolve the issuing clone FROM the root and re-check validity, so a
        // revoked tag stops verifying.
        // NOTE (M5): issuance MUST still `issue(R)` the per-tag root into a DogTagIssuer clone, not only
        // `mintCustodial`, or every verification reverts here as "unknown root".
        address clone = rootIndex.rootIssuer(bytes32(pub[P_ROOT]));
        require(clone != address(0), "unknown root");
        require(IDogTagIssuer(clone).isValid(bytes32(pub[P_ROOT])), "cred !valid");

        emit Verified(
            pub[P_DOGTAGID], msg.sender, bytes32(pub[P_PURPOSE]), nf, pub[P_DEADLINE], block.timestamp
        );
    }

    // ---- admin ----
    function setRelayerRestriction(bool on) external onlyRole(DEFAULT_ADMIN_ROLE) {
        restrictToApprovedRelayers = on;
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
