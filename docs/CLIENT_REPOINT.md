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

## The governing condition for any repoint

> **An address may be repointed only when the successor answers THE SAME QUESTION FOR THE SAME INPUTS.**

Implementing the same selector is not sufficient evidence of that, and neither is a successful call.
A call that returns a confident wrong answer is worse than one that reverts, because nothing anywhere reports it.

So every repointed variable owes a stated answer to one question - **what does the successor answer, and under what caller context?** - and "the ABI matches" is not that answer.
A repoint whose semantic equivalence has not been established is listed as **UNVERIFIED**, not assumed.
The manifest carries that verdict per address in `semanticEquivalence`; the table below is the same verdict in prose.

This slice produced **two independent instances of the condition failing**, on opposite axes, which is why it is written as a rule rather than as an anecdote.

**The write axis.** The successor implemented the selector the variable's *reads* use, and implemented none of its *writes*.
That is the narrower property this section supersedes: *a variable naming a WRITE path may only be repointed to a contract that implements those writes.*
It is now one instance of the condition above, not a separate rule.
Worked example: `IssuerRegistry`'s admin console, below.

**The read axis.** The successor implemented the selector, answered, and its answer **meant something else**, because it branches on `msg.sender`.
Worked example: `IssuerRegistry`'s record-type reads, below.
This is the harder one, and the reason the condition cannot be discharged by reading an ABI: the branch that decides the meaning is invisible from the ABI, from the contract's own documentation, and from any test that happens to set a sender.

## What is superseded, and what is cleared to move

Six addresses are **superseded**, which is not the same as being **cleared to repoint**.
Cleared means one thing only: the `semanticEquivalence` verdict is **VERIFIED**, so count that column rather than trusting a number written beside it - a plausible-looking count is what concealed two errors in the inventory this slice was written to replace.
Run `scripts/check-cutover-consumers.sh` for the values, the per-address file counts, and each verdict as the manifest itself states it.

| Contract | Superseded by | Step | Same question, same inputs? |
|---|---|---|---|
| `DogTagIssuerFactory` | `CloneProvenanceRouter` for readers, `DogTagIssuerFactoryV2` for writers - see below | C-9 | **VERIFIED** per role |
| `IssuerRegistry` | **nothing yet** - no consumer may move at C-9; see below | **BLOCKED** on the `isRecognizedIssuer` migration | **FAILS** on the record-type key shape |
| `VerificationRegistryConsent` | `VerificationRegistryConsent` V2 | C-9 | **UNVERIFIED** - V2 is not written |
| `ProtocolRegistry` | `ProtocolRegistryV2`, under the new discovery key `dogtag-levelb/2` | C-8, then C-9 | **VERIFIED**, structurally |
| `IssuerDomainRegistry` | `ServiceDomainResolver` | C-7, then C-9 | **UNVERIFIED** - `resolveDomain` is a different shape |
| `DogTagIssuerImpl` | `DogTagIssuerV2` implementation | C-3 | **N/A** - no consumer calls it |

Read that last column as scope on the whole slice: repointing is *recorded* for all six addresses, and *cleared* for exactly the two carrying a VERIFIED verdict - `DogTagIssuerFactory` and `ProtocolRegistry`.
`ProtocolRegistry`'s verdict is structural rather than argued - the record is deliberately renamed (`getDiscoverySet`, 10 words against 8), so a generation-1 client cannot dispatch and misdecode it, while `getArtifactSet` keeps its selector precisely because that record is unchanged.
`IssuerDomainRegistry`'s is unverified for a stated reason: `ServiceDomainResolver` deliberately has no `domainOf(address) returns (string)`, because an empty string was three different facts, so `resolveDomain`'s tuple is a code change in every consumer.

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

## `IssuerRegistry` cannot be repointed at C-9 at all, and the read half is why

Both instances of the governing condition live on this one variable.
Neither is visible from the ABI, and the second was recorded as verified-safe for two revisions of this document before being caught.

### The write axis: the successor implements none of the writes

`ProviderRegistry` implements the read `isWhitelistedFor(bytes32,address)` and implements **neither** of the writes `whitelistFor` / `delistFor`, and it has no fallback function.

`stacks/admin/.env.example`'s `ISSUER_REGISTRY_ADDR` is the variable naming that write path, and it is **retained on the generation-1 registry through C-12**.
Two reasons, and the second outranks the first.

It would not fail cleanly.
`ProviderRegistry.hasRole(WHITELIST_ADMIN, owner)` returns true, so `governance::dispatch` reads the hosted key as the authority holder and **broadcasts** a reverting transaction rather than downgrading to a proposal - a config error wearing the costume of a chain problem, at the moment in the cutover when an operator can least tell them apart.

