# ProtocolRegistry: the discovery layer, and the timelock

The discovery anchor: the dogtag-governed record of which contracts and which proving artifacts are current.
DEPLOYED BY S-14, AND UNWIRED.
Cutover step C-8 ran live on ROAX, so `contracts/deployments/roax.json` carries a `ProtocolRegistry` key - see `_s14_cutover` there for the address, transaction hash and block, which are deliberately not copied into this file.
Nothing has been published on it and no `.env.example` entry or client points at it; repointing is C-9 and is separately captain-authorized.
Its `PUBLISH_TIMELOCK` was deployed at the contract's 1-hour floor with the testnet opt-in stated aloud, and it is IMMUTABLE - the reasoning is in `_s14_cutover`. **Mainnet must use exactly 2 days.**

Generation 1's deployed `ProtocolRegistry` (`0xf5492A67…`, see `docs/PROTOCOL_REGISTRY_RUNBOOK.md`) is untouched and stays live until the cutover repoints clients.

## Why a new registry is forced, not chosen

`ProtocolRegistry.ContractSet` (`contracts/src/ProtocolRegistry.sol:97-106`) is a fixed struct holding exactly `contractSetId`, `factory`, `verificationRegistry`, `sbt`, `verifier`, `circuitId`, `publishedAt` and `active`.
It has no member for a provider-authority core, for the resolver layer, or for a cross-generation provenance index.
The contract is not upgradeable: there is no proxy and no setter that could widen a struct, and a struct's shape is part of both its storage layout and its ABI.
So generation 2 cannot be published into it at all.

State it that way rather than as a preference.
The decisions this slice actually makes are what the new record holds and what delay protects it.
That there must be a new record is settled by the struct.

## The record

`ProtocolRegistry.DiscoverySet` is generation 1's eight members plus two, in this order:

| # | member | what it is |
|---|---|---|
| 0 | `discoverySetId` | `keccak256("dogtag-levelb/2")`, the map key |
| 1 | `factory` | the generation-2 `DogTagIssuerFactory`: where a provider's clone comes from |
| 2 | `verificationRegistry` | the registry a proof is submitted to, and the anti-redirect anchor |
| 3 | `sbt` | the SHARED, reused `DogTagSBTConsent` |
| 4 | `verifier` | the existing `Groth16VerifierConsent`: the frozen ceremony VK's on-chain identity |
| 5 | `providerRegistry` | the `ProviderRegistry` authority core, and the root of the resolver layer |
| 6 | `rootIndex` | whatever occupies the registry's immutable `rootIndex` slot: the `CloneProvenanceRouter` |
| 7 | `circuitId` | `keccak256("consent.circom/DogTagConsent(6)")`, unchanged |
| 8 | `publishedAt` | stamped at execute, never taken from calldata |
| 9 | `active` | flipped false by `deprecateDiscoverySet`, never deleted |

Every address member is required non-zero.
A zero would not be a smaller record, it would be a published claim that some component sits at `address(0)`, which a consumer would `staticcall` and read as empty returndata: neither a definite yes nor a definite no.
A generation that genuinely has no such component publishes no set here.

Both new members are on this axis rather than a separate one for the same structural reason: `VerificationRegistryConsent` pins them in immutable slots (`contracts/src/VerificationRegistryConsent.sol:86-88`), so neither can move without moving the verification registry, which is the definition of an on-chain-axis rotation.

### `factory` is no longer the root index, and that is the easiest thing to get wrong

Generation 1 documented `factory == verificationRegistry.rootIndex()`.
In generation 2 they are different contracts: the factory is where a clone comes from, the root index is what resolves an anchored root across generations.
A consumer that reads a factory address where the root index is meant resolves only the roots anchored in that generation and silently misses every earlier one.
That is exactly the failure the provenance router exists to prevent, so the two are carried as separate members and the publish script asserts the difference against the chain.

### There is deliberately NO `providerDirectory` or `serviceDomainResolver` member

The registry plan's S-11 asks for one address each.
Against the merged S-6 source that would be a published falsehood, and this is a correction to the plan rather than an omission.

