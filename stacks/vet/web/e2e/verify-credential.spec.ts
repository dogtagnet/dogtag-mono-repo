import { test, expect, type Page, type Request, type Route } from "@playwright/test";
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";
import { keccak256, toBytes } from "viem";

/**
 * E2E for the permissionless, direct-to-RPC credential verify panel (fm/dogtag-webverify-n3).
 *
 * Proves the decoupling: when the operator clicks "Verify credential", the browser reads the ROAX
 * chain DIRECTLY (viem `eth_call` over the public RPC) and classifies the credential itself - the
 * operator-gated `POST /verify/credential` relay is never called. We drive the REAL `Verify` page and
 * REAL `@dogtag/ui` `CredentialVerifyPanel`; only the on-chain `view` reads are mocked (by their
 * 4-byte selector) so the valid/revoked/whitelist verdicts render deterministically without a live
 * anchored credential. Integrity is the genuine offline `@dogtag/standard` recompute (real doc).
 *
 * The fake must ANCHOR THE CLONE THROUGH THE FACTORY, exactly as the shipped path does: the issuing
 * clone comes from `DogTagIssuerFactory.rootIssuer(root)`, and the record type from that clone's own
 * `recordType()`. A fake that only answered the per-clone reads would model the pre-audit world in
 * which the document's own `documentStore` gets to say whether it is valid - the forgery the
 * mandatory issuer-whitelist pillar exists to close.
 */

const OP_TOKEN_KEY = "vet.opToken";

// Function selectors the shipped web ABI produces (contracts.ts). Note isValid == 0x6a938567, the
// server's selector - NOT the mobile hard-coded 0x6d04f0bc (which reverts on the live deployment).
const SEL = {
  rootIssuer: "0x41e41d17",
  recordType: "0xe55e492c",
  isValid: "0x6a938567",
  isRevoked: "0x4294857f",
  issuedAt: "0x6240dded",
  issuedBy: "0xe0d272c0",
  isWhitelistedFor: "0x779c3985",
} as const;

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};
/** The address the chain reports as this root's originator - resolved, never typed by the operator. */
const SIGNER = "0xabc0000000000000000000000000000000000abc";
/**
 * What the issuing clone's own `recordType()` returns. DERIVED from the envelope's claim rather than
 * pinned, because the pillar compares the two: a hardcoded key that drifted from `ISSUER.recordType`
 * would silently turn every case into a relabelling failure instead of the scenario under test.
 */
const CHAIN_RECORD_TYPE_KEY = keccak256(toBytes(ISSUER.recordType));
/** `rootIssuer` for a root no factory clone ever issued - the indeterminate case. */
const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";

/** Deterministic-salt wrapped doc so integrity reconciles to a stable claimed root every run. */
function validDoc(): WrappedDoc {
  let seq = 0;
  const fixedSalt = () => new Uint8Array(16).fill(++seq);
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: { tag: TypeTag.Integer, value: "42" },
        name: { tag: TypeTag.String, value: "Rex" },
      },
    },
    ISSUER,
    fixedSalt,
  );
}

const boolWord = (b: boolean) => "0x" + (b ? "1" : "0").padStart(64, "0");
const uintWord = (n: bigint) => "0x" + n.toString(16).padStart(64, "0");
const addressWord = (a: string) => "0x" + a.replace(/^0x/, "").toLowerCase().padStart(64, "0");

interface ChainState {
  /** DogTagIssuerFactory.rootIssuer(root) - the clone every other read below is made against. */
  rootIssuer: string;
  /** The clone's own immutable recordType() key; keccak256("VACCINATION") for the fixture doc. */
  recordType: string;
  issuedAt: bigint;
  isValid: boolean;
  isRevoked: boolean;
  /** DogTagIssuer.issuedBy(root) - the H-1 originator the whitelist pillar resolves for itself. */
  issuedBy: string;
  isWhitelistedFor: boolean;
}

/**
 * Intercept the ROAX public RPC and answer the verify reads by selector. Non-`eth_call` methods
 * (eth_chainId, wagmi bootstrap) are answered benignly so the app still boots, but an `eth_call` this
 * fake does not model is a hard FAILURE naming the selector - never a fabricated zero word. Returns a
 * mutable list of every URL the page requested, so the test can assert the operator relay was NOT hit.
 */
