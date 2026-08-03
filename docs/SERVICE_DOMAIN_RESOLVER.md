# ServiceDomainResolver — the domain claim a service contract publishes, and its typed absence

Registry plan `dogtag-regplan-p3` slice **S-9**.
`contracts/src/ServiceDomainResolver.sol`, covered by `contracts/test/ServiceDomainResolver.t.sol`.

**DEPLOYED AND UNWIRED.**
Cutover step C-7 deployed it live on ROAX on 2026-08-01, so it has an address in `contracts/deployments/roax.json` — recorded under `_c7_typed_resolvers` with its transaction hash, block and signer.
It answers nothing: no consumer reads it and its store is empty, because a typed resolver does nothing until the registrar calls `setResolverApproved(DOMAIN, resolver, true)` AND each service selects it — both KYC work that C-7 did not perform.
S-15 added `VITE_SERVICE_DOMAIN_RESOLVER_ADDR` to the vet and groomer `.env.example` files and to their env plumbing, so the key EXISTS — but it ships BLANK and the code has no fallback, so nothing resolves an address from it.
Read a blank fallback-free var as STRONGER evidence of unwiredness than an absent one: an absent key can be added and silently pick up a bundled constant, and this one cannot.
Client repointing is C-9/C-10 and has NOT happened.
Its source is consequently FROZEN in the same sense as the contracts S-14 deployed: an edit makes it diverge from deployed bytecode.
It is additive in exactly the sense `ProviderRegistry`, `CloneProvenanceRouter` and the generation-2 issuer pair are: nothing already deployed changes, and no existing address moves.

It supersedes `IssuerDomainRegistry`, which stays deployed and stays the WIRED domain surface until that cutover.
The product fact is unchanged — *this service contract asserts that its domain is D, and the zone at D asserts this contract back* — and so is the record convention in [ISSUER_DOMAIN_BINDING.md](./ISSUER_DOMAIN_BINDING.md).
What moves is where the three questions behind a claim are answered: authority to the S-6 `ProviderRegistry`, provenance to the S-8 `CloneProvenanceRouter`, and the meaning of "no domain" out of an overloaded empty string.

## Superseding the shipped registry costs nothing, and that was re-verified rather than inherited

The registry this supersedes held no bindings at all, so there was nothing to migrate.
Re-verified live against `https://devrpc.roax.net` (chainId 135) at head **303690** on 2026-07-30, before any of this was written - reproduced here as the measurement that settled it, though that contract's source has since been deleted and the instance is no longer a read target:

```
$ cast call <the superseded IssuerDomainRegistry> "boundCloneCount()(uint256)"
0
$ eth_getLogs address=<same> fromBlock=0x0 toBlock=latest
0 logs
```

The log count is the stronger of the two and is the reason it was worth reading separately.
`boundCloneCount` counts only clones that have had a domain written, so on its own it says nothing about `setDomainAdmin`, which appoints a self-service key without writing a binding.
Zero logs of any kind means no binding, no withdrawal and no appointed administrator has ever existed there, so nothing is stranded by abandoning it.

If a later reader finds that count non-zero, abandonment is no longer free and the migration owes those bindings a path; say so rather than proceeding on this note.

## Absence is three facts, not an empty string

`IssuerDomainRegistry` could say "no domain" exactly once, as `""`.
That single value was reached by a clone nobody had ever written and by a clone whose claim had been withdrawn, and a third fact — *this provider has deliberately published no domain* — could not be said at all.
So a provider with no website was indistinguishable from one whose record had never been filled in.

That is the same could-not-tell-them-apart defect this fleet has removed from every other surface, and it is not hypothetical here: both mobile ports read `domainOf(clone)` and render an empty string as **"This issuer has published no domain on-chain"** (`IssuerDomainBinding.swift:265`, `IssuerDomainBinding.kt:410`).
That sentence asserts a publication decision the issuer may never have made.

`Disposition` replaces it with four values:

| Value | Meaning |
|---|---|
| `UNSET` | Nobody has written anything for this service. The day-one state. |
| `NO_DOMAIN` | A writer deliberately stated that this service publishes no domain. |
| `CLAIMED` | A domain is claimed right now, and `domain` carries it. |
| `CLEARED` | A claim existed and was withdrawn. |

`UNSET` is the zero value, so an unwritten record reads as "nobody has said" without any write having happened.

Two invariants make the stored string incapable of lying whichever branch a careless reader takes, and both are pinned by tests:

- **`domain` is non-empty if and only if `disposition == CLAIMED`.**
  `NO_DOMAIN` and `CLEARED` zero it, and the grammar makes an empty claim unrepresentable in the first place.
- **`disposition == UNSET` if and only if `revision == 0` and `updatedAt == 0`.**
  Every write stamps, including the two that store no domain, so "written" is never mistakable for "never touched".

### The withdrawn domain is deliberately not kept in state

`clearDomain` zeroes the string and emits the withdrawn value in `DomainClaimWithdrawn` instead.
A `priorDomain` field would be read as a live claim by exactly the careless reader the typed disposition protects, so the history lives where it is legible as history.
Mutating the contract to retain it reddens `test_a_withdrawn_claim_cannot_be_read_back_as_a_live_domain`.

### `CLEARED` cannot record a withdrawal that never happened

`clearDomain` reverts `NothingToWithdraw` unless the current disposition is `CLAIMED`.
From `UNSET` there was no claim; from `NO_DOMAIN` there was no claim either; from `CLEARED` it already happened and a second call would only restamp it.
A caller who wants "there is no domain here" without a prior claim wants `declareNoDomain`, which is a positive assertion rather than an absence — which is why it is a write at all rather than simply leaving the record `UNSET`.

### Re-affirming the same domain is a real write

It bumps `revision` and re-anchors `updatedAtBlock`, and that is the point: the off-chain DNS re-check job reads the block anchor, so an issuer restating an unchanged claim must be able to move it.
`updatedAtBlock` is also what makes a claim reproducible — *what domain did this service claim at block N* is answerable by a pinned `eth_call`, where *what does it claim now* is not auditable against a mutable world.

## Who may write: the captain's AND, composed and not re-derived

One call decides authorization:

```solidity
core.canWriteService(service, caller, core.SERVICE_PERMISSION_RECORD())
```

That is the authority core's own published two-predicate rule — the attached provider and service are currently cleared, **AND** the caller is the confirmed live clone owner or an owner-epoch-scoped delegate.
Composing it rather than re-deriving standing and ownership here is deliberate: two parallel implementations of one authorization rule is how the vet and mobile verdict paths came to disagree in this codebase already.

Three consequences are worth stating because each looks like an omission and is not.

**No registrar bypass.**
`IssuerDomainRegistry`'s tier 1 let `WHITELIST_ADMIN` write any binding.
The core withholds that deliberately — an issuance signer, a provider controller and the registrar all get no ordinary content-write bypass — so this contract has no registrar write path at all.
A registrar that must stop a claim reaches for `setResolverApproved(DOMAIN, resolver, false)`, which is a fleet-wide lever and not a per-record edit.

**No `domainAdmin`.**
Tier 3 existed only to rescue clones whose salted `business` was the operator's own signer.
Owner-epoch-scoped delegation in the core replaces it with a revocable grant that a completed clone-owner handover invalidates automatically.

**No salt recomputation.**
Tier 2 recomputed `predictIssuer(recordType, who) == clone` because generation-1 clones have no owner to ask.
Generation-2 clones do, so the stand-in is unnecessary.

### It uses the content bit, not the resolver-selection bit

`SERVICE_PERMISSION_RECORD`, never `SERVICE_PERMISSION_DOMAIN_RESOLVER`.
Those are different powers: choosing which resolver holds a service's domain is the owner's structural decision, while publishing the domain through the chosen resolver is content.
An operations key trusted to keep a claim current is not thereby trusted to move the record to a resolver of its own choosing, and reusing one bit for both would grant exactly that.

The mask is **read from the core at deployment** and held immutable rather than restated as a local constant, because a duplicated bit is a second source of truth that drifts silently — and a drifted permission mask refuses every legitimate write while looking like an authorization problem.
A definite zero is refused at construction with its own error, since `canWriteService` returns `false` for a zero permission by its own guard.

### Why `authorizeClone` is deliberately not composed

`docs/ISSUER_V2_OWNERSHIP.md` §8 names this contract as an intended consumer of `DogTagIssuerFactory.authorizeClone`.
This slice does not call it, and that is a considered deviation rather than an oversight.
Two reasons, either sufficient:

1. It requires `claimant == owner()` **exactly**.
   Composing it as the control term would make every owner-appointed delegate unable to publish a domain and would leave the core's `SERVICE_PERMISSION_RECORD` bit with no consumer at all.
   That is a functional loss, not a tightening, and `test_an_owner_scoped_delegate_may_publish_and_an_owner_rotation_revokes_it` could not pass under it.