`ProviderRegistry` already owns the resolver layer: `setResolverApproved(kind, resolver, approved)` allowlists MANY resolvers per `ResolverKind.DIRECTORY` / `ResolverKind.DOMAIN`, and each provider selects its own directory resolver (`setDirectoryResolver(providerId, resolver)`) while each service selects its own domain resolver (`setDomainResolver(serviceAddress, resolver)`).
A protocol-wide address for either would be read as authoritative and would be the wrong resolver for every provider that selected another.
The core is the resolution root, so `providerRegistry` is how a consumer reaches it.

A consumer computing an effective resolver must additionally re-check `isResolverApproved(kind, resolver)`, because the core keeps a deapproved selection as history.

**Precondition to confirm before C-8, because this omission is an inference and not a reading.**
The argument above is drawn from the MERGED S-6 `ProviderRegistry` source.
S-9 (service-domain-resolver) and S-10 (provider-directory) were still in flight when this slice was written, so their source could not be read from here, and the omission assumes neither introduces a single canonical protocol-wide resolver address intended for publication in the discovery record.
Whoever merges those two slices must confirm that assumption against their source before the cutover.
It matters because adding a member afterwards is not free: it is a swap-republish that restamps `publishedAt` and re-emits `DiscoverySetPublished` for a change that moves no address, the exact trap AGENTS.md records for the generation-1 script.
If either slice does publish one canonical address, the member belongs in the record and the decision above must be revisited rather than worked around.

## The timelock, which is the whole reason this deployment is the only chance

The live generation-1 registry carries `PUBLISH_TIMELOCK == 0` and the value is `immutable`.
Propose and execute can land in the same block there, so its publisher key can repoint the entire declared protocol set in one transaction with no window for anyone to notice, which is the one property a timelock exists to provide.
No repair exists short of a redeploy, and this is the redeploy.

A zero is therefore not discouraged here, it is unrepresentable.
Two guards, at different layers, and neither is redundant:

- The CONSTRUCTOR refuses anything below `MIN_PUBLISH_TIMELOCK` and reverts `PublishTimelockBelowFloor(given, floor)`.
  That is what makes a zero-delay registry impossible even for someone deploying the contract directly, without the script.
- `DeployProtocolRegistry.s.sol` additionally requires exactly the 2-day production default unless `PUBLISH_TESTNET_DEPLOY=true` is stated aloud.
  A testnet may go shorter, but not below the contract's floor and never to zero.
  This is the deliberate divergence from generation 1's deploy script, whose testnet opt-in accepts zero (it has a passing test named `test_explicit_testnet_opt_in_accepts_zero_timelock`).

### Why the floor is one hour

Derived, not chosen.
A delay is only a review window if a watcher can see the proposal inside it, and the oversight indexer that surfaces governance events is finality-aware: on ROAX the `finalized` tag sits about 80 blocks behind `latest`, so a proposal is not authoritatively visible for minutes, before any human looks.
A floor below that would be a timelock that exists only in the getter, and a guard satisfiable vacuously is worse than none because it reads as protection.
One hour clears finality plus a reaction with room to spare, and is still short enough to rehearse a whole cutover in one sitting.

Production still uses the 2-day `DEFAULT_PUBLISH_TIMELOCK`.
The floor is the boundary a testnet may descend to, not a new default.

## The renamed record is a structural guard, not cosmetics

Generation 1's `getContractSet(bytes32)` returns an 8-word static tuple.
This record is 10 words.
Had the getter kept its name, a generation-1 client pointed at this registry would DISPATCH successfully (the selector is a function of the name and arguments, not the return type) and decode the first 8 words as its own struct: reading `providerRegistry` where `circuitId` belongs and `publishedAt` where `active` belongs.
That decodes as a plausible live record, with an `active` that is a block timestamp (truthy) and a circuit id that is an address.

It is the same identical-shape/different-semantics trap this codebase has already paid for twice: the two `recordVerificationZK` arities sharing selector `0xdd080593`, and the stale hard-coded `isValid` selector that reverted on every deployed clone.

So the record that changed shape is renamed.
`DiscoverySet`, read through `getDiscoverySet` / `resolveDiscovery` / `getPendingDiscoverySet`, and a generation-1 client reverts instead of misdecoding, without depending on every client remembering to check the return width.

