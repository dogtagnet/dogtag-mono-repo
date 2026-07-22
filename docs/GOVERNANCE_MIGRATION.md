# Governance migration — deployer EOA → multisig (audit H-3)

This is the runbook for moving `DEFAULT_ADMIN_ROLE`/owner of every governed DogTag contract off the single deployer EOA (`roax.json:admin`, `0x119F8c7F…`) and onto a protocol multisig.

> **STATUS - two distinct tracks, do not conflate them.**
>
> **Track 1 - signer-1 Phase-2 (EXECUTED).**
> On 2026-07-05 (block 123835) an interim hand-off executed on-chain: the three governance authorities (`DogTagIssuerFactory` `Ownable2Step` ownership, the `IssuerRegistry` `WHITELIST_ADMIN` role, and the `IssuerRegistry`/`VerificationRegistry`/`DogTagSBT` `DEFAULT_ADMIN_ROLE`) moved off the deployer EOA (`0x119F8c7F…`) onto the single governance signer **signer-1 `0x8E27E117…`** (recorded in `deployments/roax.json` `_governance`; see AGENTS.md "Governance / admin").
> The deployer EOA is therefore **not role-free**: it still holds the legacy Level-A `DogTagSBT` `ISSUER_ROLE` and its known record-type whitelists, so never treat it as a neutral/spent key, and retiring those legacy issuance capabilities is separate, unfinished work.
>
> **Track 2 - full `N`-of-`M` multisig (PENDING, code shipped).**
> signer-1 is a single interim key, **not** the final multisig, so the full EOA-to-protocol-multisig transition this runbook targets has **not** run and **must never be described as complete.**
> The contracts, scripts, and test here are reviewed and ready; executing the multisig hand-off is a deliberate, **irreversible** ceremony (like the trusted-setup) and a captain decision that requires choosing the multisig signers and signing real transactions.
> Do **not** run the live phases below until the captain has approved and the multisig is chosen.
> Because Track 1 already moved the governance authorities to signer-1, that hand-off starts from **signer-1** (the current holder), not the deployer EOA; the "run by the deployer EOA" framing below describes the original Track-1 starting state.

## Why

Even after Track 1, a **single** externally-owned key holds the governance authorities: now signer-1 (`0x8E27E117…`), not the deployer EOA.
That key can whitelist issuers, mass-revoke roots, and (after the 2-day timelock) swap the verifier, while the legacy deployer EOA still retains the Level-A `DogTagSBT` `ISSUER_ROLE`/whitelists to mint.
A single-key admin is the largest trust gap in a soulbound-credential system.
Moving admin to an `N`-of-`M` multisig removes the single point of compromise while keeping every existing capability.

## What is governed

| Contract | Admin mechanism | Hand-off |
|---|---|---|
| `IssuerRegistry` | `AccessControlDefaultAdminRules` (3-day) + `WHITELIST_ADMIN` role | two-step `begin`/`accept` + grant/revoke `WHITELIST_ADMIN` |
| `VerificationRegistryConsent` | `AccessControlDefaultAdminRules` (2-day) | two-step `begin`/`accept` |
| `DogTagSBTConsent` (fresh deploy) | `AccessControlDefaultAdminRules` (3-day) | two-step `begin`/`accept` |
| a legacy plain-`AccessControlEnumerable` SBT instance | plain `AccessControlEnumerable` | atomic `grantRole`→`revokeRole` (see note) |
| `DogTagIssuerFactory` | `Ownable2Step` | `transferOwnership`→`acceptOwnership` |
| `DogTagIssuer` clones | governed via `IssuerRegistry` admin (`hasRole(0x00)`) | covered transitively by the `IssuerRegistry` hand-off; no own admin |
| `Groth16VerifierConsent`, `Poseidon6` | none | nothing to migrate |

> The owner-revealing Level-A contracts (`VerificationRegistry`, `DogTagSBT`, `ConsentKeyRegistry`, `Groth16Verifier`) were retired; `MigrateGovernance.s.sol` now reads the owner-hidden `VerificationRegistryConsent`/`DogTagSBTConsent` targets from `deployments/roax.json`. Their already-deployed Level-A instances remain on-chain, and the Track-1 hand-off (above) that moved their admin to signer-1 is historical.

