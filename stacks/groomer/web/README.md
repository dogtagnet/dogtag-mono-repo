# @dogtag/groomer-web

Groomer portal (impl §5.2). Vite + React 18 + React Router, built on `@dogtag/ui`. The groomer
backend is **structurally identical to the vet backend** — same `routes.rs` contracts
(genesis/custody, prepare/confirm, records, import/pull, /verify/\*, settings).

## Dev

```bash
cp .env.example .env.local      # set VITE_REOWN_PROJECT_ID etc.
pnpm --filter @dogtag/groomer-web dev
```

- Dev server: **http://localhost:43617** (`server.port` = 43617, strict).
- `/api` proxies to the groomer backend (default `http://localhost:43618`, override with
  `VITE_GROOMER_API_PROXY`).
- `pnpm --filter @dogtag/groomer-web build` runs `tsc --noEmit && vite build`.

## Pages

Nav mirrors the reference groomer dashboard:

- **Dashboard** (`/dashboard`) — welcome + quick links into the realized DogTag flows.
- **Calendar / Appointments / Clients / Groomers / Reports / Marketing** — clean placeholders that
  mirror the reference UI (not wired in this build).
- **Import from user** (`/import`) — pull a customer's pet **profile** or **vaccination** via QR
  (`POST /import/pull`); the backend third-party-verifies on chain + DNS and the portal renders the
  **three authenticity pillars** verdict (integrity / issuance / identity, plus the contextual
  `ownership` fragment which is `NOT_APPLICABLE` for a third-party importer). Decoupled from Verify.
- **Records** (`/records`) — lists the backend's OWN records DB (`GET /records`, operator-gated):
  status badges (issued/revoked/expired), the immutable on-chain proof (tx, block, contract) with a
  block-explorer link, edit off-chain label/notes (`PATCH /records/:id`), mark expired, revoke
  (`POST /records/:id/revoke`, soft — the row + proof stay). Same page as the vet portal.
- **Verify** (`/verify`) — the shared `<VerifyFlow/>` (purpose + Normal/ZK toggle → session QR →
  on-chain status). Emphasizes that a groomer can verify a vet-issued vaccination **without being an
  issuer** (the `VERIFY:<purpose>` whitelist namespace, distinct from issuer roles).
- **Setup** (`/setup`) — the same genesis/custody wizard as the vet portal (groomers can issue their
  own records too): custody admin login → genesis (24 words → confirm + passphrase → unlock) →
  derive accounts → apply for whitelist (central `POST /v1/issuer-applications`) → DNS-TXT.
- **Settings** (`/settings`) — signing-mode toggle (`PUT /settings/signing-mode`), status panel
  (`GET /issuer/signers`), theme toggle.

## Wired vs placeholder

- **Wired to backend contracts**: login, genesis/confirm/unlock/accounts, records
  list/edit/expire/revoke (`GET /records`, `PATCH /records/:id`, `POST /records/:id/revoke`),
  signing-mode get/put, issuer signers, import/pull (with 3-pillar verdict render), verify
  session start, central issuer-application apply.
- **Placeholder**: Calendar, Appointments, Clients, Groomers, Reports, Marketing.
- **Note**: like the vet portal, the Verify flow shows the session QR + awaiting-consent state and
  polls `GET /verify/session/:id` for the "pending → Verified" transition.

## Env

See `.env.example`. The Reown projectId is a placeholder by default — wallet-connect needs a real
one. The central API base is where the whitelist application is posted.