And it would disarm a later step of the same plan.
C-12's delisting freeze on the generation-1 `IssuerRegistry` runs through this same admin console, and that freeze is the operational precondition for closing `CloneProvenanceRouter`'s open mirror direction (`AGENTS.md`, "CloneProvenanceRouter").
Repointing the console at C-9 leaves C-12 with nothing to reach.

### The read axis: the successor answers a different question

`ProviderRegistry.isWhitelistedFor` (`contracts/src/ProviderRegistry.sol:787-793`) branches on `msg.sender`.
An **attached service** caller gets its own service grant; **every other caller** gets `_verifierCapabilities[key][signer]` - the orthogonal VERIFY axis, a mapping written only by `setVerifierCapability`.

**The mechanism, because it is invisible from the ABI.**
Every production read is a plain `eth_call` with **no `from` set**: `grep -c '\.from('` over `stacks/{vet,government,admin}/api/src/chain.rs` returns **0** in all three, and `packages/ui`'s `readContract` passes no `account`.
So `msg.sender` is the zero address and the verifier-capability branch **always** runs.

Which means the answer depends on **which key the caller passes**, not on which service it is.

**Compatible - the VERIFY-key reads.**
`verify_key_from_purpose_word` builds `keccak256(abi.encode("VERIFY:", purpose))`, byte-identical to `ProviderRegistry.verificationKey` (`:781`), so it indexes the very mapping that branch returns.
Four sites: `stacks/vet/api/src/routes.rs:1463`, `stacks/vet/api/src/verify.rs:505`, `stacks/government/api/src/routes.rs:1452`, `stacks/government/api/src/verify.rs:302`.

**Broken - the record-type-key reads.**
`keccak256(recordType)` is never a `verificationKey` output, so `_verifierCapabilities[keccak256(recordType)][signer]` is `false` for **every genuine issuer signer**.
Seven sites: `stacks/vet/api/src/routes.rs:549`, `:658`, `:969`, `:1258`, `:1308`; `stacks/government/api/src/routes.rs:702`; and `packages/ui`'s `isWhitelistedFor`, whose key is `recordTypeKey(...)` by construction.

**That enumeration counted GETTERS only, and it is therefore incomplete.**
A record-type key also travels as a LOG TOPIC: `Whitelisted(bytes32 indexed recordType, address indexed signer)` puts it in `topic1`, so the mandatory pillar's grant-history read is a record-type caller too, and fails against the successor for the same reason - only more quietly, because no call reverts and no answer looks wrong.
`ProviderRegistry` records grants as `IssuanceCapabilitySet(service, signer, allowed)`, a different name and `topic0`, so that filter matches nothing and the fold's empty-history rule renders the miss as a definite forgery verdict.
Both backends now guard it; see `docs/ISSUER_V2_OWNERSHIP.md` §8 for the guard and for what remains open.
Generalize the lesson rather than the list: **an inventory built by grepping for a getter name cannot see a consumer that reads the same key from an event**, which is the third way this document's own enumeration method has been wrong - after case and after prefix.

**Status after the `isRecognizedIssuer` migration.**
The getter sites above have moved, and where they did not, the reason is recorded at the call site:
`vet routes.rs`'s issuance preflight and its `issuer/signers` matrix now ask `ChainClient::issuance_capability`, which resolves the authority from the RESOLVED CLONE's own `registry()` rather than from `ISSUER_REGISTRY_ADDR`, and dispatches to `canIssue(service, signer)` or the legacy getter by generation;
the confirm-time re-check moved to the historical `whitelisted_at_issuance`, because a present-tense read there rejects a genuine, already-mined issuance whenever a key rotates between broadcast and confirm - which the C-12 freeze does to every generation-1 signer at once;
and `vet routes.rs`'s factory-less `expectedSignerState` **cannot** be migrated at all, because every generation-2 issuance-axis read is service-scoped and that branch is defined by having no resolved clone to pass. It is the one surviving record-type-keyed read of `ISSUER_REGISTRY_ADDR`.
`packages/ui`'s `isWhitelistedFor` stayed deliberately: its only consumer is the admin whitelist console, whose WRITE axis cannot move with it, and a console showing generation-2 state beside buttons that write generation 1 is worse than one uniformly a generation behind.

**Not a preference, and the reason the title of that migration is only half right:** the split is BY QUESTION.
A pre-issue gate takes `canIssue`, because `DogTagIssuerV2.issue` is gated by `onlyIssuanceCapable` == `canIssue` and a preflight on the wider `isRecognizedIssuer` passes where the write reverts.
A verifier asking whether a credential was genuinely issued takes neither getter: it asks the past, from events.

**The consequence, stated so nobody has to reconstruct it.**
`vet routes.rs:1258` and `government routes.rs:702` **are** the mandatory issuer-whitelist pillar, and that pillar treats a definite `false` as an **authenticity failure**.
Following the earlier instruction would therefore have **refused genuine credentials as forged, fleet-wide**.
`vet routes.rs:549` and `:658` are the issuance preflight, so the vet's own whitelisted signer is refused and **issuance stops** - while this same document was simultaneously claiming issuance was uninterrupted.
`vet :969` / `:1308` and the admin portal's whitelist viewer render every row false.

