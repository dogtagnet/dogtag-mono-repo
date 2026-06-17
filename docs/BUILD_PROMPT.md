# DogTag Ecosystem — Build-Out Prompt

> Paste this to a coding agent (Claude Code) working in `dogtag-mono-repo/`. It sets the goal, the operating rules, the phased plan, and the first actions. The agent should **plan, then execute phase by phase**, verifying each phase before moving on.

---

## Mission

Build the **DogTag pet-credentialing ecosystem** exactly as specified in:
- **`docs/architecture.md`** — system + smart-contract architecture (the *what* and *why*).
- **`docs/implementation.md`** — per-function pseudocode, contract bodies, endpoints, Docker, deploy (the *how*).
- **`docs/research/`** — the evidence behind every decision (5 research briefs + 3 audits).

**These three are the source of truth. Read them in full before writing any code.** Where `architecture.md §13` and `implementation.md §11` (the audit remediations) conflict with anything earlier in those docs, **the remediation sections win** — they are normative.

The system: pet owners hold pets' verifiable credentials in native mobile apps; vets/groomers run **self-hosted** Docker stacks that issue credentials by anchoring salted-Merkle roots on the **ROAX** EVM chain (chainId 135, gas token PLASMA); a **central/admin** stack (we host) runs discovery, issuer whitelisting, and is the appointment system-of-record. Verification = **three authenticity pillars** (integrity + on-chain status + DNS identity) that gate validity for everyone, **plus a contextual `ownership` fragment** (`ownerOf(dogTagId)==userWalletAddress`) that gates *only the owner's own self-import* and is `NOT_APPLICABLE` for third-party verifiers.

## Non-negotiable principles

1. **Determinism first.** The credential standard must produce **byte-identical Merkle roots in TS, Rust, and Solidity.** Build `crates/dogtag-standard-rs` and `packages/dogtag-standard-ts` against a **shared `testvectors.json`** and assert parity in CI before anything depends on them. Honor every fix in `implementation.md §11.2` (pinned decimal grammar, typed string input, NFC version pin, flatten/keyPath grammar, first-two-colons parse).
2. **Security gates are blocking.** Do **not** deploy contracts until the audit Criticals/Highs in `§13.1`/`§11.1` are implemented (`_disableInitializers`, per-record-type scoping `isWhitelistedFor`, `issuedBy` originator binding, admin-only `burn`, `AccessControlDefaultAdminRules` + multisig admin). Do **not** expose custody endpoints publicly — `/admin/*`, localhost/session-bound, `/unlock` rate-limited (`§11.4`).
3. **Contextual four-fragment verification.** Credential validity = the **three authenticity pillars** (integrity **and** on-chain status **and** DNS-identity-cross-checked-against-registry). The **`ownership`** fragment (`ownerOf(dogTagId)==userWalletAddress`) is **contextual**: it gates only the owner's *self-import*, and is `NOT_APPLICABLE` (never INVALID) for third-party verifiers — making it universally required breaks the groomer/airline/vet import flows. Fragments are 4-state `VALID|INVALID|ERROR|NOT_APPLICABLE`. Integrity alone means nothing. Code **`§11.3`** (not the superseded §1.7).
4. **Test as you build.** Each phase ships with tests and a green build before the next phase starts. No phase is "done" without its acceptance criteria met.
5. **Self-hosted reality.** Treat each business backend as an untrusted-to-others, operator-controlled deployment. Trust comes from the **central registry** + on-chain whitelist, not from the deployment claiming things.
6. **Use the pinned stacks:** Foundry + OZ v5 (`evm_version = paris`); Rust + Axum + MongoDB + Alloy (not ethers-rs); React + Vite + TS + Tailwind + shadcn; Kotlin/Compose + Swift/SwiftUI; shared verify via UniFFI. Uncommon ports: admin 39741/39742, vet 41873/41874, groomer 43617/43618; Mongo internal-only.
7. **Privacy — nothing personal on-chain, ever.** On-chain holds only **salted commitments** (per-field random 16-byte salts; salts off-chain), revocation/status, timestamps, and **non-personal refs** (DIDs, accreditation refs). A salted hash is still personal data and an unsalted hash of a low-entropy microchip number is brute-forceable — salting is the privacy mechanism, not just anti-forgery. All `Owner` PII and Art. 9 service-attestation data stay off-chain, encrypted, deletable. **Right-to-erasure = delete the off-chain record + destroy its salt/key** (renders the on-chain commitment unlinkable; documented as a mitigation, not a regulator-blessed safe harbour). **Mandatory DPIA.** CCPA/GDPR 45-day delete runs the **same** erasure flow.
8. **Neither signing mode can fake issuance.** Both `WalletStrategy` and `BackendStrategy` build the Merkle root / wrapped doc **server-side (shared SDK) — identical in both modes**; the backend **re-verifies on-chain** (`RootIssued` event + `issuedAt[root] != 0`) before marking a record issued. A lying/buggy frontend cannot fake issuance.

