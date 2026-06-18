# DogTag — ROAX Deployment Runbook

> How to deploy the DogTag contract set to **ROAX** (chainId **135**, gas token **PLASMA**) and bring
> up the three self-hostable stacks. Source of truth: `docs/implementation.md` §7 (Docker), §8
> (deploy), §11.8/§11.10 (verification subsystem). **Security gates are blocking** — do not deploy
> until the Gate B prechecks pass.

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
# Groth16Verifier is wired LATER via the registry timelock (after the trusted-setup ceremony, §4).
export ADMIN=0x<protocol-multisig>
export PRIVATE_KEY=0x<deployer-key>

forge script script/Deploy.s.sol:Deploy \
  --rpc-url $ROAX_RPC --chain 135 \
  --private-key $PRIVATE_KEY --broadcast -vvvv   # add --legacy if ROAX rejects EIP-1559
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
2. **Set the Groth16 verifier — timelocked.** `ZK_VERIFIER` was deployed as `address(0)`. After the
   ceremony (§4), the protocol admin sets the real `Groth16Verifier` through the **2-day timelock**:
   ```solidity
   VerificationRegistry.proposeZkVerifier(groth16VerifierAddr);   // starts ZK_TIMELOCK = 2 days
   // ... wait >= 2 days ...
   VerificationRegistry.executeZkVerifier();                      // activates it
   ```
   Until then, only the **normal ECDSA path** is live; the ZK path reverts (no verifier).

## 4. Trusted-setup ceremony (PRODUCTION REQUIREMENT — BLOCKING for the ZK path)

The Groth16 circuit (`circuits/`) needs a per-circuit phase-2 trusted setup. **The dev `.zkey` shipped
for tests is NOT production** — using it would let anyone forge ZK proofs.

Production requirements:

- **Phase 1:** reuse the **Hermez / Perpetual Powers of Tau** (`.ptau`) — do not run phase 1 yourself.
- **Phase 2:** a **multi-party contribution with ≥ 3 independent contributors**, ending in a **public
  random beacon** (e.g. a future drand round / block hash) so no single party knows the toxic waste.
- **Publish** the full contribution transcript and **pin the `.zkey` hash** (the prover image and CI
  assert this exact hash; the API loads the real prover only when `CIRCUITS_BUILD_DIR` is set).
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
| **admin** (central) | **39741** | **39742** | internal only (compose-net `39743`) |
| **vet** | **41873** | **41874** | internal only (compose-net `41875`) |
| **groomer** | **43617** | **43618** | internal only (compose-net `43619`) |

The **groomer** stack has **no separate api crate** — its `api` service runs the **`vet-api` binary**
with `BUSINESS_TYPE=groomer` (host `43618` → container `41874`).

## 6. Post-up custody bring-up (per business stack)

The vet/groomer api boots **locked**. Via the operator/admin portal (custody routes are
localhost/session-bound, `/admin/*`):

1. `POST /admin/genesis/start` → `/admin/genesis/confirm` (24-word BIP-39, age-encrypted seed at
   `KEYSTORE_PATH=/data/seed.age`).
2. `POST /admin/unlock` on each boot (rate-limited).
3. Apply for whitelisting (relayed to central → DNS-TXT check → on-chain `whitelistFor`); poll until
   the signer is live before issuing.
