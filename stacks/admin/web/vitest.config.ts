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
  },
});
