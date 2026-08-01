# The owner-bearing issuer clone and the self-service factory

`contracts/src/DogTagIssuerV2.sol` and `contracts/src/DogTagIssuerFactoryV2.sol`.
Registry-plan slice **S-7**.

**Status: deployed on ROAX by S-14, and unwired. Nothing reads either contract.**
Cutover steps C-3a and C-3b deployed them, so `contracts/deployments/roax.json` carries `DogTagIssuerV2Impl` and `DogTagIssuerFactoryV2` keys - see `_s14_cutover` there for the addresses, transaction hashes and blocks, which are deliberately not copied into this file.
There is still no `.env.example` key and no client config naming either, and nothing in the tree points at one: client repointing is C-9/C-10 and enabling generation-2 issuance is C-11, both outstanding and both separately captain-authorized.
State it as "deployed, unwired" rather than as "the cutover is done", and note the ledger is what this repo can speak for - a claim about every chain in existence is not one it can check.

---

## 1. Why this exists

The captain's requirement:

> only whitelisted people, AND owner of contracts can post their DNS on the smart contract

That cannot be enforced against the contracts as deployed, and the reason is not that it is unimplemented.
It is that **there is no owner to check.**
`DogTagIssuer` is `Initializable` only: it has no owner, no admin and no controller, and every write is gated on `IssuerRegistry.isWhitelistedFor(recordType, msg.sender)` and nothing else.
The question "who controls this contract?" has no on-chain answer.

`IssuerDomainRegistry` had to substitute a proxy for the missing owner - `_isSpawningBusiness`, which recomputes a clone's deterministic address from `keccak256(recordType, business)` and compares.
That is a sound proof of *what the salt was*, and it is not a proof of *who controls the contract*.
It authorizes whoever was passed as `business` at creation, and `stacks/admin/api/src/routes.rs::resolve_business` defaults that to the operator's own signer - so on every clone deployed to date, the "self-service" tier authorizes the operator rather than the organisation.
`IssuerDomainRegistry`'s own doc says as much, which is why its tier 3 `domainAdmin` exists at all.

And the second requirement:

> they can deploy their own clone contracts from the factory after being approved. then they can also change their smart contract address, to a VALID CLONED smart contract, spun off FROM our factory contract.

`DogTagIssuerFactory.createIssuer` is `onlyOwner`, so no provider can deploy anything - every clone in existence was minted by the protocol multisig.

This slice supplies both: a real owner, and a creation path a provider can reach.

## 2. Why the whole contract set has to move with it

`DogTagIssuerFactory.implementation` is `immutable`, so a new issuer implementation forces a new factory.
`VerificationRegistryConsent.rootIndex` is `immutable` and resolves the issuing clone for every proof via `rootIndex.rootIssuer(R)`, so a new factory forces a new verification registry.

That cascade is why this is slice S-7 of a sequenced plan rather than a standalone change, and it is also why **the generation-1 contracts are not touched here**.
`DogTagIssuer.sol` and `DogTagIssuerFactory.sol` are unmodified; the V2 pair is additive, exactly as `ProtocolRegistry` and `IssuerDomainRegistry` were.

Generation 2 is also bound to its **own** authority core - the S-6 `ProviderRegistry` - and not to generation 1's `IssuerRegistry`.
That separation is a precondition of the cutover rather than a preference, and `docs/CLONE_PROVENANCE_ROUTER.md` explains why: `onlyWhitelisted` asks only whether the caller holds a grant for a record type, never whether it owns or spawned the clone, so under ONE shared core a single grant authorizes anchoring on every clone of that record type in BOTH generations - and no withdrawal can then freeze the earlier generation without also stopping the later one.
The router's residual mirror direction is closed operationally by exactly that freeze (cutover step C-12), so sharing a core would leave it open in production.

The plan records a fallback worth knowing about, because it is not what was approved: self-service alone could be delivered **without** any cascade, by transferring factory ownership to a gateway contract that implements "an approved provider may create" and forwards to `createIssuer`.
Owner-bearing clones are what force the redeploy. Do not conflate the two if the cascade is ever judged too risky.

---

## 3. Ownership semantics

### It is two-step, always

`DogTagIssuerV2` inherits OZ `Ownable2Step`.
`transferOwnership` records a pending owner and changes nothing; `acceptOwnership` completes it.
A single-step transfer to a mistyped address would strand the clone forever - it would keep issuing, but could never again claim a domain, be repointed, or be handed to its real controller.

`transferOwnership(address(0))` **cancels** a pending handover.
It zeroes the pending owner, never the owner, so it is the recovery path for a fat-fingered address and not a way to orphan the contract.

### `owner()` can never become the zero address

After `initialize`, the owner is non-zero forever. Two paths could otherwise vacate it, and both are closed in the contract:

* **`renounceOwnership` is disabled.** An ownerless clone is precisely the generation-1 state this contract exists to end. OZ's default would let one transaction re-enter it, irreversibly.
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

`revoke` keeps generation 1's authority split exactly: the H-1 originator, or the core's protocol admin.
Extending revocation to the clone owner would let an owner revoke credentials it did not issue. That is a distinct governance decision and is deliberately not taken here.

### The three issuance-axis reads are a nested ladder, and the gaps are the point

The core publishes `isRecognizedIssuer ⊇ canRevoke ⊇ canIssue`, and generation 2 depends on the gaps:

* `issue` asks **`canIssue`** - the narrow rung, which additionally folds every live lifecycle term: provider and service standing, an active factory generation, and the provider's current pointer for the record type.
* the ordinary `revoke` arm asks **`canRevoke`** - which omits those terms, so a clone the provider has since superseded stays revocable by the originator that anchored on it.

