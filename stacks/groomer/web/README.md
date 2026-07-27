# @dogtag/groomer-web

Groomer portal (impl §5.2). Vite + React 18 + React Router, built on `@dogtag/ui`. The groomer
backend is the **same `vet-api` binary** run with `BUSINESS_TYPE=groomer` — but that role does **not**
mount the issuance routes (`/credentials/*`, `/records/*`, `/r/{token}`, `/profiles/issue/*`,
`/p/{token}`), because a groomer verifies and does not issue. What remains is genesis/custody,
import/pull, `/verify/*`, `/trace/*`, settings, and the shop CRM (`/clients`, `/appointments`,
`/verifications` — mounted for every role).

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

The nav leads with the shop's daily working surfaces, then the verification history, then the
supporting DogTag sections. There is deliberately **no "Issue a record" entry and no Records page** —
see the note at the top.

- **Dashboard** (`/dashboard`) — today's bookings + quick links into Calendar / Clients / All
  verifications / Ad-hoc verification.
- **Calendar** (`/calendar`) — day and week grids of the shop's bookings (the operator's daily
  surface), reading `GET /appointments` over a half-open `[from, to)` window in unix seconds.
- **Appointments** (`/appointments`, `/appointments/new`, `/appointments/:id`, `/appointments/:id/edit`)
  — the booking book: client + pet, service, slot, notes, groomer, status
  (scheduled/confirmed/in_progress/completed/cancelled/no_show); searchable and filterable
  server-side. The detail page is where a verification is **started for that visit**.
- **Clients** (`/clients`, `/clients/:id`) — the customer directory: owner particulars plus their
  pets (each pet may carry its DogTag id), standard CRUD, server-side search. From a client you can
  book an appointment or jump to that client's verification history.
- **All verifications** (`/verifications`, `/verifications/:id`) — the shop's complete, searchable
  history of every verification it has run (`GET /verifications`), joined to the appointment and
  client when the operator started it from one; filterable by client, appointment, purpose, status
  and date window. `?clientId=` / `?appointmentId=` pre-apply that filter, so the client and
  appointment pages deep-link into a scoped history.
- **Groomers / Reports / Marketing** — clean placeholders that mirror the reference UI (not wired).
- **Import from user** (`/import`) — pull a customer's pet **profile** or **vaccination** via QR
  (`POST /import/pull`); the backend third-party-verifies on chain + DNS and the portal renders the
  **three authenticity pillars** verdict (integrity / issuance / identity, plus the contextual
  `ownership` fragment which is `NOT_APPLICABLE` for a third-party importer). Decoupled from Verify.
- **Ad-hoc verification** (`/verify`) — the walk-in path, for a pet with no booking: the same
  `@dogtag/ui` `VerifyFlow` as the appointment page, minus the business context, so a verification
  means the same thing however it was started (it just lands in "All verifications" as an unlinked
  row). One owner-hidden consent flow (purpose → session QR → owner proof → on-chain status), with
  no disclosure-mode choice. It also carries the permissionless `CredentialVerifyPanel` (paste a
  wrapped doc, checked direct-to-RPC). Emphasizes that a groomer can verify a vet-issued vaccination
  **without being an issuer** (the `VERIFY:<purpose>` whitelist namespace, distinct from issuer
  roles).
- **Setup** (`/setup`) — the same genesis/custody wizard as the vet portal (the shop still needs its
  own signer: the relayer that pays gas for the on-chain verification): custody admin login →
  genesis (24 words → confirm + passphrase → unlock) → derive accounts → apply for whitelist
  (central `POST /v1/issuer-applications`, a groomer applies with `verifyPurposes`) → DNS-TXT.
  Setup owns **genesis only**: a sealed-but-locked instance (e.g. after a backend restart) is
  handed to `/unlock` rather than re-entering the wizard; the `confirm → unlock` step above is
  genesis continuation and stays.
- **Unlock** (`/unlock`) - the dedicated custody-unlock page, same as the vet portal. **Not a nav
  item**: it is reached from the Setup admin-login hand-off or a direct link, and it is the FALLBACK
  surface, not the primary one. Nothing redirects. An action refused with `not unlocked` raises an
  unlock prompt **in place** over the page the operator is already on, and the shared api client
  replays the refused request once the seal opens, so a half-filled form is never discarded;
  arriving at an already-locked backend shows a non-blocking banner instead, and read-only pages
  stay reachable (the operator password and the custody-admin password are separate credentials).
  This page restores `?next=` when it carries one. Enter the custody-admin password + passphrase
  (both prefilled in demo mode); a wrong passphrase shows an inline error and does **not** end the
  session. Both surfaces point at Setup only when the instance has no seal at all (needs genesis,
  not unlock).
- **Settings** (`/settings`) — signing-mode toggle (`PUT /settings/signing-mode`), status panel
  (`GET /issuer/signers`), theme toggle.

## Wired vs placeholder

- **Wired to backend contracts**: login, genesis/confirm/unlock/accounts, clients + appointments CRUD
  and the verification history (`/clients`, `/appointments`, `/verifications` via the shared
  `@dogtag/ui` client), signing-mode get/put, issuer signers, import/pull (with 3-pillar verdict
  render), verify session start (with the optional `appointmentId` linkage), central
  issuer-application apply.
- **Placeholder**: Groomers, Reports, Marketing.
- **Note**: like the vet portal, the verify flow shows the session QR + awaiting-consent state and
  polls `GET /verify/session/:id` for the "pending → Verified" transition; the appointment page
  refreshes itself when that poll settles.

## E2E

With the dev server running, `pnpm --filter @dogtag/groomer-web test:e2e` exercises the mocked portal
flows. `e2e/verify.spec.ts` — the only spec here — pins the single consent UI (no "Mode"/"Normal"
choice) and asserts the ad-hoc session-start request carries only the purpose and record type, with
no mode and no business context.

## Env

See `.env.example`. The Reown projectId is a placeholder by default — wallet-connect needs a real
one. The central API base is where the whitelist application is posted.
