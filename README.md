# DogTag — Pet Credentialing Ecosystem (monorepo)

Verifiable, on-chain pet credentials (identity, vaccination, service, travel) anchored as salted-Merkle roots on the **ROAX** EVM chain (chainId 135, gas token PLASMA), verified three ways: cryptographic integrity + on-chain status + DNS-bound issuer identity. An OpenAttestation-style design, **implemented from scratch** with a JSON-free, language-agnostic (TS/Rust/Solidity) canonicalization.

## Start here
- **[`docs/architecture.md`](docs/architecture.md)** — system + smart-contract architecture. (§13 = normative audit remediations.)
- **[`docs/implementation.md`](docs/implementation.md)** — per-function pseudocode, contract bodies, endpoints, Docker, deploy. (§11 = normative corrected code.)
- **[`docs/BUILD_PROMPT.md`](docs/BUILD_PROMPT.md)** — the goal-setting, phased build-out prompt to drive coding.
- **[`docs/research/`](docs/research)** — 5 research briefs + 3 security audits backing every decision.

## Components
| Path | What | Runs where |
|---|---|---|
| `apps/android`, `apps/ios` | Pet-owner apps (Kotlin/Compose, Swift/SwiftUI), 7 themes | User devices |
| `stacks/vet`, `stacks/groomer` | Self-hosted business stacks (React+Vite SPA + Rust API + Mongo) | Each business |
| `stacks/admin` | Central registry, whitelisting, mobile API, appointment source-of-truth | We host |
| `contracts` | `DogTagSBT` (ERC-5192) · `IssuerRegistry` · `DogTagIssuer` (clones) · factory | ROAX |
| `crates/dogtag-standard-rs`, `packages/dogtag-standard-ts` | The open data standard: canonicalization + Merkle + verify | Shared (UniFFI → mobile) |
| `packages/ui` | Shared React components + theme tokens | Portals |

## Ports (uncommon; Mongo internal-only)
admin `39741`/`39742` · vet `41873`/`41874` · groomer `43617`/`43618`

## Status
Design + audit complete; remediations folded in. Next: execute `docs/BUILD_PROMPT.md`, Phase 1 (the cross-language trust core) first.
