/**
 * Resolved runtime config from Vite env for the government portal.
 *
 * `demoMode` mirrors the repo-wide convention (vet/groomer/admin `src/lib/env.ts`): `VITE_DEMO_MODE`
 * set = demo/local, UNSET = production. `scripts/demo-up.sh` launches every portal (this one
 * included) with `VITE_DEMO_MODE=1`. It gates demo-only affordances such as "Fill demo data" —
 * never any behaviour of issuance itself.
 */
export const env = {
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