Substituting one for the other is invisible in ordinary states, because there the two agree.
They differ when one of the live-lifecycle terms unique to `canIssue` drops: a superseded clone is the most direct example, and a suspended provider is another.
Both substitutions are real defects in those states: upward (`issue` asking `canRevoke`) silently reopens issuance, and downward (`revoke` asking `canIssue`) strands every root already anchored as permanently unrevocable.
`test_a_superseded_clone_refuses_new_issuance_but_still_revokes` is the direct mutation catcher; `test_a_suspended_provider_anchors_nothing_but_can_still_revoke` and `test_the_authority_ladder_is_nested_not_three_independent_switches` also distinguish the rungs.

### There is exactly one creation path, and it takes no owner argument

`createIssuer` sets the owner to `msg.sender` and salts the clone with the same address.
An "operator creates on behalf of a provider" variant was considered and rejected: it is the shape that produced the generation-1 weakness above.

With a single path, **`owner()` and the salt binding agree at creation.**
They can diverge afterwards, and only through the two-step handover - which is the one act that is supposed to move control.
So the salt records who *created* a clone, not who controls it now, and a later divergence is a completed rotation rather than a fault to repair.
Do not read a salt recomputation as an ownership proof; that is the generation-1 mistake in a new place.

Note the caller need not be the provider's controller: the core's create permission is delegable, so `msg.sender` may be a delegate, and then the clone is owned by the **delegate**.
The rule is exactly "the owner is the caller", with no exception.

### The creation gate is not the issuance capability

`createIssuer` asks the core `canCreateService(providerId, recordType, msg.sender)`, which folds four registrar facts the factory could not check itself: this factory is a pinned, **active** generation on the core; the provider is cleared; the provider is approved for that record type; and the caller may act for that provider.

Creation is therefore not issuance, and a freshly created clone can anchor nothing at all - `canIssue` additionally requires a registrar attachment, an issuance grant for the signer, a confirmed owner and the provider's current pointer.
`test_a_freshly_created_clone_can_anchor_nothing` walks that gap end to end.

An earlier draft of this slice gated creation on the legacy `isWhitelistedFor(recordType, msg.sender)` on the theory that one signature could serve both generations' oracles.
It could not, and this is worth recording rather than quietly changing: on the core that selector **branches on `msg.sender`**, and for a caller that is not itself an attached service it answers the orthogonal VERIFY-key capability - so a factory asking it about a creation reads a mapping about verification relayers. It also cannot tell an issue call from a revoke call, so it cannot express the ladder above at all.

---

## 4. The forgery-proof repoint

### Which check enforces it

**`isClone[candidate]`, read from the factory's own storage.**
That mapping is written in exactly one place - `createIssuer` - so an address is in it if and only if this factory deployed it. No other path sets it.

The predicate is published once, on the factory, as `authorizeClone(candidate, claimant) -> bytes32 recordType`, so that a consumer can call it instead of re-deriving it.
S-6's service attachment is meant to, and two parallel implementations of one authorization rule is how the vet and mobile verdict paths came to disagree in this codebase already.
S-9's domain write was the other intended consumer and has since shipped composing the core's `canWriteService` instead - a considered deviation, argued at §8 and in full in `docs/SERVICE_DOMAIN_RESOLVER.md`.

**No consumer composes it yet, so this is not the only place the rule lives.**
S-9 now exists (`contracts/src/ServiceDomainResolver.sol`) but deliberately does not call it, and `ProviderRegistry` derives both halves itself: provenance through its own fail-soft `isClone` staticcall (`_factoryRecognizes`) and control through its own fail-soft `owner()` staticcall (`_readServiceOwner`).
On the core's own per-issuance predicates that shape is deliberate rather than an omission, for two specific reasons: those reads sit inside `canIssue`, which a generation-2 clone asks on every `issue`, so the reverting `authorizeClone` cannot serve that call site; and `_isOwnerConfirmed` compares the live `owner()` against the registrar's recorded `confirmedOwner`, so it needs the owner VALUE, which neither shape returns.
Both derivations fail closed, so nothing is presently unguarded.
The attachment path is the one this predicate is meant to serve, and §8 records reconciling it as S-6's obligation.

That applies inside the factory too: `authorizeClone` and the non-reverting `cloneAuthorization` are thin wrappers over ONE private `_authorization`, so the rule cannot be half-changed and the two shapes cannot end up applying its parts in different orders.
Both public forms keep their own failure vocabulary (`NotAClone` versus `NotCloneOwner`), because a provider whose clone is genuine must not be told its address is a forgery.

Three questions, three answers:

1. **Provenance** - `isClone[candidate]`, from the factory's own storage, never from the request. This is what stops a hand-rolled contract.
2. **Control** - `IOwnedIssuer(candidate).owner() == claimant`, read live from the named contract. Without it, provider A could repoint its listing at provider B's genuine clone: not contract forgery, but misattribution. *"There's no way to do any false contract inputs"* is true of provenance and silent about attribution.
3. **Attachment** - that the clone belongs to *this provider* in the authority core - is S-6's, not this factory's. This predicate answers provenance and control; the core composes it with identity.

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

Generation 1 salts a clone `keccak256(recordType, business)`, and `Clones.cloneDeterministic` reverts on a repeated `(implementation, salt)` pair.
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
Do not describe it as the thing that selects a provider's live issuer on chain - it is the self-service *statement* of that selection, and S-6's registrar-confirmed attachment is the authoritative record of which contracts belong to which organisation.
They are complementary and keyed differently on purpose: an owner address is a key, an organisation is not.

The raw mapping is the **current** stored value, not a history - one address per slot, overwritten by each repoint.
The trail of designations is the `ActiveIssuerSet` / `ActiveIssuerCleared` events; reading the mapping for history gives the present answer and nothing else.

