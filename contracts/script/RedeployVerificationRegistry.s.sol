// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {VerificationRegistry} from "../src/VerificationRegistry.sol";

/// @notice Registry-ONLY redeploy: deploys a FRESH VerificationRegistry that ships the v2 six-arg
/// `recordVerificationZK(a,b,c,pub,recordType,deadline)` (selector 0x423a45b6), wired to the EXISTING
/// live ROAX component contracts — including the EXISTING ConsentKeyRegistry (unlike
/// RedeployConsentKey.s.sol, this does NOT deploy a new CKR; every component is reused).
///
/// Root cause this fixes (see data/dogtag-zkfail-z9/report.md): the live registry
/// 0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1 was deployed at 21c9b15, BEFORE PR #7 (64ea1f8) added
/// `recordType`/`deadline` to the ZK path. Its bytecode therefore only dispatches the OLD 4-arg selector
/// 0xdd080593; the v2 server/phone now encode the 6-arg selector 0x423a45b6, which the old bytecode has
/// no case for and no fallback -> bare revert() -> `execution reverted, data: "0x"` on the groomer ZK
/// export. Redeploying the v2 contract (this script) makes 0x423a45b6 dispatchable and also finally lands
/// the F3 deadline-expiry hardening on the ZK path.
///
/// @dev VR constructor order = (issuerRegistry, sbt, zkVerifier, consentKeys, rootIndex, poseidon6, admin),
/// where `rootIndex` is the DogTagIssuerFactory. All seven default to the current live ROAX values
/// (deployments/roax.json); override any via env for another environment. ADMIN defaults to the
/// governance authority 0x8E27E117... (the Phase-2 authority), NOT the stripped deployer EOA 0x119F8c7F...
/// Broadcast with the governance signer (index 1):
///   forge script contracts/script/RedeployVerificationRegistry.s.sol:RedeployVerificationRegistry \
///     --rpc-url https://devrpc.roax.net --broadcast --private-key $GOV_KEY
contract RedeployVerificationRegistry is Script {
    address public verification; // NEW

    function run() external {
        address issuerRegistry = vm.envOr("ISSUER_REGISTRY", 0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c);
        address sbt = vm.envOr("SBT", 0x1FB8986573Ac36d532cF7d5a5352202B094D4233);
        address zkVerifier = vm.envOr("ZK_VERIFIER", 0xEEFCfAF026931b7325472A88fd14Ee780Da13559);
        address consentKeys = vm.envOr("CONSENT_KEYS", 0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671);
        address rootIndex = vm.envOr("ROOT_INDEX", 0xd3179AbBfb0274D0a5F7017d76015A93C159511D);
        address poseidon6 = vm.envOr("POSEIDON6", 0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db);
        address admin = vm.envOr("ADMIN", 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2);

        // Fail fast on a mis-wired input. The VR constructor already rejects a zero
        // ir/sbt/ck/ridx/pos6, but it ALLOWS zkVerifier == 0 (ECDSA-only registries). For this
        // ZK-path fix a zero verifier would silently reship the broken state, so reject it here too.
        require(
            issuerRegistry != address(0) && sbt != address(0) && zkVerifier != address(0)
                && consentKeys != address(0) && rootIndex != address(0) && poseidon6 != address(0)
                && admin != address(0),
            "zero component"
        );

        vm.startBroadcast();
        verification = address(
            new VerificationRegistry(
                issuerRegistry, sbt, zkVerifier, consentKeys, rootIndex, poseidon6, admin
            )
        );
        vm.stopBroadcast();

        console2.log("--- reused live components ---");
        console2.log("IssuerRegistry      ", issuerRegistry);
        console2.log("DogTagSBT           ", sbt);
        console2.log("ZkVerifier (v2)     ", zkVerifier);
        console2.log("ConsentKeyRegistry  ", consentKeys);
        console2.log("RootIndex (Factory) ", rootIndex);
        console2.log("Poseidon6           ", poseidon6);
        console2.log("Admin (governance)  ", admin);
        console2.log("--- NEW VerificationRegistry (update deployments/roax.json + apps/ios/DogTag/roax.json) ---");
        console2.log("VerificationRegistry", verification);
    }
}
