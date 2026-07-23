# DELEGATION - authorizing a non-owner for scoped consent (decision record)

**Status: DECIDED 2026-07-20. Implementation deferred post-v1.**

> **Decision.** dogtag delegation ships as a **separate delegate circuit**, published as its own protocol version.
> It is **not** a change to the owner consent circuit (`circuits/consent.circom`).
> The owner circuit, its reserved-leaf schema in `R`, and its 7 public signals stay untouched.

**Audience:** anyone designing delegation, planning the mainnet trusted-setup ceremony, or changing `build_profile_tree` / the reserved-leaf schema.

**Why this document exists:** the fork recorded here is one of only two decisions that must precede the mainnet ceremony (the other is `DEPTH`, §6).
Getting it wrong later is not a patch - it is a second forced circuit change and a second forced app-upgrade cycle.

Related: [`architecture.md` §4.2](./architecture.md) (recovery = re-issue, and why delegation is a different thing - §7 below), [`CEREMONY_TRANSCRIPT.consent.md`](./CEREMONY_TRANSCRIPT.consent.md) (the current setup is testnet-grade and explicitly re-doable before mainnet).

---

## 1. What "delegation" means here

An **owner** authorizes a **non-owner** - a caretaker taking the pet to the groomer, a boarding kennel, a walker - to give **scoped consent** on that tag, **without transferring ownership**.

That is a distinct problem from owner change.
Delegation adds a second authorized principal while the owner remains the owner; recovery/owner-change replaces the principal.
See §7 for the relationship and why they must not be conflated.

Today the repo has **no delegation machinery at all**.
This is greenfield, which is precisely why the architectural fork is worth recording before anything is built against an assumption.

---

## 2. The decision

Delegation is a **separate delegate circuit**, in which the delegate is authorized by an **owner-signed delegation message** and is therefore **not committed in `R` at all**.

Consequences that follow directly, and that this record is meant to make load-bearing:

| | |
|---|---|
| Owner consent circuit | **Frozen and untouched.** No new public signal, no change to the reserved-leaf schema. |
| `R` (the profile tree) | **Stays delegation-independent.** Exactly one reserved owner triple, forever (§5, the P-e invariant). |
| How a verifier routes to it | By **protocol version**, through the two-axis `ProtocolRegistry`. |
| When it must be built | **Deferrable.** It gates nothing on the owner path. |
| Cost of deferring | **One additional ceremony later for a genuinely new circuit - not a redo of the owner ceremony.** |

The rejected alternatives are recorded in §3 and §4, because both are attractive enough that someone will re-propose them.

---

## 3. Context - why delegation needs a new circuit at all

### 3.1 The frozen circuit already admits a non-owner, in the worst possible way

`circuits/consent.circom` pins the three reserved keyPaths and their typeTag as compile-time constants (`consent.circom:48-56`), but the **salts and the values are free private inputs**.
It then reconstructs three leaves and folds **three independent inclusion paths** that must each reach the same root (`consent.circom:155-157`):

```circom
R <== incOwner.root;
incKey.root    === R;
incSecret.root === R;
```

So the circuit proves that *some* leaf at `owner.address`, *some* leaf at `owner.consentKey` and *some* leaf at `owner.secret` are in `R`, that the key committed at `owner.consentKey` signed the consent message, and that the nullifier derives from the value committed at `owner.secret`.

It does **not** prove those three leaves belong to the same principal, and it does **not** prove they are the only leaves at those keyPaths.
Nothing enforces per-keyPath uniqueness anywhere in the stack: the profile tree is a sorted-hash **set of leaf hashes** (`crates/dogtag-standard-rs/src/merkle.rs`) with no notion of keyPath uniqueness, and two leaves at the same reserved keyPath with different salts are simply two distinct, genuine members.

**Therefore: a non-owner whose own `(address, consentKey, secret)` triple were committed into `R` under the same three reserved keyPaths could produce a consent proof that verifies under the frozen production verification key, with no circuit change and no new ceremony.**

The keyPath pinning is doing its job - it is the D5 replay defence, and it genuinely rejects an *arbitrary attribute* leaf, because an attribute leaf uses a different encoding and a different keyPath and cannot fold to `R` through the reserved reconstruction.
What pinning does not do, and never claimed to do, is bound the *number* of reserved triples in a tree.

### 3.2 Why that is not delegation worth shipping

The proof a duplicate triple produces is **bearer-style**:

| Property the caretaker scenario actually needs | Duplicate reserved triple |
|---|---|
| **Scoped** to a purpose / relayer / time window | ❌ No. The delegate gets the owner's full consent authority over the tag. |
| **Revocable** without retiring the tag | ❌ No. `profileRoot[id]` is **write-once** (`contracts/src/DogTagSBTConsent.sol`, no setter exists), so every delegate must be named **before mint** and can never be removed. For a transient caretaker relationship, that is close to unusable - revocation would mean retiring the tag and re-issuing under a new `dogTagId`. |
| **Distinguishable** from the owner by a verifier | ❌ No. Both principals emit the same `R` and the same `dogTagId`; the public-signal vector is identical except the nullifier, which is opaque. |

