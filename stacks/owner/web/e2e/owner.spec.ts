import { test, expect, type Page } from "@playwright/test";
import {
  CONSENT_TXS,
  OPEN_NULLIFIER,
  RELAYER,
  REX_DOGTAGID_TOPIC,
  VERIFIED_LOGS,
} from "./consentFixture";

/**
 * The pet-owner (holder) loop, end to end:
 *
 *   1. RECEIVE  — paste a real wrapped credential → integrity-checked → held in the wallet
 *   2. DISPLAY  — the wallet lists it; the detail view decodes its fields
 *   3. SHARE    — create an integrity-preserving selectively disclosed copy
 *   4. RECEIPT  — render government travel/health receipts with live validity
 *   5. CONSENTS - render the owner's own consent history from the owner-blind Verified events
 *   6. SETTINGS - every endpoint save reports its verdict, and a rejection really does clear
 *
 * ROAX RPC is mocked at the network layer so the live validity reads are deterministic. Owner-hidden
 * consent proving requires the private tag-profile witness held by the native wallet and is not a
 * browser-wallet surface.
 */

interface RpcRequest {
  method: string;
  params: unknown[];
}

/**
 * Install the sole remote dependency used by this browser wallet: read-only ROAX JSON-RPC,
 * dispatched per method - `eth_call` (isValid → true), `eth_getLogs` (the owner-blind Verified
 * history, served ONLY when the filter names Rex's tag id), `eth_getTransactionByHash` (the
 * recordVerificationZK calldata the app reads recordType back from). Returns the captured requests
 * so tests can assert what the wallet actually queried.
 */
async function installMocks(page: Page): Promise<RpcRequest[]> {
  const captured: RpcRequest[] = [];
  await page.route(/devrpc\.roax\.net/, async (route) => {
    let req: { id?: unknown; method?: string; params?: unknown[] } = {};
    try {
      req = JSON.parse(route.request().postData() || "{}");
    } catch {
      /* fall through to the default reply */
    }
    captured.push({ method: req.method ?? "", params: req.params ?? [] });
    const reply = (result: unknown) =>
      route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ jsonrpc: "2.0", id: req.id ?? 1, result }),
      });
    switch (req.method) {
      case "eth_chainId":
        return reply("0x87");
      case "eth_getLogs": {
        // Serve the Verified history iff the topic filter names Rex's canonical tag id - proving
        // the wallet derives + queries its OWN held-tag ids, not an unscoped feed.
        const filter = JSON.stringify(req.params?.[0] ?? {});
        return reply(filter.includes(REX_DOGTAGID_TOPIC) ? VERIFIED_LOGS : []);
      }
      case "eth_getTransactionByHash": {
        const hash = String(req.params?.[0] ?? "").toLowerCase();
        return reply(CONSENT_TXS[hash] ?? null);
      }
      default:
        // `eth_call` (DogTagIssuer.isValid) and anything else: a single true word.
        return reply("0x" + "0".repeat(63) + "1");
    }
  });
  return captured;
}

/** The RPC requests the current test's page issued (repopulated per test by the beforeEach). */
let rpc: RpcRequest[] = [];

test.beforeEach(async ({ page }) => {
  rpc = await installMocks(page);
});

test("holder loop: receive → hold → display", async ({ page }) => {
  // Fresh wallet each run.
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // The wallet starts empty and provisions a self-custodial owner address.
  await expect(page.getByTestId("empty-wallet")).toBeVisible();
  await expect(page.getByTestId("owner-address")).toContainText("0x");

  // 1. RECEIVE — fill the sample credential and add it.
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  await expect(page.getByTestId("receive-input")).toHaveValue(/VACCINATION/);
  await page.getByTestId("receive-add").click();

  // 2. DISPLAY — receiving lands on the decoded detail view; integrity is intact.
  await expect(page.getByTestId("detail-name")).toHaveText("Rex");
  await expect(page.getByTestId("detail-integrity")).toContainText("intact");
  await expect(page.getByTestId("detail-fields")).toContainText("Rabies");
  await expect(page.getByTestId("detail-fields")).toContainText("424242");
  await expect(page.getByTestId("detail-receipt")).toHaveCount(0);

  // Unsupported record types fail closed if someone manually opens the receipt route.
  await page.goto("/receipt/0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67");
  await expect(page.getByTestId("receipt-unavailable")).toContainText("VACCINATION");

  // The wallet now holds exactly one credential.
  await page.goto("/wallet");
  await expect(page.getByTestId("cred-count")).toContainText("1 held");
  await expect(page.getByTestId("cred-name")).toHaveText("Rex");

  // The retired browser proof surface is gone; unknown routes return safely to the wallet.
  await page.goto("/present");
  await expect(page.getByTestId("cred-count")).toContainText("1 held");
});

