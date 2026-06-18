# @dogtag/admin-web

Admin portal (impl §5.3). Vite + React 18 + React Router, built on `@dogtag/ui`. Talks to the
**central** backend (`stacks/admin/api`, port 39742) via the shared `createCentralClient`.

## Dev

```bash
cp .env.example .env.local      # set VITE_REOWN_PROJECT_ID, VITE_ISSUER_REGISTRY_ADDR etc.
pnpm --filter @dogtag/admin-web dev
```

- Dev server: **http://localhost:39741** (`server.port` = 39741, strict).
- `/api` proxies to the central backend (default `http://localhost:39742`, override with
  `VITE_CENTRAL_API_PROXY`).
- `pnpm --filter @dogtag/admin-web build` runs `tsc --noEmit && vite build`.

## Pages

- **Login** — admin session via `POST /v1/admin/login`.
- **Dashboard** (`/dashboard`) — live registry/application counts; appointment + observability
  panels are placeholders.
- **Business registry** (`/businesses`) — list via `GET /v1/businesses` (public discovery), register
  via `POST /v1/businesses` (admin-gated; the HMAC secret is shown once). Geo coords are rendered in
  the table; a map is intentionally omitted (no map dependency) — lat/lng are shown instead.
- **Issuer applications** (`/applications`) — list via `GET /v1/issuer-applications`; **approve**
  (`/{id}/approve` → on-chain `whitelistFor(keccak256(recordType), address)` per (address × recordType)
  after DNS-TXT + accreditation checks), **reject** (`/{id}/reject`), **delist** (`/{id}/delist` →
  on-chain `delistFor`). Shows the multi-address × multi-recordType structure and the resulting tx
  hashes (explorer links).
- **Whitelist viewer** (`/whitelist`) — expands every (recordType, address) pair from the
  applications. The *derived* column reflects application status (approved ⇒ expected whitelisted).
  When `VITE_ISSUER_REGISTRY_ADDR` is set, the *on-chain* column reads
  `IssuerRegistry.isWhitelistedFor(keccak256(recordType), address)` directly via viem against ROAX.

## Wired vs placeholder

- **Wired to central backend contracts**: admin login, businesses list/register, issuer-application
  list/approve/reject/delist, whitelist viewer (derived from applications; live on-chain read via
  viem when the registry address is configured).
- **Placeholder**: appointment + observability dashboards (the central backend exposes only
  owner-scoped appointment/consent endpoints; an aggregate admin view is future work).

## Env

See `.env.example`. The central API base is proxied to `/api` by default. `VITE_ISSUER_REGISTRY_ADDR`
enables the live on-chain whitelist read; `VITE_ROAX_RPC` overrides the ROAX RPC URL.