`resolveActiveIssuer` returns the stored address **only if it still passes the predicate**, and `address(0)` otherwise.
So a pointer left stale by an ownership handover degrades to "no active issuer recorded" rather than to a claim that is no longer true.
A stale pointer is a could-not-establish, and this codebase does not render those as established.
The pointer is not inherited by a new owner either - a repoint is an explicit act.

---

## 5. What does not regress

### Every immutable dependency is identified or behaviour-checked before it is stored

The factory has no admin, so all three constructor arguments are permanent, and for `priorIndex` a wrong value also strands the verification registry that points at the factory.
All three must be non-zero contracts, but the implementation is held to a stronger rule than ABI-shaped probing:
`impl.codehash` must equal `keccak256(type(DogTagIssuerV2).runtimeCode)`.
That is exact runtime-bytecode identity with the `DogTagIssuerV2` compiled into this factory; a lookalike that returns correctly shaped words for `owner()`, `pendingOwner()` and `recordType()` is still refused with `ImplementationCodeMismatch`.
`test_an_abi_shaped_impostor_implementation_is_refused` pins the distinction.
This exact identity is also the basis for the getter/no-revert guarantee `resolveActiveIssuer` relies on.

The authority core and prior index cannot be authenticated by a compiled-in runtime in the same way, so they are checked for code and for the exact ABI behaviour this factory needs.
An EOA is not enough - a `staticcall` to one *succeeds* with empty returndata - and neither is a contract that does not answer a selector or answers with the wrong width.
The core must answer `false` to all four zero-everything capability queries.
The prior index must answer `false` both for `isRootAnchored(bytes32(0))` and for `isGeneration(address(this))` while this factory is still under construction.
Those checks refuse an indiscriminately permissive core, an index which claims every root, and an index which falsely claims the not-yet-appended factory as a generation.

**A dependency that did not answer and one that answered a definite `true` are refused under separate errors, each naming the dependency and the probed selector.**
`AuthorityDoesNotAnswer` / `PriorIndexDoesNotAnswer` cover the four shapes in which nothing was stated - a revert, no code path for the selector, the wrong returndata width, and a word that is neither 0 nor 1.
`AuthorityAuthorizesUnconditionally`, `PriorIndexClaimsEveryRoot` and `PriorIndexPrematurelyClaimsThisFactory` are the definite-`true` answers, and each is a different accusation with a different remedy.
Both classes are equally fail-closed; only the diagnostic differs, and a single error for both told an operator to hunt a missing selector when the real cause was an authorization rule that authorizes everything - this codebase's could-not-check-rendered-as-a-neighbouring-state defect, inverted.
A non-canonical word is deliberately a non-answer rather than a `true`: it states no boolean, so reporting it as a deliberate authorization would be the same collapse in the other direction.
`test_a_dependency_answering_a_non_canonical_boolean_is_a_non_answer` holds that for both dependency slots, and the definite-`true` diagnostics are held by `test_an_authority_that_authorizes_indiscriminately_is_refused`, `test_a_prior_index_that_claims_every_root_is_refused` and `test_a_prior_index_that_prematurely_claims_this_generation_is_refused`.

### Write-once stays per contract and honest

`DogTagIssuerV2.issue` checks `issuedAt[r]` in its own storage; `registerRoot` checks `rootIssuer[root]` in its own factory's storage.
Both are unchanged. The factory adds one guard and it only ever refuses more.

`rootIssuer` is **generation-local**: this factory's mapping answers only for roots anchored by its own clones, and it can never answer for a generation-1 root.
Resolution across generations is the router's job, on the read side.

### The cross-generation guard (`priorIndex`) is mandatory

The write-once guards being **per contract** is what makes a revocation bypass possible across a generation boundary: a root anchored and then revoked on a generation-1 clone can be re-anchored on a generation-2 clone by any signer able to issue there, because neither contract has ever seen it.
The tag binding does not stop it either - the SBT is shared, so `R == profileRoot(dogTagId)` still holds.
Under newest-first resolution the provenance router would then return the fresh clone, `isValid` would be true, and **the revoked credential would verify again.**

`DogTagIssuerFactoryV2` takes an immutable `priorIndex` and **requires it to be non-zero.**
An earlier draft allowed zero to mean "first generation", which silently disabled the whole guard for a generation-2 deployment - the one place it matters most - and made a wiring omission indistinguishable from a deliberate choice.
There is no first generation to serve: generation 1 already exists, so any factory built from this source has something upstream of it.

The occupant is queried through **`isRootAnchored(bytes32)`**, the S-8 router's explicit cross-generation query, and not through `rootIssuer(bytes32)`.
`rootIssuer` is the generation-local index, so wiring it here would leave every generation before the immediately preceding one unguarded the moment a third generation exists; `isRootAnchored` spans them, which is why S-8 published it.
`test_the_upstream_guard_spans_every_generation_the_router_carries` holds the difference.

This is the **write-side** half of the router's oldest-first resolution (S-8), not a replacement for it: the router's ordering is what holds against a later generation that does not call this, and it must remain correct on its own.

**The permanent requirements on whatever occupies that slot** follow both router reads the factory needs.
It MUST answer `isRootAnchored`, MUST NOT revert, and MUST answer `false` for a root nothing has anchored.
It must also answer `isGeneration`, report this factory absent during construction, and report it present before any root can be registered.
The construction-time answers are probed before the immutable is stored.
At `registerRoot` time the membership answer is checked first; if the factory has not been appended, issuance reverts loudly with `FactoryNotRegisteredInPriorIndex(factory)` before the factory's `rootIssuer` mapping can be written, and the revert unwinds the clone's earlier `issuedAt` assignment.

