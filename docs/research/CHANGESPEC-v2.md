# DogTag v2 Change-Spec (NORMATIVE for the doc-update pass)

> Authoritative consolidation of research 06 (competitive apps), 07 (legal/privacy), 08 (wallet integration). Update agents MUST apply these decisions to `architecture.md`, `implementation.md`, and `BUILD_PROMPT.md`, using the canonical names/enums in §0. Where this conflicts with the existing docs, **this spec wins** and the docs must be edited to match. Preserve the existing normative remediation sections (arch §13 / impl §11) — extend them, don't delete them.

---

## 0. Canonical names & enums (use EXACTLY these across both docs)

- Entities: `Owner`, `Dog` (pet identity / DogTag SBT), `Issuer` (vet/groomer/authority), `Credential`, `Appointment`, `Consent`, `ConsentReceipt`.
- Rabies credential fields: `vaccineProductCode` (USDA APHIS Veterinary Biologics PCN), `vaccineProductName`, `vaccineManufacturer`, `batchLotNumber`, `vaccinationDate`, `validFrom`, `validUntil`, `nextDueDate`, `authorizedVet`, `series` (`primary`|`booster`), optional `titer{labId, sampledAt, resultIUml}`.
- Microchip object: `microchip { code: string(15), standard: enum("ISO_11784_11785","OTHER"), implantDate, bodyLocation }`. **Never a float; never a bare number.**
- Dog identity fields: `dogTagId` (SBT tokenId), `name`, `species`, `breedVbo` (Vertebrate Breed Ontology id, e.g. `VBO:0200798`), `breedLabel`, `sex` (`male`|`female`), `neuterStatus` (`intact`|`neutered`|`spayed`), `dateOfBirth`, `colour`, `distinctiveFeatures`, `weightHistory[]{value, unit:"kg"|"lb", measuredOn}`, `microchip`, `photoHashes[]` (off-chain blobs, hash only).
- Service/assistance attestation: `assistanceType` (`service_dog`|`emotional_support`|`none`), `issuerTrustTier` (`adi_accredited`|`licensed_pro`|`handler_self_attestation`|`unverified_registry`), `taskDescription`, `legalContext[]` (`ADA`|`ACAA`|`FHA`). **Special-category (Art. 9) — off-chain only, never hashed on-chain.**
- VC 2.0 envelope: `@context` = URI array (`https://www.w3.org/ns/credentials/v2` + DogTag context URI); `type` = token array (e.g. `["VerifiableCredential","RabiesVaccinationCertificate"]`); human prose goes in `description` (NOT `@context`); `id`, `issuer`, `validFrom`, `validUntil`, `credentialSubject`, `credentialStatus` (revocation), `credentialSchema`.
- Legal/trust meta (every credential): `attestationType`, `signatureTrustTier` (`accredited_authority`|`licensed_vet`|`self_attested`), `legalEffect` (`evidentiary`), `legalBasisVersion`, `jurisdiction`.
- Issuer accreditation (mandatory, structured): `usdaNan` (6-digit National Accreditation Number), `nvapCategory`, `license{number, jurisdiction, expiry}`, `aphisEndorsement{vehcsRef, endorsedAt}` (for exports).
- Signing: `signingMode` enum (`wallet`|`backend`); `signerAddress`; record audit `{signingMode, signerAddress}`.
- `recordType` on-chain = `keccak256(label)`; labels: `DOG_PROFILE`, `VACCINATION`, `SERVICE_ATTESTATION`, `TRAVEL_CLEARANCE`, `EU_HEALTH_CERT`, `DOT_SERVICE_FORM`(off-chain), `CDC_IMPORT_FORM`(off-chain).

---

## 1. Finalized data structures (research 06 + 07)

Apply to arch §3.6 (schemas) + §9 (Mongo model) + impl §1.5/§1.6/§11.5 (validators).

