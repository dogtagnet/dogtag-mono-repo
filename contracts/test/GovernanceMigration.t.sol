// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {GovernanceMigration} from "../script/GovernanceMigration.sol";

/// @notice Proves the two-phase governance hand-off for the single owner-hidden contract set.
contract GovernanceMigrationTest is Test {
    using GovernanceMigration for GovernanceMigration.Targets;

    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;
    bytes32 internal constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");
    bytes32 internal constant ISSUER_ROLE = keccak256("ISSUER");
    bytes32 internal constant VACCINATION = keccak256("VACCINATION");

    address internal constant EOA = address(0x119F8c7F);
    address internal constant MULTISIG = address(0x5AFE);
    address internal constant CUSTODIAN = address(0xC0570D1A);
    address internal constant VERIFIER = address(0xC1AC017);

    IssuerRegistry internal registry;
    VerificationRegistryConsent internal verification;
    DogTagSBTConsent internal sbt;
    DogTagIssuerFactory internal factory;

    function setUp() public {
        vm.startPrank(EOA);
        registry = new IssuerRegistry(EOA);
        DogTagIssuer impl = new DogTagIssuer();
        factory = new DogTagIssuerFactory(address(impl), address(registry), EOA);
        sbt = new DogTagSBTConsent(EOA, CUSTODIAN);
        verification =
            new VerificationRegistryConsent(address(registry), address(sbt), VERIFIER, address(factory), EOA);
        vm.stopPrank();
    }

    function _targets() internal view returns (GovernanceMigration.Targets memory) {
        return GovernanceMigration.Targets({
            issuerRegistry: address(registry),
            verificationRegistry: address(verification),
            sbt: address(sbt),
            factory: address(factory)
        });
    }

    function test_migration_hands_the_owner_hidden_set_to_multisig() public {
        GovernanceMigration.Targets memory targets = _targets();

        assertEq(registry.defaultAdmin(), EOA);
        assertEq(verification.defaultAdmin(), EOA);
        assertEq(sbt.defaultAdmin(), EOA);
        assertEq(factory.owner(), EOA);
        assertTrue(GovernanceMigration.supportsTwoStep(address(sbt)));

        vm.startPrank(EOA);
        targets.begin(MULTISIG);
        vm.stopPrank();

        (address pendingRegistry,) = registry.pendingDefaultAdmin();
        (address pendingVerification,) = verification.pendingDefaultAdmin();
        (address pendingSbt,) = sbt.pendingDefaultAdmin();
        assertEq(pendingRegistry, MULTISIG);
        assertEq(pendingVerification, MULTISIG);
        assertEq(pendingSbt, MULTISIG);
        assertEq(factory.pendingOwner(), MULTISIG);
        assertTrue(registry.hasRole(WHITELIST_ADMIN, MULTISIG));

        vm.prank(MULTISIG);
        vm.expectRevert();
        registry.acceptDefaultAdminTransfer();

        vm.warp(block.timestamp + 3 days + 1);
        vm.startPrank(MULTISIG);
        targets.accept(EOA);
        vm.stopPrank();

        _assertMultisigInControl();
    }

    function _assertMultisigInControl() internal {
        assertEq(registry.defaultAdmin(), MULTISIG, "IssuerRegistry admin");
        assertEq(verification.defaultAdmin(), MULTISIG, "VerificationRegistryConsent admin");
        assertEq(sbt.defaultAdmin(), MULTISIG, "DogTagSBTConsent admin");
        assertEq(factory.owner(), MULTISIG, "factory owner");

        assertFalse(registry.hasRole(DEFAULT_ADMIN_ROLE, EOA), "EOA !registry admin");
        assertFalse(registry.hasRole(WHITELIST_ADMIN, EOA), "EOA !whitelist admin");
        assertFalse(verification.hasRole(DEFAULT_ADMIN_ROLE, EOA), "EOA !verification admin");
        assertFalse(sbt.hasRole(DEFAULT_ADMIN_ROLE, EOA), "EOA !SBT admin");

        vm.startPrank(EOA);
        vm.expectRevert();
        registry.whitelistFor(VACCINATION, EOA);
        vm.expectRevert();
        verification.setRelayerRestriction(false);
        vm.expectRevert();
        sbt.grantRole(ISSUER_ROLE, EOA);
        vm.expectRevert();
        factory.createIssuer("x", VACCINATION, EOA);
        vm.stopPrank();

        vm.startPrank(MULTISIG);
        registry.whitelistFor(VACCINATION, EOA);
        verification.setRelayerRestriction(false);
        sbt.grantRole(ISSUER_ROLE, EOA);
        factory.createIssuer("Seaport Vacc", VACCINATION, EOA);
        vm.stopPrank();

        assertTrue(registry.isWhitelistedFor(VACCINATION, EOA));
        assertTrue(sbt.hasRole(ISSUER_ROLE, EOA));
        assertFalse(verification.restrictToWhitelistedRelayers());
    }
}
