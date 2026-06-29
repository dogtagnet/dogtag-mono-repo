# DogTag — ROAX Deployment Runbook

> How to deploy the DogTag contract set to **ROAX** (chainId **135**, gas token **PLASMA**) and bring
> up the three self-hostable stacks. Source of truth: `docs/implementation.md` §7 (Docker), §8
> (deploy), §11.8/§11.10 (verification subsystem). **Security gates are blocking** — do not deploy
> until the Gate B prechecks pass.

> **New deployment docs:** start at [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md) (index + tier decision-guide).
> For go-live hardening see [`docs/PRODUCTION_DEPLOYMENT.md`](./PRODUCTION_DEPLOYMENT.md); for building +
> installing the phone apps see [`docs/MOBILE_BUILD.md`](./MOBILE_BUILD.md).

> **ALREADY DEPLOYED.** The full contract set is **live on ROAX (chainId 135)** with the **ZK path
> wired** — addresses below and in `contracts/deployments/roax.json`. This runbook is the reproducible
> procedure; to just run the live demo see `docs/DEMO.md` / `docs/DEMO_CLICKS.md`.
>
> Snapshot — authoritative copy is `contracts/deployments/roax.json`.
>
> | Contract | Address |
> |---|---|
> | IssuerRegistry | `0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c` |
> | DogTagSBT | `0x1FB8986573Ac36d532cF7d5a5352202B094D4233` |
> | DogTagIssuerFactory | `0xd3179AbBfb0274D0a5F7017d76015A93C159511D` |
> | DogTagIssuerImpl | `0x16671686a5926606aB05f5e167fC65B0f8825B85` |
> | **ConsentKeyRegistry** (current; gasless `bindConsentKeyFor`) | `0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671` |
> | Poseidon6 | `0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db` |
> | **VerificationRegistry** (current; ZK-wired) | `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` |
> | Groth16Verifier | `0x138b433071Ad806E841B5AD53623290a9bf21761` |
> | admin / deployer | `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96` |
> | demo clone VACCINATION | `0x5c703910111f942EE0f47E02214291b5274cDb53` |
> | demo clone DOG_PROFILE | `0xdb8d39eb83DDFAaA7481C4Af4e47D0044116dB25` |
> | ~~VerificationRegistry~~ `_preMetaTx_legacy` (RETIRED) | `0x19C1B5f80c41EE864149500bdF998Dd18aec2a43` |
> | ~~VerificationRegistry~~ `_zk0_legacy` (RETIRED) | `0xb4FbbDb50D86c5208D9278413ca05c5eE309b1e8` |
> | ~~ConsentKeyRegistry~~ `_preMetaTx_legacy` (RETIRED) | `0xFD277b9B33a4b299fe0b08dfA19eA0372b70745b` |
>
> There are **THREE VerificationRegistry generations**. Current VR = `0x8bA836eCe9…` and current CKR =
> `0xA74DDe4a9b…` (the meta-tx migration is LIVE — gasless `bindConsentKeyFor`). RETIRED:
> `0x19C1B5f8…` (`_preMetaTx_legacy` VR), `0xb4FbbDb5…` (`_zk0_legacy` VR, deployed with `zkVerifier = 0`),
> and `0xFD277b9B…` (`_preMetaTx_legacy` CKR). See §3.2 for how the ZK verifier was wired (testnet redeploy)
> and the meta-tx migration vs the production timelock path.

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

# ADMIN = the protocol multisig (becomes DEFAULT_ADMIN of the registries + SBT). Default = broadcaster.
# ZK_VERIFIER intentionally defaults to address(0): the ECDSA "normal" path needs no verifier, and the
# Groth16Verifier is wired LATER (§3.2) — prod via the registry timelock, testnet via redeploy (done on ROAX).
export ADMIN=0x<protocol-multisig>
export PRIVATE_KEY=0x<deployer-key>

forge script script/Deploy.s.sol:Deploy \
  --rpc-url $ROAX_RPC --chain 135 \
  --private-key $PRIVATE_KEY --broadcast -vvvv --legacy   # ROAX needs LEGACY gas (EIP-1559 txs are accepted but never mined)
