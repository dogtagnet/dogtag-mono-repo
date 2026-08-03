// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {Deploy} from "../script/Deploy.s.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {DogTagIssuer} from "../src/DogTagIssuer.sol";
import {DogTagIssuerFactory} from "../src/DogTagIssuerFactory.sol";
import {DogTagSBTConsent} from "../src/DogTagSBTConsent.sol";
import {VerificationRegistryConsent} from "../src/VerificationRegistryConsent.sol";
import {Groth16VerifierConsent} from "../src/Groth16VerifierConsent.sol";
import {ProtocolRegistry} from "../src/ProtocolRegistry.sol";

/// @title LaunchStack — the whole deployed system, stood up by the REAL deploy script.
///
/// @notice `_deployLaunchStack()` calls `script/Deploy.s.sol` itself rather than re-assembling the
/// system. That is the point: a helper that hand-wired an equivalent stack would let every suite built
/// on it pass against a topology no deployment has, and would leave the script — the one thing a real
/// deployment runs — covered by nothing. Here the ordering, the constructor probes and the read-back
/// assertions are exercised by every suite that needs a working chain.
///
/// Nothing here is a mock. `REGISTRAR` is the `Deploy` contract itself, because the registrar wiring
/// is `onlyOwner` on a core the script has just handed to the admin it was given, so the caller and
/// the admin have to be the same address. Suites prank as `REGISTRAR` for registrar actions exactly as
/// an operator would sign as the authority key.
///
/// # Issuance takes SIX registrar facts, and a suite that skips one gets a silent refusal
///
/// `DogTagIssuer.issue` is gated by `ProviderRegistry.canIssue(service, signer)`, which folds a
/// registered provider in ACTIVE standing, a service attached under a registered factory generation,
/// a confirmed live owner, an effective service standing, that service being the provider's CURRENT
/// pointer for its record type, and an issuance grant for the signer. Missing any one of them reverts
/// the anchor rather than failing a check the test can see, so {_onboardIssuingClone} performs all of
/// them and returns a clone that can actually anchor.
abstract contract LaunchStack is Test {
    /// @dev The registrar / protocol authority. Owns `ProviderRegistry` and admins the SBT, the
    /// verification registry and the discovery registry — one key, exactly as the deploy script wires
    /// it. Set by {_deployLaunchStack} to the `Deploy` instance that performed the deployment.
    address internal REGISTRAR;

    /// @dev The SBT's immutable neutral custodian. Every tag is minted here and never to an owner, so
    /// a suite that reintroduced an `ownerOf == owner` comparison anywhere would fail rather than pass.
    address internal constant CUSTODIAN = address(0xC0FFEE);

    bytes32 internal constant IDENTITY_DIGEST = keccak256("publication-safe-identity");

    /// @dev The discovery registry's immutable delay. The floor rather than the 2-day production value,
    /// so a suite that advances time to reach an ETA does not have to advance two days.
    uint256 internal constant TEST_PUBLISH_TIMELOCK = 1 hours;

    Deploy internal deployer;
    bytes32 internal FACTORY_GENERATION;

    ProviderRegistry internal core;
    DogTagIssuer internal implementation;
    DogTagIssuerFactory internal factory;
    DogTagSBTConsent internal sbt;
    Groth16VerifierConsent internal verifier;
    VerificationRegistryConsent internal vr;
    ProtocolRegistry internal protocolRegistry;

    /// @dev Runs the real deploy script and binds its results. The script performs the ordering, the
    /// registrar wiring and its own read-back assertions, so a suite that gets past this line is on a
    /// stack that a real deployment would also have produced.
    function _deployLaunchStack() internal {
        deployer = new Deploy();
        REGISTRAR = address(deployer);
        FACTORY_GENERATION = deployer.FACTORY_GENERATION();

        deployer.deploy(REGISTRAR, CUSTODIAN, REGISTRAR, TEST_PUBLISH_TIMELOCK);

        core = ProviderRegistry(deployer.providerRegistry());
        implementation = DogTagIssuer(deployer.issuerImpl());
        factory = DogTagIssuerFactory(deployer.factory());
        sbt = DogTagSBTConsent(deployer.sbt());
        verifier = Groth16VerifierConsent(deployer.verifier());
        vr = VerificationRegistryConsent(deployer.verificationRegistry());
        protocolRegistry = ProtocolRegistry(deployer.protocolRegistry());
    }

    /// @dev Registers a provider, clears it for a record type, deploys its clone FROM the provider's
    /// own key, attaches it, and grants `signer` the issuance capability on it.
    ///
    /// The two-key split is real and is preserved rather than hidden: `createIssuer` is the provider's
    /// transaction and `attachService` is `onlyOwner`, so a helper that ran both from one key would
    /// let a suite assert a journey no provider can walk.
    function _onboardIssuingClone(
        bytes20 providerId,
        address providerKey,
        bytes32 recordType,
        address signer
    ) internal returns (address clone) {
        vm.startPrank(REGISTRAR);
        core.registerProvider(
            providerId, providerKey, IDENTITY_DIGEST, 1, 0xe3, 0x12, bytes("ipfs://identity")
        );
        core.setProviderStanding(providerId, ProviderRegistry.Standing.ACTIVE);
        core.setServiceCreationApproval(providerId, recordType, true);
        vm.stopPrank();

        vm.prank(providerKey);
        clone = factory.createIssuer(providerId, recordType, 0);

        vm.startPrank(REGISTRAR);
        core.attachService(providerId, clone, FACTORY_GENERATION, providerKey);
        core.setServiceStanding(clone, ProviderRegistry.Standing.ACTIVE);
        // The grant is on the SIGNER's address and carries no service. Idempotent because `setRights`
        // refuses a no-op, and a harness that stands up two services for the same signer would
        // otherwise revert on the second.
        uint256 issueRight = core.RIGHT_ISSUE();
        if (core.rightsOf(signer) & issueRight == 0) {
            core.setRights(signer, issueRight);
        }
        vm.stopPrank();

        // The provider's CURRENT pointer for this record type. `canIssue` requires it, so a clone that
        // exists and is attached still anchors nothing until its owner designates it.
        vm.prank(providerKey);
        core.repointService(clone);
    }

    /// @dev Mints a tag to the custodian with `profileRoot = root`. `profileRoot` is write-once and the
    /// verification registry binds `R == profileRoot(dogTagId)`, so this is the only way a tag ever
    /// comes to answer for a root.
    function _mintCustodialTag(uint256 dogTagId, bytes32 root) internal {
        bytes32 issuerRole = sbt.ISSUER_ROLE();
        vm.prank(REGISTRAR);
        sbt.grantRole(issuerRole, address(this));
        sbt.mintCustodial(dogTagId, root);
    }

    /// @dev Approves `relayer` to submit verifications for `purpose`. The verification registry asks
    /// the core's own `canVerify`, so this is the VERIFY axis and is orthogonal to issuance.
    function _approveRelayer(bytes32 purpose, address relayer) internal {
        vm.prank(REGISTRAR);
        core.setVerifierCapability(purpose, relayer, true);
    }
}
