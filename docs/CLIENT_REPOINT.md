# Client repoint (registry plan S-13)

Every consumer of an address that moves in the generation-1 to generation-2 cutover, what it becomes, and when.

**This slice repoints; it does not deploy.**
The live deployment is S-14 and every C-step in it is separately approvable.
No generation-2 address exists yet, so none is recorded anywhere here: `generation2` is `null` for every entry in the inventory, and a placeholder would be the invented data this fleet forbids.

**This document deliberately carries no addresses.**
They live in `scripts/cutover-consumers.json`, and `scripts/check-cutover-consumers.sh` prints them.
A second copy here would be free to drift from the first, and this file would then itself become a consumer that the cutover has to remember to edit.

## Why the ordering is load-bearing

The S-12 fork rehearsal (`docs/CUTOVER_REHEARSAL.md`) established three facts that fix the order.

A generation-2 credential does **not** verify on the generation-1 registry.
That is why client repointing must precede generation-2 issuance: C-9 and C-10 before C-11.

A relayer not yet migrated fails `!verify-wl`, because `restrictToWhitelistedRelayers` defaults true.
That is why capability migration precedes client repointing: C-6 before C-9.

A revoked generation-1 root **cannot** be resurrected by re-anchoring it in a generation-2 clone, because `CloneProvenanceRouter` resolves oldest-generation-first.
Do not "simplify" that ordering: newest-first is the natural way to write the loop and it makes a revoked credential verify again.

The property the whole order defends: at no point may a credential be issuable that cannot be verified.

## What moves, and what must not

Six addresses move. Run `scripts/check-cutover-consumers.sh` for the values and the per-address file counts.

| Contract | Superseded by | Step |
|---|---|---|
| `DogTagIssuerFactory` | `CloneProvenanceRouter` for readers, `DogTagIssuerFactoryV2` for writers - see below | C-9 |
| `IssuerRegistry` | `ProviderRegistry` | C-9 |
| `VerificationRegistryConsent` | `VerificationRegistryConsent` V2 | C-9 |
| `ProtocolRegistry` | `ProtocolRegistryV2`, under the new discovery key `dogtag-levelb/2` | C-8, then C-9 |
| `IssuerDomainRegistry` | `ServiceDomainResolver` | C-7, then C-9 |
| `DogTagIssuerImpl` | `DogTagIssuerV2` implementation | C-3 |

Two live addresses are **reused** by generation 2 and must not appear in any repoint.

`DogTagSBTConsent` stays because `profileRoot` is per-contract and write-once, so a new SBT holds no root for any existing tag and every one of them would fail the `R == profileRoot(dogTagId)` binding.
Reuse costs nothing, since minting gates on the SBT's own `ISSUER_ROLE` rather than on the issuer registry.
C-10b's job is granting that role on that same SBT to the generation-2 issuing signers.

`Groth16VerifierConsent` stays because the circuit and the ceremony VK are frozen and unchanged by any of this work.

## The factory splits in two, and the two halves move in opposite directions

This is the single easiest thing in the cutover to get backwards, and both directions fail silently.

A **reader** uses the factory to resolve the write-once `rootIssuer[R]`.
It repoints to the **router**, which answers `rootIssuer` and `isClone` across both generations, oldest-generation-first.
Point a reader at `DogTagIssuerFactoryV2` and every one of the 19 historical roots resolves to `address(0)`, which surfaces as an indeterminate issuer-whitelist pillar rather than as an error.
That is the exact failure the router exists to prevent.

A **writer** uses the factory to `predictIssuer` and to build `createIssuer` calldata.
It repoints to **`DogTagIssuerFactoryV2`**, because the router deploys nothing and owns nothing.

Both spellings exist inside the admin stack alone, on variables named after the same contract:

- `stacks/admin/.env.example` `FACTORY_ADDR` is a write target. Verified: no admin caller reads `root_issuer` or `is_clone` through `cfg.factory_addr`. It takes the factory.
- `stacks/admin/web/.env.example` `VITE_DOGTAG_ISSUER_FACTORY_ADDR` is the verification bench's anchor read. It takes the router.