1. **First-class `Owner` entity (off-chain PII only)** — `{name, addresses[], phones[], email, emergencyContact, contactUpdatedOn}`. `Dog.ownershipHistory[]{ownerId, from, to}`. **Record-custodian (issuer) is distinct from pet-owner.** Owner data is PII → off-chain, encrypted, deletable; **never on-chain**.
2. **Microchip** = the object in §0 (15-char string). `microchip.implantDate` mandatory (EU/VEHCS enforce "vaccination date ≥ implant date").
3. **Coded vaccine + breed** — `vaccineProductCode` (APHIS PCN) + separate `vaccineManufacturer`; `breedVbo` + `breedLabel`. Coded values hash identically across jurisdictions (EU DCC lesson).
4. **Add `nextDueDate`, `microchip.implantDate`** (CDC + VEHCS require "date next due").
5. **Service-dog = trust-tiered attestation** (the §0 enums), not a boolean. ESA distinct from service dog. **Off-chain, Art. 9.**
6. **VC 2.0 envelope fixed** (§0): arrays for `@context`/`type`, prose→`description`, add `credentialStatus`/`id`/`credentialSchema`. Revocation is first-class (`credentialStatus` mirrors on-chain `isValid`).
7. **Stop duplicating identity onto vaccine records** — vaccine credential references `dogTagId` only; do not copy name/breed/etc. (reduces drift + on-chain hash payload).
8. **Normalize**: `sex` ≠ `neuterStatus`; derive age from `dateOfBirth` (drop free-text age); `weightHistory[]` unit-bearing + dated; top-level `species`; document/photo hashes anchored off-chain.
9. **Keep our own keccak256 salted-leaf canonicalization** — do **NOT** adopt JSON-LD/RDF canonicalization (SMART Health Cards / EU DCC lesson: anchor only a hash/root, avoid RDF canonicalization). The arch §3 spec stands; just ensure coded fields are used so facts hash deterministically.

## 2. Privacy & legal (research 07) — hard constraints

Apply to arch §11 (security/privacy) + §3 + §9, and add endpoints in impl §4.

- **Nothing personal on-chain — ever.** On-chain = salted commitments (salts off-chain), revocation status, non-personal DIDs, timestamps, accreditation refs. State explicitly that **even a salted hash is personal data** and an **unsalted hash of a low-entropy microchip number is brute-forceable** — hence per-field random 16-byte salts are the privacy mechanism, not just anti-forgery.
- **Right-to-erasure = delete off-chain record + destroy salt/key** so the on-chain commitment becomes unlinkable. Document this as a mitigation (not a regulator-blessed safe harbour); require a **mandatory DPIA**; add a **CCPA/GDPR delete endpoint (45-day)** wired to the same erasure flow.
- **Retention + consent**: per-purpose `Consent`/`ConsentReceipt` records with lawful basis; `retention{basis, clock}` fields on credentials.
- **Legal posture is evidentiary, not authoritative** — no eIDAS/ESIGN presumption; authority comes from the accredited issuer. Encode `signatureTrustTier`/`legalEffect`/`legalBasisVersion`/`jurisdiction`. DOT form = self-attestation under 18 U.S.C. §1001 (record that an attestation exists, never "verified disability").
- **Mandatory issuer-accreditation fields** (§0) as structured data; **layered multi-issuer model** for EU/US exports (accredited vet → APHIS VEHCS endorsement chain).

## 3. Dual switchable signing — vet/groomer web (research 08 A)

Apply to arch §6 (rename to "Signing modes"), §4.3 (whitelist→multi-address), impl §3 (endpoints), §5 (portal UI), and BUILD_PROMPT phases.

- **`SigningStrategy` interface**, two impls: `WalletStrategy` (wagmi v2 + viem 2 + Reown AppKit; MetaMask + WalletConnect v2) and `BackendStrategy` (Alloy backend custody). Mutually exclusive, switchable anytime via a **Settings radio toggle persisted server-side**.
- **Merkle-root/wrapped-doc building is ALWAYS server-side (shared SDK) — identical in both modes.**
- `POST /credentials/prepare` → returns unsigned tx `{to, data, value, chainId:135}` (wallet mode) OR signs+broadcasts itself (backend mode).
- `POST /credentials/confirm` → backend **re-verifies on-chain** (`RootIssued` event + `issuedAt[root] != 0`) before marking the record issued — a lying/buggy frontend cannot fake issuance. Records store `{signingMode, signerAddress}` as audit metadata only.
- **Chain add**: viem `defineChain` (ROAX, PLASMA, RPC, explorer); `useSwitchChain` → `wallet_switchEthereumChain`, fallback `wallet_addEthereumChain` (EIP-3085, `chainId:'0x87'`) on error `4902`.
- **Whitelist supports MULTIPLE addresses per issuer entity** (one-to-many issuer→signers). Invariant: the **active signer must be `isWhitelistedFor(recordType, signer)`**. Switching modes is an onboarding event: new address → admin approval queue → `whitelistFor` → poll until live. Pre-flight `eth_call` to fail fast (wallet mode = user pays gas). Delist inactive-mode addresses (avoid stale over-broad whitelist). Backend key rotation = new address to whitelist.
- **UX/status panel**: wallet mode shows connected address + ROAX-chain check + whitelist badge; backend mode shows genesis state + PLASMA balance. Switching affects only future signing; in-flight prepared drafts get re-validated. Gas: wallet = user's PLASMA; backend = clinic's funded key.

