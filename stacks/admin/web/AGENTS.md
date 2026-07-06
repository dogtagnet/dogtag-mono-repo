# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

- Add durable project-specific notes here as they are discovered through real work.

## Activity dashboard (PR-D, "see on-chain activity")

The admin portal's oversight surface consuming the PR-B indexer-consumption data layer (`stacks/admin/api`):

- **Activity feed** (`src/pages/Activity.tsx`, nav `/activity`): the UNSCOPED cross-issuer event feed from `GET /v1/admin/activity` (IssuerCreated / RootRegistered / Whitelisted / Delisted / RootIssued / RootRevoked / Verified). Filterable by event type, finality, time range, signer, issuer clone, and record type; each row shows block/finality/timestamp, an explorer link (`event.txUrl`, falling back to `explorerTxUrl`), and the signer/clone BUSINESS NAME resolved by the admin directory (`actorName`/`cloneName`). No client PII.
- **Dashboard** (`src/pages/Dashboard.tsx`): registry counts + cross-issuer on-chain aggregates (`GET /v1/admin/activity/stats`) + a chain-health card (indexer watermark from `GET /v1/admin/activity/status` - head vs indexed block, lag, finality source - plus the live authority map from `GET /v1/admin/governance/authority`) + a recent-activity slice linking to `/activity`.
- **Event presentation helpers** live in `src/lib/activity.ts` (`EVENT_META`, `relativeTime`, `absoluteTime`).
- **Wire types + client methods** are in the shared package: `packages/ui/src/api/types.ts` (`ActivityEvent`, `ActivityStats`, `IndexerStatus`, `GovernanceAuthority`, …) and `packages/ui/src/api/central.ts` (`getActivity`, `getActivityStats`, `getActivityStatus`, `getActivityIssuers`, `getDirectory`, `getGovernanceAuthority`).
- **Backend note:** PR-B added the `OversightFeed::status()` method but wired no route; PR-D added the thin `GET /v1/admin/activity/status` proxy (`stacks/admin/api/src/routes.rs`) to surface the chain-health watermark. All activity/count surfaces return **503** (`indexer: not-configured`) when `INDEXER_API_BASE` is unset - the UI renders a first-class "indexer not connected" state, and the registry counts keep working.
- **Verify locally:** `pnpm --filter @dogtag/admin-web build` (tsc + vite) and `cargo test --test indexer_consume` in `stacks/admin/api`. There is no frontend lint/test script - `tsc --noEmit` (the `build`/`typecheck` scripts) is the gate.
