# Addresses as configuration - handoff brief

The captain's requirement: a setup guide you can follow from a clean slate, with **no hardcoded
contract address anywhere**. This document is the enumeration and the traps, so the crew that does
the work starts from evidence rather than re-deriving it.

The rules this serves are in `AGENTS.md`, "Addresses, the publish timelock, and the mobile rebuild".
Read those five first - especially that this is **not** rebuild-avoidance (a mobile rebuild happens
on every full redeploy anyway) but so an app can CHECK a platform's version claim instead of trusting
it, and that addresses are **operator-configurable, never holder-configurable**.

## What is already done

- `make check-addresses` (`scripts/check-no-hardcoded-addresses.sh`) is the permanent gate, wired
  into `make test`. It fails on an undeclared file carrying a ledger or retired address, AND on a
  declared file that no longer carries one.
- `scripts/address-debt.json` **is the work-list**. It declares the 65 files that hardcode an address
  today, each with a reason and a remedy. Clear an entry by making the file read from configuration,
  then delete the entry. The list can only shrink; the gate enforces that.
- The `ProtocolRegistry` discovery anchor is deployed with a zero publish timelock and its discovery
  set is **published and active**, so the anchor a phone would resolve actually answers today.
- The mobile anchor read is fixed IN CODE (`getDiscoverySet`, 9-word record) - but it is **inert until
  the bundles move**, and nothing on a phone changes until then. Both bundles still name a superseded
  generation-1 `ProtocolRegistry`, whose getter is `getContractSet`, so against the address the apps
  actually read the corrected call still hits an absent selector and reverts with empty returndata -
  the precise failure the fix removes. Publication does not close it either: it landed on the ledger's
  registry, a different address from the bundled one. See below for what remains.

## How to enumerate, and the trap that makes a naive grep wrong

Enumerate from CODE, never from a document, and grep **case-insensitively for full addresses**:

```sh
bash scripts/check-no-hardcoded-addresses.sh    # the current list, always current
```

**Three generations are mixed in the tree**, which is why a grep for one set misses most of it:

- `packages/ui/src/wallet/contracts.ts` is mostly the superseded **V2** set - 8 of its 12 entries -
  with three on generation 1.
- `crates/dogtag-prover-rs/src/manifest.rs` is all generation 1, and carries its own comment calling
  itself a redeploy landmine.
- The mobile bundles are generation 1.

**Four keys must be DELETED, not repointed** - they name contracts with no source at all:
`CloneProvenanceRouter`, `IssuerRegistry`, `IssuerDomainRegistry`, `Poseidon6`. `ISSUER_REGISTRY_ADDR`
alone appears in seven `.env` templates.

### Seventeen of those files send you to a doc and a gate that no longer exist

The comment beside a hardcoded address is frequently a dangling pointer, so read the file rather than
following it. `docs/CLIENT_REPOINT.md`, `docs/ISSUER_V2_OWNERSHIP.md` and `docs/CLONE_PROVENANCE_ROUTER.md`
were deleted with the generation they described, and `scripts/check-cutover-consumers.sh` /
`make check-cutover-consumers` were repurposed into `scripts/check-no-hardcoded-addresses.sh` /
`make check-addresses`. Still citing one or both - and the split below is what decides whether you may
fix one on its own, so check which list a file is in before you touch it.

**Twelve are declared entries in `scripts/address-debt.json`**, so the citation is corrected by the
same edit that clears the entry:

```
crates/dogtag-prover-rs/src/manifest.rs
packages/ui/src/wallet/contracts.ts
packages/ui/test/providerDomainAndDeploy.test.ts
stacks/owner/web/src/lib/config.ts
stacks/admin/.env.example
stacks/admin/web/.env.example
stacks/government/.env.example
stacks/groomer/.env.example
stacks/groomer/web/.env.example
stacks/indexer/.env.example
stacks/vet/.env.example
stacks/vet/web/.env.example
```

**Do not "tidy" a citation on its own in one of those twelve**: the gate is bidirectional, and a
declared file that stops carrying an address fails it until you also delete its entry.

**The other five are NOT declared**, so the gate does not watch them at all and the dangling citation
can be corrected on its own, today, at no cost:

```
apps/android/app/src/main/java/io/liberalize/dogtag/data/RoaxConfig.kt
apps/ios/DogTag/Models.swift
packages/ui/src/provider/liveReader.ts
stacks/vet/api/src/chain.rs
stacks/vet/api/tests/issuance_authority.rs
```

