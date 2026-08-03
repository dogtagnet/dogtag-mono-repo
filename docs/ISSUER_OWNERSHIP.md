# The owner-bearing issuer clone and the self-service factory

`contracts/src/DogTagIssuer.sol` and `contracts/src/DogTagIssuerFactory.sol` - two of the ten contracts in the launch set.

**Status: deployed on ROAX, with no provider onboarded and no client reading them yet.**
`contracts/deployments/roax.json` carries the `DogTagIssuerImpl` and `DogTagIssuerFactory` addresses, their transaction hashes and their blocks; they are deliberately not copied into this file.
`providerCount` is 0, so no clone has been created and nothing has been issued - registering a provider, clearing it for a record type, attaching its clone and granting an issuance capability are registrar decisions about a particular business, outside the deploy script.
Nothing under `stacks/`, `apps/`, `packages/` or `crates/` resolves either address yet, so the running demo stack and both mobile bundles still name contracts that no longer decide anything.
The ledger is what this repo can speak for; a claim about every chain in existence is not one it can check.

---

## 1. Why these contracts look the way they do

The captain's requirement:

> only whitelisted people, AND owner of contracts can post their DNS on the smart contract

The earlier issuer design could not enforce that, and the reason was not that it was unimplemented.
It was that **there was no owner to check.**
That clone was `Initializable` only - no owner, no admin, no controller - and every write was gated on a record-type whitelist and nothing else, so the question "who controls this contract?" had no on-chain answer.

The domain surface built against it had to substitute a proxy for the missing owner: recompute a clone's deterministic address from `keccak256(recordType, business)` and compare.
That is a sound proof of *what the salt was*, and it is not a proof of *who controls the contract*.
It authorizes whoever was passed as `business` at creation, and an operator creating on a provider's behalf passes its own signer - so that tier authorized the operator rather than the organisation.

And the second requirement:

> they can deploy their own clone contracts from the factory after being approved. then they can also change their smart contract address, to a VALID CLONED smart contract, spun off FROM our factory contract.

The earlier `createIssuer` was `onlyOwner`, so no provider could deploy anything; every clone was minted by the protocol multisig.

`DogTagIssuer` and `DogTagIssuerFactory` supply both: a real owner, and a creation path a provider can reach.
Both failures above are recorded because they are the shapes to avoid regressing into, not because either contract still exists.

## 2. Ownership semantics

### It is two-step, always

`DogTagIssuer` inherits OZ `Ownable2Step`.
`transferOwnership` records a pending owner and changes nothing; `acceptOwnership` completes it.
A single-step transfer to a mistyped address would strand the clone forever - it would keep issuing, but could never again claim a domain, be repointed, or be handed to its real controller.

`transferOwnership(address(0))` **cancels** a pending handover.
It zeroes the pending owner, never the owner, so it is the recovery path for a fat-fingered address and not a way to orphan the contract.

### `owner()` can never become the zero address

After `initialize`, the owner is non-zero forever. Two paths could otherwise vacate it, and both are closed in the contract:

* **`renounceOwnership` is disabled.** An ownerless clone is precisely the state §1 describes this contract as existing to end. OZ's default would let one transaction re-enter it, irreversibly.
* **`acceptOwnership` refuses `msg.sender == address(0)`.** OZ compares `pendingOwner() != msg.sender`; with no transfer pending both sides are the zero address, the comparison passes, and ownership transfers to zero. Unreachable in practice - nobody can sign as the zero address - but unreachable by EVM accident rather than by contract. The invariant is worth holding where a test can assert it.

### Ownership is CONTROL, and confers no capability of its own

The owner may not issue, may not revoke, and gains no privilege over roots.
Both are decided by the authority core, and that is load-bearing: **withdrawing a signer's issuance grant must stop the next `issue` and touch nothing already anchored.**
If ownership carried an issuance right, withdrawal would no longer stop issuance and that lever would be silently dead.

So a provider whose grant is withdrawn still owns its clone, may still transfer it and may still repoint its listing - it simply cannot anchor anything new.

