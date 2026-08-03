import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

/**
 * The admin portal's unit suite.
 *
 * This package's gate used to be `tsc --noEmit && vite build` alone, which is a typecheck: it proves
 * the pages COMPILE, never what they render. The defect that made this suite necessary typechecked
 * perfectly - `href={explorerTxUrl(tx)}` is well-typed for any string, including one that addresses no
 * transaction on any chain - so the only thing that can catch it coming back is rendering the
 * component and looking at the DOM.
 *
 * `@vitejs/plugin-react` is here because the tests render real `.tsx` page components; jsdom because a
 * link is a DOM fact. Kept deliberately narrow: `test/` only, so nothing sweeps up the Playwright specs
 * that live in other stacks and drive live portals.
 */
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["test/**/*.test.ts", "test/**/*.test.tsx"],
    /**
     * Raised from vitest's 5s default, which these mounted suites outgrow under contention.
     *
     * Observed: `providersPage.test.tsx` runs its 19 tests in ~1.1s when this package is tested
     * alone, and one of them tripped the 5s per-test limit during a full `pnpm -r` run with other
     * packages' suites competing for the machine. Nothing here waits on anything unbounded - the
     * tests mount real components and advance a FIXED number of real macrotask turns, because this
     * package follows the repo's no-`act()` rule - so the slowness is scheduling, not a hang, and a
     * timeout is the wrong instrument for catching a hang that cannot happen.
     *
     * A flake in a suite whose whole purpose is catching a defect that typechecks perfectly is worse
     * than the usual: it trains a reader to re-run rather than to read the failure.
     */
    testTimeout: 20_000,
  },
});