What the deleted docs said that still binds is in `AGENTS.md` - the record-type-keyed reads under the
issuer-whitelist pillar, and the `msg.sender` branch that is the reason `ISSUER_REGISTRY_ADDR` cannot
simply be repointed.

## The layers, in the order they were approved

**Layer 1 - operator surfaces (backends, portals, scripts).** Mostly already env-driven; the work is
removing hardcoded *fallbacks*. `FACTORY_ADDR` already defaults to the ZERO address in all three
backends and is the precedent to copy: a fail-closed default beats a stale one. Only two non-zero
defaults remain (`admin` `SBT_ADDR`, `government` verification registry). The real offender is
`contracts.ts`, which is a constant table and should read injected config.

**Layer 2 - the deploy writes the configuration.** `demo-up.sh` already resolves ledger keys by name
via `ledger_addr`; extend that so a fresh deploy produces the env rather than an operator pasting it.

**Layer 3 - the phones.** Bundle ONE generated address (the `ProtocolRegistry` anchor) plus chain id
and the bundled RPC, generated from the ledger at build time rather than hand-edited. Resolve
factory, verification registry, SBT, verifier and provider registry from `getDiscoverySet` at
runtime.

## What is already true on the mobile path, so you do not re-derive it

The architecture is already anchor-first. Both platforms bundle the anchor address and pass it, and
both already use the **bundled** RPC for the anchor read - never the holder's chosen endpoint, which
is a deliberate carve-out that must stay (a holder-chosen peer answering the anchor would let a
hostile portal supply both sides of the comparison).

Fixed in this branch, and worth knowing why it was broken:

- Both clients derived `getContractSet(bytes32)`. The contract's getter is `getDiscoverySet`. An
  absent selector reverts at the dispatcher with EMPTY returndata, which both clients mapped to the
  same nil as an unpublished version - so the app failed closed for a reason that was not the real
  one.
- The decoders required exactly 10 words with a separate `rootIndex`; the record is 9 and the factory
  IS the root index. The arity guard is why this surfaced as a refusal rather than as an anchor
  naming whatever sat at those offsets. Keep that guard exact.
- `rootIndex` is now a derived accessor over `factory`. The publish preflight refuses to stage a
  record whose `factory` is not `verificationRegistry.rootIndex()`, so the equality is asserted on
  chain rather than assumed on the device.

**What remains on the mobile path:**

1. `TrustedAnchor` now receives `providerRegistry` and `rootIndex` from the record, but the app still
   reads factory / SBT / verifier / verification registry from its bundle. Sourcing those from the
   resolved record is the remaining layer-3 change.
2. **The golden vectors in `AnchorResolverTests.swift` and `AnchorResolverTest.kt` still carry the old
   shapes and must be regenerated.** Do not hand-write them - the tests' own doc forbids it, because
   a hand-built vector tests an idea of the ABI rather than the ABI. Capture the real bytes:
   ```sh
   REG=$(python3 -c "import json;print(json.load(open('contracts/deployments/roax.json'))['ProtocolRegistry'])")
   cast call "$REG" "getDiscoverySet(bytes32)" "$(cast keccak 'dogtag-levelb/1')" --rpc-url https://devrpc.roax.net
   ```
   That returns the live 9-word record; the Kotlin and Swift vectors must carry identical bytes.
3. The bundles need keys ADDED as well as repointed - they currently lack `ProviderRegistry`,
   `VerificationRegistryConsent`, `DogTagSBTConsent`, `Groth16VerifierConsent`, `ProviderDirectory`,
   `ServiceDomainResolver` and `custodian`. Under layer 3 most of those stop being bundled at all.
4. Editing a bundle is not a repoint until the app is rebuilt AND reinstalled. Say that plainly
   rather than reporting mobile as done.

## Two things that are NOT this work

- **The signed manifest** (`crates/dogtag-prover-rs/src/manifest.rs`) is the named follow-up that
  would remove the LAST bundled address - the anchor itself. It is ed25519 dogtag-signed, so it is
  holder-safe, but `DOGTAG_MANIFEST_PUBKEY` is `None` and nothing fetches it. Deliberately out of
  scope; the anchor stays bundled until then.
- **Holder-configurable addresses.** Ruled out. A holder who can repoint the registry can change what
  "genuine" means, and a forged registry verifies forged credentials cleanly with every check passing.