test("holder selective disclosure: withhold a field → redacted copy still verifies", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // Receive the sample credential (lands on its detail view).
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  await page.getByTestId("receive-add").click();
  await expect(page.getByTestId("detail-name")).toHaveText("Rex");

  // Open the Share (redacted copy) flow from the detail view.
  await page.getByTestId("detail-share").click();
  await expect(page.getByTestId("share-fields")).toBeVisible();
  // dogTagId is locked-on (required, non-obfuscatable) and its toggle is disabled.
  await expect(page.getByTestId("share-locked").first()).toBeVisible();
  await expect(page.getByTestId("share-toggle-credentialSubject.dogTagId")).toBeDisabled();

  // Everything revealed by default → the copy is the full, authentic credential.
  await expect(page.getByTestId("share-withheld-count")).toContainText("Every field is revealed");
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  await expect(page.getByTestId("share-preview")).toContainText("Dr. A. Meyer");

  // Withhold the veterinarian field.
  await page.getByTestId("share-toggle-credentialSubject.veterinarian").click();
  await expect(page.getByTestId("share-withheld-count")).toContainText("1 field withheld");
  // The redacted copy STILL verifies authentic (the Merkle root is unchanged)…
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  // …but the recipient no longer sees the withheld value.
  await expect(page.getByTestId("share-preview")).toContainText("withheld by holder");
  await expect(page.getByTestId("share-preview")).not.toContainText("Dr. A. Meyer");

  // Copy the redacted credential; the output JSON drops the cleartext but keeps dogTagId + the root.
  await page.getByTestId("share-copy").click();
  await expect(page.getByTestId("share-copied")).toBeVisible();
  const out = await page.getByTestId("share-output").inputValue();
  expect(out).not.toContain("Dr. A. Meyer");
  expect(out).toContain("424242"); // dogTagId cleartext stays
  const redacted = JSON.parse(out) as { privacy: { obfuscated: string[] }; signature: { merkleRoot: string } };
  expect(redacted.privacy.obfuscated.length).toBe(1); // the withheld leaf's hash is retained
  expect(redacted.signature.merkleRoot).toBe(
    "0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67",
  ); // unchanged — it is the same on-chain credential
});