async function mockRoaxRpc(page: Page, state: ChainState): Promise<string[]> {
  const requestedUrls: string[] = [];
  page.on("request", (r: Request) => requestedUrls.push(r.url()));

  const answer = (id: unknown, result: string) => ({ jsonrpc: "2.0", id, result });
  const handleOne = (msg: { id?: unknown; method?: string; params?: unknown[] }) => {
    if (msg.method === "eth_chainId") return answer(msg.id, "0x87");
    if (msg.method === "eth_call") {
      const data = String((msg.params?.[0] as { data?: string } | undefined)?.data ?? "");
      const sel = data.slice(0, 10);
      if (sel === SEL.rootIssuer) return answer(msg.id, addressWord(state.rootIssuer));
      if (sel === SEL.recordType) return answer(msg.id, state.recordType);
      if (sel === SEL.isValid) return answer(msg.id, boolWord(state.isValid));
      if (sel === SEL.isRevoked) return answer(msg.id, boolWord(state.isRevoked));
      if (sel === SEL.issuedAt) return answer(msg.id, uintWord(state.issuedAt));
      if (sel === SEL.issuedBy) return answer(msg.id, addressWord(state.issuedBy));
      if (sel === SEL.isWhitelistedFor) return answer(msg.id, boolWord(state.isWhitelistedFor));
      // NO SILENT DEFAULT. This used to answer any unmodelled read with a zero word, and that is
      // precisely how the mock went stale: when the shipped path grew the factory `rootIssuer`
      // lookup, the fake invented a zero address for it and the suite reported a plausible-but-wrong
      // verdict instead of naming the gap - the same "an unanswered check counted as a passed one"
      // shape the mandatory issuer-whitelist pillar exists to close. A throwing route handler is
      // reported by Playwright as an unhandled error, so the selector lands in front of whoever
      // added the read.
      throw new Error(
        `verify-credential e2e: unmodelled eth_call selector ${sel}. The verify path makes a chain ` +
          `read this mock does not model - add it to SEL/ChainState instead of letting the fake ` +
          `invent an answer for it. (calldata: ${data})`,
      );
    }
    // Non-`eth_call` methods are wagmi/viem bootstrap noise that the verify path never reads, so an
    // empty word keeps the app booting; only the reads above decide a verdict.
    return answer(msg.id, "0x");
  };

  await page.route("https://devrpc.roax.net/**", async (route: Route) => {
    const body = route.request().postDataJSON();
    const batch: { id?: unknown }[] = Array.isArray(body) ? body : [body];
    const send = (replies: unknown[]) =>
      route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(Array.isArray(body) ? replies : replies[0]),
      });
    let replies: unknown[];
    try {
      replies = batch.map(handleOne);
    } catch (e) {
      // Still answer the request - as a JSON-RPC error, which viem surfaces immediately rather than
      // retrying the way it would a network-level abort - so the page fails fast and the test does
      // not sit out its 60s timeout before the rethrow below is reported. Only the mapping is
      // guarded: wrapping the fulfill too would let a "Route is already handled!" from the second
      // fulfill replace the selector message this exists to deliver.
      const error = { code: -32000, message: (e as Error).message };
      await send(batch.map((m) => ({ jsonrpc: "2.0", id: m?.id ?? null, error })));
      throw e;
    }
    await send(replies);
  });

  return requestedUrls;
}

test.beforeEach(async ({ page }) => {
  // Get past the portal Login gate (portal-level auth is unrelated to the verify action). The seeded
  // token only survives if the portal's own backend probes succeed, so stub `/api/` benignly the way
  // every sibling vet spec does - otherwise a real (or absent) vet-api answers 401 and the shared
  // client's stale-session hook clears the token mid-test, dropping the page back to "Sign in".
  // Verification itself never touches the backend; that is what `assertNoOperatorRelay` proves, and
  // this stub keeps that assertion honest by recording any relay call instead of hiding it.
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/api/, "");
    // Shapes the page's own panels destructure. A bare `{}` for these crashes the render, which
    // would take the verify panel down with it.
    if (path === "/settings/signing-mode") return route.fulfill({ json: { signingMode: "backend" } });
    if (path === "/verify/history") return route.fulfill({ json: { verifications: [] } });
    if (path === "/issuer/signers") return route.fulfill({ json: { signers: [] } });
    // Same rule as the RPC fake above, for the same reason: a catch-all `{}` is an invented answer,
    // and CLAUDE.md records that exact pattern silently redirecting sibling vet specs to `/unlock`
    // instead of the page under test. Name the path rather than guess a shape for it.
    const message = `verify-credential e2e: unmodelled backend path ${path} - give it a shape the page can destructure.`;
    await route.fulfill({ status: 501, json: { error: message } });
    throw new Error(message);
  });
  await page.addInitScript(([k]) => window.localStorage.setItem(k as string, "op-token-e2e"), [
    OP_TOKEN_KEY,
  ]);
});

/**
 * A genuinely anchored credential: the factory names the very clone the envelope claims, that clone
 * reports the record type the envelope claims, and its originator is whitelisted. Each test overrides
 * only the one fact it is about.
 */