**The converse does not hold, and it is the half most likely to surprise an operator.**
Ownership is not a capability, yet the core folds the **confirmed** owner into both `canIssue` and `canRevoke`.
A completed two-step handover therefore suspends both until the registrar calls `confirmServiceOwner`: between acceptance and confirmation the live owner and the confirmed owner disagree, and the core reads that disagreement as unresolved rather than as authorization.
Read the two statements together - ownership grants nothing, and moving ownership pauses everything until the registrar catches up.
`test_a_handover_suspends_issuance_until_the_registrar_reconfirms` pins it.

`revoke` keeps the long-standing authority split exactly: the H-1 originator, or the core's protocol admin.
Extending revocation to the clone owner would let an owner revoke credentials it did not issue. That is a distinct governance decision and is deliberately not taken here.

### The three issuance-axis reads are a nested ladder, and the gaps are the point

The core publishes `isRecognizedIssuer ⊇ canRevoke ⊇ canIssue`, and this pair depends on the gaps:

* `issue` asks **`canIssue`** - the narrow rung, which additionally folds every live lifecycle term: provider and service standing, an active factory generation, and the provider's current pointer for the record type.
* the ordinary `revoke` arm asks **`canRevoke`** - which omits those terms, so a clone the provider has since superseded stays revocable by the originator that anchored on it.

Substituting one for the other is invisible in ordinary states, because there the two agree.
They differ when one of the live-lifecycle terms unique to `canIssue` drops: a superseded clone is the most direct example, and a suspended provider is another.
Both substitutions are real defects in those states: upward (`issue` asking `canRevoke`) silently reopens issuance, and downward (`revoke` asking `canIssue`) strands every root already anchored as permanently unrevocable.
`test_a_superseded_clone_refuses_new_issuance_but_still_revokes` is the direct mutation catcher; `test_a_suspended_provider_anchors_nothing_but_can_still_revoke` and `test_the_authority_ladder_is_nested_not_three_independent_switches` also distinguish the rungs.

### There is exactly one creation path, and it takes no owner argument

`createIssuer` sets the owner to `msg.sender` and salts the clone with the same address.
An "operator creates on behalf of a provider" variant was considered and rejected: it is the shape that produced the §1 weakness above.

With a single path, **`owner()` and the salt binding agree at creation.**
They can diverge afterwards, and only through the two-step handover - which is the one act that is supposed to move control.
So the salt records who *created* a clone, not who controls it now, and a later divergence is a completed rotation rather than a fault to repair.
Do not read a salt recomputation as an ownership proof; that is the §1 mistake in a new place.

Note the caller need not be the provider's controller: the core's create permission is delegable, so `msg.sender` may be a delegate, and then the clone is owned by the **delegate**.
The rule is exactly "the owner is the caller", with no exception.

### The creation gate is not the issuance capability

`createIssuer` asks the core `canCreateService(providerId, recordType, msg.sender)`, which folds four registrar facts the factory could not check itself: this factory is a pinned, **active** generation on the core; the provider is cleared; the provider is approved for that record type; and the caller may act for that provider.

Creation is therefore not issuance, and a freshly created clone can anchor nothing at all - `canIssue` additionally requires a registrar attachment, an issuance grant for the signer, a confirmed owner and the provider's current pointer.
`test_a_freshly_created_clone_can_anchor_nothing` walks that gap end to end.

An earlier draft of this slice gated creation on the legacy `isWhitelistedFor(recordType, msg.sender)` on the theory that one signature could serve both generations' oracles.
It could not, and this is worth recording rather than quietly changing: on the core that selector **branches on `msg.sender`**, and for a caller that is not itself an attached service it answers the orthogonal VERIFY-key capability - so a factory asking it about a creation reads a mapping about verification relayers. It also cannot tell an issue call from a revoke call, so it cannot express the ladder above at all.

---

## 3. The forgery-proof repoint

### Which check enforces it

**`isClone[candidate]`, read from the factory's own storage.**
That mapping is written in exactly one place - `createIssuer` - so an address is in it if and only if this factory deployed it. No other path sets it.

The predicate is published once, on the factory, as `authorizeClone(candidate, claimant) -> bytes32 recordType`, so that a consumer can call it instead of re-deriving it.
`ProviderRegistry`'s service attachment is meant to, and two parallel implementations of one authorization rule is how the vet and mobile verdict paths came to disagree in this codebase already.
`ServiceDomainResolver`'s domain write was the other intended consumer and instead composes the core's `canWriteService` - a considered deviation, argued in full in `docs/SERVICE_DOMAIN_RESOLVER.md`.

