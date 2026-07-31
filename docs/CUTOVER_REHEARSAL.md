# Registry cutover: rehearsal result and step-by-step approval record

Slice S-12 of the provider-registry plan (`dogtag-regplan-p3`).

**Nothing in this slice was deployed live.**
The whole sequence was executed against `anvil --fork-url https://devrpc.roax.net` pinned at block **304000**, chain 135.

> **S-14 has since executed this sequence live on ROAX** (2026-08-01, captain-authorised, testnet only), through `contracts/script/ExecuteCutover.s.sol` - which calls the same `CutoverSequence` functions this rehearsal asserts, and which refuses the pinned block above so it cannot run on the rehearsal fork (it still permits a fork at the current head, deliberately).
> The addresses, transaction hashes and blocks are in `contracts/deployments/roax.json` under `_s14_cutover`; the operational notes are in `AGENTS.md`.
> S-14 deployed and appended only: the clients were **not** repointed (C-9/C-10) and C-6, C-7, C-10b, C-11, C-12 and the rest of C-2 were **not** performed, so §4's remaining steps all still stand.
> The statement above remains true of **this** slice - it records the rehearsal, not the live run.

A fork rather than a testnet deploy because a fork answers what the **deployed bytecode actually permits**, not what the source suggests - and that distinction produced three corrections to the plan that reading it could not have.

Reproduce with `make rehearse-cutover`.
The transaction list is `docs/CUTOVER_TRANSACTIONS.md`; it is generated, not hand-written.

---

## 1. The headline: the plan's order cannot be executed

The plan's §2 sequences **C-3 (deploy the generation-2 factory) before C-4 (deploy the provenance router)**, and specifies the router "over `[factoryV2, factoryV1]`".

Both are wrong against the contracts as merged, and they fail in opposite ways - one loudly, one silently.

**Correction 1 - the router must be deployed FIRST.**
`DogTagIssuerFactoryV2`'s `priorIndex` is `immutable`, mandatory, non-zero, and *behaviour-probed in the constructor*.
At C-3 time there is no router to pass, and passing nothing reverts `ZeroAddress`.
Passing a placeholder does not help either: an EOA staticcall succeeds with empty returndata, which the probe rejects as `NotAContract`.
So this is not two independent steps in the wrong order; the earlier one cannot execute at all.

**Correction 2 - `[factoryV2, factoryV1]` is the revocation bypass.**
The constructor stores the array as given and `rootIssuer` iterates from index 0 upward, returning the first non-zero answer.
Index 0 is consulted **first**, so that literal array makes generation 2 answer first - newest-first, which is exactly what S-8 exists to prevent.
It fails silently: nothing reverts, a revoked credential simply starts verifying again.
The corrected argument is `[factoryV1]` alone, with generation 2 appended afterwards.

**Correction 3 - there is a step the plan does not contain.**
Appending the generation-2 factory to the router (**C-4b**) is absent from the plan's sequence.
Until it runs, `DogTagIssuerFactoryV2.registerRoot` reverts `FactoryNotRegisteredInPriorIndex` and every clone the factory makes is inert.
It fails loudly, but only when someone first tries to issue - well after the deployment looked complete.

All three are **executed assertions**, not arguments:
`test_correction_1_the_generation_2_factory_cannot_be_deployed_before_the_router`,
`test_correction_2_the_plans_generation_array_resurrects_a_revoked_credential`,
`test_correction_3_c2_cannot_attach_an_ownerless_generation_1_clone`.

### The corrected on-chain order

| plan | corrected | why |
|---|---|---|
| C-1 | **C-1** deploy `ProviderRegistry` | unchanged |
| C-3 | **C-3a** deploy `DogTagIssuerV2` impl | split out; the implementation has no dependencies |
| C-4 | **C-4** deploy `CloneProvenanceRouter([factoryV1])` | **moved before the factory**, and the array corrected |
| C-3 | **C-3b** deploy `DogTagIssuerFactoryV2(impl, core, router)` | **moved after the router** |
| - | **C-4b** `router.appendGeneration(factoryV2)` | **new step** |
| C-2 | **C-2** `core.addFactoryGeneration(GEN2_ID, factoryV2)` | chain half only - see §4 |
| C-5 | **C-5** deploy `VerificationRegistryConsent` V2 | unchanged |
| C-6 | **C-6** migrate `VERIFY:` relayer capabilities | unchanged |
| C-8 | **C-8** deploy `ProtocolRegistryV2` | unchanged |

---

## 2. A second, independent finding: C-2 is not executable as written

The plan's C-2 says "register providers, **attach the existing clones**, mirror today's capability grants."