**Why the variable cannot move for the compatible half either.**
`ISSUER_REGISTRY_ADDR` is *one value* read by both key shapes **in the same process**: vet `routes.rs:1463` (VERIFY key) and `routes.rs:549` (record-type key) both read `st.cfg.issuer_registry_addr`.
There is nothing to split at the config layer, so the whole variable stays on generation 1 until the record-type callers migrate to the service-scoped `isRecognizedIssuer(service, signer)` - a different selector with different arguments, i.e. a code change in five consumers, recorded as a cutover blocker in `docs/ISSUER_V2_OWNERSHIP.md` section 8.

Leaving the readers on generation 1 is **not** a fix either: a generation-2 root then resolves nowhere and the pillar is indeterminate.
Both states are broken, differently, and only the migration closes either.

**The shape, in one sentence, because it is this project's own defect class expressed in Solidity.**
`isWhitelistedFor` returns `false` where the honest answer is "this contract cannot answer that question for this caller" - a definite negative standing in for an unanswerable question, which is the same collapse this fleet keeps finding in application code, could-not-check rendered as failed.
It is also why no caller could have detected it by observing behaviour: the call **succeeds** and returns a well-typed answer.

**The contract side is pinned. The consumer side is not, and that is the real gap.**

`contracts/test/ProviderRegistry.t.sol` exercises `isWhitelistedFor` at **both** caller contexts.
`vm.prank` affects only the *next* call, so the pranked attached-service cases - which take the first branch - are `:552`, `:588`, `:590`, `:592` and `:841`, while `:598`, `:836` and `:845` run **unpranked**, at production's zero-address caller.

`:598` is precisely the case that matters: unpranked, keyed on `RECORD_TYPE`, asserting `false` - and `:599` immediately asserts `isRecognizedIssuer(service, signer)` is **true** for that same signer, directly beneath the inline comment stating the split-by-question migration rule.
So the suite pins both halves: the legacy selector answers `false` at the caller context every real reader uses, and the successor selector answers `true`.
An earlier revision of this section claimed the opposite, and overstating a coverage gap is the same failure as understating one, pointed the other way.

The residual gap is one layer up: **nothing anywhere asserts what the consumers do when handed that `false`.**
The mandatory issuer-whitelist pillar's outcome against `ProviderRegistry` is unpinned in every suite - no Rust or TypeScript test references `ProviderRegistry` at all, and `MemChain::is_whitelisted_for` is a flat `(registry, recordType, signer)` map lookup that cannot model the `msg.sender` branch, so the case is not merely unwritten but unrepresentable in the hermetic fakes without a new one.
Closing it belongs with the `isRecognizedIssuer` migration, which is the change that gives those consumers something correct to assert against.

## The indexer is an append, not a swap - and it is the one that fails quietest

`INDEXER_GENERATIONS` is a list.
Generation 1 keeps its object forever, because its 19 anchored roots and their revocations stay in the oversight feed; generation 2 gets a **second object beside it**.
Replacing the object instead of appending drops the entire pre-cutover history out of oversight.

Its `factory` member is the **factory**, not the router - the opposite of every other stack - because that list is an emitter allowlist keyed by who signed the log, and the router emits nothing.
A router address there would silently match no event ever.

Its `issuerRegistry` member fails the governing condition in a third way, and appending the address is **not sufficient on its own**.
Every member of this triple is an *emitter*, so the question each must answer is "which events does it emit" - and `ProviderRegistry` emits **neither `Whitelisted` nor `Delisted`**.
Its issuance grants are `IssuanceCapabilitySet(service, signer, allowed)` and `VerifierCapabilitySet(...)`: different names, different `topic0`, different argument shapes.
The scanner filters by the `Whitelisted`/`Delisted` `topic0` (`stacks/indexer/api/src/chain.rs`), so appending `ProviderRegistry` without teaching the decoder the new events leaves that generation's whitelist axis permanently dark - and it fails the same silent way a late append does, with no error and no counter.
Pair it with a decoder change, or do not append it.

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

Issuance is uninterrupted for the whole of that wait, by design rather than by luck - **provided `ISSUER_REGISTRY_ADDR` has not been moved**.
Generation 1 is not frozen until C-12, so providers keep issuing through their existing clones and the router makes those roots resolve for old and new clients alike.
The adoption wait is a window in which both generations verify, not a gap in which nothing can be issued.

