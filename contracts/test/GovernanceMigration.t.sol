// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {AccessControlEnumerable} from "@openzeppelin/contracts/access/extensions/AccessControlEnumerable.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBT} from "../src/DogTagSBT.sol";
import {ConsentKeyRegistry} from "../src/ConsentKeyRegistry.sol";
import {VerificationRegistry} from "../src/VerificationRegistry.sol";
import {GovernanceMigration} from "../script/GovernanceMigration.sol";

/// @dev Stand-in for the CURRENTLY-LIVE DogTagSBT, which predates the two-step upgrade and is still plain
/// `AccessControlEnumerable` (no on-chain retrofit without a state-orphaning redeploy). Exercises the
/// migration's legacy atomic grant->revoke hand-over branch.
contract LegacySbtMock is AccessControlEnumerable {
    constructor(address admin) {
        _grantRole(DEFAULT_ADMIN_ROLE, admin);
    }

    /// @notice admin-gated action (mirrors the live SBT's `burn`) used to prove the EOA loses power.
    function adminAction() external view onlyRole(DEFAULT_ADMIN_ROLE) {}
}

/// @notice Proves the H-3 governance hand-off: after the two-phase migration the multisig holds
/// admin/ownership of every governed contract and the deployer EOA can no longer act — including the
/// timelock actually gating Phase 2 and the legacy-SBT atomic hand-over.
contract GovernanceMigrationTest is Test {
    using GovernanceMigration for GovernanceMigration.Targets;

    bytes32 constant DEFAULT_ADMIN_ROLE = 0x00;
    bytes32 constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");
    bytes32 constant ISSUER_ROLE = keccak256("ISSUER");
    bytes32 constant VACCINATION = keccak256("VACCINATION");

    address eoa = address(0x119F8c7F); // the deployer EOA being decommissioned
    address multisig = address(0x5AFE); // the protocol multisig taking over

    IssuerRegistry registry;
    VerificationRegistry verification;
    DogTagIssuerFactory factory;
    ConsentKeyRegistry consentKeys;

    function _deployCommon() internal {
        vm.startPrank(eoa);
        registry = new IssuerRegistry(eoa);
        DogTagIssuer impl = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(impl), address(registry), eoa);
        consentKeys = new ConsentKeyRegistry();
        vm.stopPrank();
    }

    /// VerificationRegistry only needs non-zero wiring for this admin-surface test; it is never invoked.
    function _deployVerification(address sbt) internal {
        vm.prank(eoa);
        verification = new VerificationRegistry(
            address(registry),
            sbt,
            address(0),
            address(consentKeys),
            address(factory),
            address(consentKeys),
            eoa
        );
    }

    function _targets(address sbt) internal view returns (GovernanceMigration.Targets memory) {
        return GovernanceMigration.Targets({
            issuerRegistry: address(registry),
            verificationRegistry: address(verification),
            sbt: sbt,
            factory: address(factory)
        });
    }

    // -------------------------------------------------------------------------------------------------
    // Full set with the UPGRADED two-step DogTagSBT (the shape a fresh `Deploy.s.sol` now produces).
    // -------------------------------------------------------------------------------------------------
    function test_migration_two_step_full_set() public {
        _deployCommon();
        vm.prank(eoa);
        DogTagSBT sbt = new DogTagSBT(eoa);
        _deployVerification(address(sbt));
        GovernanceMigration.Targets memory t = _targets(address(sbt));

        // sanity: EOA starts as admin/owner everywhere.
        assertEq(registry.defaultAdmin(), eoa);
        assertEq(verification.defaultAdmin(), eoa);
        assertEq(sbt.defaultAdmin(), eoa);
        assertEq(factory.owner(), eoa);
        assertTrue(GovernanceMigration.supportsTwoStep(address(sbt)));

        // ---- Phase 1: begin (EOA) ----
        vm.startPrank(eoa);
        t.begin(multisig);
        vm.stopPrank();

        // Mid-flight: transfers are PENDING; EOA is still admin; multisig pre-holds WHITELIST_ADMIN.
        (address pendIR,) = registry.pendingDefaultAdmin();
        assertEq(pendIR, multisig);
        assertEq(registry.defaultAdmin(), eoa, "EOA still admin during timelock");
        assertTrue(registry.hasRole(WHITELIST_ADMIN, multisig));
        assertEq(factory.pendingOwner(), multisig);

        // The timelock is REAL: the multisig cannot accept IssuerRegistry before it elapses.
        vm.prank(multisig);
        vm.expectRevert();
        registry.acceptDefaultAdminTransfer();

        // ---- warp past the longest delay (3 days) ----
        vm.warp(block.timestamp + 3 days + 1);

        // ---- Phase 2: accept (multisig) ----
        vm.startPrank(multisig);
        t.accept(eoa);
        vm.stopPrank();

        _assertMultisigInControl(sbt);
    }

    // -------------------------------------------------------------------------------------------------
    // LEGACY live DogTagSBT (plain AccessControlEnumerable) — atomic grant->revoke hand-over branch.
    // -------------------------------------------------------------------------------------------------
    function test_migration_legacy_sbt_atomic_handover() public {
        _deployCommon();
        vm.prank(eoa);
        LegacySbtMock legacy = new LegacySbtMock(eoa);
        _deployVerification(address(legacy));
        GovernanceMigration.Targets memory t = _targets(address(legacy));

        assertFalse(GovernanceMigration.supportsTwoStep(address(legacy)), "legacy is not two-step");

        vm.startPrank(eoa);
        t.begin(multisig);
        vm.stopPrank();

        // Legacy hand-over has no timelock: multisig is granted admin immediately, EOA still admin too.
        assertTrue(legacy.hasRole(DEFAULT_ADMIN_ROLE, multisig));
        assertTrue(legacy.hasRole(DEFAULT_ADMIN_ROLE, eoa));

        // The ACDAR contracts still need their delay before accept.
        vm.warp(block.timestamp + 3 days + 1);

        vm.startPrank(multisig);
        t.accept(eoa);
        vm.stopPrank();

        // Legacy SBT: EOA stripped, multisig in sole control.
        assertFalse(legacy.hasRole(DEFAULT_ADMIN_ROLE, eoa), "EOA admin revoked on legacy SBT");
        assertTrue(legacy.hasRole(DEFAULT_ADMIN_ROLE, multisig));
        vm.prank(eoa);
        vm.expectRevert();
        legacy.adminAction();
        vm.prank(multisig);
        legacy.adminAction(); // multisig can

        // The ACDAR contracts + factory are also fully handed off.
        assertEq(registry.defaultAdmin(), multisig);
        assertEq(verification.defaultAdmin(), multisig);
        assertEq(factory.owner(), multisig);
    }

    function _assertMultisigInControl(DogTagSBT sbt) internal {
        // ---- multisig holds admin/ownership everywhere ----
        assertEq(registry.defaultAdmin(), multisig, "IssuerRegistry admin");
        assertEq(verification.defaultAdmin(), multisig, "VerificationRegistry admin");
        assertEq(sbt.defaultAdmin(), multisig, "DogTagSBT admin");
        assertEq(factory.owner(), multisig, "Factory owner");

        // ---- the old EOA holds NO governance roles ----
        assertFalse(registry.hasRole(DEFAULT_ADMIN_ROLE, eoa), "EOA !IR admin");
        assertFalse(registry.hasRole(WHITELIST_ADMIN, eoa), "EOA !whitelist admin");
        assertFalse(verification.hasRole(DEFAULT_ADMIN_ROLE, eoa), "EOA !VR admin");
        assertFalse(sbt.hasRole(DEFAULT_ADMIN_ROLE, eoa), "EOA !SBT admin");

        // ---- the old EOA can no longer act ----
        vm.startPrank(eoa);
        vm.expectRevert();
        registry.whitelistFor(VACCINATION, eoa);
        vm.expectRevert();
        verification.setRelayerRestriction(false);
        vm.expectRevert();
        sbt.grantRole(ISSUER_ROLE, eoa);
        vm.expectRevert();
        factory.createIssuer("x", VACCINATION, eoa);
        vm.stopPrank();

        // ---- the multisig CAN act ----
        vm.startPrank(multisig);
        registry.whitelistFor(VACCINATION, eoa); // multisig has WHITELIST_ADMIN
        verification.setRelayerRestriction(false);
        sbt.grantRole(ISSUER_ROLE, eoa);
        factory.createIssuer("Seaport Vacc", VACCINATION, eoa);
        vm.stopPrank();
        assertTrue(registry.isWhitelistedFor(VACCINATION, eoa));
    }
}