**The topology is router FIRST, then the factory, then the append, then issuance - and nothing about it is circular.**
An earlier draft recommended pointing generation 2 at the generation-1 factory directly on the grounds that the router must know the generation-2 factory's address, which was simply wrong: `CloneProvenanceRouter` is append-only *precisely* to invert that ordering, and its own doc says so.
Deploy the router over generation 1, deploy the generation-2 factory with `priorIndex` set to that router, then `appendGeneration` the factory.
A factory already in the router's list may safely call `isRootAnchored`, because `registerRoot` checks before it writes, so its own mapping still answers zero for the root under consideration.
The append is an enforced issuance precondition rather than operator prose: `test_issuance_reverts_until_this_factory_is_appended_to_the_router` proves an attempted `issue` rolls back before append and anchors normally after append.
`test_the_router_topology_is_router_first_then_factory_then_append` pins the deployment ordering, and a generation-1 factory is *refused* in the slot outright because it answers neither router query.

**The residual, which no constructor can close.**
The probes prove the occupant *answers* both queries; they cannot authenticate it as the real router or prove that its generation list is complete.
A conforming always-`false` stub still passes construction, but it no longer reinstates the bypass: every later `registerRoot` loudly fails the membership check, so issuance cannot be counted as successful.
The remaining authenticity residual is a lying or stateful occupant that reports this factory present after deployment while omitting prior roots; a genuine router configured without every earlier generation is incomplete in the same material way.
Wiring the real router with **every** earlier generation therefore remains a cutover precondition, pinned as a deliberate limitation rather than a passing property by the updated `test_a_conforming_stub_prior_index_is_accepted_and_that_is_a_residual`.

### The S-6 capability ladder, under a widened clone set

Self-service widens who may add to `isClone`: under generation 1 only the protocol owner could, and now any approved provider can.
For generation 2, the S-6 capability ladder supersedes every older whitelist-only description: V2 does **not** reduce creation or writes to the legacy `isWhitelistedFor(recordType, signer)` question.
Creation asks `canCreateService(providerId, recordType, caller)`; anchoring asks `canIssue(clone, caller)`; revocation asks `canRevoke(clone, caller)`.
Those core reads compose the registrar's provider approval, service attachment, confirmed ownership, record type and signer grants.
A provider approved only for VACCINATION can therefore create only a VACCINATION clone, whose core attachment and `recordType()` remain VACCINATION, so nothing it produces can read as a TRAVEL credential.
Whitelist-only wording remains accurate for generation 1 and is superseded for this pair.

The other thing a widened `isClone` set could have carried is a **free-text `name()`**, and that is closed at the source - see §6.

Root squatting is unchanged in kind: any approved signer could already burn a root through `"root taken"` in generation 1.
State the salting defence narrowly, because it is narrower than it sounds: salted Poseidon roots make it infeasible to **guess** a root, and a root becomes public the moment an `issue` transaction is observable, so an attacker watching pending transactions can still front-run one specific anchor.
What salting rules out is blind, untargeted squatting - not a targeted race.

### The factory has no admin surface

No owner, no privileged function.
Nothing about it can be repointed or captured - the property `IssuerDomainRegistry`'s doc asks of a factory reference: *"a repointable factory reference would let one transaction redefine what counts as a genuine clone."*
Here there is nothing to repoint, in either direction.

`test_the_factory_has_no_owner_or_transfer_ownership_selector` holds this for the three named selectors only, and its own comment is careful about what it establishes.
Two things about that test are load-bearing and were wrong in an earlier draft: the calldata must be **well-formed**, since a genuine function reverts on a malformed decode, and the mutating probe must be a real `call` rather than a `staticcall`, since a genuine state-changing function reverts under `staticcall` regardless. With both mistakes present the test would have passed even if `transferOwnership` existed and worked.
A failing call establishes that the selector is **not reachable** - no such function and no fallback. It would also pass for a function that exists and reverts, so it is not evidence that the source declares none; that part is held by review.

### Event compatibility

`IssuerCreated(address indexed clone, bytes32 indexed recordType, string name)` and the clone's `RootIssued`/`RootRevoked` are **byte-identical** to generation 1's, as is `RootRegistered`, so the oversight indexer's existing decoders read a generation-2 clone with no change.
The owner arrives as an additive `IssuerOwnerRegistered(clone, owner, providerId, cloneNonce)` from the factory - not by widening an existing event, which would change its topic0 and silently drop it from the feed.

`test_the_legacy_creation_and_anchoring_event_topics_are_unchanged` asserts each `topic0` against the **literal generation-1 signature string**, not against this generation's own declaration, so a widened event fails there even though it would still be self-consistent.
It also asserts the emitted `name` is empty, which is the wire half of §6.

---

## 6. The legacy `name()` is permanently empty, and that is the fix

Generation 1's `name` was written by the factory's `onlyOwner` `createIssuer` at KYC time, and **that provenance is the only reason a consumer could read it as an authoritative issuer identity.**
The three deferred consumers of this field are `stacks/government/api/src/routes.rs`,
`packages/ui/src/domain/issuerDomainBinding.ts`, and the event-detail rendering in
`packages/ui/src/chain/provenance.ts`.
They must all treat generation 2's empty value as identity unavailable and never turn the document's own claim into an authoritative fallback.

Self-service breaks that argument at its root.
A caller-supplied name would be a provider-chosen string arriving with genuine factory provenance: a signer approved only for VACCINATION could create a clone named `"US Department of Agriculture"`, and because the clone really is factory-descended, link-1 provenance resolves, the on-chain name is read, and it renders as the authoritative issuer beside a green check.
That is precisely the attack the on-chain name read exists to defeat.

So caller control is removed rather than documented:

* `createIssuer` takes **no name argument**.
* `initialize` takes none either, and nothing in `DogTagIssuerV2` ever writes the slot, so `name()` answers `""` for the life of every generation-2 clone.
* `IssuerCreated` still carries the field - the signature and topic0 are wire-compatible - and always emits the empty string.

A consumer reading `name()` on a generation-2 clone must therefore report authoritative identity **unavailable**, and must not fall back to the document's own claim.
Registrar-controlled identity for this generation comes from the authority core instead: its publication-safe identity anchor, reached through the core's directory resolver.

**Reconciling the existing readers is a later slice, and is out of scope here.**
Those files are not part of S-7 and are deliberately untouched; what this slice guarantees is that there is no provider-chosen string for them to mistake.

---

## 7. What a later generation inherits

A generation-3 issuer implementation would inherit, unchanged:

* **The ownership model.** Two-step, non-renounceable, non-zeroable, control-not-capability. Nothing about it is generation-specific.
* **The authorization predicate's shape.** `authorizeClone(candidate, claimant) -> recordType` is a pure function of `isClone` plus the clone's own getters, so a generation-3 factory implements the same signature over its own storage. A consumer binds to the shape, not to a particular factory.
* **The `priorIndex` contract.** Its two-query root and membership requirements and its mandatory non-zero-ness are unchanged; generation 3 points it at a router carrying generations 1 and 2, then must be appended before issuance.
* **Dependency identity and probes.** A generation-3 factory has the same immutability, so it has the same obligation to pin the exact intended issuer runtime and behaviour-check the authority and prior index before construction completes.

What a later generation does **not** inherit, and must re-establish:

* **`isClone` and `rootIssuer` are per-factory storage.** A generation-3 factory knows nothing of generation-2 clones. Cross-generation provenance is the router's job on the read side, and `priorIndex`'s on the write side.
* **`activeIssuer` pointers.** Owner-keyed and per-factory; a provider that moves to a generation-3 clone repoints there explicitly.
* **Nothing already anchored moves.** `rootIssuer[R]` is write-once per factory, so a generation-2 root resolves to its generation-2 clone forever. That is what makes the router's oldest-first resolution both necessary and safe.

---

## 8. Coordination: what the sibling slices must satisfy

This slice was built against the siblings' **interfaces**, not against their branches, and the suite's authority is a local stand-in rather than the real `ProviderRegistry`.
Each obligation below is permanent, because the thing that carries it is immutable.

**To S-6 (`ProviderRegistry`).** The pair asks the core for exactly **four** functions, and every one of them is load-bearing:

| function | asked by | if the core lacks it |
|---|---|---|
| `canCreateService(bytes20,bytes32,address)` | the factory, on every creation | no provider can ever create a clone |
| `canIssue(address,address)` | a clone, on every `issue` | the generation can never anchor anything |
| `canRevoke(address,address)` | a clone, on the ordinary `revoke` arm | no originator can ever invalidate a root |
| `hasRole(bytes32,address)` | a clone, on `adminRevoke` and the admin `revoke` arm | `adminRevoke` - the compromised-signer mass-revoke lever - reverts for every call, forever |

The factory's `registry` is `immutable` and a clone pins its own at `initialize` with no setter, so none of this is repointable after deployment.
An earlier draft of this document claimed the factory asked for "exactly one function, `isWhitelistedFor`"; that was wrong in both the count and the choice, and §3 records why the legacy selector cannot serve.

Two further obligations that are easy to miss because they are registrar *actions* rather than interface shapes:

* **`addFactoryGeneration(generationId, factoryAddress)` must be called, and the generation must be active, before the factory can create anything.** `canCreateService` resolves the factory from `msg.sender`, so an unpinned factory is refused every time - correctly, and with no diagnostic that names the omission. `test_the_core_must_pin_this_factory_as_an_active_generation` records it.
* **The core's providerId-keyed service attachment should compose `authorizeClone`** rather than re-deriving provenance and control. That is why the predicate is published as a function instead of being inlined.

**To S-8 (`CloneProvenanceRouter`).** `priorIndex` is a refinement the plan assigns to S-8, but it is implemented here because it **must** be in the factory's bytecode: the reference is `immutable` and a deployed factory cannot gain it later.
Wire the router itself, deployed over every earlier generation, and deploy it **before** the factory - the append-only design exists to permit exactly that ordering, and the "it would be circular" reading is wrong (§5).
Append the new factory before attempting issuance; `registerRoot` checks router membership and reports the missing append as `FactoryNotRegisteredInPriorIndex`, rather than letting the issuance appear successful.
The two-query requirements on the slot and the authenticity/completeness residual are in §5 as well.

**To S-9 (`ServiceDomainResolver`) - SHIPPED, and it deliberately does NOT compose `authorizeClone`.**
The instruction below was written before that slice existed and is kept because it still holds for the OTHER named consumer, S-6's attachment path.
Do not act on it for S-9: `contracts/src/ServiceDomainResolver.sol` composes the core's `canWriteService(service, caller, SERVICE_PERMISSION_RECORD)` instead, for two reasons either of which is sufficient - `authorizeClone` requires `claimant == owner()` exactly, so it cannot admit an owner-appointed delegate and would leave `SERVICE_PERMISSION_RECORD` with no consumer at all; and it lives on a generation-specific factory.
The full argument is in `docs/SERVICE_DOMAIN_RESOLVER.md` §"Why `authorizeClone` is deliberately not composed"; the anti-drift property this note protects is satisfied there, because that contract derives neither standing nor ownership itself.

The original instruction, which remains correct for S-6: the captain's AND is now checkable - the capability half from the core, the owner half from `authorizeClone`.
Call it rather than reimplementing `_isSpawningBusiness` - the salt-recomputation stand-in exists only because generation-1 clones have no owner, and generation-2 clones do.
Note also that the identity a resolver publishes must come from the core's identity anchor, never from the clone's `name()`, which is empty by construction (§6).