**No consumer composes it yet, so this is not the only place the rule lives.**
`ServiceDomainResolver` deliberately does not call it, and `ProviderRegistry` derives both halves itself: provenance through its own fail-soft `isClone` staticcall (`_factoryRecognizes`) and control through its own fail-soft `owner()` staticcall (`_readServiceOwner`).
On the core's own per-issuance predicates that shape is deliberate rather than an omission, for two specific reasons: those reads sit inside `canIssue`, which a clone asks on every `issue`, so the reverting `authorizeClone` cannot serve that call site; and `_isOwnerConfirmed` compares the live `owner()` against the registrar's recorded `confirmedOwner`, so it needs the owner VALUE, which neither shape returns.
Both derivations fail closed, so nothing is presently unguarded.
The attachment path is the one this predicate is meant to serve, and reconciling it remains `ProviderRegistry`'s obligation.

That applies inside the factory too: `authorizeClone` and the non-reverting `cloneAuthorization` are thin wrappers over ONE private `_authorization`, so the rule cannot be half-changed and the two shapes cannot end up applying its parts in different orders.
Both public forms keep their own failure vocabulary (`NotAClone` versus `NotCloneOwner`), because a provider whose clone is genuine must not be told its address is a forgery.

Three questions, three answers:

1. **Provenance** - `isClone[candidate]`, from the factory's own storage, never from the request. This is what stops a hand-rolled contract.
2. **Control** - `IOwnedIssuer(candidate).owner() == claimant`, read live from the named contract. Without it, provider A could repoint its listing at provider B's genuine clone: not contract forgery, but misattribution. *"There's no way to do any false contract inputs"* is true of provenance and silent about attribution.
3. **Attachment** - that the clone belongs to *this provider* in the authority core - is `ProviderRegistry`'s, not this factory's. This predicate answers provenance and control; the core composes it with identity.

`_authorization` also refuses a zero `claimant` before it makes any external call.
That arm is **defence in depth and no test distinguishes it**, deliberately so on both counts: a real clone's `owner()` can never be zero (see above), so the owner comparison already refuses a zero claimant, and the guard only saves the call. It is recorded here rather than given a mutation-table row it could not honestly earn.

### It resolves, it does not accept

The record type is **returned, not accepted**.
`authorizeClone` reads `recordType()` off the clone and hands it back, so a caller keying anything by record type gets a chain-resolved key in the same call that authorized the address.

This is the codebase's standing rule applied to a write path:

> An address may be an **argument to a write**, checked against the registry's own factory reference.
> An address may never be an **argument to a read that decides trust** - there it must be resolved from the chain.

`setActiveIssuer(address clone)` takes exactly one value. Both the authorization and the storage key come from chain state.
There is no argument by which a caller could name a slot it did not earn.

The same rule covers `providerId` in `createIssuer`: it is an argument to a write, checked against the factory's own immutable core reference, and the core resolves the *factory* from `msg.sender` - so a caller can name neither a provider it cannot act for nor a generation it is not.

### Provenance is checked first, so a hostile address never executes

`isClone` is a storage read and it short-circuits.
A non-clone is refused before any external call, so it cannot execute at all - not even to revert.
`test_a_hostile_impostor_is_never_even_called` holds this by pointing the factory at a contract whose getters revert with a distinctive error and asserting the factory's own `NotAClone` comes back instead.

### The nonce, and why the repoint has no target without it

The earlier factory salted a clone `keccak256(recordType, business)`, and `Clones.cloneDeterministic` reverts on a repeated `(implementation, salt)` pair.
So a provider has **exactly one possible clone address per record type**, and a second `createIssuer` for the same pair simply reverts.
*"They can also change their smart contract address to a VALID CLONED smart contract"* is then unreachable: there is no second address to move to.

`keccak256(recordType, business, cloneNonce)` gives a provider a fresh clone for key rotation or after a compromise, while keeping everything the old salt bought - the address is still deterministic and still exactly predictable before deployment via `predictIssuer`.

`providerId` is deliberately **not** in the salt.
The factory stores no provider binding, and the core resolves a service's provider from its own registrar attachment, so a salted `providerId` would be unresolvable by any consumer - an appearance of binding with nothing behind it, which is the shape this slice exists to remove.
The only thing it would buy is address separation for two providers that share one caller address, and bumping the nonce already buys that.
The intended provider is recorded in the additive `IssuerOwnerRegistered` event instead, where it reads as the creation's stated intent rather than as a resolvable fact.