## Operating procedure

1. **Plan:** produce a short task plan for the current phase (use a todo list), list files you'll create, and the acceptance check.
2. **Execute** the phase.
3. **Verify** against the acceptance criteria (run tests/build); report results honestly.
4. **Checkpoint:** summarize what changed, then proceed to the next phase. Commit per phase on a feature branch (never the default branch directly).
5. If a spec gap or contradiction appears, **stop and surface it** with a recommendation rather than guessing.

---

## Phased plan (build in this order — matches `implementation.md §10`)

### Phase 1 — The trust core: shared standard SDKs
**Goal:** identical Merkle roots across languages.
- **Lock the FINALIZED schemas first (before coding validators):** coded vaccine (`vaccineProductCode` = USDA APHIS PCN + separate `vaccineManufacturer`), VBO breed (`breedVbo` + `breedLabel`), `microchip` **object** (15-char string, never a float), trust-tiered **service attestation** (`assistanceType`/`issuerTrustTier`/`legalContext`, Art. 9 off-chain), **VC 2.0 envelope** (array `@context`/`type`, prose→`description`, `credentialStatus`/`id`/`credentialSchema`), and first-class **`Owner`** entity (off-chain PII). Use canonical names/enums from CHANGESPEC §0.
- Implement `crates/dogtag-standard-rs` and `packages/dogtag-standard-ts`: `encodeValue` (with pinned decimal/integer grammar, `assertNotFloat`), `hashLeaf` (domain-sep `0x00`, length-prefixed), `buildMerkle` (`0x01` node sep, commutative `sortPair`, odd-promotion), `merkleProof`/`processProof`, `flatten`/`unflatten` (pinned grammar), `wrapDocument` (typed input), `obfuscate`, `verify` (3-pillar, tri-state).
- Author `testvectors.json` (leaf hashes, roots, proofs; include decimal `22.7`/`0.5`, timestamps with `:`, nested/array/empty, single-leaf, odd sizes 1–9, obfuscation).
- Add a Solidity node-hash test proving on-chain agreement at the node level.
**Acceptance:** both SDKs pass the *same* vectors in CI; a value cannot be swapped/removed while keeping the root; obfuscation preserves the root.

