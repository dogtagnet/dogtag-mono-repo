# CloneProvenanceRouter

`contracts/src/CloneProvenanceRouter.sol`, tests in `contracts/test/CloneProvenanceRouter.t.sol`.

**Status: built and tested only. Nothing is deployed.**
There is no address for it in `contracts/deployments/roax.json`, no `.env.example` entry, and no consumer points at it.
Deployment is a separate, captain-authorized step (registry plan `dogtag-regplan-p3` slice S-8, cutover step C-4).

## What it is

A read-only router that occupies `VerificationRegistryConsent`'s immutable `rootIndex` slot in place of a single factory, and answers for an ordered list of factory generations.

It implements exactly the read shape the registry consumes:

```solidity
function rootIssuer(bytes32 root) external view returns (address);
```

plus `isClone(address)`, which is the other factory read the mobile apps and the web verifier make directly (`packages/ui/src/wallet/contracts.ts`, `RoaxRpc.kt`).
Pointing those consumers at the router rather than at one factory address is what lets a single app build recognise clones of every generation.

It does **not** implement `registerRoot`.
Writes stay on each generation's own factory, where the `isClone[msg.sender]` gate lives.

## Why it exists

`VerificationRegistryConsent.rootIndex` is `immutable` (`VerificationRegistryConsent.sol:88`) and is read for every proof (`:186`).
A root can only ever be written into a factory's index by a clone of that same factory: `DogTagIssuerFactory.registerRoot` requires `isClone[msg.sender]` (`DogTagIssuerFactory.sol:51`).

So a new verification registry pointed straight at a new factory cannot see a single root any earlier clone anchored.
Every existing credential answers `unknown root` and stops verifying, permanently.
It cannot be repaired afterwards, because the pointer cannot be rewritten and the new factory refuses any caller that is not one of its own clones.

That is the one unrecoverable step in the whole registry cutover, and this contract is what makes it recoverable.
The captain asked for the mechanism explicitly, for every future generation and not only for the pending one:

> "We do need such an upgradability and backwards compatibility feature, for the future new versions as well."

## THE PROPERTY THAT MATTERS: resolution is OLDEST GENERATION FIRST

Newest-first is the natural way to write the loop, and it is a revocation bypass.

The write-once guards are per-**contract**, not protocol-global.
`DogTagIssuer.issue` checks `issuedAt[r]` in its own storage (`DogTagIssuer.sol:53`).
`DogTagIssuerFactory.registerRoot` checks `rootIssuer[root]` in its own storage (`DogTagIssuerFactory.sol:52`).

So a root anchored and then **revoked** on a generation-1 clone can be re-anchored on a generation-2 clone by any signer whitelisted for that clone's record type.
Both guards pass, because neither generation-2 contract has ever seen that root.
The tag binding does not stop it either: the SBT is shared across generations, so `R == profileRoot(dogTagId)` still holds.

Under newest-first the router returns the fresh clone, `isValid(R)` reads true, and the revoked credential verifies again, resurrected by a provider other than the one whose credential it is.

Oldest-first closes it.
A root any earlier generation holds resolves to its **original** clone forever, so the revocation recorded there keeps answering.
A root only a later generation holds is simply absent from every earlier mapping and falls through to it, so new issuance is unaffected.

This removes no capability that exists today.
Cross-generation re-anchoring becomes inert, which is the same thing `issuedAt[r] != 0` and `rootIssuer[root] == address(0)` already enforce *within* a generation.
Oldest-first makes that write-once behaviour protocol-global instead of per-contract.

### The ordering is proved by a real attack, not by an assertion

`test_a_revoked_root_cannot_be_resurrected_by_a_later_generation` performs the attack: anchor on generation 1, revoke there, re-anchor the same root on generation 2, then assert the router still resolves the original clone and the credential still reads revoked.
It asserts the attack setup **succeeds** on chain before asserting the router's answer, so a setup that silently failed cannot make it pass vacuously.

`test_resurrection_attempt_is_refused_by_the_real_registry` drives the same attack end to end through the real `VerificationRegistryConsent` with a real Groth16 proof and the router in the immutable slot, and requires `recordVerificationZK` to revert `cred !valid`.

Both were confirmed by mutation.
Reversing the loop to newest-first turns the first into a wrong-clone failure and the second into `next call did not revert as expected`, which is the registry accepting a revoked credential and emitting `Verified`.

### Generation 2 in those tests is the real, unguarded factory

The later generation is a second instance of the production `DogTagIssuerFactory`, not a mock, because today's factory carries no cross-generation check and that is the honest model.
The router's ordering has to hold against a later generation that is unguarded, buggy or hostile.
A guarded factory there would make the attack setup revert and the test would pass while proving nothing.

## Why it does NOT revert when two generations answer

Reverting on a conflict looks like the fail-closed choice and it is a denial of service.
Anyone able to anchor in a later generation could kill an honest credential permanently by re-anchoring its root in a clone they control, and the victim has no remedy, because the router cannot be repointed.

Oldest-first is deterministic and cannot be perturbed by an attacker, and that is the property that matters here.
A duplicate is not an error condition to be surfaced; it is a claim that arrived too late to count.

Someone will propose the revert, because it reads as safer.
It is written down in the contract for that reason.

## The cross-generation write guard (defence in depth)

`isRootAnchored(bytes32)` is the hook a later generation's `registerRoot` should call, refusing any root an earlier generation already holds, so the duplicate never comes into existence at all.

It is safe for a factory that is itself in the router's list to call: `registerRoot` checks its own mapping before it writes, so the router's answer reflects only the other generations.
All reads, so there is no reentrancy surface.

Read the two mechanisms together carefully, because conflating them silently weakens the guarantee:

- **Oldest-first is load-bearing.** It holds no matter how a later factory behaves.
- **The write guard is defence in depth.** It stops the duplicate being created by a *well-behaved* factory.

A router whose safety depended on every future factory being well-behaved would rest on exactly the assumption it exists to remove.
The guard is never a substitute for the ordering.

The guard belongs to the generation-2 factory, which is slice S-7.
What S-8 owns is the hook and the proof that wiring it works, covered by a `GuardedIssuerFactory` double in the test file.

## The generation list is append-only, not immutable

The plan called for an immutable list.
It is append-only instead, for two reasons that were not available when the plan was written.

**The captain's ruling on keys.** One key for now, but the key switch must work, which requires an admin surface that actually governs something.
An owner that governs nothing is a check that runs and guards nothing.

**Appending at the tail is monotone.** A new last generation is consulted only after every existing one has answered `address(0)`, so an append can add an answer for a root that previously resolved to nothing and can never change an answer that already exists.
`test_appending_a_generation_cannot_change_an_existing_answer` pins this with a competing copy of the same root in the newest generation, and it fails under the newest-first mutation.

The other three shapes are absent by construction: no insert, no replace, no reorder, and no **removal**.
Removal is the same denial of service as the revert above, aimed at a whole generation: it would strip every credential that generation anchored of its issuer and leave the registry answering `unknown root`, with no repair.
A hostile generation is remedied by a new router, the same remedy this codebase already uses for every immutable binding.
`test_no_mutation_other_than_append_exists` scans the deployed bytecode for those selectors.

Append-only also has a consequence for the cutover order.
A frozen list forces the router to be deployed *after* its newest factory (plan step C-4 follows C-3), which makes the write guard unwireable: a factory cannot name a router that does not exist yet.
Append-only inverts it.
Deploy the router first over `[factoryV1]`, deploy factory V2 naming the router, then append it.

The owner can therefore add a factory whose clones this router will vouch for.
That is a real authority, and it is deliberately the same tier as the existing protocol admin, which can already whitelist an arbitrary signer on any record type.
It cannot reach backwards: no append changes what any already-anchored root resolves to.

### An appended address must answer both reads before it lands

`rootIssuer` makes a high-level call to **every** generation, so a single entry that reverts or returns nothing makes the whole router revert for every root.
With no removal, and with the registry's `rootIndex` immutable, an appended EOA or wrong address would brick verification protocol-wide with no repair.
The one mutable surface on this contract is therefore the one place a typo could reproduce exactly the unrecoverable failure the contract exists to prevent.

Both the constructor and `appendGeneration` therefore `staticcall` `rootIssuer(bytes32(0))` and `isClone(address(0))` and require a 32-byte answer, reverting `GenerationDoesNotAnswer(factory)` otherwise.
A `staticcall` rather than a code-size check, because it catches all three shapes at once: an EOA (succeeds with empty returndata), a contract without the function, and one that answers with the wrong width.

It cannot prove the address is an honest factory; nothing on chain can.
It only proves the call will not brick the router.
`test_append_refuses_an_address_that_cannot_answer` covers an EOA and a real live contract of this protocol that is simply not a factory, and both go red if the check is removed.

### The list is bounded, and the bound is not cosmetic

`MAX_GENERATIONS` is 8.
`rootIssuer` loops with one external call per generation and runs inside `recordVerificationZK`, a **state-changing** transaction, so every generation is charged to the relayer submitting a verification.
An unbounded append would let the owner price verification out of reach.

## Ownership: two-step handover, and renounce is closed

`Ownable2Step`, matching `DogTagIssuerFactory`, which is the contract this router most directly generalizes and whose owner holds a comparable authority.
`AccessControlDefaultAdminRules` (used by `IssuerRegistry`, `VerificationRegistryConsent` and `ProtocolRegistry`) adds a mandatory timelock, which answers a different question: time to react to a *compromised* transfer.
The captain named the failure mode as stranding, and two-step answers stranding directly with less machinery, given one key for now.

`renounceOwnership` is inherited from `Ownable`, is **not** two-step, and drops the role to `address(0)` in one transaction with no acceptance and no way back.
That is precisely the permanent stranding the two-step pattern was chosen to prevent, so it is overridden to revert `RenounceDisabled` for everyone, owner included.

`test_two_step_handover_rotates_the_key_and_a_pending_transfer_strands_nothing` performs a real rotation:

1. the old key appends a generation, establishing that it can act;
2. `transferOwnership` to a mistyped address that never accepts, after which the old key **can still append** (this is the anti-stranding arm, and the whole reason for two steps);
3. re-target and `acceptOwnership` from the new key;
4. the old key's append now reverts `OwnableUnauthorizedAccount`, and the new key's succeeds.

## What it deliberately does not carry

No `predictIssuer`.
The factory read is `predictIssuer(recordType, business)` and the answer is generation-specific: it depends on that factory's `implementation` and its own address.
A router-level version would have to pick a generation and would silently answer for the wrong one.

The only consumer today is `IssuerDomainRegistry`, which holds its own factory reference and reads it directly, so nothing regresses.
The service-domain resolver that supersedes it (slice S-9) must take a generation explicitly rather than expect this router to guess.

## Running the tests

```sh
cd contracts && forge test --match-contract CloneProvenanceRouter
```

Use `forge test`, never a bare `forge build`: a full build tries to compile the vendored OpenZeppelin submodule's `certora/harnesses/*`, which import generated files that are not present, and fails with "File not found".
That is a submodule artifact, not a project error.

To re-run the ordering mutation, reverse the loop in `rootIssuer` to `for (uint256 i = n; i > 0; i--)` over `_generations[i - 1]`, run the suite, and revert.
Three tests must go red.