### What a repoint does NOT do

It changes only where **new** credentials are anchored.
`rootIssuer[R]` is write-once, so everything the old clone already issued keeps resolving to the old clone and stays revocable there.
That is correct behaviour rather than a limitation: retroactively re-attributing issued credentials to a contract that did not issue them is exactly the misattribution the control check exists to prevent.

### The pointer is ADVISORY, and re-validated on read

`activeIssuer[owner][recordType]` is the owner-keyed, self-service record of which of a provider's own clones its controlling key currently designates as live.

**Nothing routes through it.**
An `issue` call reaches whichever clone the caller called, and whether it succeeds is decided entirely by the core's `canIssue` - which folds the core's OWN providerId-keyed pointer, not this one.
So this pointer authorizes nothing, redirects nothing, and a stale one cannot cause a wrong anchor; a clone with no designation at all anchors normally (`test_the_pointer_is_advisory_and_gates_no_issuance`).
Do not describe it as the thing that selects a provider's live issuer on chain - it is the self-service *statement* of that selection, and `ProviderRegistry`'s registrar-confirmed attachment is the authoritative record of which contracts belong to which organisation.
They are complementary and keyed differently on purpose: an owner address is a key, an organisation is not.

The raw mapping is the **current** stored value, not a history - one address per slot, overwritten by each repoint.
The trail of designations is the `ActiveIssuerSet` / `ActiveIssuerCleared` events; reading the mapping for history gives the present answer and nothing else.

`resolveActiveIssuer` returns the stored address **only if it still passes the predicate**, and `address(0)` otherwise.
So a pointer left stale by an ownership handover degrades to "no active issuer recorded" rather than to a claim that is no longer true.
A stale pointer is a could-not-establish, and this codebase does not render those as established.
The pointer is not inherited by a new owner either - a repoint is an explicit act.

---

## 4. What does not regress

### Every immutable dependency is identified or behaviour-checked before it is stored

The factory has no admin, so both constructor arguments - the implementation and the authority core - are permanent.
Both must be non-zero contracts, but the implementation is held to a stronger rule than ABI-shaped probing:
`impl.codehash` must equal `keccak256(type(DogTagIssuer).runtimeCode)`.
That is exact runtime-bytecode identity with the `DogTagIssuer` compiled into this factory; a lookalike that returns correctly shaped words for `owner()`, `pendingOwner()` and `recordType()` is still refused with `ImplementationCodeMismatch`.
`test_an_abi_shaped_impostor_implementation_is_refused` pins the distinction.
This exact identity is also the basis for the getter/no-revert guarantee `resolveActiveIssuer` relies on.

The authority core cannot be authenticated by a compiled-in runtime in the same way, so it is checked for code and for the exact ABI behaviour this factory needs.
An EOA is not enough - a `staticcall` to one *succeeds* with empty returndata - and neither is a contract that does not answer a selector or answers with the wrong width.
The core must answer `false` to all four zero-everything capability queries: `canCreateService`, `canIssue`, `canRevoke` and `hasRole`.
That refuses an indiscriminately permissive core - one which would authorize a zero provider, a zero clone and a zero signer.

**A dependency that did not answer and one that answered a definite `true` are refused under separate errors, each naming the dependency and the probed selector.**
`AuthorityDoesNotAnswer` covers the four shapes in which nothing was stated - a revert, no code path for the selector, the wrong returndata width, and a word that is neither 0 nor 1.
`AuthorityAuthorizesUnconditionally` is the definite-`true` answer, and it is a different accusation with a different remedy.
Both classes are equally fail-closed; only the diagnostic differs, and a single error for both told an operator to hunt a missing selector when the real cause was an authorization rule that authorizes everything - this codebase's could-not-check-rendered-as-a-neighbouring-state defect, inverted.
A non-canonical word is deliberately a non-answer rather than a `true`: it states no boolean, so reporting it as a deliberate authorization would be the same collapse in the other direction.
`test_a_dependency_answering_a_non_canonical_boolean_is_a_non_answer` holds that, and the definite-`true` diagnostic is held by `test_an_authority_that_authorizes_indiscriminately_is_refused`.

