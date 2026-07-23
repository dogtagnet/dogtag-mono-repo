# DogTag — ROAX Deployment Runbook

> How to deploy the DogTag contract set to **ROAX** (chainId **135**, gas token **PLASMA**) and bring
> up the three self-hostable stacks. Source of truth: `docs/implementation.md` §7 (Docker), §8
> (deploy), §11.8/§11.10 (verification subsystem). **Security gates are blocking** — do not deploy
> until the Gate B prechecks pass.

> **New deployment docs:** start at [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md) (index + tier decision-guide).
> For go-live hardening see [`docs/PRODUCTION_DEPLOYMENT.md`](./PRODUCTION_DEPLOYMENT.md); for building +
> installing the phone apps see [`docs/MOBILE_BUILD.md`](./MOBILE_BUILD.md).

> **ALREADY DEPLOYED.** The contract set is **deployed on ROAX (chainId 135)** - addresses below and in
> `contracts/deployments/roax.json`; the live protocol surface is the **owner-hidden consent set**
> (`DogTagSBTConsent` + `VerificationRegistryConsent` + `Groth16VerifierConsent`).
> The retired owner-revealing contracts remain in the ledger only as historical records.
> This runbook is the reproducible procedure; to just run the live demo see `docs/DEMO.md` /
> `docs/DEMO_CLICKS.md`.
>
> Snapshot — authoritative copy is `contracts/deployments/roax.json`.
>
> | Contract | Address |
> |---|---|
> | IssuerRegistry | `0xAEE540350292E49A9AeDf19Dd4C3BAc6ABeE6c21` |
> | DogTagSBT (RETIRED owner-revealing SBT; source deleted; historical reads only) | `0x1FB8986573Ac36d532cF7d5a5352202B094D4233` |
> | DogTagSBTConsent (**the live owner-hidden SBT**; write-once `profileRoot`; minted via `POST /profiles/issue/custodial-bind` → `mintCustodial(dogTagId, R)`) | `0xBEbc45A838643D27004827b797b30A464b2b02c0` |
> | DogTagIssuerFactory | `0xED20269E3eBF0119739aaB5258741F3aEb49F140` |
> | DogTagIssuerImpl | `0xe4aC139eB257C309Ec448C116A6F657Dab5590BA` |
> | ProtocolRegistry (two-axis discovery anchor; zero-timelock testnet instance; `dogtag-levelb/1` published + active) | `0xf5492A671E69b1A13f7Fd123C021830eB1ea8081` |
> | ConsentKeyRegistry (RETIRED; the consent key is now a per-tag leaf inside the tree - no on-chain key registry) | `0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671` |
> | Poseidon6 (deployed with the retired owner-revealing set; historical) | `0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db` |
> | VerificationRegistry (RETIRED owner-revealing registry; source deleted; final instance kept for historical reads) | `0x4E2f0996e1CB4E24F1053346f3da2186906835E8` |
> | VerificationRegistryConsent (**the live owner-hidden registry**; 4-arg `recordVerificationZK`; owner-blind `Verified`) | `0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B` |
> | ~~VerificationRegistryConsent~~ `_M4_mutableRoot_legacy` (**DEPRECATED / DO NOT USE for Level-B**; never live; zero `Verified`) | `0x53F988Ae0124b96069d90CBC78E6245FeB01E125` |
> | ~~VerificationRegistryConsent~~ `_preErasureGate_legacy` (RETIRED; lacks the erasure gate, never live) | `0x57A2998668B0F6332f7342016F5Df2Bb05cB900F` |
> | Groth16Verifier (RETIRED; paired with the retired verification circuit) | `0xEEFCfAF026931b7325472A88fd14Ee780Da13559` |
> | ~~Groth16Verifier~~ `_v1_legacy` (RETIRED) | `0x138b433071Ad806E841B5AD53623290a9bf21761` |
> | Groth16VerifierConsent (**the live consent verifier**; wired into the registry above) | `0x1A9027986B859dc3879896B053deA78F636BE9b1` |
> | deployer EOA (genesis; governance/admin authority removed; legacy issuer/whitelist capabilities remain, so **not a neutral custodian**) | `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96` |
> | **governance authority / admin** (signer-1; live since Phase-2) | `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` |
> | demo clone VACCINATION | `0x1456f93f7376789c46408CC4616751eB853edD9A` |
> | demo clone DOG_PROFILE | `0x0e56Ae2e1ef684d3e90d7699B981C6B76df922bf` |
> | ~~VerificationRegistry~~ `_4arg_legacy` (RETIRED) | `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` |
> | ~~VerificationRegistry~~ `_preMetaTx_legacy` (RETIRED) | `0x19C1B5f80c41EE864149500bdF998Dd18aec2a43` |
> | ~~VerificationRegistry~~ `_zk0_legacy` (RETIRED) | `0xb4FbbDb50D86c5208D9278413ca05c5eE309b1e8` |
> | ~~ConsentKeyRegistry~~ `_preMetaTx_legacy` (RETIRED) | `0xFD277b9B33a4b299fe0b08dfA19eA0372b70745b` |
>
> There are **FOUR generations of the retired VerificationRegistry** - `0xb4FbbDb5…` (`_zk0_legacy`,
> deployed with `zkVerifier = 0`), `0x19C1B5f8…` (`_preMetaTx_legacy`), `0x8bA836eCe9…` (`_4arg_legacy`),
> and the final `0x4E2f0996…` - plus the retired CKRs `0xA74DDe4a9b…` (final) and `0xFD277b9B…`
> (`_preMetaTx_legacy`).
> The whole owner-revealing line is retired; none of these is a live write target.
> See §3.2 for that wiring history and the live consent registry's timelock path.