There is also an issuance-protocol problem that no circuit change fixes: every reserved leaf is derived inside `build_profile_tree` from the **owner's seed**, so whoever assembles `R` knows every salt and value in it.
Committing a delegate's owner-secret leaf either hands the delegate's secret to the owner (who can then compute all of the delegate's nullifiers) or requires an entirely new delegate-side key ceremony and SDK entry point.
That is a new issuance protocol, not a relaxed guard.

**Conclusion: scoped, revocable, or owner-distinguishable delegation requires a new circuit, and therefore a new trusted setup.**
The current consent ceremony is testnet-grade and explicitly re-doable before mainnet, so there is runway - but only if the *shape* of delegation is settled before the mainnet re-run.
That is what §4 settles.

---

## 4. Why SEPARATE rather than growing the owner circuit

The real fork was never "design all of delegation now or pay for two ceremonies".
It was **unified vs separate**, and only the unified branch is urgent.

**Unified (rejected):** one circuit handles both principals, with a `principalClass` public signal saying which.
That grows the owner circuit's public-signal vector from 7 to 8, which regenerates the verifier, changes the `VerificationRegistryConsent` signature, changes the relayer's `sol!` interface, and changes both apps' public-signal index constants.
This repo has already been bitten once by exactly that drift class: the 4-arg-deployed / 6-arg-selector mismatch left `recordVerificationZK` reverting bare on-chain (`contracts/deployments/roax.json`, `_verification_registry_redeploy`).
It was nearly bitten a second time by the public-signal index divergence between the consent circuit and the since-retired owner-revealing verification circuit, which review caught as finding e9 E-1 before anything shipped - commit `25d4065` records that routing the call sites through the new constants was zero behaviour change, with no live read flipped.
That caught near-miss is precisely why `public_signals.rs` and its Swift and Kotlin twins exist at all.
Under unified, the entire delegation design - scope granularity, revocation, whether verifiers must distinguish principals - becomes blocking work before the mainnet ceremony, and the submission-path and app-release milestones must hold.

**Separate (chosen):** the delegate is authorized by an **owner-signed delegation message**.
The owner's consent key is already proven to be in `R` via the existing `owner.consentKey` leaf, so it can authorize a delegate off-chain, at any time after mint.
The delegate is consequently **never a leaf in `R`**, and that single property dissolves every problem §3.2 listed:

| Problem with delegate-in-`R` | Under a separate, message-authorized delegate circuit |
|---|---|
| Delegates fixed at mint (`profileRoot` is write-once) | Gone. Authorization is an off-chain signed message, issuable any time after mint. |
| Unrevocable without retiring the tag | Tractable. Revocation is a property of the delegation message and of that future circuit's design, not of the write-once tree. |
| Owner learns the delegate's secret | Gone. The delegate holds its own key; the owner signs over a commitment to it. |
| A second reserved triple voids the owner circuit's soundness premise | Gone. `R` keeps exactly one reserved triple (§5). |
| Owner circuit's public signals must change | Gone. The owner circuit is frozen and untouched. |

**The routing already exists.**
`ProtocolRegistry.ContractSet` (`contracts/src/ProtocolRegistry.sol`) carries `verificationRegistry`, `verifier` and `circuitId` **together** on the on-chain axis, while the off-chain artifact axis rotates independently - the two-axis split landed as R-5.
A delegate circuit is therefore a **new protocol version under its own `contractSetId`**, not a modification of this one.

**One consequence to design for deliberately, not discover later.**
`VerificationRegistryConsent` holds a single `zkVerifier` (swappable only through a timelock) and its `consumed` nullifier set is per-registry storage.
So a delegate circuit needs its **own registry deployment**, and the nullifier space is **fragmented across registries**: a nullifier spent in the owner registry is not marked spent in the delegate registry.
Each registry independently enforces `R == profileRoot(dogTagId)`, and the two circuits derive nullifiers from different preimages, so this is manageable - but it is a real property of the chosen path and belongs in the delegate circuit's design brief.

**This record deliberately stops here.**
It fixes the architecture and its consequences.
It does **not** specify the delegate circuit's internals - the delegation message format, the scope encoding, the revocation mechanism, and the in-circuit authorization chain are all design work for whenever delegation actually ships.

---

## 5. Normative invariant: exactly one reserved triple per `R` (P-e)

> **NORMATIVE. The SDK MUST forbid committing a second `(owner.address, owner.consentKey, owner.secret)` reserved triple into any profile tree `R`.**

**Why it is normative.**
`consent.circom`'s own stated soundness argument is that "pinning keyPath forces the unique real leaf", which is what makes `ownerSecret` and `(Ax, Ay)` collision-bound to the genuine owner leaves.
As §3.1 shows, that is an **assumption about the tree, not a property the circuit enforces**.
It is true today only because exactly one triple is ever built.
Introduce a second and the stated property becomes false as written: `ownerSecret` would be bound to *some* leaf at `owner.secret` and `(Ax, Ay)` to *some* leaf at `owner.consentKey`, not necessarily the same principal's, and two principals would independently consume the same logical consent slot for the same `(dogTagId, purpose, relayer, nonce)` with different nullifiers.

**Current enforcement, stated precisely.**
`build_profile_tree` already rejects an *attribute* whose keyPath resolves to one of the three reserved keyPaths (`crates/dogtag-standard-rs/src/profile_tree.rs`, compared on the derived keyPath field rather than the raw string, so NFC-normalization aliases cannot slip through).
The three reserved leaves themselves are derived internally from the owner's seed, so today the only tree the SDK can build has exactly one triple.
The invariant is therefore **currently satisfied** - what this record makes normative is that it stays satisfied.

**What it binds.**
Any future issuance entry point - a delegate-issuance API, an externally-supplied leaf-commitment handshake, a batch or import path, a relaxation of the guard above - **MUST NOT** result in an `R` containing more than one leaf at any reserved keyPath.
Under the decision in §2 this is free, because delegates are never leaves.
A future proposal that needs a second triple is not a guard tweak: it is a proposal to change the owner circuit's soundness argument, and it must be treated as such.

**Not ceremony-gated.**
Because the fork landed on separate, triple coherence never has to become a circuit constraint.
An SDK-level guard plus a test pinning it is the whole fix, shippable at any time, with no circuit change and no ceremony.
(Implementing that guard for future entry points is a separate task; this record only fixes the invariant.)

---

## 6. Still to lock before the mainnet ceremony: `DEPTH`

With the fork decided, **`DEPTH` is the only remaining ceremony-gated decision.**

`consent.circom` instantiates `DogTagConsent(6)`, capping a profile tree at 2^6 = 64 leaves.
That ceiling is baked into the protocol version's identity: `ProtocolRegistry.ContractSet.circuitId` is `keccak256("consent.circom/DogTagConsent(6)")`.
Raising it later is a new circuit and a new ceremony.

The budget is 3 reserved leaves + owner-identity attributes + pet attributes, with no delegate leaves ever (§2).
64 is probably ample - but it must be a **deliberate** decision rather than an inherited default, taken before the mainnet re-run.

Explicitly **not** ceremony-gated, now that the fork is decided:

- **Triple coherence.** Covered by the §5 SDK invariant.
  It would only have become a circuit constraint under the rejected duplicate-triple path.
- **Delegation scope granularity, and the delegate revocation mechanism.** These gate the *delegate* circuit's own future ceremony, not this one.
- **Whether a verifier can distinguish owner from delegate.** Under the chosen path the distinction is structural: a delegate proof verifies against a different verifier under a different `contractSetId`.
  No new public signal is needed.

---

## 7. Delegation is not ownership change

These are separate mechanisms and must not be merged.

**Delegation** (this document): the owner stays the owner and authorizes an additional, non-owning principal for scoped consent.
Nothing on-chain changes; no new `dogTagId`; no new `R`.

**Owner change / recovery**: the principal is replaced.
The retired owner-revealing SBT exposed `recover()`, a consensual signature-authorized rebind preserving `tokenId` and `issuerOf` - that model is history.
`DogTagSBTConsent` has no `recover()` and no `RECOVERY_ROLE` (D3), because naming a new owner on-chain is exactly the linkage owner-unlinkability removes.
Recovery is instead a **fresh custodial issuance under a new `dogTagId` and a new `R`**, and referencing credentials do not survive it (accept-the-break, 2026-07-16; [`architecture.md` §4.2](./architecture.md)).

So: a delegation must never be implemented as a partial owner rebind, and an owner change must never be implemented as a delegation.
Because `profileRoot` is write-once, anything that tries to express delegation *through* the tree collapses into the retire-and-re-issue path - which is precisely the trap §3.2 documents.
A future handover-consent protocol would share machinery with the delegate circuit (both verify an owner-signed authorization message), so it may be cheaper to design them together than sequentially, but they remain distinct protocols.

---

## 8. Decision log

| Date | Decision |
|---|---|
| 2026-07-20 | **Delegation = separate delegate circuit, owner-signed authorization, delegate never committed in `R`.** Owner circuit frozen. Routes as a new protocol version through the two-axis `ProtocolRegistry`. Implementation deferred post-v1; deferral costs one additional ceremony later, not a redo. |
| 2026-07-20 | **Duplicate reserved triple explicitly rejected** as a delegation mechanism, despite requiring no circuit change (§3.2). |
| 2026-07-20 | **P-e recorded as a normative invariant:** exactly one reserved triple per `R` (§5). |
| 2026-07-20 | **`DEPTH` is the sole remaining ceremony-gated decision** (§6). |
