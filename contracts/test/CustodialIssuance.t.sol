// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test, Vm} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";

/// @notice M5 acceptance: custodial issuance produces owner-unlinkable tags that M4 actually verifies.
///
/// This suite exercises the PRODUCTION Level-B pairing - `VerificationRegistryConsent` pointing at the
/// `DogTagSBTConsent` custodial SBT - which is what the M5 redeploy establishes. (`ConsentRegistry.t.sol`
/// deliberately still pairs the registry with the Level-A `DogTagSBT`; that remains valid coverage and
/// usefully proves the registry is SBT-agnostic, since it reads only `profileRoot` + `ownerOf`.)
///
/// The load-bearing property, per spec §"Issuance": **the owner's wallet never appears on-chain - not as
/// `to`, not as `msg.sender`.** `test_owner_wallet_absent_from_issuance_state_and_calldata` is the guard:
/// it sweeps every storage slot the issuance writes AND scans the raw calldata bytes, so a regression that
/// reintroduced the owner anywhere in issuance fails here rather than silently shipping.
///
/// Fixture (a,b,c,pub + the hidden `_ownerAddress`) from `node circuits/scripts/gen-consent-fixture.mjs`.
contract CustodialIssuanceTest is Test {
    using stdJson for string;

    IssuerRegistry registry;
    DogTagIssuerFactory factory;
    DogTagSBTConsent sbt;
    DogTagIssuer vacc;
    VerificationRegistryConsent vr;
    Groth16VerifierConsent verifier;

    address admin = address(0xA11CE);
    address vetSigner = address(0xBEEF);
    /// @dev The neutral custodian: the ONLY address `ownerOf` can ever return on this contract.
    address custodian = address(0xC0FFEE);

    bytes32 constant VACCINATION = keccak256("VACCINATION");

    uint256[2] a;
    uint256[2][2] b;
    uint256[2] c;
    uint256[7] pub;

    uint256 dogTagId;
    bytes32 purpose;
    address relayer;
    bytes32 nullifier;
    bytes32 root;
    uint256 deadline;
    /// @dev The REAL owner behind the fixture's tree - the address that must be absent everywhere on-chain.
    address ownerAddress;

    function setUp() public {
        string memory j = vm.readFile("test/consent-fixture.json");
        uint256[] memory av = j.readUintArray(".a");
        uint256[] memory b0 = j.readUintArray(".b[0]");
        uint256[] memory b1 = j.readUintArray(".b[1]");
        uint256[] memory cv = j.readUintArray(".c");
        uint256[] memory pv = j.readUintArray(".pub");
        a = [av[0], av[1]];
        b = [[b0[0], b0[1]], [b1[0], b1[1]]];
        c = [cv[0], cv[1]];
        for (uint256 i; i < 7; i++) {
            pub[i] = pv[i];
        }

        dogTagId = pub[0];
        purpose = bytes32(pub[1]);
        relayer = address(uint160(pub[2]));
        nullifier = bytes32(pub[3]);
        root = bytes32(pub[4]);
        deadline = pub[6];
        ownerAddress = j.readAddress("._ownerAddress");

        vm.startPrank(admin);
        registry = new IssuerRegistry(admin);
        DogTagIssuer impl = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(impl), address(registry), admin);
        sbt = new DogTagSBTConsent(admin, custodian); // the M5 custodial SBT
        verifier = new Groth16VerifierConsent();
        vr = new VerificationRegistryConsent( // the M5 redeploy: same registry code, new SBT
            address(registry),
            address(sbt),
            address(verifier),
            address(factory),
            admin
        );
        vm.stopPrank();

        vm.prank(admin);
        vacc = DogTagIssuer(factory.createIssuer("Vacc", VACCINATION, vetSigner));
        vm.prank(admin);
        registry.whitelistFor(VACCINATION, vetSigner);

        // Read the role BEFORE pranking: vm.prank only survives to the next CALL, and `sbt.ISSUER_ROLE()`
        // would otherwise consume it.
        bytes32 issuerRole = sbt.ISSUER_ROLE();
        vm.prank(admin);
        sbt.grantRole(issuerRole, vetSigner);

        vm.prank(admin);
        registry.whitelistFor(keccak256(abi.encode("VERIFY:", purpose)), relayer);
    }

    /// @dev The full M5 issuance: `issue(R)` into the clone, then mint custodially. BOTH writes are
    /// required - see `test_root_must_also_be_issued_into_a_clone`.
    function _issueCustodial() internal {
        vm.prank(vetSigner);
        vacc.issue(root);
        vm.prank(vetSigner);
        sbt.mintCustodial(dogTagId, root);
    }

    /// @dev Byte-level scan: does `hay` contain the 20 raw bytes of `needle` at ANY offset? Stronger than a
    /// word-compare - it catches the owner appearing packed, offset, or inside a nested struct.
    function _contains(bytes memory hay, address needle) internal pure returns (bool) {
        bytes20 n = bytes20(needle);
        if (hay.length < 20) return false;
        for (uint256 i; i + 20 <= hay.length; i++) {
            bool hit = true;
            for (uint256 k; k < 20; k++) {
                if (hay[i + k] != n[k]) {
                    hit = false;
                    break;
                }
            }
            if (hit) return true;
        }
        return false;
    }

    // ---- D1: the tag is custodial, and the owner is nowhere on-chain ----

    /// @notice POSITIVE CONTROL for the owner-absence guard. `_contains` returning false is the whole
    /// basis of `test_owner_wallet_absent_from_issuance_state_and_calldata`; if the scanner were broken it
    /// would return false for EVERYTHING and that test would pass vacuously forever. Prove it detects the
    /// owner when the owner really is present, in each shape issuance could leak one.
    function test_owner_absence_scanner_actually_detects_the_owner() public view {
        assertTrue(_contains(abi.encode(ownerAddress), ownerAddress), "must detect a word-aligned owner");
        assertTrue(
            _contains(abi.encodePacked(bytes4(0x12345678), ownerAddress), ownerAddress),
            "must detect an unaligned owner (e.g. behind a selector)"
        );
        assertTrue(
            _contains(
                abi.encodeWithSignature("mint(address,uint256,bytes32)", ownerAddress, dogTagId, root),
                ownerAddress
            ),
            "must detect the owner in a Level-A style mint calldata"
        );
        assertFalse(_contains(abi.encode(custodian), ownerAddress), "must not false-positive on custodian");
    }

    function test_mint_goes_to_the_custodian_never_the_owner() public {
        _issueCustodial();
        assertEq(sbt.ownerOf(dogTagId), custodian, "Level-B tags are held by the neutral custodian");
        assertTrue(sbt.ownerOf(dogTagId) != ownerAddress, "the owner must never hold the tag");
        assertEq(sbt.profileRoot(dogTagId), root, "profileRoot(dogTagId) == R");
        assertEq(uint8(sbt.status(dogTagId)), uint8(DogTagSBTConsent.Status.Active));
    }

    /// @notice M5 app-side: the root the OWNER'S APP builds locally is exactly what the contract seals as
    /// `profileRoot`. Spec §"Issuance" steps 2-3: *the owner's app builds the tree locally, computes `R`;
    /// the issuer sets `profileRoot(dogTagId) = R`*.
    ///
    /// `device-profile-root.json` is emitted by the REAL device builder
    /// (`dogtag-standard-rs profile_tree::build_profile_tree`, run over a fixed demo wallet seed via
    /// `cargo run -p dogtag-standard-rs --bin gen-device-profile-root`), NOT hand-written here. The Rust
    /// gate keeps it honest from the other side: `profile_tree_parity.rs` fails if the committed file
    /// drifts from a fresh build, and separately proves that the builder's primitives reproduce an `R`
    /// the M2 circuit proved and this suite verifies on-chain.
    ///
    /// These are two separate legs: the primitives reproduce the circuit-verified fixture root, while
    /// `build_profile_tree` produces the distinct demo root this test stores as `profileRoot`. The demo root
    /// is not itself circuit-proven; that end-to-end prover path belongs to M7. The assertion is genuinely
    /// device-derived rather than a re-assertion of `setUp`'s because the device root DIFFERS from the
    /// fixture's; the two fixtures deliberately SHARE one dogTagId (424242). Since
    /// `mintCustodial` is write-once per id, they can therefore never both be minted in the same test: any
    /// future change that makes `setUp` mint, or that adds `_issueCustodial()` here, MUST first give the
    /// device witness a distinct id - which moves `R`, so `device-profile-root.json` must be regenerated.
    function test_device_built_root_is_what_the_contract_stores_as_profileRoot() public {
        string memory j = vm.readFile("test/device-profile-root.json");
        bytes32 deviceRoot = bytes32(j.readUint(".R"));
        uint256 deviceTagId = vm.parseUint(j.readString(".dogTagId"));
        address deviceOwner = j.readAddress("._ownerAddress");

        assertTrue(deviceRoot != bytes32(0), "device root must be non-zero");

        vm.prank(vetSigner);
        vacc.issue(deviceRoot);
        vm.prank(vetSigner);
        sbt.mintCustodial(deviceTagId, deviceRoot);

        assertEq(sbt.profileRoot(deviceTagId), deviceRoot, "profileRoot == the app-built R");
        // The device-side move must not reintroduce the linkability M5 removes.
        assertEq(sbt.ownerOf(deviceTagId), custodian, "device-built tag is still custodial");
        assertTrue(sbt.ownerOf(deviceTagId) != deviceOwner, "the owner must never hold the tag");
    }

    /// @notice THE load-bearing test (spec §"Issuance" step 4: *the owner's wallet never appears on-chain -
    /// not as `to`, not as `msg.sender`*). Three independent checks, because each catches a different
    /// regression:
    ///   1. CALLDATA - a byte scan of the exact calldata both issuance txs carry.
    ///   2. STATE - every storage slot the issuance WRITES, on the SBT and on the issuing clone.
    ///   3. LOGS + `msg.sender` - the owner is not an emitter, a topic, or event data.
    function test_owner_wallet_absent_from_issuance_state_and_calldata() public {
        bytes memory mintCalldata = abi.encodeCall(DogTagSBTConsent.mintCustodial, (dogTagId, root));
        bytes memory issueCalldata = abi.encodeCall(DogTagIssuer.issue, (root));
        assertFalse(_contains(mintCalldata, ownerAddress), "owner leaked into mint calldata");
        assertFalse(_contains(issueCalldata, ownerAddress), "owner leaked into issue calldata");

        vm.recordLogs();
        vm.record();
        _issueCustodial();

        // 2. STATE: sweep every slot written on both contracts touched by issuance.
        _assertOwnerAbsentFromWrites(address(sbt), "SBT");
        _assertOwnerAbsentFromWrites(address(vacc), "issuer clone");

        // The owner is not the minter either: `msg.sender` throughout is the vet signer.
        assertEq(sbt.issuerOf(dogTagId), vetSigner, "issuerOf is the vet, not the owner");
        assertTrue(sbt.issuerOf(dogTagId) != ownerAddress, "owner leaked into issuerOf");
        assertEq(vacc.issuedBy(root), vetSigner, "issuedBy is the vet, not the owner");

        // 3. LOGS: no emitter, topic or data word is the owner.
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertGt(logs.length, 0, "issuance must emit");
        bytes32 ownerWord = bytes32(uint256(uint160(ownerAddress)));
        for (uint256 i; i < logs.length; i++) {
            assertTrue(logs[i].emitter != ownerAddress, "owner emitted a log");
            for (uint256 t; t < logs[i].topics.length; t++) {
                assertTrue(logs[i].topics[t] != ownerWord, "owner leaked into a log topic");
            }
            assertFalse(_contains(logs[i].data, ownerAddress), "owner leaked into log data");
        }
    }

    /// @dev Reads back every slot `target` wrote during the recorded window and asserts none holds the
    /// owner. Scoped to WRITES because that is precisely "the on-chain state issuance created". Uses the
    /// same byte scan as the calldata leg rather than a word compare, so an owner packed alongside another
    /// field in one slot (e.g. `address` + `uint96`) cannot slip through.
    function _assertOwnerAbsentFromWrites(address target, string memory label) internal {
        (, bytes32[] memory writes) = vm.accesses(target);
        for (uint256 i; i < writes.length; i++) {
            assertFalse(
                _contains(abi.encode(vm.load(target, writes[i])), ownerAddress),
                string.concat("owner leaked into ", label, " storage")
            );
        }
    }

    /// @notice The owner's wallet is not even expressible in the mint: `mintCustodial` takes no `to`. A
    /// future change re-adding one would fail to compile against this test rather than quietly ship.
    function test_mint_has_no_recipient_parameter() public view {
        // Selector is (uint256,bytes32) - no address. Level-A's was mint(address,uint256,bytes32).
        assertEq(
            DogTagSBTConsent.mintCustodial.selector,
            bytes4(keccak256("mintCustodial(uint256,bytes32)")),
            "mintCustodial must never take a recipient"
        );
        assertEq(sbt.custodian(), custodian, "custodian is fixed at construction");
    }

    // ---- write-once profileRoot: the setProfileRoot hijack, structurally closed ----

    /// @notice The M4 hijack: with `ownerOf`/`keyOf` identity checks gone, `R == profileRoot` is the SOLE
    /// tag<->owner binding, so anyone able to repoint `profileRoot` could forge a `Verified` for a pet they
    /// do not own. This SBT has NO setter at all - not a re-gated one. The call must not even dispatch.
    function test_profileRoot_has_no_setter_at_all() public {
        _issueCustodial();
        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "setProfileRoot(uint256,bytes32)", dogTagId, keccak256("attacker root")
                )
            );
        assertFalse(ok, "setProfileRoot must not exist on the Level-B SBT");
        assertEq(sbt.profileRoot(dogTagId), root, "root is write-once");
    }

    /// @notice AUTHORITY_ROLE was one of the hijack's two vectors on Level-A. Here the role still governs
    /// status/lifecycle but has NO power over the root - there is nothing to overwrite.
    function test_authority_role_cannot_touch_the_root() public {
        _issueCustodial();
        bytes32 authorityRole = sbt.AUTHORITY_ROLE();
        vm.prank(admin);
        sbt.grantRole(authorityRole, admin);

        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "setProfileRoot(uint256,bytes32)", dogTagId, keccak256("attacker root")
                )
            );
        assertFalse(ok, "an AUTHORITY holder must have no root-write path");
        assertEq(sbt.profileRoot(dogTagId), root, "victim tag still points at its real root");

        // ...and the tag still verifies, i.e. the hijack did not merely fail - it never happened.
        vm.prank(relayer);
        vr.recordVerificationZK(a, b, c, pub);
    }

    /// @notice A tag is never created rootless - there is no later call that could seal it.
    function test_mintCustodial_rejects_a_zero_root() public {
        vm.prank(vetSigner);
        vm.expectRevert(DogTagSBTConsent.BadRoot.selector);
        sbt.mintCustodial(dogTagId, bytes32(0));
    }

    function test_mintCustodial_is_issuer_gated() public {
        vm.prank(address(0xBAD));
        vm.expectRevert();
        sbt.mintCustodial(dogTagId, root);
    }

    function test_duplicate_dogTagId_reverts() public {
        _issueCustodial();
        vm.prank(vetSigner);
        vm.expectRevert();
        sbt.mintCustodial(dogTagId, root);
    }

    /// @notice The seal must survive a BURN, not just a duplicate mint. ERC-721 `_burn` leaves no tombstone
    /// (`_owners[id]` goes back to zero) and `_mint` only rejects a re-mint when the previous owner was
    /// non-zero, so without an explicit `profileRoot` guard an ISSUER holder could re-mint a burned id and
    /// overwrite its sealed root - reopening the very hijack this SBT exists to close, and un-erasing a
    /// GDPR-erased tag. `test_duplicate_dogTagId_reverts` never burns first, so it does not cover this.
    function test_burned_dogTagId_can_never_be_reminted() public {
        _issueCustodial();
        vm.prank(admin);
        sbt.burn(dogTagId);

        bytes32 attackerRoot = keccak256("attacker root");
        vm.prank(vetSigner);
        vm.expectRevert(DogTagSBTConsent.BadRoot.selector);
        sbt.mintCustodial(dogTagId, attackerRoot);

        assertEq(sbt.profileRoot(dogTagId), root, "the sealed root survives a burn/re-mint attempt");
    }

    // ---- the flow must actually produce tags M4 verifies ----

    /// @notice M5's whole purpose: a custodially-issued tag verifies end-to-end against the M4 registry and
    /// emits an owner-blind `Verified`. Uses a REAL Groth16 proof against the production M3 VK.
    function test_custodial_issuance_verifies_end_to_end() public {
        _issueCustodial();

        vm.expectEmit(true, true, true, true);
        emit VerificationRegistryConsent.Verified(
            dogTagId, relayer, purpose, nullifier, deadline, block.timestamp
        );
        vm.prank(relayer);
        vr.recordVerificationZK(a, b, c, pub);
        assertTrue(vr.consumed(nullifier), "nullifier consumed");
    }

    /// @notice Guards the M4 handoff note: issuance MUST `issue(R)` into a clone as well as set
    /// `profileRoot`, or the registry's revocation lookup (`rootIssuer[R]` -> `isValid`) reverts on EVERY
    /// verify. Minting alone yields a tag that can never be used - the exact trap the spec's step list
    /// (which names only `profileRoot`) invites.
    function test_root_must_also_be_issued_into_a_clone() public {
        vm.prank(vetSigner);
        sbt.mintCustodial(dogTagId, root); // profileRoot set, but issue(R) skipped

        vm.prank(relayer);
        vm.expectRevert("unknown root");
        vr.recordVerificationZK(a, b, c, pub);
    }

    /// @notice Revocation still works under Level-B: revoke the root, the tag stops verifying.
    function test_revoked_root_stops_verifying() public {
        _issueCustodial();
        vm.prank(vetSigner);
        vacc.revoke(root);

        vm.prank(relayer);
        vm.expectRevert("cred !valid");
        vr.recordVerificationZK(a, b, c, pub);
    }

    // ---- soulbound, absolutely (D3: no rebind - recovery is a fresh issuance) ----

    function test_tag_is_soulbound_with_no_recovery_bypass() public {
        _issueCustodial();
        vm.prank(custodian);
        vm.expectRevert(DogTagSBTConsent.Soulbound.selector);
        sbt.transferFrom(custodian, address(0xD00D), dogTagId);
    }

    /// @notice D3/M8: Level-A's `recover(...)` named the new owner ON-CHAIN, which is exactly what Level-B
    /// removes. Recovery is a fresh custodial issuance, so the function must not exist here.
    function test_no_recover_surface() public {
        _issueCustodial();
        (bool ok,) = address(sbt)
            .call(
                abi.encodeWithSignature(
                    "recover(uint256,address,uint256,uint256,bytes,bytes)",
                    dogTagId,
                    address(0xD00D),
                    uint256(0),
                    block.timestamp + 1,
                    bytes(""),
                    bytes("")
                )
            );
        assertFalse(ok, "recover must not exist on the Level-B SBT");
    }

    /// @notice Erasure still fails closed (the registry's `ownerOf` existence gate), and `profileRoot`
    /// deliberately survives the burn - which is why that gate is load-bearing rather than redundant.
    function test_burned_tag_cannot_verify() public {
        _issueCustodial();
        vm.prank(admin);
        sbt.burn(dogTagId);

        assertEq(sbt.profileRoot(dogTagId), root, "burn does not clear profileRoot");
        vm.prank(relayer);
        vm.expectRevert(); // ERC721NonexistentToken
        vr.recordVerificationZK(a, b, c, pub);
    }
}
