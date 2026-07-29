# The owner-bearing issuer clone and the self-service factory

`contracts/src/DogTagIssuerV2.sol` and `contracts/src/DogTagIssuerFactoryV2.sol`.
Registry-plan slice **S-7**.

**Status: built and tested only. Nothing is deployed.**
There is no address for these contracts on ROAX or anywhere else, so no `.env.example`, no `deployments/roax.json` entry and no client config carries one yet.
The address propagation is part of the cutover (`S-13` / `S-14`), and deploying is a separately captain-authorized step.

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

### Ownership is CONTROL, not an issuance capability

The owner may not issue, may not revoke, and gains no privilege over roots.
Issuance stays gated solely on the registry whitelist, and that is load-bearing: **delisting a signer must stop the next `issue` and touch nothing already anchored.**
If ownership carried an issuance right, delisting would no longer stop issuance and the delist lever would be silently dead.

So a delisted provider still owns its clone, may still transfer it and may still repoint its listing - it simply cannot anchor anything new.

`revoke` keeps generation 1's authority exactly: the H-1 originator, or the registry's protocol admin.
Extending revocation to the clone owner would let an owner revoke credentials it did not issue. That is a distinct governance decision and is deliberately not taken here.

### There is exactly one creation path, and it takes no owner argument

`createIssuer` sets the owner to `msg.sender` and salts the clone with the same address.
An "operator creates on behalf of a provider" variant was considered and rejected: it is the shape that produced the generation-1 weakness above.
With a single path, **`owner()` and the salt binding can never disagree**, and an operator wanting a protocol-owned clone simply creates one itself (the governance signer is already whitelisted for the live record types, so migration is unaffected).

### The creation gate is the issuance-signing capability, and that conflation is transient

`createIssuer` is gated on `isWhitelistedFor(recordType, msg.sender)` - the *issuance-signing* capability.
The plan is explicit that a provider's controller is **not** its issuance signer (§4 item 2), so a freshly created clone is owned by a signing key rather than by the organisation's controller.

That is deliberate and self-correcting: the correct next step after creation is the two-step handover to the controller, which is exactly what `test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller` exercises.
Binding to `isWhitelistedFor` is what lets this factory be deployed against today's `IssuerRegistry` **and** later against the S-6 `ProviderRegistry` with no code change and no adapter, because that signature is a load-bearing interface requirement on the core.

---

## 4. The forgery-proof repoint

### Which check enforces it

**`isClone[candidate]`, read from the factory's own storage.**
That mapping is written in exactly one place - `createIssuer` - so an address is in it if and only if this factory deployed it. No other path sets it.

The predicate is published once, on the factory, as `authorizeClone(candidate, claimant) -> bytes32 recordType`, so consumers call it rather than re-deriving it.
Two contracts need it (S-6's service attachment and S-9's domain write), and two parallel implementations of one authorization rule is how the vet and mobile verdict paths came to disagree in this codebase already.

Three questions, three answers:

1. **Provenance** - `isClone[candidate]`, from the factory's own storage, never from the request. This is what stops a hand-rolled contract.
2. **Control** - `IOwnedIssuer(candidate).owner() == claimant`, read live from the named contract. Without it, provider A could repoint its listing at provider B's genuine clone: not contract forgery, but misattribution. *"There's no way to do any false contract inputs"* is true of provenance and silent about attribution.
3. **Attachment** - that the clone belongs to *this provider* in the authority core - is S-6's, not this factory's. This predicate answers provenance and control; the core composes it with identity.

### It resolves, it does not accept

The record type is **returned, not accepted**.
`authorizeClone` reads `recordType()` off the clone and hands it back, so a caller keying anything by record type gets a chain-resolved key in the same call that authorized the address.

This is the codebase's standing rule applied to a write path:

> An address may be an **argument to a write**, checked against the registry's own factory reference.
> An address may never be an **argument to a read that decides trust** - there it must be resolved from the chain.

`setActiveIssuer(address clone)` takes exactly one value. Both the authorization and the storage key come from chain state.
There is no argument by which a caller could name a slot it did not earn.

### Provenance is checked first, so a hostile address never executes

`isClone` is a storage read and it short-circuits.
A non-clone is refused before any external call, so it cannot execute at all - not even to revert.
`test_a_hostile_impostor_is_never_even_called` holds this by pointing the factory at a contract whose getters revert with a distinctive error and asserting the factory's own `NotAClone` comes back instead.

### The nonce, and why the repoint has no target without it

Generation 1 salts a clone `keccak256(recordType, business)`, and `Clones.cloneDeterministic` reverts on a repeated `(implementation, salt)` pair.
So a provider has **exactly one possible clone address per record type**, and a second `createIssuer` for the same pair simply reverts.
*"They can also change their smart contract address to a VALID CLONED smart contract"* is then unreachable: there is no second address to move to.