### Write-once stays per contract and honest

`DogTagIssuer.issue` checks `issuedAt[r]` in its own storage; `registerRoot` checks `rootIssuer[root]` in its own factory's storage.
Both are unchanged. The factory adds one guard and it only ever refuses more.

`rootIssuer` is **factory-local**: this factory's mapping answers only for roots anchored by its own clones.
A root anchored through any earlier factory resolves to the zero address here, which is why `VerificationRegistryConsent.rootIndex` naming this factory retires every previously anchored credential - see `_root_index` in the ledger.

### The capability ladder, under a widened clone set

Self-service widens who may add to `isClone`: previously only the protocol owner could, and now any approved provider can.
So creation and writes are **not** reduced to a record-type whitelist question.
Creation asks `canCreateService(providerId, recordType, caller)`; anchoring asks `canIssue(clone, caller)`; revocation asks `canRevoke(clone, caller)`.
Those core reads compose the registrar's provider approval, service attachment, confirmed ownership, record type and signer grants.
A provider approved only for VACCINATION can therefore create only a VACCINATION clone, whose core attachment and `recordType()` remain VACCINATION, so nothing it produces can read as a TRAVEL credential.

The other thing a widened `isClone` set could have carried is a **free-text `name()`**, and that is closed at the source - see §5.

Root squatting is unchanged in kind: any approved signer could already burn a root through `"root taken"`.
State the salting defence narrowly, because it is narrower than it sounds: salted Poseidon roots make it infeasible to **guess** a root, and a root becomes public the moment an `issue` transaction is observable, so an attacker watching pending transactions can still front-run one specific anchor.
What salting rules out is blind, untargeted squatting - not a targeted race.

### The factory has no admin surface

No owner, no privileged function.
Nothing about it can be repointed or captured, which is the property any consumer holding a factory reference needs: a repointable factory reference would let one transaction redefine what counts as a genuine clone.
Here there is nothing to repoint, in either direction.

`test_the_factory_has_no_owner_or_transfer_ownership_selector` holds this for the three named selectors only, and its own comment is careful about what it establishes.
Two things about that test are load-bearing and were wrong in an earlier draft: the calldata must be **well-formed**, since a genuine function reverts on a malformed decode, and the mutating probe must be a real `call` rather than a `staticcall`, since a genuine state-changing function reverts under `staticcall` regardless. With both mistakes present the test would have passed even if `transferOwnership` existed and worked.
A failing call establishes that the selector is **not reachable** - no such function and no fallback. It would also pass for a function that exists and reverts, so it is not evidence that the source declares none; that part is held by review.

### Event compatibility

`IssuerCreated(address indexed clone, bytes32 indexed recordType, string name)` and the clone's `RootIssued`/`RootRevoked` kept their **historic signatures byte for byte**, as did `RootRegistered`, so the oversight indexer's existing decoders read these clones with no change.
The owner arrives as an additive `IssuerOwnerRegistered(clone, owner, providerId, cloneNonce)` from the factory - not by widening an existing event, which would change its topic0 and silently drop it from the feed.

`test_the_legacy_creation_and_anchoring_event_topics_are_unchanged` asserts each `topic0` against the **literal historic signature string**, not against this contract's own declaration, so a widened event fails there even though it would still be self-consistent.
It also asserts the emitted `name` is empty, which is the wire half of §5.

---

## 5. The legacy `name()` is permanently empty, and that is the fix

The earlier factory wrote `name` from its `onlyOwner` `createIssuer` at KYC time, and **that provenance was the only reason a consumer could read it as an authoritative issuer identity.**
The three deferred consumers of this field are `stacks/government/api/src/routes.rs`,
`packages/ui/src/domain/issuerDomainBinding.ts`, and the event-detail rendering in
`packages/ui/src/chain/provenance.ts`.
They must all treat the empty value as identity unavailable and never turn the document's own claim into an authoritative fallback.

Self-service breaks that argument at its root.
A caller-supplied name would be a provider-chosen string arriving with genuine factory provenance: a signer approved only for VACCINATION could create a clone named `"US Department of Agriculture"`, and because the clone really is factory-descended, link-1 provenance resolves, the on-chain name is read, and it renders as the authoritative issuer beside a green check.
That is precisely the attack the on-chain name read exists to defeat.

