# Governance migration — deployer EOA → multisig (audit H-3)

This is the runbook for moving `DEFAULT_ADMIN_ROLE`/owner of every governed DogTag contract off the single deployer EOA (`roax.json:admin`, `0x119F8c7F…`) and onto a protocol multisig.

> **STATUS: code shipped, on-chain execution PENDING.**
> The contracts, scripts, and test in this PR are reviewed and ready.
> Executing the hand-off is a deliberate, **irreversible** ceremony — like the trusted-setup — and is a captain decision: it requires choosing the multisig signers and signing real transactions.
> Do **not** run the live phases below until the captain has approved and the multisig is chosen.

## Why

Today one externally-owned key can whitelist issuers, mint/burn SBTs, mass-revoke roots, recover/re-bind tokens, and (after the 2-day timelock) swap the verifier.
A single-key admin is the largest trust gap in a soulbound-credential system.
Moving admin to an `N`-of-`M` multisig removes the single point of compromise while keeping every existing capability.

## What is governed

| Contract | Admin mechanism | Hand-off |
|---|---|---|
| `IssuerRegistry` | `AccessControlDefaultAdminRules` (3-day) + `WHITELIST_ADMIN` role | two-step `begin`/`accept` + grant/revoke `WHITELIST_ADMIN` |
| `VerificationRegistry` | `AccessControlDefaultAdminRules` (2-day) | two-step `begin`/`accept` |
| `DogTagSBT` (fresh deploy) | `AccessControlDefaultAdminRules` (3-day) | two-step `begin`/`accept` |
| `DogTagSBT` (**currently live**) | plain `AccessControlEnumerable` | atomic `grantRole`→`revokeRole` (see note) |
| `DogTagIssuerFactory` | `Ownable2Step` | `transferOwnership`→`acceptOwnership` |
| `DogTagIssuer` clones | governed via `IssuerRegistry` admin (`hasRole(0x00)`) | covered transitively by the `IssuerRegistry` hand-off; no own admin |
| `ConsentKeyRegistry` | none (permissionless bind) | nothing to migrate |
| `Groth16Verifier`, `Poseidon6` | none | nothing to migrate |

**Legacy `DogTagSBT` note.**
The live SBT (`0x1FB8…`) predates the two-step upgrade in this PR and is still plain `AccessControlEnumerable`.
A soulbound token contract is not upgradeable, and redeploying would orphan every minted pet + every referencing credential, so the two-step protection cannot be retrofitted on-chain.
The migration therefore hands the live SBT over with an atomic `grantRole(DEFAULT_ADMIN_ROLE, multisig)` then `revokeRole(DEFAULT_ADMIN_ROLE, eoa)` — the multisig itself is the security boundary.
The upgraded two-step code applies to any **fresh** SBT deploy.
`script/GovernanceMigration.sol` auto-detects which branch a given SBT needs (`supportsTwoStep`).

## Procedure

### 0. Prerequisites

- The multisig is deployed and its address known → `export MULTISIG=0x…`.
- The deployer EOA (current admin) still controls `roax.json:admin` and has gas.
- `deployments/roax.json` reflects the live addresses.
- `export ROAX_RPC=…` and have the EOA key available to `forge script --broadcast`.

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

`IssuerRegistry` and the fresh `DogTagSBT` use a 3-day delay; `VerificationRegistry` uses 2 days.
Phase 2 `acceptDefaultAdminTransfer()` reverts until each ETA passes.

### 3. Phase 2 — accept (run by / through the multisig)

The multisig must execute, for each contract, the calls in `GovernanceMigration.accept`:

- `IssuerRegistry.acceptDefaultAdminTransfer()` then `IssuerRegistry.revokeRole(WHITELIST_ADMIN, oldAdmin)`
- `VerificationRegistry.acceptDefaultAdminTransfer()`
- `DogTagSBT.acceptDefaultAdminTransfer()` **or** (legacy) `DogTagSBT.revokeRole(0x00, oldAdmin)`
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