2. It lives on a **generation-specific** factory.
   Reaching it means either pinning this resolver to one generation — the thing the router exists to avoid — or hopping through the core's generation record to find the right factory.

What that handoff note is protecting is that the rule lives in ONE place.
Composing the core's `canWriteService` satisfies that: this contract derives neither standing nor ownership itself.
Reconciling the core's own internal derivation with `authorizeClone` is recorded there as S-6's obligation, and when it lands this contract inherits it without changing.

## Provenance: the router, and why the core's own check does not cover it

Every write also requires `router.isClone(service)`.
Without it a stranger's hand-rolled contract could claim a domain, publish a matching TXT record, and present as verified with nothing behind it — DNS would agree, this resolver would agree, and none of it would mean anything, because that contract never passed through a KYC-gated creation.

**This is not redundant with the core's own provenance check, and the difference is the whole point.**
`canWriteService` proves clone-hood against the factory pinned to the generation the service was *attached* under.
The router proves it against the generation list that `VerificationRegistryConsent`'s immutable `rootIndex` actually resolves roots through.

Those two lists are administered by two separate owner-only calls on two separate contracts — `ProviderRegistry.addFactoryGeneration` and `CloneProvenanceRouter.appendGeneration` — so they can genuinely disagree.
A service attached under a core generation whose factory was never appended to the router is one whose credentials answer `unknown root` at every verification.
Without this term it could still publish a domain, so a verified-looking identity would sit beside credentials that cannot verify at all.
Requiring both makes that unrepresentable: **a domain claim is honoured only for a service whose lineage is the same lineage that resolves its roots.**

`test_a_service_the_verification_lineage_does_not_vouch_for_cannot_claim_a_domain` is the test that goes red if this term is removed, and it is built to prove the non-redundancy directly: it asserts `core.canWriteService(...)` is `true` and `unroutedFactory.isClone(...)` is `true` while `router.isClone(...)` is `false`, then asserts the write is refused.
`test_appending_the_generation_to_the_router_makes_the_same_claim_succeed` shows the refusal is about the router and nothing else — appending the generation, with no other change, makes the identical call succeed.

### Lineage and authorization keep separate diagnoses

`NotRecognizedByLineage` and `NotAuthorized` are different errors on purpose, and the split is the same one `authorizeClone` keeps between `NotAClone` and `NotCloneOwner`.
A provider whose contract is genuine must never be told its address is a forgery, and a stranger's contract must never be told it merely lacks a permission.
Different accusations, different remedies.

## The resolver must still be selected, and still be approved

The core stores a service's selected domain resolver but **never clears it** — not on resolver deapproval, and not on the generation deprecation that freezes the selection write permanently.
The stored selector is a historical record of the last selection the core accepted.
So a consumer computing an effective resolver must require both halves, and so does every write here:

- `core.service(service).domainResolver == address(this)` — this service selected *us*, so a second approved resolver cannot write records for a service that never chose it.
- `core.isResolverApproved(DOMAIN, address(this))` — the registry authority's fleet-wide kill switch still permits us.
  That lever is the only way to disable a resolver for services whose selection write is already frozen, so reading the selector alone would defeat it.

Both are re-read on the **read** side too, because a stored record outlives the permission that wrote it.
A record written before a deapproval stops being authoritative while its content stays exactly as the issuer left it: deapproval says the resolver is no longer authoritative, not that the issuer withdrew anything.

### A frozen service, and the fourth standing term

Two core lifecycle states are terminal.
`setServiceStanding` refuses to leave `RETIRED`, and `deprecateFactoryGeneration` has no reactivation path.
Either one makes `_serviceStandingIsEffective` false forever, so `canWriteService` is false forever, so **every write here reverts `NotAuthorized` permanently - for the owner, for every delegate, and for the registrar, which has no bypass by design.**
A `CLAIMED` record on such a service is frozen as history.

None of the three terms above notices.
The router's generation list is append-only, fleet approval is unrelated, and the core never clears a stored selector - so all three stay `true` and `isAuthoritativeFor` stays `true` with them.
That is correct rather than a bug: the record genuinely is the last thing this resolver accepted from that service.
What it is not is a claim that the service can still maintain it.

So `claimStanding` carries a **fourth** term, `serviceStandingEffective`, sourced from `core.effectiveService` rather than re-derived here.
It is reported separately and deliberately **not** folded into `isAuthoritativeFor`, which answers a question about the RESOLVER's standing; conflating the two would make one bool answer two questions with different remedies, which is the collapse this contract exists to avoid.
**A consumer MUST read it before rendering a claim as current.**