**To the cutover (S-13/S-14): the mandatory issuer-whitelist pillar must resolve authority for the generation that anchored the root.**
This is the obligation that gates credential VALIDITY, and it is the one sibling obligation that is a code change in each consumer rather than a configuration flip.
There is one owner-hidden verification pillar and it stays one - what follows is not a mode, a second path, or an A/B choice.

The pillar today resolves the issuing clone from the verifier's OWN configured factory, reads `clone.recordType()` and `issuedBy[R]` off that clone, and then asks the verifier's OWN configured **generation-1 `IssuerRegistry`**, `isWhitelistedFor(recordTypeKey, signer)`.
Against a generation-2 root that read produces one of two refusals, and **both must be stated**, because they are different claims:

* A verifier still configured with the generation-1 factory gets `rootIssuer(R) == 0` for a generation-2 root - the mapping is generation-local - so resolution is `NoRecord`, the pillar is **indeterminate**, and the credential is refused as unresolved.
* A verifier resolving against the generation-2 factory, or against the S-8 router, resolves the clone and reaches `issuedBy[R]`. That signer's authority exists only in the S-6 `ProviderRegistry` under `canIssue`; the legacy `isWhitelistedFor` is deliberately not that oracle (§3), so the generation-1 registry returns a **definite `false`** and the pillar's tri-state treats it as a real authenticity failure.

The second is the worse shape: an honestly issued credential rendered as a forged one.
It is also not transitional.
`RpcAdapter::is_whitelisted_for` takes no registry address by design - the implementor supplies it from its own config, which is exactly what keeps a document from choosing the answering contract - so making the read generation-aware is a code change per consumer.
And the router's C-12 cutover freeze delists every signer in the generation-1 `IssuerRegistry`, so the generation-1 answer moves further from correct rather than closer.

The consumers that must move together, all of which read the pillar from a verifier-owned registry:

* `packages/ui/src/wallet/verifyCredential.ts`
* `stacks/government/api/src/routes.rs::verify`
* `stacks/vet/api/src/routes.rs::verify_credential`
* `crates/dogtag-standard-rs/src/verify.rs`
* the two mobile importers, `RoaxRpc.issuerWhitelistPillar` and `RecordImporter.foldIssuerWhitelist`

### CORRECTION (PR #127 superseded the paragraphs above): the pillar's blocker is an EVENT vocabulary, not a getter

Everything above was written while the pillar read the CURRENT-state getter.
It no longer does.
PR #127 moved it to the historical question - was a grant in force AT THE BLOCK THIS ROOT WAS ANCHORED - reconstructed from `Whitelisted`/`Delisted` logs, so any third party with an RPC reaches the same verdict without trusting our code.
Three consequences, and the middle one is a live defect rather than a wording problem.