function anchoredChain(over: Partial<ChainState> = {}): ChainState {
  return {
    rootIssuer: ISSUER.documentStore,
    recordType: CHAIN_RECORD_TYPE_KEY,
    issuedAt: 1_699_000_000n,
    isValid: true,
    isRevoked: false,
    issuedBy: SIGNER,
    isWhitelistedFor: true,
    ...over,
  };
}

async function openVerifyAndSubmit(page: Page, doc: WrappedDoc) {
  await page.goto("/verify");
  await expect(page.getByText("Check credential status")).toBeVisible();
  // The new permissionless copy ships on the real surface.
  await expect(
    page.getByText(
      /Permissionless - checked in-browser through your chain-guarded endpoint selection/i,
    ),
  ).toBeVisible();

  // Deliberately no "Expected issuer signer": the whitelist pillar resolves its own signer from the
  // chain, so every assertion below holds with zero operator input.
  await page.getByPlaceholder("Paste wrappedDoc JSON").fill(JSON.stringify(doc));
  await page.getByRole("button", { name: "Verify credential" }).click();
}

/** The value cell of a named pillar tile, so "Yes"/"No" is read off the pillar under test. */
function pillar(page: Page, label: string) {
  return page.getByText(label, { exact: true }).locator("xpath=..");
}

function assertNoOperatorRelay(urls: string[]) {
  const relayCalls = urls.filter((u) => /\/verify\/credential\b/.test(u));
  expect(relayCalls, `operator relay must NOT be called; saw: ${relayCalls.join(", ")}`).toEqual([]);
}

/** The verify panel Card (ancestor of its title) - the region we screenshot. */
function panelCard(page: Page) {
  return page
    .getByText("Check credential status")
    .locator('xpath=ancestor::div[contains(@class,"rounded-lg")][1]');
}

test("valid credential: reads chain directly, renders Verdict pass / Valid, no operator relay", async ({
  page,
}) => {
  const urls = await mockRoaxRpc(page, anchoredChain());
  const doc = validDoc();
  await openVerifyAndSubmit(page, doc);

  await expect(page.getByText("Verdict: pass")).toBeVisible();
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  // The whitelist pillar is answered - not skipped - even though the operator typed no signer: it
  // resolved the originator from `issuedBy` itself. That self-resolution is what makes it mandatory.
  await expect(pillar(page, "Issuer whitelist")).toHaveText(/Yes$/);
  await expect(page.getByText(new RegExp(`^${SIGNER}$`, "i"))).toBeVisible();

  // The claimed root the panel read on-chain equals the doc's signature root (and the recompute).
  await expect(page.getByText(doc.signature.merkleRoot).first()).toBeVisible();

  assertNoOperatorRelay(urls);
  // Prove the browser actually hit the public RPC for the reads.
  expect(urls.some((u) => u.startsWith("https://devrpc.roax.net"))).toBeTruthy();

  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-valid.png" });
});

test("revoked credential: renders Verdict fail / Revoked", async ({ page }) => {
  const urls = await mockRoaxRpc(page, anchoredChain({ isValid: false, isRevoked: true }));
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  // Scope to the status Badge span ("Revoked" also appears as a pillar label).
  await expect(page.locator("span").filter({ hasText: /^Revoked$/ })).toBeVisible();

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-revoked.png" });
});

test("whitelist pillar gates the verdict: valid on-chain but non-whitelisted issuer fails", async ({
  page,
}) => {
  const urls = await mockRoaxRpc(page, anchoredChain({ isWhitelistedFor: false }));
  await openVerifyAndSubmit(page, validDoc());

  // The record is valid on-chain, yet the credential fails: the address that actually issued this
  // root is not whitelisted for the record type the clone reports. No operator input was needed to
  // reach that verdict, which is the point - the pillar can no longer be left unrun.
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(pillar(page, "Issuer whitelist")).toHaveText(/No$/);

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-whitelist-fail.png" });
});

test("unresolvable issuer renders Unresolved and fails closed, never a silent pass", async ({
  page,
}) => {
  // No factory clone ever issued this root, so there is nobody to ask. The audit defect was exactly
  // this shape - an unanswered check counted as a passed one - so the pillar must render as a
  // FAILURE to establish the claim, not as a neutral step that was skipped.
  const urls = await mockRoaxRpc(page, anchoredChain({ rootIssuer: ZERO_ADDRESS }));
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(page.getByText("Not issued", { exact: true })).toBeVisible();
  await expect(pillar(page, "Issuer whitelist")).toHaveText(/Unresolved$/);

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-unresolved-issuer.png" });
});