### Phase 2 — Smart contracts
**Goal:** deployable, audit-clean contracts on ROAX.
- Implement the **corrected** `IssuerRegistry`, `DogTagIssuer` (clone, `_disableInitializers`, `issuedBy`, `isValid`, admin mass-revoke), `DogTagIssuerFactory` (permissioned, deterministic salt) per `§11.1`, and the **granular `DogTagSBT`** per `§11.7(a)`: ERC-5192 + `AccessControlEnumerable` with `ISSUER_ROLE`/`UPDATER_ROLE`/`AUTHORITY_ROLE`/`RECOVERY_ROLE`, immutable `issuerOf[tokenId]`, the `DogTagStatus {Active,Lost,TransferPending,Deceased,Revoked}` soft-status model (**never burn for lifecycle**; `Deceased`/`Revoked` terminal, set by issuer-or-`AUTHORITY_ROLE`, never owner), EIP-712 `recover()` (preserves `tokenId`+`issuerOf`), and admin-only `burn` reserved for GDPR erasure. `dogTagId` is a **non-personal** random/sequential id (never `keccak256(microchip)`).
- Foundry tests: soulbound transfer revert; only `isWhitelistedFor(recordType,signer)` can issue/revoke; cross-type/cross-clone isolation; originator-only revoke; uninitialized-impl attack blocked; clone init-once; factory determinism; admin two-step. **SBT-specific:** issuer-or-authority gates update/status; owner CANNOT call `setStatus`; `Deceased`/`Revoked` are terminal; `recover()` requires a valid EIP-712 destination signature (wrong-chain/replayed/non-destination sig reverts) and preserves `tokenId`; `dogTagId != keccak256(microchip)`.
- `script/Deploy.s.sol`; `foundry.toml` `evm_version=paris`, pinned solc.
**Acceptance:** all Foundry tests green; `cast chain-id --rpc-url https://devrpc.roax.net` returns 135 (RPC liveness precheck — it was 502 at design time, so confirm before deploying); deploy to ROAX, verify on Blockscout, write `deployments/roax.json`.

### Phase 3 — Vet business backend (Rust)
**Goal:** issue → share → verify round-trip, with safe custody.
- Genesis/custody under `/admin/*` (24-word BIP39, age-encrypted seed, unlock-on-boot, multi-account) per `§3.1`/`§11.4`; Alloy signing (EIP-1559 with legacy fallback).
- `/records` issue (build VC → `wrapDocument` → `issue(root)` tx), `/records/{id}/revoke`, `/records/{id}/share` (EdDSA record-scoped JWT, `exp=180s`, atomic `jti`), `GET /records/{id}` (mirror asserts), `/import/*`.
- **Dual switchable signing:** `SigningStrategy` interface with `WalletStrategy` (wagmi v2 + viem 2 + Reown AppKit; MetaMask + WalletConnect v2) and `BackendStrategy` (Alloy custody) — mutually exclusive, switchable via a **server-side-persisted Settings radio toggle**. Merkle-root/wrapped-doc build is **always server-side, identical in both modes**.
- `POST /credentials/prepare` → unsigned tx `{to, data, value, chainId:135}` (wallet mode) or sign+broadcast (backend mode); **hardened** `POST /credentials/confirm` per `§11.7(e)`/§11.6: **derive `signer` from the transaction** (never the request body), require `tx.to`/`tx.input`/`value:0`/`chainId:135` to equal the prepared draft, pin the emitting contract for the `RootIssued` log, require `isWhitelistedFor(recordType, signer)` at confirm, **wait N confirmations** (reorg-safe), idempotent on `txHash`.
- **Operator-session auth on ALL issuance/settings/signer routes** (`prepare`, `confirm`, `/records/*`, `settings/signing-mode`, `issuer/signers`, `import/*`, `calendar/*`); only `GET /records/{id}` (record-JWT) and HMAC routes are unauthenticated. **Retire or operator-gate the legacy `POST /records`** (else remote unauthenticated issuance + gas-drain). Reject `PUT /settings/signing-mode` (409) while a `prepared` draft is outstanding.
- **Multi-address whitelist preflight:** active signer must be `isWhitelistedFor(recordType, signer)`; pre-flight `eth_call` to fail fast (wallet mode = user pays gas). Mode switch = onboarding event (new address → admin approval → `whitelistFor` → poll until live).
- Schema validator per `§11.5` (conditional microchip incl. `microchip.implantDate`, booster-aware 21-day, titer/EU/CDC/DOT rules) over the finalized coded schemas.
**Acceptance:** integration test against local anvil (chainId 135) — issue a rabies cert, fetch via one-time JWT, `verify()` → VALID; revoke → issuance pillar INVALID; reused JWT → 401; custody endpoints unreachable from public surface; `/credentials/confirm` refuses to mark issued unless on-chain re-verification passes; a non-whitelisted active signer fails preflight.