`ArtifactSet` is byte-for-byte generation 1's, so `getArtifactSet` and `getActiveArtifactSet` deliberately KEEP their names and selectors.
No misdecode is possible there, and an artifact decoder written for generation 1 stays correct.

Defence in depth on the client side: both mobile decoders now require the arity EXACTLY rather than as a lower bound, and each refuses the other generation's return.
The generation-1 decoders previously accepted `>= 8` words, which is precisely how the misdecode above would have happened.

## Which keys move, and which do not

- The discovery key BUMPS to `dogtag-levelb/2`.
  Generation 2 rotates the factory, the verification registry, the authority core and the root index, and the two generations must be tellable apart by a reader looking at a claim or at the registry.
- The artifact key stays `dogtag-levelb-artifacts/1`.
  This is a correctness choice, for two reasons in order of weight.
  First, it is the truth: the circuit, the ceremony, the frozen VK and all four artifact pins are byte-for-byte unchanged by the provider-registry work, and minting `…-artifacts/2` for identical bytes would publish a second identity for one artifact set, which this codebase's own rule forbids.
  Second, it keeps the app-gate diagnostic actionable: both mobile anchor resolvers carry the artifact identity as a compile-time constant and `validate` refuses an anchor whose `artifactSet` does not hash to its `artifactSetId`, so a moved artifact key would make an old build fail with a caller-integrity error about a stitched anchor, which is true and useless to the holder.
  With the key unchanged, an old build reaches the check designed to speak to this case and is refused with `AppTooOld`, naming the floor.
- The circuit id does not move, because no part of this work touches the circuit.
- `minAppVersion` therefore differs between the two registries for the same artifact id, and it should: the floor is a statement about which app BUILD may act on a generation, not part of the artifacts' identity.
  It is a mandatory publish input with NO default, because the generation-2 floor is the release that reads the root index instead of the bundled factory, and that release number is not knowable from source.
  Publishing a guessed number would publish a floor nobody verified.

## Publishing

Two phases, mirroring generation 1.
The discovery set, the artifact set and the binding are three timelocked writes whose windows run CONCURRENTLY, so phase 1 proposes all three.

```
# Phase 1 - preflight, then propose (starts the timelock on all three)
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY

# Phase 2 - after the printed ETAs, execute sets first, then the binding
forge script contracts/script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
  --rpc-url $ROAX_RPC --broadcast --legacy --private-key $PUBLISHER_KEY
```

Every input is a mandatory environment variable under the `PUBLISH_` namespace, with no stale-network fallbacks: `PUBLISH_PROTOCOL_REGISTRY`, `PUBLISH_FACTORY`, `PUBLISH_VERIFICATION_REGISTRY`, `PUBLISH_SBT`, `PUBLISH_VERIFIER`, `PUBLISH_PROVIDER_REGISTRY`, `PUBLISH_ROOT_INDEX`, `PUBLISH_ZKEY_SHA256`, `PUBLISH_WITNESS_MOBILE_SHA256`, `PUBLISH_R1CS_SHA256`, `PUBLISH_WASM_SHA256`, `PUBLISH_ARTIFACTS_URL`, `PUBLISH_MIN_APP_VERSION`.
The deploy script takes `PUBLISH_ADMIN` (mandatory, no default), `PUBLISH_PUBLISHER`, `PUBLISH_PUBLISH_TIMELOCK_SECS` and `PUBLISH_TESTNET_DEPLOY`.

**Phase 2 needs the SAME environment phase 1 had, not just the registry address.**
It used to read only `PUBLISH_PROTOCOL_REGISTRY`.
It now reads every publish variable in that list: the six addresses feed the re-preflight, and all thirteen feed the two staged-versus-environment checks that confirm the bytes about to be activated are still the bytes this environment describes.
On mainnet phase 2 runs two days after phase 1, plausibly in a different shell or a different operator's session, so load the same `.env` rather than exporting the registry address alone.
A missing variable reverts inside `vm.envAddress` / `vm.envBytes32` / `vm.envString` before anything is broadcast, so it fails safe, but the message names only the variable and says nothing about the two-phase procedure.