That proviso is not decorative, and it is the one claim in this document that an earlier revision falsified.
The vet's issuance preflight (`stacks/vet/api/src/routes.rs:549`, `:658`) reads that variable, so repointing it at C-9 refuses the vet's own whitelisted signer and stops issuance outright - see the read-axis section above.
The uninterrupted-issuance property holds for the **unmoved** registry only.

## The inventory is checked, not written down

`scripts/cutover-consumers.json` is the data; `scripts/check-cutover-consumers.sh` is the gate.
It fails when the tree and the manifest disagree in **either** direction: a file that carries a moving address and is not declared, and a declared file that no longer carries one.

It exists because the cutover's realistic failure mode is not repointing the wrong address.
It is repointing 22 of the 25 files in the repoint group and shipping the other 3 still aimed at generation 1, silently.
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

And the gate `export`s `LC_ALL=C` for the whole script rather than prefixing the `sort`s.
`comm` compares with the locale collating sequence, so a C-sorted input fed to a UTF-8 `comm` mis-merges: `README.md` sorts after `packages/` in `en_US.UTF-8` and before `apps/` in C.
The sorts alone were qualified at first, which left the answer dependent on the **caller's** locale - green in a `LC_COLLATE=C` shell and a dozen bogus "undeclared AND stale" pairs in an ordinary terminal.
That direction is noisy rather than unsafe, since distinct paths never collate equal so a genuine miss cannot be swallowed into `comm`'s third column - but a gate that fails loudly and wrongly on its first real invocation is a gate that gets deleted.

## A superseded address is invisible to a grep for the current one

The inventory is derived by grepping the **current** generation-1 value, so it is structurally blind to a consumer already holding an **older** one: such a file carries no moving address at all, so it reads as clean.

That blind spot was live.
`stacks/owner/web/src/lib/config.ts` pinned the superseded M5 `VerificationRegistryConsent` while its own comment called it the live one, and `src/lib/chain.ts`'s `fetchVerifiedLogs` uses it as the `eth_getLogs` address for the owner's consent-history scan.
A retired contract still answers `eth_getLogs` - with nothing - so the surface rendered "no consent history" for absence of evidence rather than for an absent history, permanently and without an error.
This repo's own standing defect class, on a user-facing surface.

`retiredAddresses` in the manifest closes it, with the same declared-allowlist discipline as the inventory rather than a blanket ban, because a retired address legitimately appears in many places: the historical ledger records it by design, golden-ABI encodings are pinned to it, and hermetic MemChain and mocked-network fixtures reuse it cosmetically.
An **undeclared** carrier is an error; a declared carrier that no longer holds it is only a note, since an over-long allowlist is not a repointing hazard.
The allowlist is scoped **per address**, so a file cleared to carry one retired address is not thereby cleared to carry another.
Matching reuses the same full-40-hex case-insensitive rule and the same untracked-file scan.

The manifest declares **itself** on that allowlist too - it is the record of what moved, so it holds every retired address by design - for exactly the reason it declares itself in `consumers`.
An implicit self-exemption is a hole in the one check whose job is finding holes, and this file has already been on the wrong side of that once.

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

**No consumer of `ISSUER_REGISTRY_ADDR` moves in this slice, on either axis**, and both halves fail the governing condition rather than merely being awkward.
The admin console's variable names a write path the successor does not implement; every record-type reader gets a definite `false` from a successor that answers a different question for a zero-address caller.
Both are worked out in full under "`IssuerRegistry` cannot be repointed at C-9 at all" above, and both are recorded as properties rather than to-dos because a property is checkable.
Closing the write half needs `whitelistFor`/`delistFor` equivalents reachable from the console under the new core's own model; closing the read half is the `isRecognizedIssuer(service, signer)` migration in `docs/ISSUER_V2_OWNERSHIP.md` section 8.
Both are behavioural changes, not address repoints, which is why neither is opened here.

**`ProviderDirectory` has no consumer yet.**
The indexer's provider directory still reads the admin business source.
It is listed in the plan as S-5/S-17 work, and no address repoint is pending for it because nothing points at it.

## Evidence

```
make check-cutover-consumers                # inventory + the gate; also runs FIRST in `make test`
cargo test -p dogtag-prover-rs --lib manifest
cargo test -p vet-api --test protocol_manifest
cargo check -p government-api --tests
```

The gate is in `make test` rather than excluded like `test-consent-parity` and `rehearse-cutover*`, which are excluded for a stated reason (slow, or needs an endpoint).
This one is a handful of `git grep`s plus python and needs neither, and an unrun gate has exactly the property it objects to in a hand-maintained list.

Mutations that must redden it, each verified: an untracked file carrying a moving address; a tracked file carrying one and not declared; a declared file repointed away; dropping `-i` from the match; truncating the match to the 8-hex prefix; and a tracked, non-allowlisted file carrying a retired address.
Run it under a non-C locale as well as your own shell's - the `comm` merge is what that catches, and a `LC_COLLATE=C` shell cannot see the difference.