A third case is neither: `INDEXER_GENERATIONS.factory` is an **emitter allowlist**, keyed by who signed a log.
It takes `DogTagIssuerFactoryV2`, because the router emits nothing and a router address there would silently match no event ever.

The vet, groomer and government stacks are all readers.
Government is verified rather than assumed: `stacks/government/api/src/routes.rs:579` calls `root_issuer(&factory_cfg, ...)`, and `:1001` calls `is_factory_clone` for the issuer-domain binding's LINK 1 clone-provenance check.

That second one is worth stating precisely, because the router's `isClone` is **deliberately wider** than the generation-1 factory's: it answers true for a clone of any registered generation.
That is exactly the question a post-cutover provenance check must ask.
A generation-1-only answer would report every genuine generation-2 clone as `NotFactoryDeployed` - a definite false rather than an indeterminate, so it would fail the binding rather than degrade it.

The manifest encodes this split in the data: the factory's top-level `supersededBy` names the split rather than an address, the three diverging consumers carry their own `supersededBy`, and `check-cutover-consumers.sh` prints those divergences on every run.
S-14 drives from that file, and reading only the top-level entry would get two of them backwards.

## The indexer is an append, not a swap - and it is the one that fails quietest

`INDEXER_GENERATIONS` is a list.
Generation 1 keeps its object forever, because its 19 anchored roots and their revocations stay in the oversight feed; generation 2 gets a **second object beside it**.
Replacing the object instead of appending drops the entire pre-cutover history out of oversight.

Its `factory` member is the **factory**, not the router - the opposite of every other stack - because that list is an emitter allowlist keyed by who signed the log, and the router emits nothing.
A router address there would silently match no event ever.

The append must land, and the service restart, **before** generation 2 emits its first event.
The anti-spoof gate drops an unrecognised emitter with no error and no counter, so a late edit does not produce a failure - it produces an oversight feed that reads as merely quiet.
`/v1/status.watchedGenerations` reports the exact set in force, which is how to check rather than assume.

`START_BLOCK` does not rescue a late append: once the finalized-watermark cursor is persisted, restarts resume from it, so a generation added after its events occurred needs a deliberate fresh rebuild and rescan.

## What takes effect on deploy, and what needs a rebuild

Not every repoint in this inventory is live when it is merged, and the difference decides the cutover's wall-clock.

**Takes effect on deploy or restart** - every `.env.example`-driven service (vet, groomer, government, admin, indexer), because these are values an operator sets and the process reads at boot.
Reversible: reads are reversible, which is why C-9 precedes C-10 and C-11.

**Takes effect on the next web build** - `packages/ui/src/wallet/contracts.ts` and the three `VITE_*` portal templates.
A portal is rebuilt and redeployed; no user action is required.
Note `VITE_DOGTAG_ISSUER_FACTORY_ADDR` **falls back to the SDK default** when unset (`packages/ui/src/wallet/verificationBench.ts`), so leaving it unset after the cutover does not disable the bench's anchor check - it silently keeps reading generation 1.
`VITE_ISSUER_DOMAIN_REGISTRY_ADDR` has no fallback and fails closed instead.
Two adjacent lines with opposite failure modes; the fallback one is the one that can lie.

**Needs an app rebuild AND a reinstall on each handset** - `apps/ios/DogTag/roax.json` and `apps/android/app/src/main/assets/roax.json`.
These are compile-time bundles.
**Editing the source file is not a repoint**; it is the prerequisite for a build that is a repoint, and nothing between the two states says so.
This is C-10 and it is the long pole of the entire cutover.
An installed old build reads `rootIssuer` from the generation-1 factory baked into its bundle, so a generation-2 root resolves to zero: on mobile the monotone fold degrades the credential to `UNVERIFIED`, and on web the verdict is a hard false.
Enforce the floor through the artifact axis `minAppVersion` rather than hoping.

Issuance is uninterrupted for the whole of that wait, by design rather than by luck.
Generation 1 is not frozen until C-12, so providers keep issuing through their existing clones and the router makes those roots resolve for old and new clients alike.
The adoption wait is a window in which both generations verify, not a gap in which nothing can be issued.