`keccak256(recordType, business, cloneNonce)` gives a provider a fresh clone for key rotation or after a compromise, while keeping everything the old salt bought - the address is still deterministic and still exactly predictable before deployment via `predictIssuer`.

### What a repoint does NOT do

It changes only where **new** credentials are anchored.
`rootIssuer[R]` is write-once, so everything the old clone already issued keeps resolving to the old clone and stays revocable there.
That is correct behaviour rather than a limitation: retroactively re-attributing issued credentials to a contract that did not issue them is exactly the misattribution the control check exists to prevent.

### The pointer is re-validated on read

`activeIssuer[owner][recordType]` is the raw record; `resolveActiveIssuer` returns it **only if it still passes the predicate**, and `address(0)` otherwise.
So a pointer left stale by an ownership handover degrades to "no active issuer recorded" rather than to a claim that is no longer true.
A stale pointer is a could-not-establish, and this codebase does not render those as established.

The pointer is not inherited by a new owner either - a repoint is an explicit act.

### How it relates to S-6

`activeIssuer` is the **owner-keyed, self-service** pointer: which of a provider's own clones is currently live for a record type.
`ProviderRegistry`'s providerId-keyed, registrar-confirmed service attachment (S-6) is the **authoritative** record of which contracts belong to which organisation.
They are complementary, not competing, and they are keyed differently on purpose: an owner address is a key, an organisation is not.

---

## 5. What does not regress

### Write-once stays per contract and honest

`DogTagIssuerV2.issue` checks `issuedAt[r]` in its own storage; `registerRoot` checks `rootIssuer[root]` in its own factory's storage.
Both are unchanged. The factory adds one guard and it only ever refuses more.

### The cross-generation guard (`priorIndex`)

The write-once guards being **per contract** is what makes a revocation bypass possible across a generation boundary: a root anchored and then revoked on a generation-1 clone can be re-anchored on a generation-2 clone by any signer whitelisted for that clone's record type, because neither contract has ever seen it.
The tag binding does not stop it either - the SBT is shared, so `R == profileRoot(dogTagId)` still holds.
Under newest-first resolution the provenance router would then return the fresh clone, `isValid` would be true, and **the revoked credential would verify again.**

`DogTagIssuerFactoryV2` takes an immutable `priorIndex`. When set, `registerRoot` also refuses a root any earlier generation already holds, so the duplicate never comes into existence.
This is the **write-side** half of the router's oldest-first resolution (S-8), not a replacement for it: this guard is only as good as the deployed `priorIndex`, and the router's ordering must remain correct on its own.

**Two permanent requirements on whatever occupies that slot**, because the reference is immutable and cannot be fixed later:

* it MUST return `address(0)` for a root it has never seen, and
* it MUST NOT revert.

The factory treats the call as a gate on every `registerRoot`, so a reverting occupant would brick issuance for the whole generation.

Type it as `rootIssuer(bytes32) view returns (address)` - the exact and only shape `VerificationRegistryConsent` consumes - so the same address serves as a prior factory (generation 2) or as a `CloneProvenanceRouter` spanning every earlier generation (generation 3+).
Generation 2's `priorIndex` should be the **generation-1 factory directly**, not the router: the router must know the generation-2 factory's address, so pointing the factory at the router would be circular.

### The issuer-whitelist pillar, under a widened clone set

Self-service widens who may add to `isClone`: under generation 1 only the protocol owner could, and now any whitelisted signer can.
The mandatory issuer-whitelist pillar is unaffected, because it keys its whitelist question on `clone.recordType()` rather than on any claim.
A provider cleared only for VACCINATION can create only a VACCINATION clone, whose `recordType()` is VACCINATION, so nothing it produces can read as a TRAVEL credential.

Root squatting is unchanged in kind: any whitelisted signer could already burn a root through `"root taken"` in generation 1, and it stays infeasible against salted Poseidon roots because the attacker must first know one.

### The factory has no admin surface

No owner, no privileged function.
Nothing about it can be repointed or captured - the property `IssuerDomainRegistry`'s doc asks of a factory reference: *"a repointable factory reference would let one transaction redefine what counts as a genuine clone."*
Here there is nothing to repoint, in either direction.

### Event compatibility

`IssuerCreated(address indexed clone, bytes32 indexed recordType, string name)` and the clone's `RootIssued`/`RootRevoked` are **byte-identical** to generation 1's, so the oversight indexer's existing decoders read a generation-2 clone with no change.
The owner arrives as an additive `IssuerOwnerRegistered(clone, owner, cloneNonce)` from the factory - not by widening an existing event, which would change its topic0 and silently drop it from the feed.

---

## 6. What a later generation inherits

A generation-3 issuer implementation would inherit, unchanged:

* **The ownership model.** Two-step, non-renounceable, non-zeroable, control-not-capability. Nothing about it is generation-specific.
* **The authorization predicate's shape.** `authorizeClone(candidate, claimant) -> recordType` is a pure function of `isClone` plus the clone's own getters, so a generation-3 factory implements the same signature over its own storage. S-6 and S-9 consume the shape, not a particular factory.
* **The `priorIndex` contract.** Generation 3 sets it to a router over generations 1 and 2 rather than to a single factory. The interface and its two requirements are unchanged.

What a later generation does **not** inherit, and must re-establish:

* **`isClone` and `rootIssuer` are per-factory storage.** A generation-3 factory knows nothing of generation-2 clones. Cross-generation provenance is the router's job, on the read side, and `priorIndex`'s on the write side.
* **`activeIssuer` pointers.** Owner-keyed and per-factory; a provider that moves to a generation-3 clone repoints there explicitly.
* **Nothing already anchored moves.** `rootIssuer[R]` is write-once per factory, so a generation-2 root resolves to its generation-2 clone forever. That is what makes the router's oldest-first resolution both necessary and safe.

---

## 7. Coordination: what the sibling slices must satisfy

This slice was built against interfaces, not against `dogtag-provreg-s6`'s or `dogtag-router-s8`'s
branches. Three obligations flow out of it, and each is permanent because the thing that carries it is
immutable.

**To S-8 (`CloneProvenanceRouter`).** `priorIndex` is a refinement the plan assigns to S-8, but it is
implemented here because it **must** be in the factory's bytecode: the reference is `immutable` and a
deployed factory cannot gain it later. Whatever occupies that slot must return `address(0)` for a root
it has never seen and **must not revert** - the factory gates every `registerRoot` on it, so a reverting
occupant bricks issuance for the whole generation with no way to repoint. Generation 2 should point it
at the generation-1 factory directly rather than at the router: the router needs the generation-2
factory's address, so the reverse would be circular. This closes the revocation bypass on the write
side; the router's oldest-first resolution is still required on the read side, because this guard is
only as good as the deployed `priorIndex`.

**To S-6 (`ProviderRegistry`).** The factory asks the approval oracle for exactly one function,
`isWhitelistedFor(bytes32,address)`, which the plan already makes a load-bearing interface requirement
on the core. Keeping that signature is what lets this factory be deployed against today's
`IssuerRegistry` and later against the core with no code change and no adapter. The core's
providerId-keyed service attachment should compose `authorizeClone` rather than re-deriving provenance
and control - that is why the predicate is published as a function instead of being inlined.

**To S-9 (`ServiceDomainResolver`).** The captain's AND is now checkable: the whitelist half from the
registry, the owner half from `authorizeClone`. Call it rather than reimplementing
`_isSpawningBusiness` - the salt-recomputation stand-in exists only because generation-1 clones have no
owner, and generation-2 clones do.

## 8. Build and test

```sh
cd contracts && forge test --match-contract IssuerV2Test
```

28 tests. Use `forge test`, never a bare `forge build`: a full build tries to compile the vendored OZ submodule's `certora/harnesses/*`, which import generated `../patched/*` files that are not present, and fails with "File not found" - a submodule artifact, not a project error.

A fresh worktree has no `contracts/lib` contents; run `git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts` first.

The suite is not vacuous by inspection - it was verified by mutation.
Eleven single-line changes to the two sources were each applied, run, and reverted; every one turned a named test red:

| mutation | test that caught it |
|---|---|
| `authorizeClone` drops the `isClone` check | `test_a_repoint_cannot_accept_an_address_the_factory_did_not_produce` |
| `authorizeClone` drops the owner check | `test_a_repoint_cannot_take_another_providers_genuine_clone` |
| `cloneAuthorization` checks the owner before `isClone` | `test_a_hostile_impostor_is_never_even_called` |
| `registerRoot` drops the `priorIndex` guard | `test_a_revoked_prior_generation_root_cannot_be_re_anchored` |
| `resolveActiveIssuer` trusts the raw mapping | `test_a_stale_pointer_degrades_to_absent_rather_than_to_a_false_claim` |
| `setActiveIssuer` keys a slot by something other than the clone | `test_the_record_type_key_is_resolved_from_the_clone_not_supplied` |
| `renounceOwnership` re-enabled | `test_owner_can_never_become_the_zero_address` |
| `acceptOwnership` drops the zero-sender guard | `test_owner_can_never_become_the_zero_address` |
| ownership transfer made one-step | `test_key_rotation_hands_control_from_the_issuance_signer_to_a_controller` |
| `initialize` sets no owner | `test_an_approved_provider_deploys_its_own_clone_and_owns_it` |
| the creation approval gate removed | `test_an_unapproved_caller_cannot_create` |
| the nonce dropped from the salt | `test_the_nonce_gives_a_provider_a_second_address` |