## 4. Mobile self-custodial wallet — Telegram-style (research 08 B)

Apply to arch §10 (mobile) + new wallet section, §5 verify (ownership), §9 model; impl §6 (wallet module + import verification); BUILD_PROMPT.

- **Under Settings**, like Telegram's TON Space. Default: **embedded MPC wallet** (MetaMask Embedded Wallets / Privy — real TSS, social/passkey login, no seed-phrase UX for non-crypto owners). Offer **raw BIP-39 self-custody export** (web3j 4.12.x / web3swift 3.3.2) as advanced.
- **Storage**: encrypt-then-store — seed/secret encrypted by Secure Enclave (iOS) / StrongBox (Android) hardware key, **biometric-gated**.
- Show address + PLASMA balance, send/receive; dApp connect via **Reown WalletKit** (both platforms).
- **DogTag SBT owned by the user's self-custodial address.** (Update arch §4.2/§4.1 mint to mint to the user's wallet address.)
- **Import verification (4 checks)**: (1) recompute `targetHash` + merkle membership offline; (2) `DogTagIssuer.isValid(merkleRoot)` via RPC; (3) DNS+registry identity; (4) **`DogTagSBT.ownerOf(dogTagId) == myWalletAddress`** — a record imports as "yours" only when the on-chain owner is the address you control. Add **`ownership`** as a verification fragment in arch §5 / impl §1.7 (tri-state).
- **Claim/transfer** (soulbound): admin **burn-and-remint** authorised by the user's signature proving control of the destination address.

## 5. Light/dark theme — vet & groomer web apps (NEW)

Apply to arch §10/portals + impl §5 + `packages/ui`.

- `packages/ui` semantic tokens gain **light + dark** palettes; a persisted theme toggle in each portal (vet, groomer, admin). Matches the groomer reference aesthetic (dark sidebar / light content) but as a real user-switchable light/dark mode.
- The **mobile** app keeps its 7 color themes (black/white/blue/red/pink/green/yellow) each with light+dark — unchanged. Portals are light/dark only (not the 7 colorways).

## 6. Doc-update tasks summary (per file)

**architecture.md**: rewrite §3.6 schemas to the finalized field sets; add Owner/Consent to §9; rewrite §6 as "Signing modes (dual, switchable)"; change §4.1/§4.2 SBT to be owned by user wallet + multi-address whitelist note in §4.3; add `ownership` pillar to §5; add a "Mobile wallet (Settings)" subsection to §10 + portal light/dark; expand §11 privacy (on-chain minimization, salt-as-erasure-lever, DPIA, CCPA/GDPR delete, evidentiary posture, trust tiers); extend §13 with the new normative items (dual-signing confirm re-verification, ownerOf import check, PII-off-chain absolute rule, multi-address whitelist, MPC wallet storage).

**implementation.md**: finalized schema + coded-value validators (§1.6/§11.5); `prepare`/`confirm` endpoints + SigningStrategy (§3); multi-address whitelist + chain-add calldata; `ownership` fragment in `verify` (§1.7/§11.3); mobile wallet module + 4-check import verification (§6); consent/retention/erasure + CCPA-delete endpoints (§4); portal light/dark + wallet-connect UI + signing toggle (§5); contract note: mint SBT to user wallet, `ownerOf` used by verifier; extend §11 remediations.

**BUILD_PROMPT.md**: add the dual-signing + wallet work to the phase plan (Phase 3 gets `prepare`/`confirm` + SigningStrategy; new wallet sub-phase in Phase 6 for mobile MPC wallet + ownerOf import; Phase 5 gets portal light/dark + wallet-connect); add privacy/erasure + PII-off-chain to the non-negotiable principles and Phase 8 gates; add finalized-schema-first note to Phase 1.