**`isRecognizedIssuer` is NOT the pillar's migration target, and `ProviderRegistry.sol`'s own doc comment said otherwise until this correction.**
That contract landed 2026-07-30 (#111) and #127 landed 2026-07-31, so the comment describes a pillar that had already changed.
`_isRecognizedIssuer` is `s.providerId != bytes20(0) && _issuanceCapabilities[serviceAddress][signer]` - current storage, no block, no root.
Handing its boolean to the pillar would revert #127 under a new name.

**The pillar is ITSELF a record-type caller, via LOGS rather than a getter, and against generation 2 it produces a confident forgery verdict.**
`Whitelisted(bytes32 indexed recordType, address indexed signer)` puts the record-type key in `topic1`, so the grant query fails against the successor for exactly the reason `docs/CLIENT_REPOINT.md` gives for the getter - only more quietly, because nothing reverts.
`ProviderRegistry` records grants as `IssuanceCapabilitySet(service, signer, allowed)`: different name, different `topic0`, different argument shape.
That filter therefore matches NOTHING there, and the fold's empty-history rule - deliberately a definite `NotAuthorized`, because on generation 1 an empty log really is evidence that `onlyWhitelisted` could not have passed - turns "we asked the wrong contract in the wrong language" into "this credential is forged".
It reaches `POST /v1/verify`, which is unauthenticated.

**What shipped, and what is still open.**
All five surfaces now guard that rule: an empty history is a definite refusal ONLY when the authority positively speaks generation 1, established by probing `isRecognizedIssuer` - a selector `IssuerRegistry` provably does not implement, since its entire external surface is `whitelistFor`/`delistFor`/`isWhitelistedFor` and it has no fallback.
The probe's ANSWER is discarded; it identifies the generation and nothing else.
It is scoped to the EMPTY case because a non-empty history is itself proof the authority speaks generation 1, so the extra call lands on the refusal path only and cannot perturb any answer #127 established.
A generation-2 root now reports **could not determine**, never a forgery verdict.

Read "all five" as a correction rather than as the original claim.
It first shipped on the two Rust backends alone while this section already stated the guarantee globally - `packages/ui/src/wallet/contracts.ts` (`authorityGenerationOf`, consumed by `verifyCredential.ts`), Kotlin `RoaxRpc.grantAtIssuance` and Swift `RoaxRpc.grantAtIssuance` all still folded an empty history to a definite refusal.
A claim ahead of its code is its own defect: the gap was set to surface at the C-9/C-10 client repoint, as a genuine credential refused as forged on the vet/groomer verify panel, the admin bench and both mobile importers, rather than at review.
The SDK path needs no sixth change - `crates/dogtag-standard-rs/src/verify.rs` reaches the chain through vet's `ChainRpcAdapter`, which delegates to `whitelisted_at_issuance` and inherits the guard.

**A REVERT and an UNDELIVERED PROBE are different facts, and only the first licenses the generation-1 conclusion.**
The guard shipped on all three Rust sites as `.is_ok()` / `if let Ok(..)`, which reads a timeout, a reset connection or a rate-limit response as "the contract refused it" - could-not-check rendered as a definite answer, inside the guard built to remove exactly that.
On the pillar a transport failure leaves the definite `NotAuthorized` standing, i.e. a forgery accusation from a read that never happened.
In `issuance_capability` it is worse: the fall-through asks the LEGACY getter, which `ProviderRegistry` *does* implement and answers `false` for off the orthogonal VERIFY axis at a zero `msg.sender`, so a genuinely authorised generation-2 signer is refused with 403 "address not approved for this recordType yet".
**A node-level error is not a contract answer either**, and that is the same defect one level in.
The first cut keyed on "the node returned a JSON-RPC error for a request it processed" - alloy's `RpcError::ErrorResp`, mobile's "HTTP 200 carrying an `error` member", the web's `ContractFunctionRevertedError`.
A `-32005` rate limit, a `-32603` internal error, a `-32601` method-not-found and a `-32002` resource-unavailable all satisfy that, and none is evidence the contract executed anything - so one of them left an empty grant history standing as a definite forgery verdict, on the unauthenticated `POST /v1/verify` where rate limiting is realistic rather than hypothetical.
Only an EXECUTION REVERT licenses the generation-1 conclusion, which is also exactly the signal wanted: `IssuerRegistry` has no `isRecognizedIssuer` and no fallback, so its dispatcher reverts.
The rule is `code == 3 || message contains "execution reverted"`, now identical on all four surfaces: Rust `answered_with_execution_revert`, Kotlin and Swift `isExecutionRevert`, and viem's `ExecutionRevertedError`, whose `getNodeError` keys on that same pair.
`ErrorPayload::as_revert_data` is deliberately not the discriminator - it requires revert DATA, and this revert is a bare dispatcher refusal carrying none (`"data":"0x"`, confirmed against ROAX on 2026-07-31 with the real generation-1 registry).
The web probe uses `client.call` rather than `readContract` for the same reason: `getContractError` folds both code `3` and `-32603` into `ContractFunctionRevertedError`, so walking for that class read an internal error as a refusal.
`MemChain` is a different `ChainClient` and cannot reach the alloy code at all, so the classifier is extracted (`answered_with_error` / `generation_from_probe`) and pinned by `chain::tests`, while the MemChain cases pin the trait's contract; `scripts/verify-issuance-authority-mutations.sh` mutates the two separately.

Still open, and it is what the cutover actually needs: **answering the historical question for generation 2 means decoding `IssuanceCapabilitySet`**, mirrored across all five surfaces above.
That is not attempted here - `ProviderRegistry` has no deployed address, so a generation-2 event decoder could not be validated against anything, and the mirrored fold is #127-scale work (36 files, ~3,400 insertions).
Until it lands, generation-2 credentials are honestly unresolvable rather than dishonestly refused, and `stacks/vet/api/tests/issuance_authority_migration.rs` pins that a generation-2 issuance fails LOUDLY at confirm rather than stranding a record silently.

**Sequencing: this must land before a generation-2 clone anchors anything a consumer will verify.**
Unlike the deferred `name()` readers of §6 - where the guarantee this slice ships is that there is no provider-chosen string to mistake - an unreconciled pillar does not degrade to "identity unavailable"; it refuses genuine credentials.
None of those files is touched here, and nothing in this branch wires a consumer to either contract: the pair is built and undeployed, so the obligation is recorded for the cutover rather than discharged in this slice.

## 9. Build and test

```sh
cd contracts && forge test --match-contract IssuerV2
```

63 tests in `IssuerV2Test`, 3 more in `IssuerV2ProviderAuthorityInterfaceTest`; 231 in the whole `contracts` suite.
That total moved from 202 when S-6's `ProviderRegistryTest` landed in the same tree, so a `202` anywhere is stale rather than a different way of counting.
`--match-contract IssuerV2` (not `IssuerV2Test`) is what runs both of this slice's suites.
Prefer `forge test` over a bare `forge build`: it compiles only the real dependency closure.
The blanket "never a bare `forge build`" rule recorded here described a real failure - a full build compiled the vendored OpenZeppelin submodule's `certora/harnesses/*`, which import generated files that are not present, and failed with "File not found" (a submodule artifact, not a project error).
That **no longer reproduces**: re-measured 2026-07-30 on Foundry 1.5.1-stable with the submodules at their pinned revisions (`openzeppelin-contracts` `v4.8.0-743-g69c8def5`, `forge-std` `v1.9.4`), a bare `forge build` from a removed `out/` exits 0.
The mechanism of the change was not established, so read that as a measurement on that toolchain rather than a guarantee.

A fresh worktree has no `contracts/lib` contents; run `git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts` first.

### `IssuerV2ProviderAuthorityInterfaceTest`, and why its negative control has the shape it does

The pair reaches the authority core through an interface it declares locally, and Solidity checks nothing across that seam, so this suite pins the agreement against the REAL `ProviderRegistry` on both axes a signature has: selector equality for the argument lists, and single written external function types both sides are assigned to, so a diverged return type or state mutability fails the BUILD instead of surviving to misdecode at runtime.

Five mutations were applied, run and reverted against a temporary harness (not committed, matching §9's convention below). Each landed on its expected outcome, and the tree was asserted byte-identical afterwards:

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

The owner term is read **live** off the clone and compared against the registrar-confirmed owner, which is what makes the handover-suspension behaviour in §3 real rather than stipulated.

### Historical mutation evidence: one-off, not a committed gate

During S-7 review, a one-off temporary harness applied, ran and reverted thirty-three mutations to the two sources; every one turned a named test red, and no mutation went uncaught.
The table was re-derived from that run rather than recounted.
Most are a single changed line; the widened-event row needs two coordinated edits to compile, which is why the count is stated as mutations rather than as single-line changes.
Where several tests fail, the one named is the test written for that property.

That temporary harness was **not committed**, so this repository does not provide a repeatable command that reproduces the mutations.
The committed source and named tests make every table mapping inspectable, and `forge test` reruns those tests against the checked-in source; it does not apply the mutations.
Treat the table as true historical evidence, not as a mutation gate that silently ran as part of the ordinary suite.
The later exact-runtime-identity, pre-append-membership and constructor-diagnostic regressions are held directly by their named tests in §5; they are not retroactively claimed as rows in this thirty-three-mutation run.
In particular, collapsing the definite-`true` refusals back into the no-answer errors is caught by the three named tests above and not by any row below.

| mutation | test that caught it |
|---|---|
| `_authorization` drops the `isClone` check | `test_a_hostile_impostor_is_never_even_called` |
| `_authorization` drops the owner check | `test_a_repoint_cannot_take_another_providers_genuine_clone` |
| `_authorization` checks the owner before `isClone` | `test_a_hostile_impostor_is_never_even_called` |
| `registerRoot` drops the cross-generation guard | `test_a_revoked_prior_generation_root_cannot_be_re_anchored` |
| `registerRoot` drops its own write-once guard | `test_register_root_stays_clone_only_and_write_once` |
| `resolveActiveIssuer` trusts the raw mapping | `test_a_stale_pointer_degrades_to_absent_rather_than_to_a_false_claim` |
| `setActiveIssuer` keys a slot by something other than the clone | `test_the_record_type_key_is_resolved_from_the_clone_not_supplied` |
| the creation approval gate is removed | `test_an_unapproved_caller_cannot_create` |
| creation asks the legacy whitelist question instead of `canCreateService` | `test_an_approved_provider_deploys_its_own_clone_and_owns_it` |
| the nonce is dropped from the salt | `test_the_nonce_gives_a_provider_a_second_address` |
| a caller-chosen name reaches the legacy creation event | `test_the_legacy_creation_and_anchoring_event_topics_are_unchanged` |
| `IssuerCreated` is widened with the owner | `test_the_legacy_creation_and_anchoring_event_topics_are_unchanged` |
| `initialize` writes a name into the legacy slot | `test_the_legacy_name_getter_is_permanently_empty` |
| the then-current implementation getter probe is dropped | `test_a_wrong_implementation_whose_getters_revert_is_refused` (renamed after the run) |
| the dependency code check is dropped | `test_an_eoa_dependency_is_refused` |
| the authority ABI probe is dropped | `test_an_authority_that_authorizes_indiscriminately_is_refused` |
| the prior-index ABI probe is dropped | `test_a_prior_index_that_does_not_answer_is_refused` |
| the prior-index probe accepts any single word | `test_a_prior_index_that_claims_every_root_is_refused` |
| a zero prior index is accepted again | `test_a_zero_dependency_is_refused` |
| `renounceOwnership` re-enabled | `test_owner_can_never_become_the_zero_address` |
| `acceptOwnership` drops the zero-sender guard | `test_owner_can_never_become_the_zero_address` |
| ownership transfer made one-step | `test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller` |
| `initialize` sets no owner | `test_an_approved_provider_deploys_its_own_clone_and_owns_it` |
| `issue` gated on `canRevoke` instead of `canIssue` | `test_a_superseded_clone_refuses_new_issuance_but_still_revokes` |
| the ordinary `revoke` arm gated on `canIssue` instead of `canRevoke` | `test_a_superseded_clone_refuses_new_issuance_but_still_revokes` |
| `revoke` drops the originator binding | `test_a_capable_signer_that_did_not_issue_a_root_cannot_revoke_it` |
| `adminRevoke` stops reporting what it skipped | `test_admin_revoke_reports_every_skipped_root_and_still_revokes_the_rest` |
| `adminRevoke` reverts on a skip instead of reporting it | `test_admin_revoke_reports_every_skipped_root_and_still_revokes_the_rest` |
| `adminRevoke` drops its admin gate | `test_only_the_protocol_admin_may_admin_revoke` |
| `bulkIssue` drops its capability gate | `test_an_empty_batch_is_where_the_two_bulk_gates_differ` |
| `bulkRevoke` skips a root it cannot revoke instead of reverting | `test_bulk_revoke_reverts_on_the_first_unrevocable_root` |
| the authority probe asks only the legacy-compatible subset | `test_a_generation_one_registry_cannot_be_the_authority_core` |
| the factory gains an owner surface | `test_the_factory_has_no_owner_or_transfer_ownership_selector` |

Two mutations that were tried and are **not** in the table, because they change no behaviour and a row for them would be vacuous evidence:

* removing `_authorization`'s zero-`claimant` short-circuit - redundant with the owner comparison, since a clone's `owner()` can never be zero (§4);
* re-wrapping the cross-generation guard in `if (priorIndex != 0)` - unreachable, because the constructor already refuses a zero `priorIndex`. The guard that matters is the constructor's, and its own row is above.

`adminRevoke`'s skip reporting deserves one closing note, since it is the only place a loop deliberately does not revert: silence there was the defect.
A caller submitting a compromised signer's full history got one successful transaction whether the sweep revoked everything or nothing, and those two outcomes are the difference between a contained compromise and an uncontained one.
`bulkRevoke` still reverts on the first root it cannot revoke, because a targeted batch naming an already-revoked root is a caller mistake worth surfacing - the asymmetry is deliberate.
