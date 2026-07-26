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
  (USDA#/license# → central `POST /v1/issuer-applications`) → DNS-TXT instructions. Setup owns
  **genesis only**: an instance that is already sealed but merely locked (e.g. after a backend
  restart) is handed to `/unlock` instead of re-entering the wizard. The `confirm → unlock` step
  above is genesis continuation and stays.
- **Unlock** (`/unlock`) - the dedicated custody-unlock page. **Not a nav item**: it is a redirect
  target. Whenever the backend reports a locked seal - on load, or on the first action refused with
  `not unlocked` - the portal sends the operator here with `?next=` carrying where they were headed,
  and returns them there once custody is open. Enter the custody-admin password + passphrase (both
  prefilled in demo mode); a wrong passphrase shows an inline error and does **not** end the
  session. It links to Setup only when the instance has no seal at all (needs genesis, not unlock).
- **Issue a record** (`/issue`) — recordType picker → schema-driven form (§1.6 fields) with
  client-side validation → `POST /credentials/prepare`; **wallet mode** signs the `unsignedTx`
  with the connected wallet then `POST /credentials/confirm`, **backend mode** auto-confirms in
  prepare; shows txHash + "Show QR" (`POST /records/:id/share`).
- **Records** (`/records`) — lists the backend's OWN records DB (`GET /records`, operator-gated):
  status badges (issued/revoked/expired), the immutable on-chain proof (tx, block, contract) with a
  block-explorer link, edit off-chain label/notes (`PATCH /records/:id`), mark expired (off-chain
  status transition), re-generate QR, revoke (`POST /records/:id/revoke`, soft — the row + proof stay).
- **Traceability** (`/traceability`) — this business's on-chain credential activity (`GET /trace/activity`
  + `GET /trace/stats`, operator-gated), scoped server-side to its own signer(s)/clone(s) and joined to
  its own records: an "In scope" / "Matched to a record" summary strip, per-event type + finality badges,
  the matched local record highlighted, and block-explorer links — so an operator never sees another
  operator's activity. Reads the standalone oversight indexer (`INDEXER_API_BASE` + a scoped bearer); when
  the indexer is unset the page shows a first-class "Oversight indexer not connected" state.
- **Import from user** (`/import`) — scan prompt → `POST /import/pull` (off-chain, decoupled
  from Verify).
- **Verify** (`/verify`) — one owner-hidden consent flow (purpose → session QR → owner proof →
  on-chain status), with no disclosure choice. Boarding and travel vaccination checks are offered;
  `SERVICE_ATTESTATION` remains excluded because it is off-chain-only and cannot be verified through
  the on-chain consent registry.
- **Settings** (`/settings`) — signing-mode toggle (`PUT /settings/signing-mode`), status panel
  (`GET /issuer/signers`), theme toggle.

## Env

See `.env.example`. The Reown projectId is a placeholder by default — wallet-connect needs a
real one. The central API base is where the whitelist application is posted.

## Wired vs visual-only

- **Wired to backend contracts**: login, genesis/confirm/unlock/accounts, prepare/confirm
  (both modes), records list/edit/expire (`GET /records`, `PATCH /records/:id`), revoke,
  share QR, signing-mode get/put, issuer signers, import/pull, verify session start, central
  issuer-application apply, traceability feed (`GET /trace/activity`, `GET /trace/stats`; 503 →
  "indexer not connected" empty state).
- **Visual / partially wired**: the Verify flow renders the session QR and the
  awaiting-consent state and polls `GET /verify/session/:id` for the "pending → Verified"
  transition.

## E2E

With the dev server running, `pnpm --filter @dogtag/vet-web test:e2e` exercises the mocked portal
flows. `e2e/verify-consent.spec.ts` pins the single consent UI and asserts its session-start request
carries only the purpose and record type.
