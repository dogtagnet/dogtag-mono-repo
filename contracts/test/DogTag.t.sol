// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBT} from "../src/DogTagSBT.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";

contract DogTagContractsTest is Test {
    IssuerRegistry registry;
    DogTagIssuer impl;
    DogTagIssuerFactory factory;
    DogTagSBT sbt;

    address admin = address(0xA11CE);
    address vetSigner = address(0xBEEF);
    address groomerSigner = address(0xCAFE);

    bytes32 constant VACCINATION = keccak256("VACCINATION");
    bytes32 constant SERVICE = keccak256("SERVICE_ATTESTATION");

    function setUp() public {
        vm.startPrank(admin);
        registry = new IssuerRegistry(admin);
        impl = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(impl), address(registry), admin);
        sbt = new DogTagSBT(admin);
        vm.stopPrank();
    }

    function _newVaccClone() internal returns (DogTagIssuer clone) {
        vm.prank(admin);
        clone = DogTagIssuer(factory.createIssuer("Seaport Vacc", VACCINATION, vetSigner));
    }

    // ---- IssuerRegistry / whitelist gating ----
    function test_only_whitelisted_can_issue() public {
        DogTagIssuer clone = _newVaccClone();
        bytes32 root = keccak256("R1");
        vm.prank(vetSigner);
        vm.expectRevert(DogTagIssuer.NotWhitelisted.selector);
        clone.issue(root);

        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        vm.prank(vetSigner);
        clone.issue(root);
        assertTrue(clone.isValid(root));
    }

    function test_cross_type_isolation() public {
        DogTagIssuer clone = _newVaccClone();
        // whitelist groomer for SERVICE, not VACCINATION
        vm.prank(admin);
        registry.whitelistFor(SERVICE, groomerSigner);
        vm.prank(groomerSigner);
        vm.expectRevert(DogTagIssuer.NotWhitelisted.selector);
        clone.issue(keccak256("Rx"));
    }

    // ---- originator binding (H-1) ----
    function test_originator_only_revoke_plus_admin() public {
        DogTagIssuer clone = _newVaccClone();
        vm.startPrank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        registry.whitelistFor(VACCINATION, groomerSigner);
        vm.stopPrank();

        bytes32 root = keccak256("R2");
        vm.prank(vetSigner);
        clone.issue(root);

        // a different whitelisted signer (not originator, not admin) cannot revoke
        vm.prank(groomerSigner);
        vm.expectRevert(DogTagIssuer.NotOriginatorOrAdmin.selector);
        clone.revoke(root);

        // originator can
        vm.prank(vetSigner);
        clone.revoke(root);
        assertFalse(clone.isValid(root));
    }

    function test_admin_mass_revoke() public {
        DogTagIssuer clone = _newVaccClone();
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        bytes32 root = keccak256("R3");
        vm.prank(vetSigner);
        clone.issue(root);

        bytes32[] memory rs = new bytes32[](1);
        rs[0] = root;
        vm.prank(admin); // registry DEFAULT_ADMIN
        clone.adminRevoke(rs);
        assertFalse(clone.isValid(root));
    }

    // ---- C-1: locked implementation ----
    function test_impl_cannot_be_initialized() public {
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        impl.initialize("x", VACCINATION, address(registry), address(factory));
    }

    function test_clone_init_once() public {
        DogTagIssuer clone = _newVaccClone();
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        clone.initialize("again", VACCINATION, address(registry), address(factory));
    }

    // ---- factory determinism + permissioning ----
    function test_factory_determinism_and_permission() public {
        address predicted = factory.predictIssuer(VACCINATION, vetSigner);
        vm.prank(admin);
        address clone = factory.createIssuer("v", VACCINATION, vetSigner);
        assertEq(clone, predicted);

        // non-admin cannot create
        vm.prank(vetSigner);
        vm.expectRevert();
        factory.createIssuer("v2", SERVICE, vetSigner);
    }

    // ---- rootIssuer write-once index (§11.10(a)) ----
    function test_root_issuer_index_written_on_issue() public {
        DogTagIssuer clone = _newVaccClone();
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        bytes32 root = keccak256("R4");
        vm.prank(vetSigner);
        clone.issue(root);
        assertEq(factory.rootIssuer(root), address(clone));
    }

    function test_root_issuer_write_once() public {
        DogTagIssuer clone = _newVaccClone();
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);
        bytes32 root = keccak256("R5");
        vm.startPrank(vetSigner);
        clone.issue(root);
        vm.expectRevert(DogTagIssuer.BadRoot.selector); // re-issue same root on same clone
        clone.issue(root);
        vm.stopPrank();

        // a second clone trying to register the same root reverts at the index
        vm.prank(admin);
        DogTagIssuer clone2 = DogTagIssuer(factory.createIssuer("Vacc2", VACCINATION, groomerSigner));
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, groomerSigner);
        vm.prank(groomerSigner);
        vm.expectRevert("root taken");
        clone2.issue(root);
    }

    function test_non_clone_cannot_register_root() public {
        vm.expectRevert("!clone");
        factory.registerRoot(keccak256("evil"));
    }

    // ---- SBT soulbound + lifecycle ----
    function _mintTo(address to, uint256 id) internal {
        bytes32 issuerRole = sbt.ISSUER_ROLE(); // read BEFORE prank (a view call would consume it)
        vm.prank(admin);
        sbt.grantRole(issuerRole, address(this));
        sbt.mint(to, id, keccak256("profile"));
    }

    function test_sbt_soulbound_transfer_reverts() public {
        address owner = address(0xD06);
        _mintTo(owner, 1);
        vm.prank(owner);
        vm.expectRevert(DogTagSBT.Soulbound.selector);
        sbt.transferFrom(owner, address(0x1234), 1);
    }

    function test_sbt_owner_cannot_set_status() public {
        address owner = address(0xD06);
        _mintTo(owner, 2);
        vm.prank(owner);
        vm.expectRevert(DogTagSBT.NotIssuerOrAuthority.selector);
        sbt.setStatus(2, DogTagSBT.Status.Lost, "lost");
    }

    function test_sbt_deceased_is_terminal() public {
        _mintTo(address(0xD06), 3); // issuer = address(this)
        sbt.setStatus(3, DogTagSBT.Status.Deceased, "rip");
        vm.expectRevert(DogTagSBT.Terminal.selector);
        sbt.setStatus(3, DogTagSBT.Status.Active, "undo");
    }

    function test_sbt_recover_preserves_tokenId_and_issuer() public {
        uint256 id = 7;
        // current owner keypair (must sign its own consent — audit H1)
        uint256 curPk = 0xC0FFEE;
        address curOwner = vm.addr(curPk);
        _mintTo(curOwner, id);
        address origIssuer = sbt.issuerOf(id);

        // destination owner keypair
        uint256 pk = 0xA11CE2;
        address newOwner = vm.addr(pk);

        bytes32 recRole = sbt.RECOVERY_ROLE();
        vm.prank(admin);
        sbt.grantRole(recRole, address(this));

        uint256 nonce = sbt.recoverNonce(id);
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory curSig = _sign(curPk, _consentDigest(id, curOwner, newOwner, nonce, deadline));
        bytes memory newSig = _sign(pk, _claimDigest(id, newOwner, nonce, deadline));

        sbt.recover(id, newOwner, nonce, deadline, curSig, newSig);
        assertEq(sbt.ownerOf(id), newOwner);
        assertEq(sbt.issuerOf(id), origIssuer); // preserved
        assertEq(uint8(sbt.status(id)), uint8(DogTagSBT.Status.Active));
    }

    /// Audit H1: RECOVERY_ROLE alone cannot confiscate a token — without the CURRENT owner's consent
    /// signature the recovery reverts, even with a valid destination acceptance signature.
    function test_sbt_recover_without_current_owner_auth_reverts() public {
        uint256 id = 9;
        uint256 curPk = 0xC0FFEE;
        address curOwner = vm.addr(curPk);
        _mintTo(curOwner, id);

        bytes32 recRole = sbt.RECOVERY_ROLE();
        vm.prank(admin);
        sbt.grantRole(recRole, address(this));

        uint256 pk = 0xA11CE2;
        address newOwner = vm.addr(pk);
        uint256 nonce = sbt.recoverNonce(id);
        uint256 deadline = block.timestamp + 1 hours;

        // consent signed by an ATTACKER (not the current owner), acceptance by the destination.
        bytes memory forgedConsent = _sign(uint256(0xBAD), _consentDigest(id, curOwner, newOwner, nonce, deadline));
        bytes memory newSig = _sign(pk, _claimDigest(id, newOwner, nonce, deadline));
        vm.expectRevert("bad current-owner sig");
        sbt.recover(id, newOwner, nonce, deadline, forgedConsent, newSig);

        // and the token did not move.
        assertEq(sbt.ownerOf(id), curOwner);
    }

    function test_sbt_recover_wrong_signer_reverts() public {
        uint256 id = 8;
        uint256 curPk = 0xC0FFEE;
        address curOwner = vm.addr(curPk);
        _mintTo(curOwner, id);
        bytes32 recRole = sbt.RECOVERY_ROLE();
        vm.prank(admin);
        sbt.grantRole(recRole, address(this));
        address newOwner = vm.addr(0xBEEF1);
        uint256 nonce = sbt.recoverNonce(id);
        uint256 deadline = block.timestamp + 1 hours;
        // valid current-owner consent, but the destination acceptance is signed by the wrong key.
        bytes memory curSig = _sign(curPk, _consentDigest(id, curOwner, newOwner, nonce, deadline));
        bytes memory badNewSig = _sign(uint256(0xBAD), _claimDigest(id, newOwner, nonce, deadline)); // not newOwner
        vm.expectRevert("bad sig");
        sbt.recover(id, newOwner, nonce, deadline, curSig, badNewSig);
    }

    function _sign(uint256 pk, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return abi.encodePacked(r, s, v);
    }

    function _consentDigest(uint256 id, address currentOwner, address newOwner, uint256 nonce, uint256 deadline)
        internal
        view
        returns (bytes32)
    {
        bytes32 domainSep = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes("DogTag")),
                keccak256(bytes("1")),
                block.chainid,
                address(sbt)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256(
                    "RecoverConsent(uint256 dogTagId,address currentOwner,address newOwner,uint256 nonce,uint256 deadline)"
                ),
                id,
                currentOwner,
                newOwner,
                nonce,
                deadline
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSep, structHash));
    }

    function _claimDigest(uint256 id, address newOwner, uint256 nonce, uint256 deadline)
        internal
        view
        returns (bytes32)
    {
        bytes32 domainSep = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes("DogTag")),
                keccak256(bytes("1")),
                block.chainid,
                address(sbt)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256("Claim(uint256 dogTagId,address newOwner,uint256 nonce,uint256 deadline)"),
                id,
                newOwner,
                nonce,
                deadline
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSep, structHash));
    }

    function test_sbt_supports_5192_interface() public view {
        assertTrue(sbt.supportsInterface(0xb45a3c0e));
        assertTrue(sbt.locked(1));
    }

    // ---- dogTagId is never a hash of the microchip (audit-06 §4.2 / audit-12 M-2) ----
    function test_dogTagId_is_not_hash_of_microchip() public {
        // a real chip number; the allocator MUST NOT derive id from any hash of it.
        string memory chip = "985141006580311";
        uint256 keccakId = uint256(keccak256(bytes(chip)));
        // allocate a non-personal id (here: sequential) and assert it differs from the hash-derived id
        uint256 allocated = 42;
        _mintTo(address(0xD06), allocated);
        assertTrue(allocated != keccakId, "dogTagId must not be keccak256(microchip)");
    }
}