---

## 1. Gate B prechecks (BLOCKING)

The ROAX dev RPC returned 502 at design time — confirm liveness **before** broadcasting.

```bash
export ROAX_RPC=https://devrpc.roax.net

# (a) chain-id MUST be 135
cast chain-id --rpc-url $ROAX_RPC            # expect: 135

# (b) BN254 pairing precompiles present (0x06 add, 0x07 mul, 0x08 pairing) — required by the
#     Groth16 verifier and on-chain Poseidon. A non-empty, non-reverting return confirms support.
cast call 0x0000000000000000000000000000000000000006 0x --rpc-url $ROAX_RPC   # ecAdd
cast call 0x0000000000000000000000000000000000000007 0x --rpc-url $ROAX_RPC   # ecMul
cast call 0x0000000000000000000000000000000000000008 0x --rpc-url $ROAX_RPC   # ecPairing
```

If `cast chain-id` is not 135 or the precompiles are unavailable, **stop**.

## 2. Deploy the contracts

`contracts/foundry.toml` pins `evm_version = "paris"` and a pinned `solc`. Deploy with `Deploy.s.sol`
(writes `contracts/deployments/roax.json`).

```bash
cd contracts

# ADMIN = the protocol multisig (becomes DEFAULT_ADMIN of the shared base). Default = broadcaster.
export ADMIN=0x<protocol-multisig>
export PRIVATE_KEY=0x<deployer-key>

forge script script/Deploy.s.sol:Deploy \
  --rpc-url $ROAX_RPC --chain 135 \
  --private-key $PRIVATE_KEY --broadcast -vvvv --legacy   # ROAX needs LEGACY gas (EIP-1559 txs are accepted but never mined)
```

Deployed set (order in `Deploy.s.sol`): `IssuerRegistry` → `DogTagIssuer` (clone impl) →
`DogTagIssuerFactory` — **the shared base only**. The owner-hidden stack (`Groth16VerifierConsent` →
`DogTagSBTConsent` → `VerificationRegistryConsent`) is deployed separately by
`DeployCustodialIssuance.s.sol`, which wires the consent verifier into the registry at construction.
The on-chain `ProtocolRegistry` discovery anchor has its own script, `DeployProtocolRegistry.s.sol`
(see `docs/PROTOCOL_REGISTRY_RUNBOOK.md`).
The retired owner-revealing contracts (`DogTagSBT`/`ConsentKeyRegistry`/`PoseidonT6`/`VerificationRegistry`)
are no longer deployed by any script; their earlier instances remain in the deployment ledger for
historical reads.

### Verify on Blockscout

```bash
forge verify-contract --rpc-url $ROAX_RPC \
  --verifier blockscout --verifier-url https://explorer.roax.net/api/ \
  <ADDRESS> src/DogTagIssuer.sol:DogTagIssuer
# repeat per contract; addresses are in deployments/roax.json
```

## 3. Post-deploy wiring

1. **Whitelist issuers (admin).** Each issuer entity is approved per `recordType` after **DNS-TXT
   verification** of its `DEPLOYMENT_DOMAIN`. The central admin flow triggers the on-chain
   `whitelistFor(recordType, signer)` — the registry supports **multiple signer addresses per issuer
   entity** (one-to-many). Delist inactive-mode addresses.