## The inventory is checked, not written down

`scripts/cutover-consumers.json` is the data; `scripts/check-cutover-consumers.sh` is the gate.
It fails when the tree and the manifest disagree in **either** direction: a file that carries a moving address and is not declared, and a declared file that no longer carries one.

It exists because the cutover's realistic failure mode is not repointing the wrong address.
It is repointing 21 of the 24 files in the repoint group and shipping the other 3 still aimed at generation 1, silently.
A hand-maintained list in a document goes stale the moment a file is added, and the staleness is invisible until the cutover.

Every file is classified, and the classes split into two groups.

**Repoint**: `runtime-constant`, `service-default`, `config-template`, `operational-script`, `documentation`.

**Preserve**: `historical-ledger` (`contracts/deployments/roax.json` - S-14 *adds* generation-2 keys and must never rewrite a generation-1 key or a `_legacy` record), `pinned-fixture` (`contracts/rehearsal/fixtures/historical-roots.json` - the S-12 fork test refuses to run if its pinned block disagrees with the fork; re-derive it with `scripts/derive-cutover-inventory.sh`, never hand-edit), and `hermetic-fixture` (MemChain and mocked-network constants that resolve nothing live).

So "no old addresses remain" is a claim about the **repoint** group.
A repo-wide empty grep is not achievable and not desirable: the ledger is the deployment record, and rewriting the pinned fixture breaks the rehearsal.

## Two ways to grep for an address wrongly, both of which have already happened

The registry plan's own inventory (`dogtag-regplan-p3` section 9.6) reports 17 tracked files for the factory.
Re-running both greps at the plan's own commit `aa5f4c6` reproduces that 17 exactly and shows the true figure is **21**, because the plan's list both **omits real consumers** and **includes files that do not carry the address**.
Neither error is visible from reading the list, and they partly cancel, which is why the total looks plausible:

```bash
B=aa5f4c6083fd9da1f98ca798ac4ef6dc19760151
F=$(python3 -c 'import json;m=json.load(open("scripts/cutover-consumers.json"));print(next(a["generation1"] for a in m["movingAddresses"] if a["contract"]=="DogTagIssuerFactory"))')

git grep -lI    "${F:0:10}" $B | wc -l   # the plan's grep: case-sensitive, 8-hex prefix  -> 17
git grep -lI -i "$F"        $B | wc -l   # full address, case-insensitive                 -> 21
```

(The address is read from the manifest rather than written here, so this file carries none and cannot drift from it.)

It reconciles exactly as `17 = 21 - 7 + 3`: seven real consumers missed, three non-consumers invented.

**Case.**
Addresses are stored EIP-55-checksummed in some files and lowercased in others - the indexer lowercases.
A case-sensitive grep for the checksummed form is blind to every lowercased consumer.
That is how the plan's list omitted `stacks/indexer/api/src/main.rs`, which is the very file its own section 9.7 analysis of the silent-drop gate is about, and the one service whose late repoint is invisible rather than loud.

**Prefix.**
An 8-hex-prefix grep matches synthetic addresses that merely share a prefix, and it matches elided prose.
The three it invented at the plan's commit were `packages/ui/test/provenance.test.ts`, which uses `0xED20269E1234567890abcdefABCDEF1234567890` and is not the factory at all, plus `AGENTS.md` and `docs/ROLE_APPS.md`, which mention `0xED20269E…` in prose and carry no address to repoint.

So the checker matches **full 40-hex addresses, case-insensitively**.
Both halves are mutation-proven: dropping `-i` makes seven real consumers vanish from the inventory, and truncating to the 8-hex prefix pulls three non-consumers in.

Elided prose references (`0xED20269E…`) can be matched by neither, since widening the pattern to catch them reintroduces the prefix false positives.
They are **declared** in the manifest's `elidedReferences` and checked for presence instead, so a doc that narrates a moving address is not forgotten at cutover.

