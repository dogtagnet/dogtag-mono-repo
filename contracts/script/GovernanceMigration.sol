// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {
    IAccessControlDefaultAdminRules
} from "@openzeppelin/contracts/access/extensions/IAccessControlDefaultAdminRules.sol";

/// @notice Minimal Ownable2Step surface (OZ v5 ships no `IOwnable2Step`).
interface IOwnable2Step {
    function owner() external view returns (address);
    function pendingOwner() external view returns (address);
    function transferOwnership(address newOwner) external;
    function acceptOwnership() external;
}

/// @title GovernanceMigration — moves protocol admin/owner from the deployer EOA to a multisig.
/// @notice Single source of truth for the H-3 hand-off, shared by the forge scripts (live execution)
/// and the forge/anvil test (proof on a local fork). The migration is a TWO-PHASE, two-actor flow:
///
///   Phase 1 `begin()`  — broadcast by the CURRENT EOA admin. Proposes / starts the hand-off on every
///                        governed contract and pre-grants the multisig the operational `WHITELIST_ADMIN`.
///   Phase 2 `accept()` — executed BY (or through) the multisig AFTER each contract's timelock elapses.
///                        The multisig accepts admin/ownership everywhere, then strips the EOA's residual
///                        roles so the deployer key can no longer act.
///
/// Coverage of the governed surface (see architecture §13.1 H-3):
///   - IssuerRegistry        AccessControlDefaultAdminRules (3-day) DEFAULT_ADMIN + WHITELIST_ADMIN role
///   - VerificationRegistry  AccessControlDefaultAdminRules (2-day) DEFAULT_ADMIN
///   - DogTagSBT             AccessControlDefaultAdminRules (3-day) DEFAULT_ADMIN  *for a fresh deploy*;
///                           a LEGACY live instance is still plain AccessControlEnumerable (no on-chain
///                           two-step retrofit without a state-orphaning redeploy), so it is handed over
///                           with an atomic grant→revoke instead. `_supportsTwoStep` auto-detects which.
///   - DogTagIssuerFactory   Ownable2Step owner
///   - DogTagIssuer clones    governed THROUGH IssuerRegistry's DEFAULT_ADMIN (hasRole(0x00)) — covered
///                           transitively by the IssuerRegistry hand-off; they hold no own admin.
///   - ConsentKeyRegistry / Groth16Verifier / Poseidon6 — permissionless / no admin; nothing to migrate.
library GovernanceMigration {
    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;
    // keccak256("WHITELIST_ADMIN") — IssuerRegistry's operational whitelist role (kept off DEFAULT_ADMIN).
    bytes32 internal constant WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN");

    struct Targets {
        address issuerRegistry;
        address verificationRegistry;
        address sbt;
        address factory;
    }

    /// @dev True iff `c` implements the two-step `AccessControlDefaultAdminRules` admin flow.
    function supportsTwoStep(address c) internal view returns (bool) {
        try IAccessControlDefaultAdminRules(c).defaultAdmin() returns (address) {
            return true;
        } catch {
            return false;
        }
    }

    /// @notice Phase 1 — must be sent by the current EOA admin (broadcast / prank as that key).
    function begin(Targets memory t, address multisig) internal {
        require(multisig != address(0), "multisig=0");

        // IssuerRegistry: start the two-step DEFAULT_ADMIN transfer + pre-grant the operational role so
        // the multisig can whitelist the instant it accepts (no whitelist-admin gap during the timelock).
        IAccessControlDefaultAdminRules(t.issuerRegistry).beginDefaultAdminTransfer(multisig);
        IAccessControl(t.issuerRegistry).grantRole(WHITELIST_ADMIN, multisig);

        // VerificationRegistry: start the two-step DEFAULT_ADMIN transfer.
        IAccessControlDefaultAdminRules(t.verificationRegistry).beginDefaultAdminTransfer(multisig);

        // DogTagSBT: two-step for a fresh deploy; atomic grant for a legacy plain-AccessControl instance.
        if (supportsTwoStep(t.sbt)) {
            IAccessControlDefaultAdminRules(t.sbt).beginDefaultAdminTransfer(multisig);
        } else {
            IAccessControl(t.sbt).grantRole(DEFAULT_ADMIN_ROLE, multisig);
        }

        // DogTagIssuerFactory: start the Ownable2Step ownership transfer.
        IOwnable2Step(t.factory).transferOwnership(multisig);
    }

    /// @notice Phase 2 — must be sent by/through the multisig AFTER the per-contract timelocks elapse.
    /// @param oldAdmin the deployer EOA whose residual roles are stripped.
    function accept(Targets memory t, address oldAdmin) internal {
        // IssuerRegistry: accept admin, then revoke the EOA's operational WHITELIST_ADMIN. (Accepting the
        // two-step transfer already revokes the EOA's DEFAULT_ADMIN_ROLE.)
        IAccessControlDefaultAdminRules(t.issuerRegistry).acceptDefaultAdminTransfer();
        IAccessControl(t.issuerRegistry).revokeRole(WHITELIST_ADMIN, oldAdmin);

        // VerificationRegistry: accept admin (EOA's DEFAULT_ADMIN is revoked by the accept).
        IAccessControlDefaultAdminRules(t.verificationRegistry).acceptDefaultAdminTransfer();

        // DogTagSBT: accept (two-step) or strip the EOA's admin (legacy atomic hand-over).
        if (supportsTwoStep(t.sbt)) {
            IAccessControlDefaultAdminRules(t.sbt).acceptDefaultAdminTransfer();
        } else {
            IAccessControl(t.sbt).revokeRole(DEFAULT_ADMIN_ROLE, oldAdmin);
        }

        // DogTagIssuerFactory: accept ownership (revokes the EOA owner).
        IOwnable2Step(t.factory).acceptOwnership();
    }
}
