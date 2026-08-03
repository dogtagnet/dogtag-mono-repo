# DogTag — ROAX Deployment Runbook

> How to deploy the DogTag contract set to **ROAX** (chainId **135**, gas token **PLASMA**) and bring
> up the three self-hostable stacks. Source of truth: `docs/implementation.md` §7 (Docker), §8
> (deploy), §11.8/§11.10 (verification subsystem). **Security gates are blocking** — do not deploy
> until the Gate B prechecks pass.

> **New deployment docs:** start at [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md) (index + tier decision-guide).
> For go-live hardening see [`docs/PRODUCTION_DEPLOYMENT.md`](./PRODUCTION_DEPLOYMENT.md); for building +
> installing the phone apps see [`docs/MOBILE_BUILD.md`](./MOBILE_BUILD.md).

> **ALREADY DEPLOYED.** The contract set is **deployed on ROAX (chainId 135)**; every address is in
> `contracts/deployments/roax.json`.
> There is one set and one owner-hidden model - the ledger carries no retired generation, and no
> contract in it is superseded by another.
> This runbook is the reproducible procedure; to just run the live demo see `docs/DEMO.md` /
> `docs/DEMO_CLICKS.md`.
>
> Snapshot - the authoritative copy is `contracts/deployments/roax.json`, and the addresses are
> deliberately not transcribed here.
> This block used to carry the full table, and every entry in it went stale at once when the launch
> set was deployed; a runbook that names superseded addresses is worse than one that names none,
> because it reads as a checked fact.
>
> What is deployed is one set of ten contracts, deployed by a single run of
> `contracts/script/Deploy.s.sol`: `ProviderRegistry`, `DogTagIssuer` (the clone implementation),
> `DogTagIssuerFactory`, `DogTagSBTConsent`, `Groth16VerifierConsent`,
> `VerificationRegistryConsent`, `ProviderDirectory`, `ServiceDomainResolver` and `ProtocolRegistry`.
> Read the ledger's `_roles`, `_root_index`, `_frozen_verifier` and `_provisioning` notes before
> deploying anything against it - between them they record which key must broadcast, why replacing
> the factory means replacing the verification registry too, that no zero-knowledge artifact rotated,
> and that no provider is onboarded yet.

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

**`Deploy.s.sol` stands up the whole system in one run** - there is no longer a separate script per
layer, and `DeployCustodialIssuance.s.sol` / `DeployProtocolRegistry.s.sol` no longer exist.
Order: `ProviderRegistry` → `DogTagIssuer` (clone impl) → `DogTagIssuerFactory` → `DogTagSBTConsent`
→ `Groth16VerifierConsent` → `VerificationRegistryConsent` → `ProviderDirectory` →
`ServiceDomainResolver` → `ProtocolRegistry`, followed by three `onlyOwner` registrar-wiring calls -
`addFactoryGeneration`, then `setResolverApproved` for the directory and the domain resolver.

**The broadcasting key must be the `admin` the script hands the core to**, because that wiring is
`onlyOwner` on a core the script has just transferred; a different key deploys nine contracts and
then reverts on the tenth transaction.

`PublishProtocolVersions.s.sol` (two-phase, timelocked) and `PinConsentWitnessGraph.s.sol` are the
only other scripts, and both are operational rather than part of standing the system up - see
`docs/PROTOCOL_REGISTRY_RUNBOOK.md`.

### Verify on Blockscout

```bash
forge verify-contract --rpc-url $ROAX_RPC \
  --verifier blockscout --verifier-url https://explorer.roax.net/api/ \
  <ADDRESS> src/DogTagIssuer.sol:DogTagIssuer
# repeat per contract; addresses are in deployments/roax.json
```

## 3. Post-deploy wiring

1. **Onboard the provider (registrar, `onlyOwner` on `ProviderRegistry`).** `whitelistFor` no longer
   exists anywhere; authority is service-scoped now, and it is a SEQUENCE rather than one call:
   `registerProvider` (which writes standing `PENDING`, so this alone leaves the provider inert) →
   `setProviderStanding(providerId, ACTIVE)` → `setServiceCreationApproval(providerId, recordType,
   true)`. The provider then deploys its own clone from the factory, and the registrar completes it
   with `attachService`, `confirmServiceOwner` and `setIssuanceCapability(service, signer, true)`.
   Verify capability is a separate axis: `setVerifierCapability(purpose, relayer, true)` - an issuer
   is not implicitly a verifier.
2. **Wire the Groth16 verifier.** `VerificationRegistryConsent` is deployed with its
   `Groth16VerifierConsent` wired in at construction by `Deploy.s.sol`, so no separate activation
   step exists.
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