### Phase 4 — Central / admin backend (Rust)
**Goal:** discovery, whitelisting, appointment source-of-truth, mobile API.
- Mobile-user API (auth, pets, mint SBT with microchip-uniqueness, credentials import, `/v1/share/{ref}` with full asserts per `§11.4`).
- Business registry/discovery (geo); issuer-application queue → admin approve triggers on-chain `whitelistFor` (verify DNS TXT before whitelisting); registry self-write impossible. **Whitelist accepts MULTIPLE addresses per issuer entity** (one-to-many issuer→signers); delist inactive-mode addresses.
- **Consent/retention + erasure (crypto-shredding):** per-purpose `Consent`/`ConsentReceipt` with lawful basis + `retention{basis, clock}`; **CCPA/GDPR delete (45-day)** wired to the `§11.7`/§11.6 flow: per-record DEK destroyed (crypto-shred — makes salts in all replicas/backups undecryptable), off-chain record deleted, **erasure propagated central→every business backend** (the vet is the GDPR controller), and the **SBT burned** to drop the live `ownerOf↔wallet` link. Consent withdrawal triggers retention re-eval → erase.
- Appointments as system-of-record: **central is sole `rev` allocator**; `PUT` to business + `appointment-events` callback with **ownership binding** (`§11.4`); catch-up pulls.
**Acceptance:** end-to-end whitelist flow puts a real `isWhitelistedFor` on-chain and supports multiple addresses per issuer; a business cannot act on another's appointment; rev never collides; a delete request runs the erasure flow (off-chain record gone + salt/key destroyed) within 45 days.

### Phase 5 — Web portals (React + Vite + TS)
- `packages/ui` (theme tokens + shared components, **light + dark** semantic palettes) → vet portal (setup wizard incl. DNS-TXT instructions, schema-driven issue form, QR, records, import, calendar/appointments), groomer portal (reference dashboard + import profile/vaccination via QR), admin portal (registry, issuer-application approvals, whitelist viewer).
- **Persisted light/dark theme toggle** in each portal (vet, groomer, admin) — light/dark only, not the mobile 7 colorways.
- **Wallet-connect + signing-mode UI:** MetaMask / WalletConnect via wagmi v2 + viem 2 + Reown AppKit; chain-add via viem `defineChain` + `useSwitchChain` (`wallet_switchEthereumChain`, fallback `wallet_addEthereumChain` on `4902`); Settings radio for `signingMode` (wallet vs backend); status panel (wallet: connected address + ROAX-chain check + whitelist badge; backend: genesis state + PLASMA balance).
**Acceptance:** vet can issue and show a QR; groomer can import + verify a shared credential; admin can approve an issuer; theme toggle switches light/dark and persists; a wallet-mode issue connects MetaMask, switches to ROAX, and signs while backend re-verifies on-chain.