`PUBLISHER_ROLE` is `keccak256("PUBLISHER")`, not `keccak256("PUBLISHER_ROLE")`.
Read it off the deployed contract rather than recomputing it from the variable name.

### This pair is FIRST-ROLLOUT only

`PublishProtocolVersionsPropose` stages all three writes unconditionally, so the two phases send six transactions between them.
Reaching for it to move ONE axis re-executes the discovery set: `executeDiscoverySet` assigns the record unconditionally, so it restamps `publishedAt` and re-emits `DiscoverySetPublished` for a change that moves no address, rewriting the generation's on-chain provenance and destroying the previous `publishedAt` with nothing recording it.
The same trap is recorded for generation 1's script in AGENTS.md.

A later artifact-only rotation is `proposeArtifactSet` plus `proposeArtifactBinding` and their two executes, with no `DiscoverySet` write at all.
`contracts/script/PinConsentWitnessGraph.s.sol` is the narrow single-axis shape to copy.

### Why phase 2 re-runs the preflight

The preflight is a snapshot of relations that held when the record was staged, and the record is not activated until a whole timelock later.
Four of the five relations cannot move in between: `issuerRegistry`, `sbt` and `rootIndex` are `immutable` on `VerificationRegistryConsent`, and the router's `isGeneration` is append-only monotone.
The single drifting relation is `verifier == verificationRegistry.zkVerifier()`, because `zkVerifier` is that registry's one mutable member and is swappable behind `ZK_TIMELOCK = 2 days` - the same length as the mainnet publish timelock, so a swap proposed shortly before a publish executes squarely inside the publish window.
Without the re-check, `executeDiscoverySet` would write a verifier the registry no longer uses and publish it as dogtag-certified.

Phase 2 additionally asserts that the staged bytes still equal the record this environment describes, because `executeDiscoverySet` writes the staged bytes and never reads the environment.
The remedy after either refusal is a fresh propose, and therefore a fresh timelock - never editing the environment to agree with the chain, which would leave the retired verifier staged while the preflight reported everything in order.

### Why the `PUBLISH_` namespace, rather than reusing generation 1's names

Two reasons, and the first is an operator hazard rather than tidiness.

Generation 1's publish script reads `ROOT_INDEX`, and on that generation the root index IS the factory.
So an operator with a generation-1 `.env` loaded would publish generation 2 with a factory address in its root-index slot, which is exactly the mistake the preflight exists to catch.
Sharing a name whose generation-1 value is wrong here makes that mistake likelier, not less likely.
The same applies to `SBT_CONSENT_ADDR` and `CONSENT_VERIFIER`: generation 2 reuses those contracts, but "reuses" is a fact to state and have checked, not one to inherit silently from an environment.
And generation 1's ROAX deployment sets `PUBLISH_TIMELOCK_SECS=0` with `TESTNET_DEPLOY=true`, which must not be able to reach this deployment's timelock decision at all.

Second, `vm.setEnv` mutates the PROCESS environment and `forge test` runs test contracts in parallel, so two suites driving two scripts through the same variable names interfere.
The generation-1 deploy test sets `ROOT_INDEX` to its own factory, and the generation-2 publish script read it mid-test.
That produced a genuine flake, the preflight failing on `rootIndex` in roughly three of five full-suite runs, which the namespace removes by construction rather than by timing.

`PUBLISH_ADMIN` additionally has NO default, unlike generation 1's `ADMIN`, which defaults to the current governance EOA.
That EOA has no contract code and already holds `DEFAULT_ADMIN_ROLE` on four contracts plus `WHITELIST_ADMIN` and factory ownership.
Inheriting it by default would quietly reproduce that concentration on the one deployment whose purpose is to harden the pointer, so the script refuses to choose the holder silently.

### The coherence preflight

A discovery record is a set of addresses an operator transcribes, and a mis-transcribed one is the unrecoverable failure this generation is being deployed to avoid.
Nothing about either mistake looks wrong at publish time.

`PublishV2Base.preflight` reads the deployed contracts back and refuses to propose on any disagreement:

- `rootIndex == verificationRegistry.rootIndex()`
- `sbt == verificationRegistry.sbt()`
- `providerRegistry == verificationRegistry.issuerRegistry()`
- `verifier == verificationRegistry.zkVerifier()`
- `rootIndex.isGeneration(factory)`, because the factory is not reachable from the registry in generation 2, and generation membership is the property that makes it a factory whose roots that root index can resolve

The preflight lives in the SCRIPT rather than in the registry, following `DeployIssuerDomainRegistry.s.sol`, which preflights `factory.registry()` the same way.
The registry stores data and asserts nothing about the semantics of what it stores; binding it to one verification registry's ABI would mean a later generation with a differently-shaped registry could not be published at all.

It deliberately does NOT check that the bound artifacts prove against `verifier`.
Pins are byte-integrity and the verifier is a VK identity, so no on-chain read can relate them.
That is a governance judgement, which is why the binding is timelocked rather than validated.

## Client changes in this slice, and what is deliberately deferred

Nothing is deployed, so NO address moves.
Both mobile bundles (`apps/*/roax.json`) keep their generation-1 `ProtocolRegistry` address, and the deployment ledger is untouched.
Adding a `ProtocolRegistry` key with a placeholder would be exactly the invented data this fleet forbids.

What changed:

- `crates/dogtag-standard-rs/src/discovery.rs`: `TrustedAnchor` and `ValidatedVersion` gained `provider_registry` and `root_index`, both `Option<String>`.
  `None` is the honest shape of a generation-1 record, whose struct has no such member.
  Read that precisely: it is an accurate observation about the record's shape, and it is NOT a could-not-check, because a read that FAILED must surface as a failed resolution and must never reach `validate` as a `None`.
  A value the caller DID report must be usable (`0x` plus 40 hex digits, not the zero address) or `validate` fails closed with `MalformedAnchorAddress`, naming the member.
  Shape is the only check available for these two, because nothing claims them: a garbled `verification_registry` fails its comparison against the platform's claim, and there is no equivalent comparison here.
- `crates/dogtag-prover-rs/src/manifest.rs`: `VersionDeployment`, `Manifest` and `OnchainContractSet` gained the same two, and `reconcile` compares them like any other mirrored address.
  A present-versus-absent disagreement in either direction is a recorded conflict, which is what makes the two generations' manifests non-interchangeable.
  The `Manifest` members are `skip_serializing_if`, so a generation-1 manifest serializes to exactly the bytes it did before they existed and any signature already produced over it still verifies.
- `stacks/vet/api/src/discovery.rs`: the manifest-to-anchor mapping carries both through.
  Never substitute `factory` for `root_index` here.
- Both mobile `AnchorResolver`s: an exact-arity guard on the generation-1 decoder, a new `decodeDiscoverySet` for the 10-word record, and the `dogtag-levelb/2` constant.
  Both are pinned against a golden ABI encoding that `contracts/test/ProtocolRegistry.t.sol` asserts from the other end, so a change to the record's shape fails in Solidity first and names the two files to regenerate.
- Both UniFFI bindings were regenerated, which is mandatory after any FFI record change: Android CI bundles the committed `.kt` as-is and never regenerates it.

Deferred, deliberately:

- The mobile `RoaxRpc` / `Net.swift` call that would fetch `getDiscoverySet`, and the `ScanScreen` repoint that would populate the two anchor members from it.
  Those are cutover steps C-9 and C-10.
  Both `ScanScreen` call sites pass `nil`/`null` today with a comment naming what must change and where.
- The mandatory issuer-whitelist pillar still asks the generation-1 `IssuerRegistry`, so it does not yet answer for a generation-2 root.
  That is a cutover blocker recorded in `docs/ISSUER_V2_OWNERSHIP.md` section 8, not something this slice closes.
  Carrying `providerRegistry` on the validated anchor is what gives those five consumers an attested address to migrate TO.

## Tests

