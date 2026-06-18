// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBT} from "../src/DogTagSBT.sol";
import {ConsentKeyRegistry} from "../src/ConsentKeyRegistry.sol";
import {VerificationRegistry, IPoseidon6} from "../src/VerificationRegistry.sol";
import {MockGroth16Verifier} from "./mocks/MockGroth16Verifier.sol";

contract VerificationTest is Test {
    uint256 constant R_FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617;

    IssuerRegistry registry;
    DogTagIssuerFactory factory;
    DogTagSBT sbt;
    ConsentKeyRegistry consentKeys;
    VerificationRegistry vr;
    MockGroth16Verifier mockZk;
    IPoseidon6 poseidon6;
    DogTagIssuer vacc;

    address admin = address(0xA11CE);
    address vetSigner = address(0xBEEF);
    address relayer = address(0x9E1A);

    bytes32 constant VACCINATION = keccak256("VACCINATION");
    bytes32 purpose; // reduced mod r
    bytes32 root; // issued credential root R
    uint256 dogTagId = 12345;

    uint256 subjectPk; // set in setUp
    address subject;

    function setUp() public {
        subjectPk = uint256(keccak256("subject-key")) % R_FIELD;
        subject = vm.addr(subjectPk);
        purpose = bytes32(uint256(keccak256("GROOMING_INTAKE")) % R_FIELD);
        root = bytes32(uint256(keccak256("credential-root")) % R_FIELD);

        vm.startPrank(admin);
        registry = new IssuerRegistry(admin);
        DogTagIssuer impl = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(impl), address(registry), admin);
        sbt = new DogTagSBT(admin);
        consentKeys = new ConsentKeyRegistry();
        mockZk = new MockGroth16Verifier();
        vm.stopPrank();

        // deploy the circomlib-exact Poseidon(6) from initcode (same nullifier as circom/TS/Rust)
        bytes memory initcode = vm.parseBytes(vm.readFile("test/poseidon6.initcode"));
        address p6;
        assembly {
            p6 := create(0, add(initcode, 0x20), mload(initcode))
        }
        require(p6 != address(0), "poseidon6 deploy");
        poseidon6 = IPoseidon6(p6);

        vm.prank(admin);
        vr = new VerificationRegistry(
            address(registry), address(sbt), address(mockZk), address(consentKeys), address(factory), p6, admin
        );

        // issue a credential root on a VACCINATION clone
        vm.prank(admin);
        vacc = DogTagIssuer(factory.createIssuer("Seaport Vacc", VACCINATION, vetSigner));
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        vm.prank(vetSigner);
        vacc.issue(root);

        // mint the pet SBT to the subject
        bytes32 issuerRole = sbt.ISSUER_ROLE();
        vm.prank(admin);
        sbt.grantRole(issuerRole, address(this));
        sbt.mint(subject, dogTagId, keccak256("profile"));

        // authorize the relayer to verify this purpose
        vm.prank(admin);
        registry.whitelistFor(keccak256(abi.encode("VERIFY:", purpose)), relayer);
    }

    // ---------- helpers ----------
    function _consent() internal view returns (VerificationRegistry.VerificationConsent memory c) {
        c = VerificationRegistry.VerificationConsent({
            dogTagId: dogTagId,
            recordType: VACCINATION,
            purpose: purpose,
            credentialRoot: root,
            challenge: keccak256("challenge-1"),
            relayer: relayer,
            subject: subject,
            nonce: 1,
            deadline: block.timestamp + 5 minutes
        });
    }

    function _consentDigest(VerificationRegistry.VerificationConsent memory c) internal view returns (bytes32) {
        bytes32 domainSep = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes("DogTag")),
                keccak256(bytes("1")),
                block.chainid,
                address(vr)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                vr.VERIFICATION_CONSENT_TYPEHASH(),
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
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSep, structHash));
    }

    function _sign(uint256 pk, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return abi.encodePacked(r, s, v);
    }

    function _expectedNullifier(VerificationRegistry.VerificationConsent memory c) internal view returns (bytes32) {
        uint256[6] memory in_ =
            [uint256(4), c.dogTagId, uint256(c.purpose), uint256(uint160(c.relayer)), uint256(uint160(c.subject)), c.nonce];
        return bytes32(poseidon6.poseidon(in_));
    }

    // ---------- NORMAL path ----------
    function test_normal_path_records_verified() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        bytes32 nf = _expectedNullifier(c);

        vm.expectEmit(true, true, true, true);
        emit VerificationRegistry.Verified(dogTagId, relayer, subject, purpose, nf, block.timestamp);
        vm.prank(relayer);
        vr.recordVerification(c, sig);
        assertTrue(vr.consumed(nf));
    }

    function test_normal_replay_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(relayer);
        vr.recordVerification(c, sig);
        vm.prank(relayer);
        vm.expectRevert("replayed");
        vr.recordVerification(c, sig);
    }

    function test_normal_not_relayer_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(address(0xDEAD));
        vm.expectRevert("not relayer");
        vr.recordVerification(c, sig);
    }

    function test_normal_bad_subject_sig_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(uint256(0xBAD), _consentDigest(c)); // not subject
        vm.prank(relayer);
        vm.expectRevert("bad sig");
        vr.recordVerification(c, sig);
    }

    function test_normal_not_owner_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        c.dogTagId = 99999; // a token the subject does not own (not minted)
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(relayer);
        vm.expectRevert(); // ownerOf reverts for nonexistent token (ERC721NonexistentToken)
        vr.recordVerification(c, sig);
    }

    function test_normal_service_attestation_rejected() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        c.recordType = keccak256("SERVICE_ATTESTATION");
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(relayer);
        vm.expectRevert(bytes("art9"));
        vr.recordVerification(c, sig);
    }

    function test_normal_unwhitelisted_relayer_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        c.relayer = address(0x7777);
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(address(0x7777));
        vm.expectRevert("!verify-wl");
        vr.recordVerification(c, sig);
    }

    function test_normal_unknown_root_reverts() public {
        VerificationRegistry.VerificationConsent memory c = _consent();
        c.credentialRoot = bytes32(uint256(123)); // never issued
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(relayer);
        vm.expectRevert("unknown root");
        vr.recordVerification(c, sig);
    }

    function test_normal_revoked_root_invalid() public {
        vm.prank(vetSigner);
        vacc.revoke(root);
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        vm.prank(relayer);
        vm.expectRevert("cred !valid");
        vr.recordVerification(c, sig);
    }

    // ---------- ZK path (mock verifier) ----------
    // keyHash = Poseidon(Ax,Ay) in reality, so it is a field element < r.
    bytes32 keyHash = bytes32(uint256(keccak256("babyjub-keyhash")) % R_FIELD);

    function _bindKey() internal {
        // subject binds a consent key via EIP-712 to ConsentKeyRegistry
        uint256 nonce = consentKeys.bindNonce(subject);
        bytes32 domainSep = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes("DogTag")),
                keccak256(bytes("1")),
                block.chainid,
                address(consentKeys)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256("BindConsentKey(bytes32 babyJubPubKeyHash,address wallet,uint256 nonce)"),
                keyHash,
                subject,
                nonce
            )
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", domainSep, structHash));
        vm.prank(subject);
        consentKeys.bindConsentKey(keyHash, _sign(subjectPk, digest));
    }

    function _pub() internal view returns (uint256[7] memory pub) {
        // [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
        pub[0] = dogTagId;
        pub[1] = uint256(purpose);
        pub[2] = uint160(relayer);
        pub[3] = uint160(subject);
        pub[4] = uint256(keccak256("zk-nullifier")) % R_FIELD;
        pub[5] = uint256(keyHash);
        pub[6] = uint256(root);
    }

    function _zeroProof() internal pure returns (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) {}

    function test_zk_path_records_verified() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vr.recordVerificationZK(a, b, c, pub);
        assertTrue(vr.consumed(bytes32(pub[4])));
    }

    function test_zk_not_relayer_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(address(0xDEAD));
        vm.expectRevert("not relayer");
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_out_of_field_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        pub[4] = R_FIELD; // >= r
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("!field");
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_wrong_keyhash_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        pub[5] = uint256(keccak256("wrong-key")) % R_FIELD; // < r but != keyOf[subject]
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("subject !key");
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_wrong_owner_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        pub[0] = 99999; // unowned token
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert(); // ownerOf reverts (nonexistent) or subject !owner
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_wrong_purpose_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        pub[1] = uint256(keccak256("AIRLINE_CHECKIN")) % R_FIELD; // relayer not whitelisted for this
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("!verify-wl");
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_bad_proof_reverts() public {
        _bindKey();
        mockZk.setResult(false);
        uint256[7] memory pub = _pub();
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("bad proof");
        vr.recordVerificationZK(a, b, c, pub);
    }

    function test_zk_unknown_root_reverts() public {
        _bindKey();
        uint256[7] memory pub = _pub();
        pub[6] = uint256(bytes32(uint256(777))); // never issued
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory c) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("unknown root");
        vr.recordVerificationZK(a, b, c, pub);
    }

    // ---------- shared nullifier double-spend across paths ----------
    function test_shared_nullifier_blocks_cross_path() public {
        // record on the NORMAL path, then attempt the SAME nullifier on the ZK path
        VerificationRegistry.VerificationConsent memory c = _consent();
        bytes memory sig = _sign(subjectPk, _consentDigest(c));
        bytes32 nf = _expectedNullifier(c);
        vm.prank(relayer);
        vr.recordVerification(c, sig);

        _bindKey();
        uint256[7] memory pub = _pub();
        pub[4] = uint256(nf); // same nullifier
        (uint256[2] memory a, uint256[2][2] memory b, uint256[2] memory cc) = _zeroProof();
        vm.prank(relayer);
        vm.expectRevert("replayed");
        vr.recordVerificationZK(a, b, cc, pub);
    }

    // ---------- timelock on verifier swap ----------
    function test_zk_verifier_timelock() public {
        address newV = address(new MockGroth16Verifier());
        vm.prank(admin);
        vr.proposeZkVerifier(newV);
        vm.prank(admin);
        vm.expectRevert("timelock");
        vr.executeZkVerifier();
        vm.warp(block.timestamp + 2 days);
        vm.prank(admin);
        vr.executeZkVerifier();
        assertEq(address(vr.zkVerifier()), newV);
    }
}
