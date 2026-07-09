import { test, expect, type Page, type Request, type Route } from "@playwright/test";
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";

/**
 * E2E for the permissionless, direct-to-RPC credential verify panel (fm/dogtag-webverify-n3).
 *
 * Proves the decoupling: when the operator clicks "Verify credential", the browser reads the ROAX
 * chain DIRECTLY (viem `eth_call` over the public RPC) and classifies the credential itself - the
 * operator-gated `POST /verify/credential` relay is never called. We drive the REAL `Verify` page and
 * REAL `@dogtag/ui` `CredentialVerifyPanel`; only the four on-chain `view` reads are mocked (by their
 * 4-byte selector) so the valid/revoked/whitelist verdicts render deterministically without a live
 * anchored credential. Integrity is the genuine offline `@dogtag/standard` recompute (real doc).
 */

const OP_TOKEN_KEY = "vet.opToken";

// Function selectors the shipped web ABI produces (contracts.ts). Note isValid == 0x6a938567, the
// server's selector - NOT the mobile hard-coded 0x6d04f0bc (which reverts on the live deployment).
const SEL = {
  isValid: "0x6a938567",
  isRevoked: "0x4294857f",
  issuedAt: "0x6240dded",
  isWhitelistedFor: "0x779c3985",
} as const;

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};
const SIGNER = "0xabc0000000000000000000000000000000000abc";

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

interface ChainState {
  issuedAt: bigint;
  isValid: boolean;
  isRevoked: boolean;
  isWhitelistedFor: boolean;
}

/**
 * Intercept the ROAX public RPC and answer the four verify reads by selector. Every other JSON-RPC
 * method (eth_chainId, wagmi bootstrap calls) is answered benignly so the app still boots. Returns a
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
      if (sel === SEL.isValid) return answer(msg.id, boolWord(state.isValid));
      if (sel === SEL.isRevoked) return answer(msg.id, boolWord(state.isRevoked));
      if (sel === SEL.issuedAt) return answer(msg.id, uintWord(state.issuedAt));
      if (sel === SEL.isWhitelistedFor) return answer(msg.id, boolWord(state.isWhitelistedFor));
      return answer(msg.id, boolWord(false));
    }
    return answer(msg.id, "0x");
  };

  await page.route("https://devrpc.roax.net/**", async (route: Route) => {
    const body = route.request().postDataJSON();
    const json = Array.isArray(body) ? body.map(handleOne) : handleOne(body);
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(json) });
  });

  return requestedUrls;
}

test.beforeEach(async ({ page }) => {
  // Get past the portal Login gate (portal-level auth is unrelated to the verify action).
  await page.addInitScript(([k]) => window.localStorage.setItem(k as string, "op-token-e2e"), [
    OP_TOKEN_KEY,
  ]);
});

async function openVerifyAndSubmit(page: Page, doc: WrappedDoc, signer?: string) {
  await page.goto("/verify");
  await expect(page.getByText("Check credential status")).toBeVisible();
  // The new permissionless copy ships on the real surface.
  await expect(
    page.getByText(/Permissionless - verified in-browser over the public RPC/i),
  ).toBeVisible();

  await page.getByPlaceholder("Paste wrappedDoc JSON").fill(JSON.stringify(doc));
  if (signer) await page.getByPlaceholder("0x... optional").fill(signer);
  await page.getByRole("button", { name: "Verify credential" }).click();
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
  const urls = await mockRoaxRpc(page, {
    issuedAt: 1_699_000_000n,
    isValid: true,
    isRevoked: false,
    isWhitelistedFor: false,
  });
  const doc = validDoc();
  await openVerifyAndSubmit(page, doc);

  await expect(page.getByText("Verdict: pass")).toBeVisible();
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  // Integrity + on-chain + issued pillars all pass; revoked shows No; whitelist "Not checked" (no signer).
  await expect(page.getByText("Not checked")).toBeVisible();

  // The claimed root the panel read on-chain equals the doc's signature root (and the recompute).
  await expect(page.getByText(doc.signature.merkleRoot).first()).toBeVisible();

  assertNoOperatorRelay(urls);
  // Prove the browser actually hit the public RPC for the reads.
  expect(urls.some((u) => u.startsWith("https://devrpc.roax.net"))).toBeTruthy();

  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-valid.png" });
});

test("revoked credential: renders Verdict fail / Revoked", async ({ page }) => {
  const urls = await mockRoaxRpc(page, {
    issuedAt: 1_699_000_000n,
    isValid: false,
    isRevoked: true,
    isWhitelistedFor: false,
  });
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  // Scope to the status Badge span ("Revoked" also appears as a pillar label).
  await expect(page.locator("span").filter({ hasText: /^Revoked$/ })).toBeVisible();

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-revoked.png" });
});

test("whitelist pillar gates the verdict: valid on-chain but non-whitelisted signer fails", async ({
  page,
}) => {
  const urls = await mockRoaxRpc(page, {
    issuedAt: 1_699_000_000n,
    isValid: true,
    isRevoked: false,
    isWhitelistedFor: false,
  });
  await openVerifyAndSubmit(page, validDoc(), SIGNER);

  // Status is valid (chain state) but the verdict fails because the issuer signer is not whitelisted.
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  await expect(page.getByText("Verdict: fail")).toBeVisible();

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-whitelist-fail.png" });
});
