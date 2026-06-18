// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {AccessControlDefaultAdminRules} from
    "@openzeppelin/contracts/access/extensions/AccessControlDefaultAdminRules.sol";

interface IGroth16Verifier {
    function verifyProof(uint256[2] calldata a, uint256[2][2] calldata b, uint256[2] calldata c, uint256[7] calldata pub)
        external
        view
        returns (bool);
}

interface IIssuerRegistry {
    function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
}

interface IDogTagIssuer {
    function isValid(bytes32 root) external view returns (bool);
}

interface IDogTagSBT {
    function ownerOf(uint256 tokenId) external view returns (address);
}

interface IConsentKeyReg {
    function keyOf(address wallet) external view returns (bytes32);
}

interface IRootIndex {
    function rootIssuer(bytes32 root) external view returns (address);
}

/// @notice circomlib-exact on-chain Poseidon(6) (deployed from circomlibjs `poseidonContract` initcode).
/// poseidon-solidity 0.0.5 ships only T2..T6, so the 6-input (t=7) nullifier uses this deterministic
/// circomlib instantiation — CI-asserted bit-identical to circom/TS/Rust (poseidon-vectors.json).
interface IPoseidon6 {
    function poseidon(uint256[6] calldata inputs) external view returns (uint256);
}

/// @title VerificationRegistry — on-chain proof-of-verification (normal ECDSA + Groth16 ZK).
/// @notice Both paths share one `consumed` nullifier set, enforce `msg.sender == relayer`, gate on the
/// purpose-scoped `VERIFY:` whitelist, resolve the issuing clone FROM the root via the write-once
/// `rootIssuer[R]` index (§11.10(a)), and re-check `isValid(R)` directly. Single Poseidon root `R` —
/// no rKec/rZk, no zkCommit/kecOf/zkIndex (CHANGESPEC-v4). Code: impl §11.8(a)/§11.9(b,e)/§11.10(a,c).
contract VerificationRegistry is EIP712, AccessControlDefaultAdminRules {
    uint256 internal constant SNARK_SCALAR_FIELD =
        21888242871839275222246405745257275088548364400416034343698204186575808495617; // BN254 r
    uint256 internal constant DS_NULLIFIER = 4;
    uint256 internal constant ZK_TIMELOCK = 2 days;

    // Art. 9: SERVICE_ATTESTATION has no on-chain root → NOT verifiable on-chain (§11.9(h)).
    bytes32 internal constant SERVICE_ATTESTATION = keccak256("SERVICE_ATTESTATION");

    struct VerificationConsent {
        uint256 dogTagId;
        bytes32 recordType;
        bytes32 purpose; // keccak256(label) reduced mod r (§11.10(c)); DISTINCT from recordType
        bytes32 credentialRoot; // the single Poseidon root R
        bytes32 challenge; // one-time session binding (validated off-chain at submit)
        address relayer;
        address subject;
        uint256 nonce;
        uint256 deadline;
    }

    bytes32 public constant VERIFICATION_CONSENT_TYPEHASH = keccak256(
        "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 purpose,bytes32 credentialRoot,bytes32 challenge,address relayer,address subject,uint256 nonce,uint256 deadline)"
    );

    IIssuerRegistry public immutable issuerRegistry;
    IDogTagSBT public immutable sbt;
    IRootIndex public immutable rootIndex;
    IPoseidon6 public immutable poseidon6;

    // Consent-key registry is SWAPPABLE (set in the constructor, then timelock-rotated): the original
    // immutable forced a full VR redeploy to point at a fixed ConsentKeyRegistry. Now it can be rotated
    // via the same 2-day timelock as the verifier, without a redeploy.
    IConsentKeyReg public consentKeys;
    IConsentKeyReg public pendingConsentKeys;
    uint256 public consentKeysEta;

    IGroth16Verifier public zkVerifier;
    IGroth16Verifier public pendingZkVerifier;
    uint256 public zkVerifierEta;

    mapping(bytes32 => bool) public consumed; // SHARED nullifier set across BOTH paths
    bool public restrictToWhitelistedRelayers = true;

    event Verified(
        uint256 indexed dogTagId,
        address indexed relayer,
        address indexed subject,
        bytes32 purpose,
        bytes32 nullifier,
        uint256 ts
    );
    event ZkVerifierProposed(address indexed verifier, uint256 eta);
    event ZkVerifierUpdated(address indexed verifier);
    event ConsentKeysProposed(address indexed consentKeys, uint256 eta);
    event ConsentKeysUpdated(address indexed consentKeys);

    constructor(address ir, address sbt_, address zk, address ck, address ridx, address pos6, address admin)
        EIP712("DogTag", "1")
        AccessControlDefaultAdminRules(2 days, admin)
    {
        require(ir != address(0) && sbt_ != address(0) && ck != address(0) && ridx != address(0) && pos6 != address(0), "zero");
        issuerRegistry = IIssuerRegistry(ir);
        sbt = IDogTagSBT(sbt_);
        zkVerifier = IGroth16Verifier(zk);
        consentKeys = IConsentKeyReg(ck);
        rootIndex = IRootIndex(ridx);
        poseidon6 = IPoseidon6(pos6);
    }

    function _verifyKey(bytes32 purpose) internal pure returns (bytes32) {
        return keccak256(abi.encode("VERIFY:", purpose));
    }

    function _consumeAndResolve(bytes32 nf, bytes32 R) internal {
        require(!consumed[nf], "replayed");
        consumed[nf] = true;
        address clone = rootIndex.rootIssuer(R);
        require(clone != address(0), "unknown root"); // §11.10(a): resolve clone FROM the root
        require(IDogTagIssuer(clone).isValid(R), "cred !valid"); // isValid(R) directly
    }

    // ---- NORMAL path: ECDSA over the single Poseidon root R ----
    function recordVerification(VerificationConsent calldata c, bytes calldata userSig) external {
        require(block.timestamp <= c.deadline, "expired");
        require(msg.sender == c.relayer, "not relayer");
        require(c.recordType != SERVICE_ATTESTATION, "art9"); // §11.9(h)
        if (restrictToWhitelistedRelayers) {
            require(issuerRegistry.isWhitelistedFor(_verifyKey(c.purpose), msg.sender), "!verify-wl");
        }
        // §11.10(c): pin inputs into the scalar field BEFORE Poseidon so ids congruent mod r can't collide.
        require(c.dogTagId < SNARK_SCALAR_FIELD && c.nonce < SNARK_SCALAR_FIELD && uint256(c.purpose) < SNARK_SCALAR_FIELD, "!field");

        bytes32 digest = _hashTypedDataV4(
            keccak256(
                abi.encode(
                    VERIFICATION_CONSENT_TYPEHASH,
                    c.dogTagId,
                    c.recordType,
                    c.purpose,
                    c.credentialRoot,
                    c.challenge,
                    c.relayer,
                    c.subject,
                    c.nonce,
                    c.deadline
                )
            )
        );
        require(ECDSA.recover(digest, userSig) == c.subject, "bad sig");
        require(sbt.ownerOf(c.dogTagId) == c.subject, "subject !owner");

        bytes32 nf = bytes32(
            poseidon6.poseidon(
                [DS_NULLIFIER, c.dogTagId, uint256(c.purpose), uint160(c.relayer), uint160(c.subject), c.nonce]
            )
        );
        _consumeAndResolve(nf, c.credentialRoot);
        emit Verified(c.dogTagId, c.relayer, c.subject, c.purpose, nf, block.timestamp);
    }

    // ---- ZK path: Groth16 over pub = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R] ----
    function recordVerificationZK(
        uint256[2] calldata a,
        uint256[2][2] calldata b,
        uint256[2] calldata c,
        uint256[7] calldata pub
    ) external {
        require(address(uint160(pub[2])) == msg.sender, "not relayer");
        if (restrictToWhitelistedRelayers) {
            require(issuerRegistry.isWhitelistedFor(_verifyKey(bytes32(pub[1])), msg.sender), "!verify-wl");
        }
        for (uint256 i; i < 7; i++) {
            require(pub[i] < SNARK_SCALAR_FIELD, "!field"); // range-check ALL signals (#358)
        }
        require(consentKeys.keyOf(address(uint160(pub[3]))) == bytes32(pub[5]), "subject !key"); // subject<->key
        require(sbt.ownerOf(pub[0]) == address(uint160(pub[3])), "subject !owner");

        bytes32 nf = bytes32(pub[4]); // nullifier is a PUBLIC SIGNAL (#383), never derived from (a,b,c)
        require(zkVerifier.verifyProof(a, b, c, pub), "bad proof");
        _consumeAndResolve(nf, bytes32(pub[6]));
        emit Verified(pub[0], msg.sender, address(uint160(pub[3])), bytes32(pub[1]), nf, block.timestamp);
    }

    // ---- admin ----
    function setRelayerRestriction(bool on) external onlyRole(DEFAULT_ADMIN_ROLE) {
        restrictToWhitelistedRelayers = on;
    }

    /// @notice Real timelock on the verifier swap (§11.10(g)): propose, then execute after ZK_TIMELOCK.
    function proposeZkVerifier(address v) external onlyRole(DEFAULT_ADMIN_ROLE) {
        pendingZkVerifier = IGroth16Verifier(v);
        zkVerifierEta = block.timestamp + ZK_TIMELOCK;
        emit ZkVerifierProposed(v, zkVerifierEta);
    }

    function executeZkVerifier() external onlyRole(DEFAULT_ADMIN_ROLE) {
        require(address(pendingZkVerifier) != address(0), "none");
        require(block.timestamp >= zkVerifierEta, "timelock");
        zkVerifier = pendingZkVerifier;
        pendingZkVerifier = IGroth16Verifier(address(0));
        emit ZkVerifierUpdated(address(zkVerifier));
    }

    /// @notice Real timelock on the consent-key registry swap (mirrors the verifier swap): propose,
    /// then execute after ZK_TIMELOCK. Root-cause fix for the old immutable `consentKeys`.
    function proposeConsentKeys(address ck) external onlyRole(DEFAULT_ADMIN_ROLE) {
        pendingConsentKeys = IConsentKeyReg(ck);
        consentKeysEta = block.timestamp + ZK_TIMELOCK;
        emit ConsentKeysProposed(ck, consentKeysEta);
    }

    function executeConsentKeys() external onlyRole(DEFAULT_ADMIN_ROLE) {
        require(address(pendingConsentKeys) != address(0), "none");
        require(block.timestamp >= consentKeysEta, "timelock");
        consentKeys = pendingConsentKeys;
        pendingConsentKeys = IConsentKeyReg(address(0));
        emit ConsentKeysUpdated(address(consentKeys));
    }
}