**`git grep` sees TRACKED files only, and that blinded this gate to its own manifest.**
`scripts/cutover-consumers.json` holds every generation-1 address by design - it is the record of what moved - so it is itself a file the gate must account for.
While it was untracked, `git grep` could not see it and the gate reported a clean tree throughout: the check was passing by not running, which is precisely the defect it exists to catch, aimed at itself.
It surfaced the moment the file was committed.

The fix is two-part, and the second half is the general one.
The manifest now **declares itself** under the `inventory-manifest` class rather than being silently skipped - an implicit self-exemption is a hole in the one check whose job is finding holes.
And the gate separately scans the untracked-but-not-ignored set, because any file about to be committed is invisible to `git grep` for exactly as long as it matters.

One more shell trap, because the checker would be worthless if it hit it: the repo's default shell is zsh, which does **not** word-split an unquoted `"$var"`.
Iterating a space-separated address list that way runs one iteration with the whole string as the pattern, every `git grep` misses, and the script reports a clean tree - a check that passes by not running.
`scripts/check-cutover-consumers.sh` is `#!/usr/bin/env bash` and uses explicit arrays regardless.

## Loud unset, where the address genuinely is not known

`crates/dogtag-prover-rs/src/manifest.rs` recognises `dogtag-levelb/2` as a version key and records that it has **no deployment**, via `DeploymentStatus::AwaitingDeployment`.
Previously `deployment_for("dogtag-levelb/2")` returned `None`, which is indistinguishable from a typo.

The two absences have unrelated remedies.
An unknown version is the caller's to fix; a recognised-but-undeployed one is fixed only by the cutover running.
Reporting the second as `unknown version` sends an operator hunting a misspelling that does not exist.

`GET /protocol/manifest?version=dogtag-levelb/2` therefore answers 404 with `version not yet deployed`, naming the pending contracts and who records them, rather than 404 `unknown version`.
Both still fail closed and serve nothing - this changes the diagnosis, never the outcome.
The record carries **no address field at all**, so it cannot decay into a placeholder.

Note the generation-2 discovery key is not an artifact key.
The proving artifacts are byte-for-byte generation 1's, so they keep `dogtag-levelb-artifacts/1`; a second artifact identity for identical bytes would be a falsehood.
`crate::artifact::resolve(Some("dogtag-levelb/2"))` therefore fails closed, which is a second independent reason no generation-2 manifest can be served yet.

## Still open after this slice

**The mandatory issuer-whitelist pillar does not answer for a generation-2 root.**
It asks the verifier's own configured generation-1 `IssuerRegistry`, so a generation-2 root either resolves nowhere (indeterminate) or resolves a signer whose authority lives only in `ProviderRegistry` under `canIssue`, giving a definite `false` - a genuine credential rendered as forged.
C-12's delisting freeze makes the generation-1 answer worse, not transitional.
This is a code change in five consumers and it changes verification behaviour, so it is not an address repoint and is deliberately not opened here.
It is recorded as a cutover blocker in `docs/ISSUER_V2_OWNERSHIP.md` section 8.
Carrying `providerRegistry` on the validated anchor (S-11) is what gives those five consumers an attested address to migrate to.

**Mobile's generation-2 wiring is INERT, not merely unset**, and the distinction matters because S-11 already landed parts of it.
Both `AnchorResolver`s carry the `dogtag-levelb/2` constant and a `decodeDiscoverySet` for the 10-word record, so the *decoder* exists - but nothing calls it.
The `RoaxRpc`/`Net.swift` `getDiscoverySet` fetch does not exist, and both `ScanScreen` call sites pass `nil`/`null` with a comment naming what must change (`docs/PROTOCOL_REGISTRY_V2.md`).
So do not read "the constant is there" as "mobile is repointed": mobile reads generation 1 only, and closing that is C-9 and C-10.

**`ProviderDirectory` has no consumer yet.**
The indexer's provider directory still reads the admin business source.
It is listed in the plan as S-5/S-17 work, and no address repoint is pending for it because nothing points at it.

## Evidence

```
scripts/check-cutover-consumers.sh          # inventory + the gate, exits non-zero on disagreement
cargo test -p dogtag-prover-rs --lib manifest
cargo test -p vet-api --test protocol_manifest
cargo check -p government-api --tests
```
