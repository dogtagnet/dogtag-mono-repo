#!/usr/bin/env bash
# Write the mobile address bundles from the deploy ledger.
#
#   scripts/gen-mobile-roax-config.sh        (or: make gen-mobile-config)
#
# WHY THIS EXISTS
#
# `apps/*/roax.json` is COMPILE-TIME configuration: it ships inside the APK assets and the iOS app
# bundle, so what a phone reads was fixed when the app was built. For most of this repo's life those
# two files were hand-edited copies of the deploy ledger, and they drifted - by the time this script
# was written both still named a SUPERSEDED generation-1 `ProtocolRegistry`, whose getter really is
# `getContractSet`. The `getDiscoverySet` fix had already landed in both clients, and it was INERT:
# against the address the apps actually read, the corrected call hits an absent selector and reverts
# at the dispatcher with empty returndata, which both clients map to the same nil as an unpublished
# version. So the app failed closed for a reason that was not the real one, and no diff said so.
#
# The remedy is the same one layer 2 applied to the scripts: THE DEPLOY WRITES THE CONFIGURATION.
# `contracts/deployments/roax.json` is written by `contracts/script/Deploy.s.sol`; this script is its
# projection onto the two bundles. A redeploy is: run Deploy, re-run this, rebuild, reinstall.
#
# THE BUNDLES ARE GITIGNORED, AND THAT IS THE POINT
#
# A tracked `roax.json` carrying real addresses IS a hardcoded address in the tree - exactly what
# `make check-addresses` exists to refuse. Both bundle paths are in `apps/.gitignore`, alongside the
# vendored `consent_final.zkey` / `consent.graph`, which are gitignored for the same reason and whose
# absence AGENTS.md already calls "the guard, not a project bug". A fresh checkout has neither, and
# both apps refuse to build until this runs (iOS through its `pbxproj` resource reference, Android
# through the `requireRoaxConfig` gradle check).
#
# WHY THE KEYS ARE THE LEDGER'S NAMES, NOT THE OLD ONES
#
# The keys written below are the ledger's own key names. The previous bundles used a different set
# (`IssuerRegistry`, `DogTagSBT`), and renaming them is what makes a STALE bundle fail closed rather
# than fail silently. That case is real and is the sharpest hazard in this change: a developer with an
# existing checkout who pulls it keeps their old `roax.json` on disk - git stops tracking the file but
# does not remove it - so without a rename that phone would go on reading four retired contracts
# forever, with nothing in any diff to say so. With the rename, a stale bundle yields "" for every
# address, and every consumer already treats "" as could-not-check: the whitelist pillar answers
# `Unknown("no IssuerRegistry configured")`, the anchor read fails closed, and the credential shows
# UNVERIFIED rather than a confident wrong verdict.
#
# WHAT IS DELIBERATELY NOT WRITTEN
#
#   IssuerDomainRegistry  no source in this repo and no launch-set deployment. The one consumer (the
#                         credential-detail domain row) already renders a blank as "could not read",
#                         which is the honest state; writing a retired address would make it read a
#                         contract that decides nothing.
#   Poseidon6             no source, and no consumer on either platform.
#   DogTagIssuerImpl      the clone implementation. No mobile consumer.
#   admin                 a governance SIGNER, not a contract the phone reads.
#
# WHAT IS STILL BUNDLED THAT AGENTS.MD RULE 5 SAYS SHOULD NOT BE
#
# Rule 5 is "the phones bundle ONE generated anchor address and resolve factory, verification
# registry, SBT, verifier and provider registry from `getDiscoverySet` at runtime". This script does
# the first half. The second half is NOT a configuration change: `RecordImporter` / `Net.swift`
# `whitelistedAtIssuance` take these as SYNCHRONOUS string parameters and `CredentialRefresher` reads
# the SBT synchronously per credential, across 15 `RoaxConfig.load()` call sites on the two platforms.
# Making those resolve from an async, failable anchor read mints a new tri-state at every one of them,
# in the exact path AGENTS.md says must never render could-not-check as a verdict. It is recorded in
# AGENTS.md as the remaining layer-3 change rather than smuggled into this one.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/lib/ledger.sh
source "$ROOT/scripts/lib/ledger.sh"

IOS="$ROOT/apps/ios/DogTag/roax.json"
ANDROID="$ROOT/apps/android/app/src/main/assets/roax.json"

CHAIN_ID="$(ledger_chain_id)"
[ -n "$CHAIN_ID" ] || { echo "::error::the ledger declares no chainId" >&2; exit 1; }

# Resolved ONCE, up front, so a missing key fails before either file is touched. A partially written
# bundle is worse than none: half of it looks configured, and the half that is missing reads as
# could-not-check on a phone that has no way to tell the two apart.
PROTOCOL_REGISTRY="$(ledger_require ProtocolRegistry)"
PROVIDER_REGISTRY="$(ledger_require ProviderRegistry)"
FACTORY="$(ledger_require DogTagIssuerFactory)"
SBT="$(ledger_require DogTagSBTConsent)"

write_bundle() {
  local dest="$1"
  mkdir -p "$(dirname "$dest")"
  cat > "$dest" <<EOF
{
  "_": "GENERATED by scripts/gen-mobile-roax-config.sh from contracts/deployments/roax.json. Do not hand-edit: re-run after every deploy, then rebuild AND reinstall the app. Gitignored on purpose - a tracked copy is a hardcoded address.",
  "chainId": $CHAIN_ID,
  "ProtocolRegistry": "$PROTOCOL_REGISTRY",
  "ProviderRegistry": "$PROVIDER_REGISTRY",
  "DogTagIssuerFactory": "$FACTORY",
  "DogTagSBTConsent": "$SBT"
}
EOF
}

write_bundle "$IOS"
write_bundle "$ANDROID"

echo "wrote both mobile bundles from the ledger (chainId $CHAIN_ID, anchor ${PROTOCOL_REGISTRY:0:10}…):"
echo "  $IOS"
echo "  $ANDROID"
echo
echo "iOS: run this BEFORE 'xcodegen' - it sweeps apps/ios/DogTag/, so regenerating first silently"
echo "drops roax.json from the pbxproj's Copy Bundle Resources, exactly as it does the prover pair."
echo
echo "Editing a bundle is NOT a repoint. The app must be rebuilt AND reinstalled before any phone"
echo "reads these addresses - that is standing process on every full redeploy, not a caveat."