test("holder receipt: travel clearance renders and respects redacted re-imports", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // Receive the CDC-modeled government travel-clearance sample.
  await page.goto("/receive");
  await page.getByTestId("receive-sample-travel").click();
  await expect(page.getByTestId("receive-input")).toHaveValue(/TRAVEL_CLEARANCE/);
  await page.getByTestId("receive-add").click();

  // Detail now recognizes the nested animal name + exposes the receipt action and derived status.
  await expect(page.getByTestId("detail-name")).toHaveText("Blaze");
  await expect(page.getByTestId("detail-receipt")).toBeVisible();
  await expect(page.getByTestId("detail-onchain")).toContainText("Receipt: Valid");

  // The top-level Receipts nav lists holder-renderable receipt credentials.
  await page.goto("/receipts");
  await expect(page.getByTestId("receipt-count")).toContainText("1 available");
  await expect(page.getByTestId("receipt-row")).toContainText("Blaze");
  await expect(page.getByTestId("receipt-row")).toContainText("9RVBXK8AFQ2C");
  await expect(page.getByTestId("receipt-row-status")).toContainText("Valid");

  await page.getByTestId("receipt-row").click();
  const sheet = page.getByTestId("receipt-sheet");
  await expect(sheet).toBeVisible();
  await expect(page.getByTestId("receipt-status")).toContainText("VALID");
  await expect(page.getByTestId("receipt-id")).toContainText("9RVBXK8AFQ2C");
  await expect(sheet).toContainText("Section A - Person Importing the Animal");
  await expect(sheet).toContainText("Importer (sample)");
  await expect(sheet).toContainText("DEMO-ID-000000");
  await expect(sheet).toContainText("Section B - Animal Information");
  await expect(sheet).toContainText("Blaze");
  await expect(sheet).toContainText("Section C - Travel Information");
  await expect(sheet).toContainText("AC 8552");
  // The QR is built from `protocol.statusBaseUrl` - the reachable origin the issuer stamped - and
  // NEVER from `issuer.domain`, which is the `did:web` identity `gov.example` (RFC-2606 reserved, so
  // every QR built from it encoded an NXDOMAIN link that still read as a working live-status check).
  await expect(page.getByTestId("receipt-public-url")).toContainText(
    "https://travel.authority.example-demo.net/r/9RVBXK8AFQ2C",
  );
  await expect(page.getByTestId("receipt-public-url")).not.toContainText("gov.example");
  await expect(page.getByTestId("receipt-qr").locator("svg")).toBeVisible();
  await expect(page.getByTestId("receipt-live")).toContainText("anchored");
  await expect(page.getByTestId("receipt-root")).toContainText(
    "0x199948111387332ec1e85a4d1dc4651ea691bb7bcbad768e3a2f8c38b290005b",
  );

  // Produce a redacted copy that withholds a Section-A identifier, then re-import it. The same root is
  // replaced in localStorage, and the receipt renders only the disclosed leaves.
  await page.getByTestId("receipt-share").click();
  await page.getByTestId("share-toggle-credentialSubject.importer.idNumber").click();
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  await page.getByTestId("share-copy").click();
  const redactedJson = await page.getByTestId("share-output").inputValue();
  expect(redactedJson).not.toContain("DEMO-ID-000000");
  const redacted = JSON.parse(redactedJson) as { privacy: { obfuscated: string[] }; signature: { merkleRoot: string } };
  expect(redacted.privacy.obfuscated.length).toBe(1);
  expect(redacted.signature.merkleRoot).toBe(
    "0x199948111387332ec1e85a4d1dc4651ea691bb7bcbad768e3a2f8c38b290005b",
  );

  await page.goto("/receive");
  await page.getByTestId("receive-input").fill(redactedJson);
  await page.getByTestId("receive-add").click();
  await page.getByTestId("detail-receipt").click();
  await expect(page.getByTestId("receipt-withheld-note")).toContainText("1 field");
  await expect(page.getByTestId("receipt-sheet")).toContainText("Importer (sample)");
  await expect(page.getByTestId("receipt-sheet")).not.toContainText("DEMO-ID-000000");
});

// The other half of the receipt-QR contract, and the one every credential issued before this change
// takes: with no stamped base there is NO status page, so the receipt must say exactly that and draw
// no QR. Falling back to `issuer.domain` was considered and rejected - a real did:web host resolves
// but does not serve `/r/`, trading an NXDOMAIN for a 404 that looks even more legitimate.
test("holder receipt: a credential with no stamped status base degrades honestly", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // Take the stamped sample and drop only the provenance block - exactly what a pre-change document
  // looks like. That block sits outside the Merkle root, so the credential stays integrity-VALID.
  await page.goto("/receive");
  await page.getByTestId("receive-sample-travel").click();
  const stamped = await page.getByTestId("receive-input").inputValue();
  const unstamped = JSON.parse(stamped) as Record<string, unknown>;
  delete unstamped.protocol;
  await page.getByTestId("receive-input").fill(JSON.stringify(unstamped, null, 2));
  await page.getByTestId("receive-add").click();
  await page.getByTestId("detail-receipt").click();

  // Same credential, same root - only the reachable base is absent.
  await expect(page.getByTestId("receipt-id")).toContainText("9RVBXK8AFQ2C");
  await expect(page.getByTestId("receipt-root")).toContainText(
    "0x199948111387332ec1e85a4d1dc4651ea691bb7bcbad768e3a2f8c38b290005b",
  );
  await expect(page.getByTestId("receipt-public-url")).toContainText(
    "published no reachable status URL",
  );
  await expect(page.getByTestId("receipt-public-url")).not.toContainText("gov.example");
  await expect(page.getByTestId("receipt-qr")).toHaveCount(0);
});

test("receive rejects a tampered credential", async ({ page }) => {
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  // Corrupt a disclosed field so the recomputed Merkle root no longer matches.
  const tampered = (await page.getByTestId("receive-input").inputValue()).replace("Rabies", "Sugar");
  await page.getByTestId("receive-input").fill(tampered);
  await page.getByTestId("receive-add").click();
  await expect(page.getByTestId("receive-error")).toContainText("integrity");
});

