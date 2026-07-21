// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Test} from "forge-std/Test.sol";
import {DeployProtocolRegistry} from "../script/DeployProtocolRegistry.s.sol";

contract DeployProtocolRegistryTest is Test {
    DeployProtocolRegistry internal deployer;

    function setUp() public {
        deployer = new DeployProtocolRegistry();
    }

    function test_mainnet_guard_accepts_only_two_day_default() public view {
        deployer.validatePublishTimelock(2 days, false);
    }

    function test_mainnet_guard_refuses_sub_two_day_timelock_without_testnet_opt_in() public {
        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(0, false);

        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days - 1, false);

        vm.expectRevert(bytes("mainnet publish timelock must be 2 days"));
        deployer.validatePublishTimelock(2 days + 1, false);
    }

    function test_explicit_testnet_opt_in_accepts_zero_timelock() public view {
        deployer.validatePublishTimelock(0, true);
    }
}