Attaching the existing clones is impossible against the merged `ProviderRegistry`.
`attachService` reads `owner()` off the service and refuses a failed or zero answer; a generation-1 `DogTagIssuer` **has no owner at all** - its stored identity is `registry`/`rootIndex`/`recordType`/`name`, set once in `initialize` with no setter.
The five live clones therefore cannot be attached, and the call reverts `InvalidServiceMetadata`.

This is consistent with the plan's own §4 item 9, which already recommends retiring all five and re-issuing - but §2 was never updated to match, so C-2 reads as executable and is not.

**C-2 must be restated as: record the generation-2 factory generation, register providers, and mirror capability grants. Generation-1 clones stay unattached** and keep issuing through the generation-1 `IssuerRegistry`, which the plan already requires for an unrelated reason (a clone pins its registry at `initialize`).

A third blocker surfaced only by executing it: `registerProvider` refuses a zero identity digest, schema or hash algorithm (`BadIdentityAnchor`).
A provider cannot be registered at all without the registrar-controlled public identity statement of §4 item 6.

---

## 3. The seven assertions

Each corresponds to a distinct way the cutover fails **silently**. All executed against the fork; all pass. Assertion 7 was added at review: C-12 had been *described* as exercised in three places while `delistFor` was declared and never called, and a claim broader than the code is the defect this project removes everywhere.

| # | assertion | test | what a silent failure would look like |
|---|---|---|---|
| 1 | All 19 historical roots resolve through the router to their original clone | `test_1_every_historical_root_resolves_through_the_router` | every existing credential answers `unknown root`, permanently and unrepairably |
| 2 | A generation-1 credential verifies on the generation-2 registry | `test_2_...` | the historical credentials stop verifying the moment clients repoint |
| 3 | A generation-2 credential verifies on the generation-2 registry | `test_3_...` | new issuance produces credentials nothing can verify |
| 4 | A generation-2 credential does **not** verify on the generation-1 registry | `test_4_...` | the C-9-before-C-11 ordering constraint would look optional |
| 5 | An unmigrated relayer fails `!verify-wl` | `test_5_...` | the C-6-before-C-9 ordering constraint would look optional |
| 6 | A revoked generation-1 root cannot be resurrected in generation 2 | `test_6a_...`, `test_6b_...` | a revoked credential starts verifying again, with nothing reverting |
| 7 | The generation-1 freeze (C-12) stops new issuance and breaks **no** existing credential | `test_7_...` | either the old registry's `WHITELIST_ADMIN` stays a live trust surface for the new verifier, or the freeze silently invalidates all 19 historical credentials |

Three things about how they are asserted are load-bearing:

**Assertion 1 compares against the chain, not against a transcribed list.**
Each root is checked as `router.rootIssuer(r) == factoryV1.rootIssuer(r)`, so the expected clone is read from the generation-1 factory itself.
The root list is derived, not typed: `scripts/derive-cutover-inventory.sh` regenerates it from `RootRegistered` logs at the pinned block, and the test refuses to run if the fixture's pinned block disagrees with the fork's.
That derivation walks the block range in bounded chunks and cross-checks the result against one full-range query, because an endpoint that caps a log range and **truncates instead of erroring** would otherwise be undetectable: `rootCount` is computed from the same short list, so the test's own count assertion is self-consistent by construction and the rehearsal reports green over fewer roots than the chain holds.
A disagreement between the two scans is a refusal to write the fixture; an endpoint that cannot answer the full range at all says so rather than passing in silence.
The count was **re-derived as 19**, matching the plan's figure independently rather than carrying it forward.

**Assertion 4 needs a precondition or it passes for the wrong reason.**
`!verify-wl` fires at `VerificationRegistryConsent.sol:153`, *before* root resolution at `:187`.
So an unwhitelisted relayer reverts for a reason that has nothing to do with root resolution, and a bare `vm.expectRevert()` would go green while proving nothing.
The test therefore whitelists the relayer on generation 1 first, asserts that precondition explicitly, and requires the exact string `"unknown root"`.

**Assertion 6 is two tests, because the two halves are not the same claim.**
`6a` is *defence in depth*: the guarded generation-2 factory asks the router `isRootAnchored` and refuses, so the duplicate never exists.
`6b` is *load-bearing*: oldest-first holds even against a later generation that is unguarded, buggy or hostile.
`6b` uses a real, unmodified generation-1 `DogTagIssuerFactory` as the hostile later generation, against an `IssuerRegistry` the test admins - because a guarded factory would make the attack setup revert and the test would pass while proving nothing.

### Every assertion was proved able to fail