test("consent history: owner-blind Verified events render as the owner's receipts", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // With no held credentials there is no tag id to look up - the page says so, without touching RPC.
  await page.goto("/consents");
  await expect(page.getByTestId("empty-consents-no-credentials")).toBeVisible();
  expect(rpc.filter((r) => r.method === "eth_getLogs")).toHaveLength(0);

  // Receive Rex's credential (tag handle 424242) → its consent history becomes attributable.
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  await page.getByTestId("receive-add").click();
  await expect(page.getByTestId("detail-name")).toHaveText("Rex");

  await page.goto("/consents");
  await expect(page.getByTestId("consent-count")).toContainText("2 granted");
  const rows = page.getByTestId("consent-row");
  await expect(rows).toHaveCount(2);

  // The wallet queried the chain for exactly its own canonical (field-hashed) tag id.
  const logQueries = rpc.filter((r) => r.method === "eth_getLogs");
  expect(logQueries.length).toBeGreaterThan(0);
  expect(JSON.stringify(logQueries[0]!.params)).toContain(REX_DOGTAGID_TOPIC);

  // Newest first: the open-window boarding intake, then the closed-window travel check.
  await expect(rows.nth(0)).toContainText("Boarding intake");
  await expect(rows.nth(0)).toContainText("Rex");
  await expect(rows.nth(0)).toContainText("VACCINATION");
  await expect(rows.nth(0).getByTestId("consent-row-window")).toHaveText("Window open");
  await expect(rows.nth(1)).toContainText("Travel check");
  await expect(rows.nth(1).getByTestId("consent-row-window")).toHaveText("Window closed");

  // Consent is a point-in-time act - history offers NOTHING to cancel, on the list or the detail.
  await expect(page.locator("body")).not.toContainText(/revoke|cancel|withdraw/i);

  // Detail: purpose, record type (recovered from the tx calldata), relayer, window, confirmation.
  await rows.nth(0).click();
  await expect(page.getByTestId("consent-detail")).toBeVisible();
  await expect(page.getByTestId("consent-detail-purpose")).toHaveText("Boarding intake");
  await expect(page.getByTestId("consent-detail-recordtype")).toHaveText("VACCINATION");
  await expect(page.getByTestId("consent-detail-tag")).toContainText("Rex");
  await expect(page.getByTestId("consent-detail-tag")).toContainText("424242");
  await expect(page.getByTestId("consent-detail-relayer")).toHaveText(RELAYER);
  await expect(page.getByTestId("consent-detail-window")).toContainText("Window open");
  await expect(page.getByTestId("consent-detail-onchain")).toContainText("Recorded on-chain");
  await expect(page.getByTestId("consent-detail-granted")).toContainText("block 200000");
  await expect(page.getByTestId("consent-detail-tx")).toContainText("0xd1d1");
  await expect(page.getByTestId("consent-detail-nullifier")).toHaveText(OPEN_NULLIFIER);
  await expect(page.locator("body")).not.toContainText(/revoke|cancel|withdraw/i);
  // The print affordance is the only export; there is no share path out of the private history.
  await expect(page.getByTestId("consent-print")).toBeVisible();
  await expect(page.getByTestId("consent-detail-loading")).toHaveCount(0);
});

/** The bundled endpoint, normalized the way `normalizeRpcUrl` stores and displays it. */
const BUNDLED_RPC = "https://devrpc.roax.net/";
const GOOD_RPC = "https://good-peer.rpc.test/";
const WRONG_CHAIN_RPC = "https://other-chain.rpc.test/";
const RPC_STORAGE_KEY = "dogtag.roax-rpc-url.v1";

interface EndpointCall {
  url: string;
  method: string;
}

/**
 * Endpoint-probe mocks for the Settings surface: the bundled peer and one candidate answer ROAX
 * chain 135, another answers chain 5. Returns every request with its URL, so a test can assert that
 * a rejected peer received `eth_chainId` and nothing else.
 *
 * Replaces the credential mocks for this test rather than layering on them, so route precedence
 * cannot decide whether a request is recorded.
 */