```

Deployed set (order in `Deploy.s.sol`): `IssuerRegistry` → `DogTagIssuer` (clone impl) →
`DogTagIssuerFactory` → `DogTagSBT` → `ConsentKeyRegistry` → `PoseidonT6` (circomlib-exact creation
bytecode, `POSEIDON6_INITCODE`) → `VerificationRegistry`.

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
2. **Wire the Groth16 verifier.** The first registry was deployed with `ZK_VERIFIER = address(0)`
   (only the normal ECDSA path live; ZK reverts). There are two ways to activate the ZK path:

   **(a) Production — 2-day timelock (the safe, canonical path).** After the ceremony (§4), the protocol
   admin sets the real `Groth16Verifier` through the registry's **2-day timelock**:
   ```solidity
   VerificationRegistry.proposeZkVerifier(groth16VerifierAddr);   // starts ZK_TIMELOCK = 2 days
   // ... wait >= 2 days ...
   VerificationRegistry.executeZkVerifier();                      // activates it
   ```

   **(b) Testnet — redeploy (what we did on ROAX).** Rather than wait out the 2-day timelock on testnet,
   the **VerificationRegistry was REDEPLOYED** with the live `Groth16Verifier` (`0x138b4330…`) wired in
   at construction, so the ZK path is active immediately. That ZK-wired redeploy was `0x19C1B5f8…`; the
   original zk=0 instance is kept as `VerificationRegistry_zk0_legacy` `0xb4FbbDb5…`.

   **(c) Meta-tx migration (the CURRENT generation).** `0x19C1B5f8…` is itself now legacy
   (`VerificationRegistry_preMetaTx_legacy`). A later **meta-tx migration** produced the **current VR
   `0x8bA836eCe9…`** plus the **current CKR `0xA74DDe4a9b…`**, enabling the gasless `bindConsentKeyFor`
   path that is now LIVE. So there are **THREE VR generations** — `0xb4FbbDb5…` (zk0) → `0x19C1B5f8…`
   (preMetaTx) → `0x8bA836eCe9…` (current) — and `0x19C1B5f8…` is NOT the current registry.

   The testnet trusted setup (3 contributions + beacon) is recorded in `docs/CEREMONY_TRANSCRIPT.md`
   (zkey sha256 `45d0b6fb…`); the on-chain wiring + the prod timelock procedure are in `docs/CEREMONY_RUNBOOK.md`
   (concise version: `docs/CEREMONY.md`).

## 4. Trusted-setup ceremony (PRODUCTION REQUIREMENT — BLOCKING for the ZK path)

The Groth16 circuit (`circuits/`) needs a per-circuit phase-2 trusted setup. **The dev `.zkey` shipped
for tests is NOT production** — using it would let anyone forge ZK proofs.

Production requirements:

- **Phase 1:** reuse the **Hermez / Perpetual Powers of Tau** (`.ptau`) — do not run phase 1 yourself.
- **Phase 2:** a **multi-party contribution with ≥ 3 independent contributors**, ending in a **public
  random beacon** (e.g. a future drand round / block hash) so no single party knows the toxic waste.
- **Publish** the full contribution transcript and **pin the `.zkey` hash** — the prover binary itself
  **enforces** it at load (fail-closed on a hash mismatch — audit M4), not just CI. The crate's hardcoded
  pin is the testnet hash, so a production ceremony zkey needs `EXPECTED_ZKEY_SHA256` set to its sha256;
  the API loads the real prover only when `CIRCUITS_BUILD_DIR` is set.
- **Generate** `Groth16Verifier.sol` via `snarkjs zkey export solidityverifier`, deploy it, then wire
  it through the timelock (§3.2).

**Production circuit note:** the circuit is **N=24 max leaves, depth 5 (24→12→6→3→2→1) with
odd-promotion** exactly matching the SDK `buildMerkle` (commutative sorted-pair, single-leaf root,
lone-odd promotion). This is already in place in `circuits/verification.circom`; the parity is gated by
the Poseidon 4-language anchor vectors (t=2/3/6/7) and the in-circuit-root == SDK-`R` test.

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