`make rehearse-cutover-mutations` applies one deliberate break at a time and **requires the corresponding test to go red**; a mutation whose test stays green is reported as a failure of the harness.
It refuses to start on a dirty tree and restores every mutated file through a trap, including on interrupt - and it verifies that restore against the committed tree afterwards, because an unchecked copy-back would leave a weakened contract source with `git status` as the only detection.

**RED means a test actually ran and failed, never merely that `forge` exited non-zero**, and that distinction is the difference between this table being evidence and being decoration.
`forge test` exits non-zero for reasons that have nothing to do with an assertion catching anything - a compile error above all, which is the likely outcome of a mutation that only partly applied.
Crediting that as red would record a mutation that **never ran** as proof its assertion pins something: the exact inverse of what the harness is for, and it would hollow out all ten rows at once while the summary still printed `10 of 10`.
The three cases were measured on Foundry 1.5.1 rather than assumed, because the classifier had to be tightened correctly rather than merely made harder to satisfy:

| what happened | exit | `Ran N test … for` | `[FAIL` line | verdict |
|---|---|---|---|---|
| `setUp()` aborted | 1 | present | present, naming `setUp()` | **RED** - it compiled, ran and failed |
| `--match-test` matched nothing | **0** | absent | absent | setup failure |
| the mutated source did not compile | 1 | absent | absent | setup failure |

So a run that executed no test is a setup failure *whatever its exit code*, and every replacement inside a mutation must apply or the mutation aborts before writing a source that would not compile.
The same did-anything-actually-run check guards the rehearsal's own assertion step in `scripts/rehearse-cutover.sh`, where a `--match-path` matching nothing would otherwise report seven assertions green having executed none.

| mutation | assertion that must redden | observed |
|---|---|---|
| router built without generation 1 | 1 | `router lost a historical root's issuing clone` |
| registry `rootIndex` bound to the generation-2 factory instead of the router | 2 | `unknown root` |
| C-4b (`appendGeneration`) omitted | 3 | `FactoryNotRegisteredInPriorIndex` |
| generation-1 relayer precondition dropped | 4 | `precondition: relayer must be authorised on generation 1` |
| relayer restriction defaults off | 5 | `next call did not revert as expected` |
| cross-generation write guard removed | 6a | `next call did not revert as expected` |
| router resolution reversed to newest-first | 6b | `router did not resolve oldest generation first` |
| core given a legacy-controller adapter | correction 3 | `next call did not revert as expected` |
| the C-12 delisting is not performed | 7 (issuance half) | `next call did not revert as expected` |
| the freeze also revokes the anchored root | 7 (history half) | `cred !valid` |

10 of 10 redden.
Those ten results were **re-measured under the stricter criterion** shown above - the one that requires a test to have actually RUN and a real `[FAIL` line, so a compile error or a zero-match filter is classified as a setup failure instead of being credited as red.
All ten still redden. That distinction matters: under the previous criterion any non-zero `forge` exit counted, so the figure was not established at all, and the earlier revision of this document correctly declined to claim it until someone measured.
It is measured now, at `fc17251`, against a live fork.

The harness earned its keep **twice**, both times catching an inert mutation of mine rather than being believed:

- The legacy-adapter mutation was first written as a one-line change and **stayed green**, because `_readServiceMetadata` folds the failed-owner read into a single `metadataOk` and the downstream guard it relaxed was already unreachable.
- Assertion 7 was first paired with a mutation of `src/DogTagIssuer.sol`'s `isValid`, which also **stayed green** - and for a reason worth keeping in mind for any fork rehearsal: **that assertion verifies through the REAL DEPLOYED clone and registry, so no edit to this tree can reach that bytecode.** On a fork the only mutable surface is what the rehearsal itself deploys or does. Both replacements therefore target the freeze the test performs, one per claim.

An inert mutation that "confirms" an assertion is exactly the trap this fleet has been bitten by.

---

## 4. Approving the cutover step by step

For each step: what it does, what it makes irreversible, and what would have to be true to roll it back.

**Deployments need no authority. Only C-4b, C-2, C-6, C-10b, C-11 and C-12 require the governance key.**
That split is what makes the list approvable one step at a time - six of the eight rehearsed transactions grant the deployer nothing and can be sent by anyone.

### C-1 - deploy `ProviderRegistry`
Deploys the authority core, empty, holding authority over nothing.
**Irreversible:** no. Nothing reads it until C-11 and a core with no grants authorizes nobody.
**Rollback:** abandon the address.

### C-3a - deploy `DogTagIssuerV2` implementation
**Irreversible:** no. Inert until a factory names it. Identified by code hash, so who deploys it is irrelevant.
**Rollback:** abandon the address.