2. **Wire the Groth16 verifier.** The live `VerificationRegistryConsent` is deployed with its
   `Groth16VerifierConsent` wired in at construction (`DeployCustodialIssuance.s.sol`), so no separate
   activation step exists.
   A later verifier swap (e.g. after a production ceremony) goes through the registry's **2-day timelock**:
   ```solidity
   VerificationRegistryConsent.proposeZkVerifier(newVerifierAddr);   // starts ZK_TIMELOCK = 2 days
   // ... wait >= 2 days ...
   VerificationRegistryConsent.executeZkVerifier();                  // activates it
   ```

   > **Historical - the retired owner-revealing registry's wiring generations.** The first
   > VerificationRegistry was deployed with `ZK_VERIFIER = address(0)` (ZK calls reverted); a testnet
   > **redeploy** (`0x19C1B5f8…`) wired in the then-live v1 verifier at construction, retiring the zk=0
   > instance as `_zk0_legacy` `0xb4FbbDb5…`; a **meta-tx migration** produced VR `0x8bA836eCe9…` plus
   > CKR `0xA74DDe4a9b…` (gasless `bindConsentKeyFor`); and a **registry-only redeploy** produced the
   > final VR `0x4E2f0996…` after `0x8bA836eCe9…` turned out to dispatch only the old 4-arg
   > `recordVerificationZK` selector (`0xdd080593`) and bare-revert the 6-arg call (`0x423a45b6`).
   > Four VR generations in all; the entire line is retired, and its testnet trusted setup stays
   > recorded in `docs/CEREMONY_TRANSCRIPT.md` as provenance.

   The live consent-circuit trusted setup (public Hermez ptau + a single contribution + drand beacon, zkey
   sha256 `f83a111f…`) is recorded in `docs/CEREMONY_TRANSCRIPT.consent.md`.
   The prod ceremony + timelock procedure are in `docs/CEREMONY_RUNBOOK.md` (concise version:
   `docs/CEREMONY.md`) and `docs/PRODUCTION_DEPLOYMENT.md` §3.2.

## 4. Trusted-setup ceremony (PRODUCTION REQUIREMENT — BLOCKING for the ZK path)

> **RETIRED / HISTORICAL - not runnable.**
> This section covered the retired owner-revealing `verification.circom` ceremony + verifier deploy.
> That circuit, its ceremony scripts (`scripts/ceremony.sh`, `scripts/setup.sh`), the
> `npm run compile-circuit` / `npm run build-circuit` entry points, and the
> `Groth16Verifier`/`VerificationRegistry` contract sources were all removed when the owner-revealing
> layer was retired; only its transcript (`docs/CEREMONY_TRANSCRIPT.md`) and frozen build products are
> kept as provenance for the already-deployed instances.
> The live ceremony is the **consent-circuit** one: `circuits/scripts/ceremony-consent.sh`, transcript
> `docs/CEREMONY_TRANSCRIPT.consent.md`, production runbook `docs/CEREMONY_RUNBOOK.md` (concise version
> `docs/CEREMONY.md`).
> The production requirements (Hermez ptau phase 1, ≥ 3 independent phase-2 contributors ending in a
> public random beacon, published transcript, pinned zkey hash enforced by the prover) live in those
> docs and still BLOCK a production ZK go-live.

## 5. Bring up the stacks (Docker)

Each stack is `web` (nginx serving the Vite build) + `api` (Rust) + `mongo` (**internal to the compose
network only — NEVER published to the host**). Build context for all images is the **monorepo root**
(the web SPAs consume the pnpm workspace; the Rust crates path-depend on the workspace).

```bash
# from the repo root — copy + fill each stack's env first:
cp stacks/admin/.env.example   stacks/admin/.env     # fill ISSUER_REGISTRY_ADDR / SBT_ADDR / secrets
cp stacks/vet/.env.example     stacks/vet/.env
cp stacks/groomer/.env.example stacks/groomer/.env

make up-admin     # central : web 39741, api 39742  (cd stacks/admin && docker compose up -d)
make up-vet       # vet     : web 41873, api 41874
make up-groomer   # groomer : web 43617, api 43618  (vet-api binary, BUSINESS_TYPE=groomer)
```

### Port map (host)

| Stack | web | api | mongo |
|---|---|---|---|
| **admin** (central) | **39741** | **39742** | `27017` internal-only (never published) |
| **vet** | **41873** | **41874** | `27017` internal-only (never published) |
| **groomer** | **43617** | **43618** | `27017` internal-only (never published) |

The **groomer** stack has **no separate api crate** — its `api` service runs the **`vet-api` binary**
with `BUSINESS_TYPE=groomer` (host `43618` → container `43618`).

## 6. Post-up custody bring-up (per business stack)

The vet/groomer api boots **locked**. Via the operator/admin portal (custody routes are
localhost/session-bound, `/admin/*`):

1. `POST /admin/genesis/start` → `/admin/genesis/confirm` (24-word BIP-39). Custody persists as a
   `CustodyBlob` in **Mongo** (back up the stack's mongo volume). The `KEYSTORE_PATH`/`seed.age` volume
   is **DEAD CODE** — nothing is written to `/data/seed.age`.
2. `POST /admin/unlock` on each boot (rate-limited).
3. Apply for whitelisting (relayed to central → DNS-TXT check → on-chain `whitelistFor`); poll until
   the signer is live before issuing.
