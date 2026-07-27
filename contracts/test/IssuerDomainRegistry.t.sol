// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IssuerRegistry} from "../src/IssuerRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {IssuerDomainRegistry} from "../src/IssuerDomainRegistry.sol";

/// @notice The on-chain half of the bidirectional issuer↔domain binding.
///
/// The load-bearing properties, in order of what a regression would cost:
///
///   * **Tier 2 authorization is a real proof, not a label.** `test_spawning_business_may_self_update`
///     and `test_random_address_may_not_set` together assert that the ONLY non-admin address a clone
///     accepts is the one whose `predictIssuer(recordType, who)` reproduces that exact clone address.
///     This is what lets an issuer self-serve with no new clone implementation.
///   * **"No binding" is a normal state, not an error.** `test_unknown_clone_reads_as_no_binding` pins
///     that `getBinding` returns a zeroed record rather than reverting, and that `updatedAt` is the
///     discriminator. A resolver that mistook a zeroed record for a verified one would reintroduce the
///     fail-open the whole feature exists to remove.
///   * **A malformed domain is unrepresentable.** `test_rejects_malformed_domains` keeps unusable
///     claims out of storage, so the off-chain verifier never has to render "could not check" forever
///     because someone stored `https://Foo.COM/`.
contract IssuerDomainRegistryTest is Test {
    IssuerRegistry registry;
    DogTagIssuerFactory factory;
    IssuerDomainRegistry domains;
    DogTagIssuer clone_;

    address admin = address(0xA11CE);
    /// @dev The business the clone is SALTED for — tier 2's self-service key.
    address business = address(0xB0B);
    /// @dev A KYC'd organisation key appointed under tier 3 (never a salt input).
    address appointed = address(0xDA1);
    address stranger = address(0xBAD);

    bytes32 constant VACCINATION = keccak256("VACCINATION");

    function setUp() public {
        registry = new IssuerRegistry(admin);
        address impl = address(new DogTagIssuer());
        factory = new DogTagIssuerFactory(impl, address(registry), admin);

        vm.prank(admin);
        clone_ = DogTagIssuer(factory.createIssuer("Vacc", VACCINATION, business));

        domains = new IssuerDomainRegistry(address(factory), address(registry));
    }

    // ---------------------------------------------------------------------------------------------
    // Tier 1 — the protocol/KYC operator
    // ---------------------------------------------------------------------------------------------

    function test_whitelist_admin_may_set_domain() public {
        vm.prank(admin);
        domains.setDomain(address(clone_), "moh.gov.sg");

        IssuerDomainRegistry.Binding memory b = domains.getBinding(address(clone_));
        assertEq(b.domain, "moh.gov.sg");
        assertEq(b.setBy, admin);
        assertTrue(b.updatedAt != 0);
        assertTrue(domains.hasBinding(address(clone_)));
        assertEq(domains.domainOf(address(clone_)), "moh.gov.sg");
    }

    // ---------------------------------------------------------------------------------------------
    // Tier 2 — the spawning business, proven via predictIssuer
    // ---------------------------------------------------------------------------------------------

    /// The headline property: the clone stores no owner, yet its own business can still write — because
    /// `predictIssuer(recordType, business)` reproduces the clone address from the CREATE2 salt.
    function test_spawning_business_may_self_update() public {
        assertEq(factory.predictIssuer(VACCINATION, business), address(clone_), "salt precondition");

        vm.prank(business);
        domains.setDomain(address(clone_), "vet.example.com");

        assertEq(domains.domainOf(address(clone_)), "vet.example.com");
        assertTrue(domains.canSetDomain(address(clone_), business));
    }

    function test_random_address_may_not_set() public {
        assertFalse(domains.canSetDomain(address(clone_), stranger));
        vm.prank(stranger);
        vm.expectRevert(IssuerDomainRegistry.NotAuthorized.selector);
        domains.setDomain(address(clone_), "attacker.example.com");
    }

    /// The business of a DIFFERENT clone must not reach this one — tier 2 is per-clone, not global.
    function test_business_of_another_clone_may_not_set() public {
        address otherBusiness = address(0xB0B2);
        vm.prank(admin);
        DogTagIssuer other = DogTagIssuer(factory.createIssuer("Other", VACCINATION, otherBusiness));

        assertFalse(domains.canSetDomain(address(clone_), otherBusiness));
        vm.prank(otherBusiness);
        vm.expectRevert(IssuerDomainRegistry.NotAuthorized.selector);
        domains.setDomain(address(clone_), "wrong.example.com");

        // ...but it CAN set its own.
        vm.prank(otherBusiness);
        domains.setDomain(address(other), "other.example.com");
        assertEq(domains.domainOf(address(other)), "other.example.com");
    }

    // ---------------------------------------------------------------------------------------------
    // Tier 3 — the appointed domainAdmin (self-service for already-salted clones)
    // ---------------------------------------------------------------------------------------------

    function test_appointed_domain_admin_may_self_update() public {
        assertFalse(domains.canSetDomain(address(clone_), appointed));

        vm.prank(admin);
        domains.setDomainAdmin(address(clone_), appointed);

        vm.prank(appointed);
        domains.setDomain(address(clone_), "appointed.example.com");
        assertEq(domains.domainOf(address(clone_)), "appointed.example.com");
    }

    function test_only_whitelist_admin_may_appoint() public {
        vm.prank(stranger);
        vm.expectRevert(IssuerDomainRegistry.NotAuthorized.selector);
        domains.setDomainAdmin(address(clone_), stranger);

        // Even the clone's own business cannot appoint — appointment is a KYC decision.
        vm.prank(business);
        vm.expectRevert(IssuerDomainRegistry.NotAuthorized.selector);
        domains.setDomainAdmin(address(clone_), stranger);
    }

    function test_appointment_is_revocable() public {
        vm.prank(admin);
        domains.setDomainAdmin(address(clone_), appointed);
        assertTrue(domains.canSetDomain(address(clone_), appointed));

        vm.prank(admin);
        domains.setDomainAdmin(address(clone_), address(0));
        assertFalse(domains.canSetDomain(address(clone_), appointed));
    }

    /// `address(0)` as an appointee must never authorize `address(0)` itself.
    function test_zero_appointee_authorizes_nobody() public view {
        assertFalse(domains.canSetDomain(address(clone_), address(0)));
    }

    // ---------------------------------------------------------------------------------------------
    // Absence is a normal state
    // ---------------------------------------------------------------------------------------------

    function test_unknown_clone_reads_as_no_binding() public view {
        IssuerDomainRegistry.Binding memory b = domains.getBinding(address(clone_));
        assertEq(b.updatedAt, 0, "updatedAt==0 is the no-binding discriminator");
        assertEq(b.domain, "");
        assertEq(b.setBy, address(0));
        assertFalse(domains.hasBinding(address(clone_)));
    }

    /// A non-clone address is not a binding target at all — a domain claim for one is meaningless.
    function test_non_clone_is_rejected() public {
        address notAClone = address(0x1234);
        assertFalse(domains.canSetDomain(notAClone, admin));
        vm.prank(admin);
        vm.expectRevert(IssuerDomainRegistry.NotAClone.selector);
        domains.setDomain(notAClone, "nope.example.com");
    }

    function test_clear_returns_to_no_binding() public {
        vm.prank(business);
        domains.setDomain(address(clone_), "vet.example.com");
        assertTrue(domains.hasBinding(address(clone_)));

        vm.prank(business);
        domains.clearDomain(address(clone_));

        assertFalse(domains.hasBinding(address(clone_)));
        assertEq(domains.domainOf(address(clone_)), "");
        // Still enumerable, so the future re-check cron can observe the withdrawal.
        assertEq(domains.boundCloneCount(), 1);
        assertEq(domains.boundClones(0), address(clone_));
    }

    function test_enumeration_does_not_duplicate_on_update() public {
        vm.prank(business);
        domains.setDomain(address(clone_), "one.example.com");
        vm.prank(business);
        domains.setDomain(address(clone_), "two.example.com");

        assertEq(domains.boundCloneCount(), 1);
        assertEq(domains.domainOf(address(clone_)), "two.example.com");
    }

    // ---------------------------------------------------------------------------------------------
    // Domain validation — a malformed claim is unrepresentable
    // ---------------------------------------------------------------------------------------------

    function test_rejects_malformed_domains() public {
        string[10] memory bad = [
            "", // empty
            "localhost", // single label
            "MOH.GOV.SG", // uppercase: rejected, not folded
            "https://moh.gov.sg", // a URL, not a name
            "moh.gov.sg/", // trailing path
            ".moh.gov.sg", // leading dot
            "moh.gov.sg.", // trailing dot
            "moh..gov.sg", // empty label
            "-moh.gov.sg", // label starts with '-'
            "moh-.gov.sg" // label ends with '-'
        ];
        for (uint256 i; i < bad.length; i++) {
            vm.prank(admin);
            vm.expectRevert(IssuerDomainRegistry.BadDomain.selector);
            domains.setDomain(address(clone_), bad[i]);
        }
    }

    function test_accepts_realistic_domains() public {
        string[4] memory good = ["moh.gov.sg", "a.io", "vet-clinic.example.co.uk", "x1.sub2.example.com"];
        for (uint256 i; i < good.length; i++) {
            vm.prank(admin);
            domains.setDomain(address(clone_), good[i]);
            assertEq(domains.domainOf(address(clone_)), good[i]);
        }
    }

    function test_rejects_overlong_domain() public {
        // 254 bytes of valid LDH + dots — one over the 253 limit.
        bytes memory buf = new bytes(254);
        for (uint256 i; i < 254; i++) {
            buf[i] = (i % 10 == 9) ? bytes1(".") : bytes1("a");
        }
        vm.prank(admin);
        vm.expectRevert(IssuerDomainRegistry.BadDomain.selector);
        domains.setDomain(address(clone_), string(buf));
    }

    function test_rejects_overlong_label() public {
        bytes memory buf = new bytes(64);
        for (uint256 i; i < 64; i++) {
            buf[i] = bytes1("a");
        }
        vm.prank(admin);
        vm.expectRevert(IssuerDomainRegistry.BadDomain.selector);
        domains.setDomain(address(clone_), string(abi.encodePacked(string(buf), ".com")));
    }

    // ---------------------------------------------------------------------------------------------
    // Construction
    // ---------------------------------------------------------------------------------------------

    function test_rejects_zero_constructor_args() public {
        vm.expectRevert(IssuerDomainRegistry.ZeroAddress.selector);
        new IssuerDomainRegistry(address(0), address(registry));
        vm.expectRevert(IssuerDomainRegistry.ZeroAddress.selector);
        new IssuerDomainRegistry(address(factory), address(0));
    }

    /// Delisting an operator key must remove its tier-1 power here too — this contract holds no copy of
    /// the role, it reads the live registry every time.
    function test_tier1_tracks_the_live_registry_role() public {
        address operator = address(0x09E7);
        // Hoisted: `vm.prank` applies to the next CALL, and `WHITELIST_ADMIN()` is one.
        bytes32 role = registry.WHITELIST_ADMIN();
        assertFalse(domains.canSetDomain(address(clone_), operator));

        vm.prank(admin);
        registry.grantRole(role, operator);
        assertTrue(domains.canSetDomain(address(clone_), operator));

        vm.prank(admin);
        registry.revokeRole(role, operator);
        assertFalse(domains.canSetDomain(address(clone_), operator));
    }
}