So caller control is removed rather than documented:

* `createIssuer` takes **no name argument**.
* `initialize` takes none either, and nothing in `DogTagIssuer` ever writes the slot, so `name()` answers `""` for the life of every clone.
* `IssuerCreated` still carries the field - the signature and topic0 are wire-compatible - and always emits the empty string.

A consumer reading `name()` on a clone must therefore report authoritative identity **unavailable**, and must not fall back to the document's own claim.
Registrar-controlled identity comes from the authority core instead: its publication-safe identity anchor, reached through the core's directory resolver.

**Reconciling the three readers named above is outstanding, and is out of scope for this file.**
They are deliberately untouched; what these contracts guarantee is that there is no provider-chosen string for them to mistake.

---

## 6. Build and test

```sh
cd contracts && forge test --match-contract DogTagIssuer
```

53 tests in `DogTagIssuerTest`, 3 more in `DogTagIssuerProviderAuthorityInterfaceTest`; 295 in the whole `contracts` suite.
`--match-contract DogTagIssuer` (not `DogTagIssuerTest`) is what runs both of this pair's suites.
Prefer `forge test` over a bare `forge build`: it compiles only the real dependency closure.
The blanket "never a bare `forge build`" rule recorded here described a real failure - a full build compiled the vendored OpenZeppelin submodule's `certora/harnesses/*`, which import generated files that are not present, and failed with "File not found" (a submodule artifact, not a project error).
That **no longer reproduces**: re-measured 2026-07-30 on Foundry 1.5.1-stable with the submodules at their pinned revisions (`openzeppelin-contracts` `v4.8.0-743-g69c8def5`, `forge-std` `v1.9.4`), a bare `forge build` from a removed `out/` exits 0.
The mechanism of the change was not established, so read that as a measurement on that toolchain rather than a guarantee.

A fresh worktree has no `contracts/lib` contents; run `git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts` first.

### `DogTagIssuerProviderAuthorityInterfaceTest`, and why its negative control has the shape it does

The pair reaches the authority core through an interface it declares locally, and Solidity checks nothing across that seam, so this suite pins the agreement against the REAL `ProviderRegistry` on both axes a signature has: selector equality for the argument lists, and single written external function types both sides are assigned to, so a diverged return type or state mutability fails the BUILD instead of surviving to misdecode at runtime.

Five mutations were applied, run and reverted against a temporary harness, which by this repo's convention is not committed. Each landed on its expected outcome, and the tree was asserted byte-identical afterwards:

| mutation | outcome |
|---|---|
| the core's `hasRole` loses `view` (interface + impl) | build fails at the `view` binding - the divergence no selector can see |
| test 3's written return type `bool` to `uint256` | build fails, so the written type is load-bearing rather than decoration |
| one loop probe aimed at an undeclared selector, well-formed | test red on `assertTrue(ok)` |
| the core gains a `fallback() external {}` | test red on the negative control, which is the case it exists to catch |
| the negative control reshaped into an arity-mismatched call against a selector the core genuinely DOES answer | **stays green** |

That last row is the point, and it is a defect being demonstrated rather than a pass. Short calldata reverts inside the ABI decoder before selector dispatch is ever established, so an arity-mismatched control reports "not answered" for every selector including ones the core answers perfectly well - it cannot fail, and a control that cannot fail is worse than none, because it certifies the loop above it. The committed control is therefore a well-formed two-address call differing from `canIssue` in the selector alone.

### The suite's authority is a stand-in, and its fidelity is the load-bearing risk

`MockProviderAuthority` replaces the real core, so the obvious failure mode is a mock that can be driven into states the core forbids - the suite would then assert its own behaviour and pass for the wrong reason.
Two things guard against that, and neither is optional if the mock is ever extended:

* **The three rungs are derived from one set of registrar facts, and none is independently settable.** Three free booleans would let a test assert `canIssue` true while `canRevoke` was false, which the real core cannot produce.
* **`test_the_authority_ladder_is_nested_not_three_independent_switches` asserts the containment directly**, walking a clone through attach, grant, repoint, suspension and handover and checking `canIssue ⇒ canRevoke ⇒ isRecognizedIssuer` at each step.

The owner term is read **live** off the clone and compared against the registrar-confirmed owner, which is what makes the handover-suspension behaviour in §2 real rather than stipulated.