Read what it claims precisely, because a `false` and a `true` are not symmetric.
A `false` is definitive: the service's lifecycle no longer permits writes here, and when the cause is one of the two terminal states above it never will again.
A `true` says only that the service is **not frozen**, and never that a write would succeed - `canWriteService` additionally requires a confirmed live owner, which this term deliberately discards, so a service quarantined by a completed-but-unconfirmed clone-owner handover reads `true` here while every key is refused.
That distinction is the point rather than a rough edge: a quarantine is cleared by `confirmServiceOwner` and a freeze is cleared by nothing, so rendering the two the same way would be the same collapse from the opposite side.
`canWriteDomain` is the surface that answers whether a write would succeed, and `test_a_quarantined_service_is_not_a_frozen_one` is the case that keeps the two apart.

`canWriteService` is still one bool, so a bare `NotAuthorized` cannot by itself distinguish "your key lacks authority" from "your provider is suspended" from "your service is retired".
Reading `serviceStandingEffective` beside the refusal is what separates the standing causes out, and that is what it is for.

`test_a_retired_service_freezes_its_claim_while_the_resolver_terms_stay_true` and `test_a_deprecated_factory_generation_freezes_its_claim_the_same_way` pin both halves, and they are two tests rather than one because the two causes reach the same frozen state through different fields of the core.

### The missing per-record withdrawal is a reduction, and the core has already ruled on it

`IssuerDomainRegistry`'s tier 1 let `WHITELIST_ADMIN` clear any binding per record.
Nothing here replaces that, so a frozen `CLAIMED` record cannot be withdrawn at all.
State it as the reduction it is.

It is not an open question, though, and the ruling should be cited rather than re-litigated.
`ProviderRegistry.deprecateFactoryGeneration`'s own documentation says a frozen service's stored selector "is history, not a live claim", that a registrar override there "would be an authority to rewrite a superseded generation's published claims", and that "withdrawing what such a selector still resolves is the typed resolver allowlist's job, not this write predicate's".

So the sanctioned lever is `setResolverApproved(DOMAIN, resolver, false)`.
The honest residual: that lever is **fleet-wide**, so withdrawing one frozen claim through it takes down every other service's claim on the same resolver.
That is a stated limitation and never a passing property, in the same register as the mirror direction `docs/CLONE_PROVENANCE_ROUTER.md` deliberately does not close.
Adding a per-record registrar withdrawal here would contradict the shipped ruling above, so it is out of scope for this slice and is the captain's call if it is ever revisited.

## The read surface, and the one getter that deliberately does not exist

| Read | Answers |
|---|---|
| `record(service)` | The whole `DomainRecord`. Never reverts; `UNSET` is a normal state, not an error. |
| `dispositionOf(service)` | The cheap discriminator. |
| `resolveDomain(service)` | `(Disposition, string)` — the disposition first. |
| `isAuthoritativeFor(service)` | The single machine-facing AND of the three **resolver** standing terms. |
| `claimStanding(service)` | The record, those three terms, and the core's own `serviceStandingEffective` - all reported **separately**. |
| `canWriteDomain(service, who)` | Whether a write by `who` would be accepted right now. Composed from `isAuthoritativeFor` plus the core's `canWriteService`. |
| `recordedServiceCount()` / `recordedServicePage(cursor, limit)` | Bounded enumeration for the off-chain re-check job. |

**There is deliberately no `domainOf(address) returns (string)`.**
That getter is what made three facts into one, and re-adding it re-creates the defect.
`resolveDomain` is a tuple so that a caller wanting only the string must write `(, string memory d) = resolveDomain(s)`, which discards the disposition visibly rather than by omission.

`claimStanding` returns the terms rather than one verdict because they have different remedies, and `isAuthoritativeFor` is the single derivation of the AND so consumers cannot drift into three slightly different versions of it.
The two are composed from the same helpers, so the breakdown and the verdict cannot disagree - the same rule `ProviderRegistry.effectiveService` already follows, and `canWriteDomain` composes `isAuthoritativeFor` rather than re-listing its terms for the same reason.
**Render the terms to a human, not the verdict** - and render the fourth one, because the first three cannot tell a live claim from a frozen one.

None of the three resolver terms catches a failed dependency call and returns `false` for it.
A swallowed failure rendered as a definite negative is exactly how *could not check* becomes *not verified*; a consumer whose call reverts has learned that it could not check, which is the honest answer.

There are two deliberate exceptions, and both are named rather than left for a reader to discover.