### Phase 6 — Mobile apps (Android then iOS)
- UniFFI-bind `dogtag-standard-rs` so mobile `verify()` == server `verify()` (test on the shared vectors).
- Screens per references (onboarding; Verify/Travel/Home/Documents/Profile; pet card + grouped credentials; add-record wizards; scan-QR import; share-QR; Maps discovery + booking).
- 7 themes (black/white/blue/red/pink/green/yellow), semantic-token theming, light+dark; persisted.
- **Self-custodial wallet under Settings** (Telegram TON-Space-style): default **embedded MPC wallet** (MetaMask Embedded Wallets / Privy — real TSS, social/passkey login, no seed-phrase UX); **raw BIP-39 self-custody export** (web3j / web3swift) as advanced. Encrypt-then-store seed/secret behind **Secure Enclave (iOS) / StrongBox (Android)** hardware key, **biometric-gated**. **Mint each pet's SBT to a fresh per-pet derived address** (breaks cross-pet enumeration). **Prefer gas sponsorship / account abstraction so owners never hold PLASMA — omit native send/receive from v1**; dApp connect (Reown WalletKit) **off by default**.
- **MPC key-loss recovery (`§11.7(f)`):** default = provider passkey/email-share recovery; catastrophic loss (no key) = `RECOVERY_ROLE` executes `recover()` after an **off-chain identity proof to the protocol** — does NOT require the lost key, and preserves `tokenId` so referencing credentials survive. DogTag's own EIP-712 `Claim` is signed only via the in-app recovery flow, never a connected dApp.
- **Import verification (contextual, `§11.3`):** mobile owner self-import = the **three authenticity pillars** (offline merkle, `isValid` RPC, DNS+registry) **plus** `ownerOf(dogTagId)==myWalletAddress` (the contextual ownership fragment, required only here). A record imports as "yours" only when the on-chain owner is the address you control.
**Acceptance:** scan a vet QR on device → VALID verdict + imported credential; theme switch recolors without layout change; mobile root matches server root on vectors; MPC wallet created with the secret hardware-encrypted + biometric-gated; SBT mints to a fresh per-pet address and self-import gates on `ownerOf == myAddress`; a simulated lost-key `recover()` (RECOVERY_ROLE + EIP-712 destination sig + off-chain proof) re-binds the owner **without** changing `tokenId` and leaves referencing credentials valid.

### Phase 7 — Calendar sync + cross-backend appointments
- Google OAuth (offline+consent), incremental sync tokens (410 full-resync), `events.watch` + poll fallback + renewal cron, `freeBusy`; **`etag`-primary echo suppression**; `extendedProperties` tagging; availability = working-hours − appts − freebusy − capacity with soft holds.
**Acceptance:** a platform booking mirrors to Google without echo loops; a Google-side human edit isn't dropped; reschedule/cancel stays consistent on both backends.

### Phase 8 — Hardening
- E2E across the whole flow; re-run the three audit lenses against the *code*; fix Mediums (delisting→admin mass-revoke path, jti atomicity, finality confirmations); load/security passes; docs/README per stack.
- **Privacy/erasure verification gate:** PII-off-chain audit incl. a **negative test** — assert **no low-entropy personal value is ever anchored** (esp. `dogTagId != keccak256(microchip)`); acknowledge the `ownerOf↔wallet` link as **pseudonymous personal data** in DPIA scope and verify **fresh-per-pet addresses**. Erasure-unlinkability test: after the flow, the on-chain commitment is unlinkable (crypto-shred DEK destroyed across replicas + off-chain record deleted + **erasure propagated to business backends** + **SBT burned**).
- **Dual-signing parity test:** wallet vs backend mode produce **identical Merkle roots and records** for the same input (root/wrapped-doc built server-side in both modes).
**Acceptance:** full E2E green; no open Critical/High; each stack `docker compose up` on its ports with Mongo internal-only; PII-off-chain audit + erasure-unlinkability test pass; dual-signing parity test confirms wallet and backend modes yield identical roots/records.

---

## First actions (do these now)
1. Read `docs/architecture.md`, `docs/implementation.md`, and skim `docs/research/audit-0{1,2,3}`.
2. Set up the workspaces (pnpm + Cargo + Foundry) and root `justfile` (`dev/build/test/deploy-contracts/up:<stack>`).
3. Start **Phase 1**: scaffold both SDKs + `testvectors.json`, implement `encodeValue`/`hashLeaf`/`buildMerkle`, and get cross-language parity green. Report the first passing vector set, then continue.

Work autonomously through the phases; checkpoint after each. Surface blockers early. Build it correct, deterministic, and secure — in that order.