async function installEndpointMocks(page: Page): Promise<EndpointCall[]> {
  const calls: EndpointCall[] = [];
  await page.unrouteAll();
  await page.route(/(devrpc\.roax\.net|rpc\.test)/, async (route) => {
    const url = route.request().url();
    let req: { id?: unknown; method?: string } = {};
    try {
      req = JSON.parse(route.request().postData() || "{}");
    } catch {
      /* recorded as an empty method below */
    }
    calls.push({ url, method: req.method ?? "" });
    const chainId = url.startsWith(WRONG_CHAIN_RPC) ? "0x5" : "0x87";
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: req.id ?? 1,
        result: req.method === "eth_chainId" ? chainId : "0x" + "0".repeat(63) + "1",
      }),
    });
  });
  return calls;
}

/**
 * Endpoint settings on the REAL owner surface, which is where the verdict is actually reported.
 *
 * The hook's own persist/reset changes the preference it subscribes to, so a re-sync effect that
 * cancelled the in-flight operation made every save that genuinely CHANGED the endpoint report
 * nothing at all - the silent worst case being a rejection that quietly cleared a working custom
 * peer behind no message. Both halves are asserted here: a save that persists must show its success,
 * and a rejection must show its alert AND have really cleared storage and reverted the field.
 */
test("endpoint settings: every save reports its verdict, and a rejection clears the override", async ({
  page,
}) => {
  const calls = await installEndpointMocks(page);

  await page.goto("/settings");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  const field = page.locator("#owner-roax-rpc");
  const save = page.getByRole("button", { name: "Check and save" });

  // No preference yet: the bundled endpoint is active and there is nothing to restore.
  await expect(field).toHaveValue(BUNDLED_RPC);
  await expect(page.getByRole("button", { name: "Restore default" })).toBeDisabled();

  // 1. A same-chain custom peer is accepted, PERSISTED, and reports success.
  await field.fill(GOOD_RPC);
  await save.click();
  await expect(page.getByRole("status")).toContainText(
    "Custom endpoint saved and confirmed on ROAX chain 135.",
  );
  await expect(field).toHaveValue(GOOD_RPC);
  expect(await page.evaluate((k) => localStorage.getItem(k), RPC_STORAGE_KEY)).toBe(GOOD_RPC);
  // Accepting it took exactly one guard probe, and no address-bound request rode along.
  expect(calls.filter((c) => c.url.startsWith(GOOD_RPC))).toEqual([
    { url: GOOD_RPC, method: "eth_chainId" },
  ]);

  // 2. Replacing it with an off-chain peer is REJECTED: alert, storage cleared, field reverted.
  await field.fill(WRONG_CHAIN_RPC);
  await save.click();
  await expect(page.getByRole("alert")).toContainText(
    "The endpoint reports chain 5; DogTag's bundled contracts are for chain 135.",
  );
  await expect(page.getByRole("alert")).toContainText(
    "The custom endpoint was removed; blockchain reads use the bundled default.",
  );
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(field).toHaveValue(BUNDLED_RPC);
  expect(await page.evaluate((k) => localStorage.getItem(k), RPC_STORAGE_KEY)).toBeNull();

  // The rejected peer learned only which chain it claims to be - never a contract read.
  expect(calls.filter((c) => c.url.startsWith(WRONG_CHAIN_RPC))).toEqual([
    { url: WRONG_CHAIN_RPC, method: "eth_chainId" },
  ]);
  // And the bundled endpoint was guarded independently before becoming the fallback.
  expect(calls.filter((c) => c.url.startsWith(BUNDLED_RPC))).toEqual([
    { url: BUNDLED_RPC, method: "eth_chainId" },
  ]);

  // 3. Restore default is the third preference-changing operation, and it reports its own verdict.
  // A reset changes the very preference the hook subscribes to, so it is exposed to the same
  // re-sync race as a save: silence here would leave the holder unable to tell a cleared override
  // from a click that did nothing.
  await field.fill(GOOD_RPC);
  await save.click();
  await expect(page.getByRole("status")).toContainText(
    "Custom endpoint saved and confirmed on ROAX chain 135.",
  );
  await page.getByRole("button", { name: "Restore default" }).click();
  await expect(page.getByRole("status")).toContainText(
    "Custom endpoint removed. Blockchain reads use the bundled default.",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(field).toHaveValue(BUNDLED_RPC);
  expect(await page.evaluate((k) => localStorage.getItem(k), RPC_STORAGE_KEY)).toBeNull();
  // And the override really is gone rather than merely reported gone: there is nothing left to restore.
  await expect(page.getByRole("button", { name: "Restore default" })).toBeDisabled();
});