`canWriteDomain` is an exception in shape only: it is a permission gate, and the core's own `canWriteService` already fails closed internally, so a `false` there means the write would be refused - accurate whichever term failed.

`serviceStandingEffective` is a real one, and it is INHERITED rather than chosen.
`core.effectiveService`'s `factoryActive` folds `_factoryRecognizes`, a staticcall that returns `false` when the factory cannot be read at all (`ProviderRegistry.sol:1104-1113`), so an unreadable factory reaches this term as a definite "not standing" and it cannot honour the propagate-a-revert rule the other three follow.
That is acceptable in this direction only: this term's `false` is a do-not-render-this-as-current signal, so failing closed errs toward not over-claiming rather than toward a claim nothing checked.
Read it as a floor and never as a ceiling, and never invert it into evidence that a service IS frozen.

## What this contract deliberately does not hold

**No name, and no legal identity.**
A generation-2 clone's `name()` is permanently empty by construction, so a provider-chosen string beside a green check would be a fabricated authority.
Registrar-controlled identity comes from the core's `publicIdentityAnchor`.
Never add an identity field here.

**No description.**
The human-readable text is the DNS record's own value, written by whoever controls the zone.
Keeping it off-chain is what makes the binding bidirectional rather than two copies of one party's claim.

**No DNS state.**
A stored record is a CLAIM.
The zone's half is observed off-chain, cannot be observed historically at all, and must never be cached here as if the chain had witnessed it.

## The canonical-domain grammar is preserved verbatim

Lowercase LDH labels only (`a-z`, `0-9`, `-`), dot-separated, at least two labels, no empty label, no label starting or ending with `-`, label length ≤ 63, total length ≤ 253.

Uppercase is **rejected rather than folded**, so the stored value is unambiguously the canonical query name.
Enforcing this on chain means the off-chain verifier never has to guess what a malformed claim meant: an unusable domain is unrepresentable rather than a silent lookup failure that would surface as "could not check" forever.
Rejecting the empty string is also what makes the `domain != "" ⇔ CLAIMED` invariant hold from the claim side.

## What the cutover owes this contract

Recorded here rather than done now, because nothing is deployed and the existing registry is still the wired one.

**Every off-chain reader branches on a string today and must branch on a disposition.**
Enumerated by grep rather than from a list, so it is current as of this slice:

| Consumer | Reads | What changes |
|---|---|---|
| `apps/ios/DogTag/Net.swift` + `IssuerDomainBinding.swift` | `domainOf(clone)`, empty ⇒ `noDomainClaimed` | Must read the disposition. `UNSET` and `CLEARED` are not "published no domain". |
| `apps/android/.../RoaxRpc.kt` + `IssuerDomainBinding.kt` | same | same |
| `stacks/government/api/src/chain.rs` | `getBinding(clone).domain` | Already branches on a discriminator (`updatedAt != 0`), so this is a shape change rather than a correctness one. |
| `packages/ui/src/wallet/contracts.ts` + `verificationBench.ts` | `getBinding(clone)` | same |

Both mobile ports also derive a `domainOf(address)` selector; a migrated build derives the new read's selector instead, and both apps carry the registry address in a **compile-time bundle**, so a repoint needs an app rebuild and reinstall.

**A consumer must read the core's standing before rendering a claim as current.**
The disposition and `isAuthoritativeFor` together still cannot tell a live claim from a frozen one, because a `RETIRED` service standing and a deprecated factory generation are both terminal and neither touches any of this resolver's three terms.
So a migrated consumer reads `claimStanding`'s `serviceStandingEffective` (or `core.effectiveService(service)` directly) beside the record, and renders a `CLAIMED` record whose standing is no longer effective as **history rather than a current claim**.
See "A frozen service, and the fourth standing term" above for why the term is separate, and the section after it for why there is no per-record way to withdraw such a record.

`docs/ISSUER_DOMAIN_BINDING.md` remains the normative home for the record convention and the six display states.
Its "Who may write a binding" section describes the superseded three tiers and applies to `IssuerDomainRegistry` until the cutover; this document describes what replaces them.
The six display states do not change, but `noDomainClaimed` becomes reachable from a real `NO_DOMAIN` disposition rather than from any empty string, and `UNSET`/`CLEARED` need their own copy rather than borrowing it.

## Running the tests

```sh
cd contracts && forge test --match-contract ServiceDomainResolver
```