### C-4 - deploy `CloneProvenanceRouter([factoryV1], governance)`
**Precedes C-3b and C-5 absolutely.**
**Irreversible:** not by itself, but this is the **last moment the decision can be made**, because C-5's `rootIndex` is immutable.
**Rollback:** free before C-5. After C-5 it requires redeploying the verification registry, which repeats every step after it.
**Verify before proceeding:** all 19 historical roots resolve (assertion 1), and `generations()` reads `[factoryV1]` - *not* `[factoryV2, factoryV1]`.

### C-3b - deploy `DogTagIssuerFactoryV2(impl, core, router)`
**Irreversible:** no, until a clone anchors a root through it. Every dependency is immutable, so a wrong argument means a new factory.
**Rollback:** free before C-4b.

### C-4b - `router.appendGeneration(factoryV2)` **[GOVERNANCE]**
**Irreversible: YES.** The router has no removal, no replace and no reorder - deliberately, because removal is a denial of service aimed at a whole generation.
**Rollback:** a new router, and therefore a new verification registry, and therefore every downstream step again.
**Do not send this until C-3b's factory address is the one you intend to keep.**

### C-2 - `core.addFactoryGeneration(GEN2_ID, factoryV2)` **[GOVERNANCE]**
**Irreversible:** partially. A generation can be added and deprecated but never repointed or removed, and `deprecateFactoryGeneration` is terminal with no reactivation path.
**Rollback:** deprecation only, which is itself irreversible.
**Blocked:** the rest of C-2 (providers, capability grants) needs the KYC inputs of plan §4, and attaching generation-1 clones is not possible at all - see §2 above.

### C-5 - deploy `VerificationRegistryConsent` V2
Constructor: core, **existing** SBT, **existing** verifier, the router, governance.
**Irreversible: the bindings are.** `issuerRegistry`, `sbt` and `rootIndex` are all immutable.
**Rollback:** deploying it changes nothing on its own (no client reads it until C-9), so rolling back before C-9 is free. A wrong *argument* is not rollback-able - it is a redeploy.
**Two do-not-move arguments:** the SBT must be reused because `profileRoot` is per-contract and write-once, so a fresh SBT has no root for any existing tag; the verifier must be reused because the ceremony VK is frozen.

### C-6 - migrate `VERIFY:` relayer capabilities **[GOVERNANCE]**
One transaction per (purpose, relayer).
**No transaction for this step appears in `docs/CUTOVER_TRANSACTIONS.md`**, and that is deliberate: which relayer serves which provider for which purpose is plan §4 item 5, a KYC fact rather than a chain one, and roughly 7 of the 33 live grants sit on this axis. Publishing a list with invented relayer addresses would read as ready. The step is nonetheless *exercised* on the fork - **assertion 5 is precisely this step being withheld and then applied on one credential**, which is what establishes the ordering constraint below.
**Precedes C-9**, because the relayer gate fires before root resolution and a client repointed ahead of its relayer reverts `!verify-wl` on every verification - an error naming a whitelist rather than a migration.
**Irreversible:** no. `setVerifierCapability(..., false)` withdraws it in one transaction.

### C-8 - deploy `ProtocolRegistryV2` with a non-zero publish timelock
**Irreversible: the timelock is.** It is immutable, and this deployment is the only opportunity to fix the live registry's `PUBLISH_TIMELOCK == 0` (re-confirmed on the fork).
The contract enforces a 1-hour floor in its constructor, so zero is unrepresentable - **but the floor is not the target**. Use `DEFAULT_PUBLISH_TIMELOCK` (2 days). The rehearsal uses the floor only so a rehearsal can be walked end to end in one sitting.
**Rollback:** redeploy and repoint every client carrying the discovery address, including two compile-time mobile bundles.

### C-9 / C-10 / C-10b - clients, then apps, then SBT roles
No transactions in the rehearsed list; C-10b is one `grantRole` per generation-2 signer.
**C-10b is easy to forget precisely because the SBT is the one contract that does not move**: minting is gated on the SBT's own `ISSUER_ROLE`, not on the registry, so a generation-2 clone whose signer can `issue(R)` still cannot `mintCustodial`. The rehearsal exercises this (it grants the role in `setUp` and the tag binding then holds).
**Irreversible:** no.

### C-11 - enable generation-2 issuance **[GOVERNANCE]**
**The first moment a credential exists that a generation-1-only client cannot verify** - which is why every client step precedes it (assertion 4).
**Irreversible:** the grant is not, but any credential issued in the window is: a root anchored in generation 2 stays there.
**Rollback:** withdraw the grant; already-issued credentials keep resolving through the router.