**Legacy plain-`AccessControlEnumerable` SBT note.**
`script/GovernanceMigration.sol` auto-detects (`supportsTwoStep`) whether a target SBT is two-step (`AccessControlDefaultAdminRules`) or a legacy plain `AccessControlEnumerable` instance.
The forward Track-2 target `DogTagSBTConsent` (`0x96Cba…`) is a **fresh two-step deploy**, so it is handed over with the timelocked two-step `begin`/`accept`.
A plain `AccessControlEnumerable` SBT cannot be retrofitted on-chain — a soulbound token contract is not upgradeable, and redeploying would orphan every minted pet + every referencing credential — so such an instance is instead handed over with an atomic `grantRole(DEFAULT_ADMIN_ROLE, multisig)` then `revokeRole(DEFAULT_ADMIN_ROLE, eoa)`, the multisig itself being the security boundary.
The still-live Level-A `DogTagSBT` (`0x1FB8…`) is such a plain-`AccessControlEnumerable` instance.

## Procedure

### 0. Prerequisites

- The multisig is deployed and its address known → `export MULTISIG=0x…`.
- The account that currently holds the governance authorities runs the hand-off and has gas. After Track 1 that is **signer-1** (`0x8E27E117…`), not the deployer EOA (`roax.json:admin` `0x119F8c7F…` is retained only as the historical deploy record); the phrasing below still says "deployer EOA" from the original Track-1 run.
- `deployments/roax.json` reflects the live addresses.
- `export ROAX_RPC=…` and have the current holder's key available to `forge script --broadcast`.

### 1. Phase 1 — begin (run by the deployer EOA)

```bash
cd contracts
MULTISIG=0x<multisig> \
forge script script/MigrateGovernance.s.sol:MigrateGovernanceBegin \
  --rpc-url "$ROAX_RPC" --broadcast --legacy
```

This starts the two-step transfer on every `AccessControlDefaultAdminRules` contract, transfers `Ownable2Step` ownership of the factory (pending acceptance), pre-grants the multisig `WHITELIST_ADMIN`, and (for the legacy SBT) grants the multisig admin.
The EOA is **still** admin until Phase 2 — nothing is lost yet, and a mistake here can be cancelled (see Rollback).
The script logs each contract's accept-ETA.

### 2. Wait for the timelocks

`IssuerRegistry` and the fresh `DogTagSBTConsent` use a 3-day delay; `VerificationRegistryConsent` uses 2 days.
Phase 2 `acceptDefaultAdminTransfer()` reverts until each ETA passes.

### 3. Phase 2 — accept (run by / through the multisig)

The multisig must execute, for each contract, the calls in `GovernanceMigration.accept`:

- `IssuerRegistry.acceptDefaultAdminTransfer()` then `IssuerRegistry.revokeRole(WHITELIST_ADMIN, oldAdmin)`
- `VerificationRegistryConsent.acceptDefaultAdminTransfer()`
- `DogTagSBTConsent.acceptDefaultAdminTransfer()` (two-step) **or**, for a legacy plain-`AccessControlEnumerable` SBT, `revokeRole(0x00, oldAdmin)`
- `DogTagIssuerFactory.acceptOwnership()`

**If the multisig is a Safe** (a contract, not a single key), submit these as Safe transactions from the Safe UI/SDK — the call list above is the exact set, and the Phase 1 log echoes the targets.

**If the multisig is a key you control** (e.g. a 1-of-1 or an EOA-threshold scheme, or for an anvil dry-run), the accept script can broadcast it:

```bash
cd contracts
MULTISIG=0x<multisig> OLD_ADMIN=0x<deployer-eoa> \
forge script script/MigrateGovernance.s.sol:MigrateGovernanceAccept \
  --rpc-url "$ROAX_RPC" --broadcast --legacy
```

### 4. Verify

For each governed contract, confirm:

- `defaultAdmin()` / `owner()` `== MULTISIG`.
- `hasRole(DEFAULT_ADMIN_ROLE, oldEoa) == false` and `hasRole(WHITELIST_ADMIN, oldEoa) == false`.
- A privileged call from the old EOA reverts; the same call from the multisig succeeds.

`GovernanceMigrationTest` (in `test/GovernanceMigration.t.sol`) proves exactly this end-to-end on a local fork — run `forge test --match-contract GovernanceMigrationTest` for the reference assertions.

## Rollback

Before Phase 2 acceptance the hand-off is reversible:

- `cancelDefaultAdminTransfer()` (EOA) clears a pending `AccessControlDefaultAdminRules` transfer.
- `transferOwnership(eoa)` re-targets `Ownable2Step` (or simply never call `acceptOwnership`).
- The pre-granted `WHITELIST_ADMIN` / legacy-SBT admin grant to the multisig can be revoked by the EOA while it is still admin.

After acceptance the EOA has no admin and **cannot** roll back — only the multisig can transfer admin onward.
