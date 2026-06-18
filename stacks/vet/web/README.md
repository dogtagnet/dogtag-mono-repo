# @dogtag/vet-web

Vet portal (impl §5.1). Vite + React 18 + React Router, built on `@dogtag/ui`.

## Dev

```bash
cp .env.example .env.local      # set VITE_REOWN_PROJECT_ID etc.
pnpm --filter @dogtag/vet-web dev
```

- Dev server: **http://localhost:41873** (`server.port` = 41873, strict).
- `/api` proxies to the vet backend (default `http://localhost:41874`, override with
  `VITE_VET_API_PROXY`).
- `pnpm --filter @dogtag/vet-web build` runs `tsc --noEmit && vite build`.

## Pages

- **Setup** (`/setup`) — wizard: custody admin login → genesis (show 24 words → confirm
  challenge words + passphrase → unlock) → derive accounts → apply for whitelist
  (USDA#/license# → central `POST /v1/issuer-applications`) → DNS-TXT instructions.
- **Issue credential** (`/issue`) — recordType picker → schema-driven form (§1.6 fields) with
  client-side validation → `POST /credentials/prepare`; **wallet mode** signs the `unsignedTx`
  with the connected wallet then `POST /credentials/confirm`, **backend mode** auto-confirms in
  prepare; shows txHash + "Show QR" (`POST /records/:id/share`).
- **Records** (`/records`) — status badges (issued/revoked), re-generate QR, revoke
  (`POST /records/:id/revoke`).
- **Import from user** (`/import`) — scan prompt → `POST /import/pull` (off-chain, decoupled
  from Verify).
- **Verify** (`/verify`) — the shared `<VerifyFlow/>` (purpose + Normal/ZK → session QR → status).
- **Settings** (`/settings`) — signing-mode toggle (`PUT /settings/signing-mode`), status panel
  (`GET /issuer/signers`), theme toggle.

## Env

See `.env.example`. The Reown projectId is a placeholder by default — wallet-connect needs a
real one. The central API base is where the whitelist application is posted.

## Wired vs visual-only

- **Wired to backend contracts**: login, genesis/confirm/unlock/accounts, prepare/confirm
  (both modes), revoke, share QR, signing-mode get/put, issuer signers, import/pull,
  verify session start, central issuer-application apply.
- **Visual / partially wired**: the Records list is backed by a local (localStorage) index
  because the vet backend exposes no "list records" endpoint. The Verify flow renders the
  session QR and the awaiting-consent state; there is no `GET /verify/session/:id` status
  endpoint in the backend, so the "pending → Verified" transition only advances when a poller
  is supplied to `<VerifyFlow/>` (left out here by design).
