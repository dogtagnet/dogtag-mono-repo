// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {IProviderAuthority} from "../src/DogTagIssuerV2.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";

/// @dev Pins the one claim the S-6/S-7 merge made checkable: that the four functions the generation-2
/// pair asks of the authority core are the core's OWN four, not a hand-copied approximation of them.
///
/// It is worth pinning because the pair reaches the core through an interface it declares LOCALLY
/// (`IProviderAuthority`, in `DogTagIssuerV2.sol`), and Solidity checks nothing across that seam. A
/// diverged argument list is a selector the core reverts on - which, on `canIssue`, means every anchor
/// through every generation-2 clone reverts - and a diverged return type or mutability is silently
/// misdecoded ABI, which is worse than a revert because it answers. Before this file the agreement
/// rested entirely on someone reading two declarations side by side, in a repo whose standing doctrine
/// is that a check which did not run counted as a check that passed.
///
/// **Why this suite uses the REAL core when `IssuerV2.t.sol` deliberately does not.** The two ask
/// different questions and need different authorities, so this is not a reversal of that file's
/// stand-in decision. `IssuerV2.t.sol` asks whether the pair consults the correct RUNG of the
/// capability ladder, which requires an authority whose registrar facts a test can move; it therefore
/// derives its three rungs in a local stand-in, and the gaps between them are the whole point. This
/// file asks whether the pair's interface and the deployed core describe the same four functions, and
/// the only thing that can answer that is the core itself. Asking the stand-in would be precisely the
/// self-agreement its own comment warns against: a copy of an interface cannot disagree with the copy.
///
/// **What this does NOT establish, stated so the pair of claims is not read as one.** Agreeing on a
/// shape says nothing about agreeing on an answer. Nothing here runs the pair against the core, so the
/// ladder's behaviour in this tree remains the stand-in's, and a divergence in either contract's
/// BEHAVIOUR would leave this suite green. Wiring the two together is a cutover step
/// (`docs/ISSUER_V2_OWNERSHIP.md` §8), not a gap this file closes.
contract IssuerV2ProviderAuthorityInterfaceTest is Test {
    /// @dev Any non-zero address. The core's constructor is `Ownable(authority)`, which rejects zero.
    address internal constant REGISTRAR = address(0xA11CE);

    ProviderRegistry internal core;

    function setUp() public {
        core = new ProviderRegistry(REGISTRAR);
    }

    /// The argument-list half of a signature. A selector is the keccak of the canonical signature, so an
    /// added, removed, reordered or retyped parameter on EITHER side moves it.
    function test_the_four_selectors_the_pair_asks_are_the_cores_own() public pure {
        assertEq(
            bytes32(IProviderAuthority.canCreateService.selector),
            bytes32(ProviderRegistry.canCreateService.selector),
            "canCreateService"
        );
        assertEq(
            bytes32(IProviderAuthority.canIssue.selector), bytes32(ProviderRegistry.canIssue.selector), "canIssue"
        );
        assertEq(
            bytes32(IProviderAuthority.canRevoke.selector), bytes32(ProviderRegistry.canRevoke.selector), "canRevoke"
        );
        assertEq(bytes32(IProviderAuthority.hasRole.selector), bytes32(ProviderRegistry.hasRole.selector), "hasRole");
    }

    /// The core must actually DISPATCH each of those selectors. Comparing declarations cannot show that:
    /// a selector both sides agree on is still dead if the deployed surface does not carry it.
    function test_the_real_core_answers_every_selector_the_pair_asks() public view {
        bytes[] memory payloads = new bytes[](4);
        payloads[0] = abi.encodeCall(IProviderAuthority.canCreateService, (bytes20(0), bytes32(0), address(0)));
        payloads[1] = abi.encodeCall(IProviderAuthority.canIssue, (address(0), address(0)));
        payloads[2] = abi.encodeCall(IProviderAuthority.canRevoke, (address(0), address(0)));
        payloads[3] = abi.encodeCall(IProviderAuthority.hasRole, (bytes32(0), address(0)));

        for (uint256 i = 0; i < payloads.length; i++) {
            (bool ok, bytes memory data) = address(core).staticcall(payloads[i]);
            assertTrue(ok, "the core did not answer a selector the pair asks");
            assertEq(data.length, 32, "the core's answer was not a single word");
        }

        // The negative control, and it is load-bearing: without it every assertion above would also pass
        // on a core that answers ANYTHING, so the loop would be measuring dispatch it never established.
        // The probe is a well-formed two-address call differing from `canIssue` in the SELECTOR ALONE,
        // because an arity-mismatched call reverts inside the ABI decoder before dispatch is ever
        // reached - which makes such a control pass while proving nothing about selectors at all.
        (bool answeredUnknown,) = address(core).staticcall(
            abi.encodeWithSelector(bytes4(keccak256("canIssueNothing(address,address)")), address(0), address(0))
        );
        assertFalse(answeredUnknown, "the core answered a selector it does not declare");
    }

    /// The half of a signature no selector can see: the return type and the state mutability. Both sides
    /// are assigned to ONE written function type, so a divergence on either side fails the BUILD instead
    /// of surviving to misdecode at runtime.
    /// Each pair is bound inside its OWN block, which is not stylistic: an external function type
    /// occupies two stack slots, so eight live at once overflows the legacy codegen's stack and the
    /// suite fails to build rather than failing to pin anything.
    function test_a_diverged_return_type_or_mutability_fails_the_build() public view {
        // Not vacuous: a binding that compiles is still worthless if it does not dispatch, so every
        // pointer is invoked, and each pair's two selectors are compared so the two sides are shown to
        // be the same function rather than merely two functions of the same shape. The inputs are chosen
        // to be answerable rather than authorized - an unattached service and a non-owner both resolve
        // to a definite `false`.
        {
            function(bytes20, bytes32, address) external view returns (bool) fromCore = core.canCreateService;
            function(bytes20, bytes32, address) external view returns (bool) fromPair =
                IProviderAuthority(address(core)).canCreateService;
            assertFalse(fromCore(bytes20(0), bytes32(0), address(0)), "canCreateService via the core");
            assertFalse(fromPair(bytes20(0), bytes32(0), address(0)), "canCreateService via the pair");
            assertEq(bytes32(fromCore.selector), bytes32(fromPair.selector), "canCreateService");
        }
        {
            function(address, address) external view returns (bool) fromCore = core.canIssue;
            function(address, address) external view returns (bool) fromPair =
                IProviderAuthority(address(core)).canIssue;
            assertFalse(fromCore(address(0), address(0)), "canIssue via the core");
            assertFalse(fromPair(address(0), address(0)), "canIssue via the pair");
            assertEq(bytes32(fromCore.selector), bytes32(fromPair.selector), "canIssue");
        }
        {
            function(address, address) external view returns (bool) fromCore = core.canRevoke;
            function(address, address) external view returns (bool) fromPair =
                IProviderAuthority(address(core)).canRevoke;
            assertFalse(fromCore(address(0), address(0)), "canRevoke via the core");
            assertFalse(fromPair(address(0), address(0)), "canRevoke via the pair");
            assertEq(bytes32(fromCore.selector), bytes32(fromPair.selector), "canRevoke");
        }
        {
            function(bytes32, address) external view returns (bool) fromCore = core.hasRole;
            function(bytes32, address) external view returns (bool) fromPair =
                IProviderAuthority(address(core)).hasRole;
            assertFalse(fromCore(bytes32(0), address(0)), "hasRole via the core");
            assertFalse(fromPair(bytes32(0), address(0)), "hasRole via the pair");
            assertEq(bytes32(fromCore.selector), bytes32(fromPair.selector), "hasRole");
        }
    }
}