### C-12 - freeze generation-1 issuance **[GOVERNANCE]**
**Irreversible:** no - re-whitelisting restores it.
**Precondition the router's safety rests on:** the freeze only works if generation 2 is bound to a *different* `IssuerRegistry`. Under one shared registry a single whitelist entry authorizes `issue` on clones of both generations, so no delisting freezes generation 1 without also stopping generation 2. The plan already satisfies this at C-5; an operator who reaches C-12 having wired both to one registry must otherwise choose between breaking new issuance and skipping the freeze - and skipping it leaves the router's one open direction (a later-generation root re-anchored in an *earlier* generation) live in production.

---

## 5. What the rehearsal deliberately does not claim

- **It does not claim C-2 is ready.** The provider id, controller, signer and identity anchor it uses are synthesised, labelled as rehearsal inputs in the test, and are exactly the KYC facts plan §4 lists as blocking. A clean run must not be read as "C-2 works".
- **The rendered addresses are fork addresses.** They derive from the governance nonce at the pinned block and will differ live. The transaction list is authoritative about order, signer and calldata - never about where a contract lands.
- **C-7 (`ServiceDomainResolver`, `ProviderDirectory`) is not in the list.** They have slack and gate nothing else - the plan's own reason - and they are built but not deployed.
- **C-6, C-10b, C-11 and C-12 take one transaction per relayer or signer**, and those sets are the same KYC reconciliation, so no such list is published - a list with invented addresses would read as ready. Each one's mechanics *are* exercised on the fork, and named individually rather than in aggregate: C-6 is assertion 5, C-10b is the SBT `grantRole` in `setUp`, C-11 is `setIssuanceCapability` in the generation-2 anchoring helper, and C-12 is assertion 7. `docs/CUTOVER_TRANSACTIONS.md` names every excluded step, so the rendered list cannot be mistaken for the whole cutover.
- **The governance concentration in plan §6 is unchanged by any of this.** One EOA still holds every admin role, the publisher role and factory ownership. C-8's timelock helps only if the key it delays is not a single key that can be lost or copied.

---

## 6. Reproducing

```bash
git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts
make rehearse-cutover              # fork, assert, broadcast, verify, render
make rehearse-cutover-mutations    # prove every assertion can fail
```

`scripts/rehearse-cutover.sh` records anvil's PID and kills **that PID and nothing else** - never by name or path, because many checkouts of this monorepo run their own.
It refuses to start if the port is already in use rather than assuming a listener is its own.

Two ordering details inside it that are not arbitrary:

- **The assertions fork upstream, not the local anvil, and run *before* the broadcast.** The broadcast mutates that anvil - the issuer implementation lands on exactly the address the test's own `new DogTagIssuerV2()` would take - so asserting against it afterwards asks whether the cutover works on a chain where the cutover already happened, and fails as a confusing `ImplementationCodeMismatch` rather than as anything about the cutover.
- **`--skip-simulation` is required, and is not a way of ignoring a failure.** Observed on Foundry 1.5.1: forge runs the script twice. The simulation attributes every CREATE to `--sender` - its addresses are exactly `governance@nonce`, confirmed with `cast compute-address` - while the second, on-chain re-execution produces different CREATE addresses and hands the factory the simulation's implementation address, which holds no code there, so it reverts `ImplementationCodeMismatch` although the transactions themselves are valid. The attribution rule for that second phase was **not** established (the divergent address is neither a governance-nonce address nor the script contract's first few), so this is the divergence that was measured rather than a mechanism. Skipping the redundant re-execution broadcasts the correctly-attributed list. Because that removes a check, the wrapper verifies the receipts and the resulting state instead - every status is `0x1`, `rootIndex` really is the router, **C-4b really took effect** (the router recognises the generation-2 factory, holds exactly two generations, and has generation 1 at index 0 with generation 2 at the tail), and all 19 roots really do resolve through the broadcast router. C-4b is checked by EFFECT rather than by receipt because it is the step the plan omits entirely and the one whose omission is fatal and silent; leaving it on a receipt alone would be the strangest gap to keep once `--skip-simulation` has already removed a check. The success banner is not taken as evidence of anything.
- **The rendered transaction list refuses to exist when it would be false.** `scripts/render-cutover-txlist.py` opens by asserting every transaction succeeded, so it will not write the file at all if any receipt is not `0x1` - previously it wrote that claim and appended a contradicting failure line beneath it. It also refuses a broadcast that is missing any step `STEPS` knows about, not just one carrying a step it does not. A stale-but-true committed list beats a fresh-but-false one, and this document exists for the captain to approve a live cutover from.