- `contracts/test/ProtocolRegistry.t.sol` (22): the constructor floor as a boundary rather than a `!= 0` check, the delay actually gating every write, the selector rename and the exact 10-word arity, `factory != rootIndex`, per-member zero refusals, R-5 independence in both directions, deprecate-cancels-a-stale-proposal, binding lifecycle, fail-closed reads, role gating, and the golden encoding the two mobile suites are pinned against.
- `contracts/test/DeployProtocolRegistry.t.sol` (10): the mainnet guard, the testnet opt-in that cannot reach zero, the contract floor holding without the script, the real scripts end to end against a REAL stack (real router over real factories, real SBT, real verifier, real verification registry), each of the preflight's five relations broken in turn plus the accept case and an unmutated positive control, a real mid-window verifier swap driven through the verification registry's own 2-day timelock so the execute phase's re-preflight refuses it, and one re-staging test per axis so neither staged-versus-environment check is vacuous.

### No test in that file writes a NON-CANONICAL environment value, and that is what makes it deterministic

`vm.setEnv` writes the PROCESS environment while forge 1.5.1 runs a suite's test functions CONCURRENTLY, so a test writing a non-canonical `PUBLISH_*` value is visible to every other test in the file for as long as it holds it.
Two tests used to do that to reach the preflight's negative cases, and the suite was nondeterministic as a result.

Those cases are now asked of `preflight` directly: it is `public view` and takes the `DiscoverySet` by value, so a mis-transcribed record is built in memory and passed as an argument with no environment involved.
That is a gain rather than a concession - the five cases were never about env plumbing, they are about which relations the preflight enforces, and asserting them through the function is more direct as well as deterministic.

The precise invariant to preserve is DIVERGENCE, not abstinence.
Four tests still call `_setPublishEnv`/`_deployRegistry`, because they drive the real scripts and the scripts read the environment.
That is safe because forge runs `setUp` once and snapshots, so every test function sees identical fixture addresses and those four writes are byte-identical to each other; concurrent identical writes cannot be observed as a change.
A new test may therefore run the scripts freely, but must not write a value that differs from the canonical set - to vary a record, mutate the struct and call `preflight` directly, or re-stage on the registry as the two execute-phase tests do.

Measured on a 10-core machine, forge 1.5.1:

| configuration | before | after |
|---|---|---|
| `--match-path test/DeployProtocolRegistry.t.sol`, default threads | 0/8 runs green | 10/10 runs green |
| whole `forge test`, default threads | not a reliable signal (see below) | 10/10 runs green |

The "before" column is the isolated-file measurement, which is the one that matters: running the file alone gives its concurrent test functions maximum interleaving. An earlier reading of 21 consecutive clean WHOLE-SUITE runs was a false clean - with 14 suites competing for the thread pool the three env-writing tests were often scheduled sequentially, so the whole-suite configuration hid the race rather than disproving it.

Separate observation, measured rather than assumed: the two other files that call `vm.setEnv` are NOT flaky by this mechanism.
`contracts/test/PinConsentWitnessGraph.t.sol` ran 8/8 green and `contracts/test/DeployProtocolRegistry.t.sol` 8/8 green, each isolated by `--match-path` at default threads.
Generation 1's file has a single env-touching test writing only canonical values (its other tests pass explicit arguments to `validatePublishTimelock`), so it has no within-suite divergence to race on.
Nothing outside this branch needed fixing.
- `crates/dogtag-standard-rs` `discovery::tests`: the generation-2 anchor validating and surfacing both members, every unusable-address form failing closed, absence passing, the error naming the member, and a platform lie still outranking the shape check.
- `crates/dogtag-prover-rs` `manifest::tests`: the additive-serialization property, a generation-2 manifest signing over both members, the generation mismatch as a conflict in both directions, and on-chain precedence on the new members.
- `stacks/vet/api/tests/discovery_validation.rs`: both members travelling manifest to reconcile to anchor to validated version, and a stale manifest unable to steer the root index.
- `apps/android/.../AnchorResolverTest.kt` and `apps/ios/DogTagTests/AnchorResolverTests.swift`: the generation-2 golden vector decoded, and neither generation's return decodable as the other's.

Neither mobile suite runs in CI (a standing decision), so the Solidity golden assertion is the only automated end of that pair.
Run both by hand before shipping any mobile change: `gradle :app:testDebugUnitTest` and `xcodebuild test -scheme DogTagTests`.