Prefer `forge test` over a bare `forge build`: it compiles only the real dependency closure.
The blanket "never a bare `forge build`" rule recorded here described a real failure - a full build compiled the vendored OpenZeppelin submodule's `certora/harnesses/*`, which import generated files that are not present, and failed with "File not found" (a submodule artifact, not a project error).
That **no longer reproduces**: re-measured 2026-07-30 on Foundry 1.5.1-stable with the submodules at their pinned revisions (`openzeppelin-contracts` `v4.8.0-743-g69c8def5`, `forge-std` `v1.9.4`), a bare `forge build` from a removed `out/` exits 0.
The mechanism of the change was not established, so read that as a measurement on that toolchain rather than a guarantee.

The fixture binds the **real** S-6 core, the **real** S-8 router and **real** generation-2 clones from the **real** self-service factory.
No authority, provenance or ownership fact in these tests comes from a double, because the claims under test are precisely about how those three contracts compose — a mocked core would let the resolver agree with a stand-in rather than with the thing it will be deployed against.
The only doubles are four deliberately broken dependencies used to exercise the constructor's refusals, plus one stranger contract that is not malformed at all.

The wiring order in `setUp` is the one the cutover has, and it is not circular: router first, then the factory (whose `priorIndex` is the router and whose constructor refuses a prior index that already claims it), then `appendGeneration`.

### Mutation evidence

Thirteen source mutations were applied, run and reverted: twelve while writing this slice, and one more for the fourth standing term added in review.
Each was caught by a named test:

| Mutation | Red |
|---|---|
| Router provenance term removed | `test_a_service_the_verification_lineage_does_not_vouch_for_cannot_claim_a_domain`, `test_a_stranger_contract_is_refused_by_lineage_and_not_by_authorization` |
| Resolver-approved term removed | `test_fleet_wide_deapproval_stops_writes_although_the_core_still_names_this_resolver`, `test_a_record_written_before_deapproval_stops_being_authoritative`, `test_is_authoritative_for_and_claim_standing_cannot_disagree` |
| Resolver-selected term removed from the write | `test_a_resolver_the_service_never_selected_cannot_write_its_record` |
| Resolver-selected term removed from `isAuthoritativeFor` | `test_a_record_stops_being_authoritative_when_the_service_selects_another_resolver`, `test_is_authoritative_for_and_claim_standing_cannot_disagree` |
| Write gated on the resolver-selection bit instead of the content bit | `test_the_write_uses_the_content_bit_and_not_the_resolver_selection_bit`, `test_the_content_permission_is_resolved_from_the_real_core`, and two others |
| `clearDomain` reachable from any disposition | `test_a_withdrawal_that_never_happened_cannot_be_recorded` |
| Withdrawn domain retained in state | `test_a_withdrawn_claim_cannot_be_read_back_as_a_live_domain`, `test_the_domain_string_is_non_empty_exactly_when_the_disposition_is_claimed` |
| `declareNoDomain` leaves the prior domain behind | `test_a_claim_can_be_replaced_by_a_deliberate_no_domain_declaration` |
| `claimStanding` AND-collapses its four terms | `test_claim_standing_reports_the_three_terms_separately_rather_than_one_verdict` |
| `_serviceStandingIsEffective` folds the core's `ownerConfirmed` | `test_a_quarantined_service_is_not_a_frozen_one` |
| Constructor drops the recognizes-everything guard | `test_construction_refuses_a_router_that_recognizes_every_address` |
| Constructor drops the zero-permission guard | `test_construction_refuses_a_core_whose_content_permission_is_zero` |
| Grammar accepts uppercase | `test_the_canonical_domain_grammar_rejects_anything_a_resolver_could_not_query` |

The fourth row is worth keeping visible: the first draft of the suite did **not** catch it, because every case that reached `isAuthoritativeFor` ran in a state where the service had selected this resolver.
The gap was found by running the mutation, not by reading the tests, and closing it is what `test_a_record_stops_being_authoritative_when_the_service_selects_another_resolver` exists for.

The last row has the same shape and is worth the same visibility.
Every other case that reads the fourth term asserts it `false`, and it is `false` under either semantics, so folding `ownerConfirmed` back in would have reddened nothing at all.
Only the quarantine case can tell the two apart, and it was added because running that mutation showed the term was pinned by its name and its comment rather than by the suite.
Beside it, `_assertVerdictExcludesServiceStanding` pins the term as OUTSIDE the verdict rather than merely absent from the fold.

The harness that applied these was not committed, so the table is historical evidence rather than a repeatable gate - the same standing as `docs/ISSUER_V2_OWNERSHIP.md` §9.
