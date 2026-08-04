import { test, expect, type Page, type Request, type Route } from "@playwright/test";
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";
import { keccak256, toBytes } from "viem";

/**
 * E2E for the permissionless, direct-to-RPC credential verify panel (fm/dogtag-webverify-n3).
 *
 * Proves the decoupling: when the operator clicks "Verify credential", the browser reads the ROAX
 * chain DIRECTLY (viem `eth_call` and `eth_getLogs` over the public RPC) and classifies the credential
 * itself - the operator-gated `POST /verify/credential` relay is never called. We drive the REAL
 * `Verify` page and REAL `@dogtag/ui` `CredentialVerifyPanel`; only the chain reads are mocked (calls
 * by their 4-byte selector, logs by their topic0 and the address they were put to) so the
 * valid/revoked/authorised verdicts render deterministically without a live anchored credential.
 * Integrity is the genuine offline `@dogtag/standard` recompute (real doc).
 *
 * The fake must ANCHOR THE CLONE THROUGH THE FACTORY, exactly as the shipped path does: the issuing
 * clone comes from `DogTagIssuerFactory.rootIssuer(root)`, and the record type from that clone's own
 * `recordType()`. A fake that only answered the per-clone reads would model the pre-audit world in
 * which the document's own `documentStore` gets to say whether it is valid - the forgery the
 * mandatory issuer-whitelist pillar exists to close.
 *
 * It must also model the pillar's HISTORICAL question rather than a current-state one. The pillar
 * reads the clone's own `registry()`, locates that clone's `RootIssued` log for the root, folds the
 * governing registry's `Whitelisted`/`Delisted` history at that point, and takes the last event at or
 * before it - because delisting is forward-only (`DogTagIssuer.sol:82`; `adminRevoke` is the
 * retroactive lever). So the scripted chain carries POSITIONED logs, and both directions of that rule
 * are asserted below: delisted-after still verifies, delisted-before still refuses.
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
  // `DogTagIssuer.registry()` - the ONE authority whose grant log answers for this contract, read off
  // the clone rather than off this client's configuration. Confirmed with `cast keccak "registry()"`.
  registry: "0x7b103999",
} as const;

/**
 * Event topic0s the pillar filters on. These are FULL 32-byte keccaks, not 4-byte selectors: a value
 * derived at the wrong width matches no log at all, which reads exactly like "never granted" and would
 * make every assertion below pass for the wrong reason. Confirmed with `cast keccak`.
 */
const TOPIC = {
  rootIssued: "0xf8cd30a628b432a1200caf81085096c82a5f570da14360572b72d4e0ba57e6d7",
  /**
   * `ProviderRegistry.RightsSet(address indexed account, uint256 rights)`.
   *
   * This REPLACED the record-type-keyed `Whitelisted`/`Delisted` pair when rights became a bitmask on
   * an address. Three things about the new shape matter to the mock below, and each is a way to model
   * it wrongly while looking right:
   *
   *   - it is indexed on the ACCOUNT alone, so topic1 is the signer and there is no record type to
   *     narrow by (the reader passes `service` and deliberately ignores it in the filter);
   *   - grant and withdrawal are the SAME event, told apart by the bits in `rights` rather than by
   *     two topics - so a withdrawal is `rights` with bit 0 CLEAR, not a second event name;
   *   - `rights` is the account's COMPLETE mask after the write, never a delta, which is what lets
   *     the fold take the last event at or before the anchoring and need no prior state.
   */
  rightsSet: "0xbc9c679fe541a4f3fcf5f2887c4adcd6e7703f7ea9d0933b8862662f8290af7f",
} as const;

/**
 * `ProviderRegistry.RIGHT_ISSUE` - bit 0, the only bit that decides the issuance axis.
 *
 * Mirrors `packages/ui/src/wallet/contracts.ts`'s constant of the same name. It is a WIRE FORMAT
 * position, so the fixtures below set it as a BIT rather than comparing the word: bit 0 is the only
 * settable bit today, so "the word equals 1" and "bit 0 is set" agree on every mask the contract can
 * currently emit - and that coincidence is exactly what would let a whole-word comparison survive
 * review until a second right is allocated. `rightsWithAnUnrelatedBit` below exists to break the tie.
 */
const RIGHT_ISSUE = 1n;

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};
/** The address the chain reports as this root's originator - resolved, never typed by the operator. */
const SIGNER = "0xabc0000000000000000000000000000000000abc";
/**
 * The `IssuerRegistry` the CLONE names, and therefore the only authority whose grant log answers for
 * its issuances. Deliberately a value of its own rather than whatever this client is configured with:
 * `IssuerRegistry._wl` and its events are per-CONTRACT, so the pillar reads this address off
 * `DogTagIssuer.registry()`. The mock serves the grant log ONLY at this address, so a regression that
 * asked the configured registry instead would read an empty log and refuse a genuine credential -
 * which is the shape this spec has to be able to catch, not merely avoid.
 */
const GOVERNING_REGISTRY = "0x00000000000000000000000000000000000000ab";
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
const quantity = (n: number) => "0x" + n.toString(16);

/** Where a mined log sits. The pillar sequences on `(blockNumber, logIndex)` and nothing else. */
interface LogPoint {
  blockNumber: number;
  logIndex: number;
}

/**
 * One `setRights` call, as the governing authority's own `RightsSet` log records it.
 *
 * `kind` is kept as the READABLE name for what the call did, because every scenario below reads as
 * "granted before / delisted after" and that vocabulary is the point being tested. What changed with
 * the bitmask is only the ENCODING: both are the same event, and the difference is whether bit 0 is
 * set in the mask this call left behind. `rights` overrides that default so a fixture can carry a
 * mask with other bits set - see `rightsOf`.
 */
interface GrantEvent extends LogPoint {
  kind: "whitelisted" | "delisted";
  /** The COMPLETE mask after this write. Defaults to just `RIGHT_ISSUE`, or 0 for a withdrawal. */
  rights?: bigint;
}

const rightsOf = (g: GrantEvent): bigint =>
  g.rights ?? (g.kind === "whitelisted" ? RIGHT_ISSUE : 0n);

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
  /** DogTagIssuer.registry() - the authority the grant log below is served at, and only there. */
  registry: string;
  /**
   * Where `DogTagIssuer.RootIssued(root)` sits, or `null` when this clone emitted none.
   *
   * `issuedAt` is a unix TIMESTAMP and cannot be compared against a log's height, so the anchoring
   * POINT is what the fold sequences the grant history against. Absent it, the pillar is indeterminate
   * rather than a refusal - never a pass.
   */
  rootIssuedAt: LogPoint | null;
  /** The governing registry's full grant history for `(recordType, issuedBy)`, in log order. */
  grants: GrantEvent[];
}

/**
 * One `eth_getLogs` row, complete enough for viem's log formatter.
 *
 * `blockNumber` and `logIndex` are MANDATORY here, and that is not tidiness: viem treats a log missing
 * either as PENDING and hands back `null`, which the shipped readers now report as `UNPOSITIONED_LOG`
 * - a log whose place in the sequence is unknown cannot be folded, so the whole pillar answers
 * indeterminate. An omitted field would therefore leave `issuerWhitelisted` null and fail a scenario
 * for a reason nobody wrote, rather than exercising the case it names.
 */
function logRow(address: string, topics: string[], at: LogPoint, data = "0x") {
  return {
    address,
    topics,
    data,
    blockNumber: quantity(at.blockNumber),
    logIndex: quantity(at.logIndex),
    transactionIndex: "0x0",
    transactionHash: "0x" + "11".repeat(32),
    blockHash: "0x" + "22".repeat(32),
    removed: false,
  };
}

/**
 * Intercept the ROAX public RPC and answer the verify reads. `eth_call` dispatches on the 4-byte
 * selector, `eth_getLogs` on `topics[0]` AND the address it was put to; remaining methods (eth_chainId,
 * wagmi bootstrap) are answered benignly so the app still boots. An `eth_call` or an `eth_getLogs` this
 * fake does not model is a hard FAILURE naming what was asked - never a fabricated answer. Returns a
 * mutable list of every URL the page requested, so the test can assert the operator relay was NOT hit.
 */
async function mockRoaxRpc(page: Page, state: ChainState): Promise<string[]> {
  const requestedUrls: string[] = [];
  page.on("request", (r: Request) => requestedUrls.push(r.url()));

  const answer = (id: unknown, result: unknown) => ({ jsonrpc: "2.0", id, result });
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
      if (sel === SEL.registry) return answer(msg.id, addressWord(state.registry));
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
    if (msg.method === "eth_getLogs") {
      const filter = (msg.params?.[0] ?? {}) as { address?: string; topics?: unknown[] };
      const addr = String(filter.address ?? "").toLowerCase();
      const t0 = String(Array.isArray(filter.topics?.[0]) ? "" : (filter.topics?.[0] ?? ""));
      // Keyed on the ADDRESS as well as the topic. A fake that answered the grant history whichever
      // contract was asked could not represent "the grant is in the registry the CLONE names, and only
      // there" - so a regression that read this client's configured registry would still find the
      // grant and this spec would go green while the shipped path asked the wrong authority. That is
      // the `MockChain` fake-integrity trap AGENTS.md records, in Playwright form.
      // The indexed arguments are echoed back from the filter rather than re-derived. viem passes its
      // `args` to `parseEventLogs`, which DROPS any log whose decoded arguments do not match them - so
      // a row carrying a topic this fake invented would be silently discarded and read as "no such
      // log", i.e. the indeterminate/refusal branch, for a fixture that meant the opposite.
      const echoed = (i: number, fallback: string) => {
        const t = filter.topics?.[i];
        return typeof t === "string" ? t : fallback;
      };
      if (t0 === TOPIC.rootIssued) {
        if (addr !== state.rootIssuer.toLowerCase() || !state.rootIssuedAt) return answer(msg.id, []);
        return answer(msg.id, [
          logRow(
            addr,
            [t0, echoed(1, "0x" + "0".repeat(64)), addressWord(state.issuedBy)],
            state.rootIssuedAt,
            uintWord(state.issuedAt),
          ),
        ]);
      }
      if (t0 === TOPIC.rightsSet) {
        // Served ONLY at the governing registry - the address the pillar read off the clone's own
        // `registry()`. A regression that asked this client's CONFIGURED registry instead would find
        // an empty log here and refuse a genuine credential, which is the shape this spec must be
        // able to catch rather than merely avoid.
        if (addr !== state.registry.toLowerCase()) return answer(msg.id, []);
        // ONE event for both directions now, so there is no `kind` to filter on: the whole history for
        // this account is returned in log order and the fold takes the last at or before the
        // anchoring. Filtering here would move the fold's own rule into the fake, where a regression
        // in it could not be observed.
        const topics = [t0, echoed(1, addressWord(state.issuedBy))];
        return answer(
          msg.id,
          state.grants.map((g) => logRow(addr, topics, g, uintWord(rightsOf(g)))),
        );
      }
      // Same rule, same reason, for the log shape. An unmodelled topic answered with `[]` reads as
      // "the registry recorded no grant", which is a DEFINITE refusal of the credential - an invented
      // accusation rather than an invented pass, and no better for it.
      throw new Error(
        `verify-credential e2e: unmodelled eth_getLogs topic0 ${t0} at ${addr}. The verify path reads ` +
          `a log this mock does not model - add it to TOPIC/ChainState instead of letting the fake ` +
          `answer with an empty history.`,
      );
    }
    // What is left is wagmi/viem bootstrap noise that the verify path never reads (the pillar's own
    // reads are all `eth_call` or `eth_getLogs` above), so an empty word keeps the app booting.
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

/** Where this root was anchored, and the two points the grant history is sequenced around it. */
const ANCHORED_AT: LogPoint = { blockNumber: 500, logIndex: 2 };
const GRANTED_BEFORE: GrantEvent = { kind: "whitelisted", blockNumber: 100, logIndex: 0 };
const DELISTED_AFTER: GrantEvent = { kind: "delisted", blockNumber: 900, logIndex: 0 };

/**
 * A genuinely anchored credential: the factory names the very clone the envelope claims, that clone
 * reports the record type the envelope claims, and the registry that GOVERNS that clone recorded a
 * grant to its originator before it anchored. Each test overrides only the one fact it is about.
 *
 * The grant is positioned strictly BEFORE the anchoring rather than merely present, because that is
 * the whole question the pillar asks: it folds the governing registry's log at the anchoring point and
 * takes the last event at or before it. A history with no position could not state this scenario.
 */
function anchoredChain(over: Partial<ChainState> = {}): ChainState {
  return {
    rootIssuer: ISSUER.documentStore,
    recordType: CHAIN_RECORD_TYPE_KEY,
    issuedAt: 1_699_000_000n,
    isValid: true,
    isRevoked: false,
    issuedBy: SIGNER,
    registry: GOVERNING_REGISTRY,
    rootIssuedAt: ANCHORED_AT,
    grants: [GRANTED_BEFORE],
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
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/Yes$/);
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

test("whitelist pillar gates the verdict: valid on-chain but unauthorised issuer fails", async ({
  page,
}) => {
  // The governing registry ANSWERED, and its own log holds no grant to this signer at or before the
  // anchoring. That is evidence about the credential - an honest `issue()` cannot pass
  // `onlyWhitelisted` in that state - so it is a definite refusal, not an indeterminate one. A read
  // that could not be made would be the other case entirely, and is covered below.
  const urls = await mockRoaxRpc(page, anchoredChain({ grants: [] }));
  await openVerifyAndSubmit(page, validDoc());

  // The record is valid on-chain, yet the credential fails: the address that actually issued this
  // root held no grant for the record type the clone reports. No operator input was needed to reach
  // that verdict, which is the point - the pillar can no longer be left unrun.
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/No$/);

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-whitelist-fail.png" });
});

test("the ISSUE BIT is read out of the mask, not the whole word", async ({ page }) => {
  // Bit 0 is the only settable bit today, so "the word equals 1" and "bit 0 is set" agree on every
  // mask the contract can currently emit - which is exactly what would let a whole-word comparison
  // survive review until a second right is allocated. At that point a mask of 0b11 decodes as
  // malformed and refuses every credential its holder ever issued, fleet-wide, on a rendered verdict.
  //
  // AGENTS.md requires a case with higher bits set in all four language ports for this reason; this is
  // the web one. The credential is otherwise the passing fixture, so a failure here can only be the
  // mask decode.
  const urls = await mockRoaxRpc(
    page,
    anchoredChain({
      grants: [{ ...GRANTED_BEFORE, rights: RIGHT_ISSUE | (1n << 7n) }],
    }),
  );
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: pass")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/Yes$/);
  assertNoOperatorRelay(urls);
});

test("a mask with OTHER bits but not the issue bit is a definite refusal", async ({ page }) => {
  // The other half of the pair, and the one that keeps the case above from passing vacuously: an
  // account holding some future right but NOT `RIGHT_ISSUE` was never authorised to issue, so this
  // must refuse. A reader that merely tested the mask for non-zero would pass both.
  const urls = await mockRoaxRpc(
    page,
    anchoredChain({ grants: [{ ...GRANTED_BEFORE, rights: 1n << 7n }] }),
  );
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/No$/);
  assertNoOperatorRelay(urls);
});

test("delisting is forward-only: a signer delisted AFTER the anchoring still verifies", async ({
  page,
}) => {
  // `DogTagIssuer.sol:82` states the rule in the contract's own source and `adminRevoke` is the
  // retroactive lever, so an ordinary key rotation, a retirement or a lapsed practice licence must not
  // render every credential that signer ever anchored a forgery. This is the direction a current-state
  // `isWhitelistedFor` read got wrong, on a rendered surface.
  const urls = await mockRoaxRpc(page, anchoredChain({ grants: [GRANTED_BEFORE, DELISTED_AFTER] }));
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: pass")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/Yes$/);

  assertNoOperatorRelay(urls);
});

test("delisting before the anchoring still refuses - the pair, not just the fix's direction", async ({
  page,
}) => {
  // The mirror. A check that only ever cleared would satisfy the test above on its own, so both
  // directions are asserted: the grant was withdrawn BEFORE this root was anchored, so the signer held
  // no authority when it acted and the credential is refused.
  const urls = await mockRoaxRpc(
    page,
    anchoredChain({
      grants: [GRANTED_BEFORE, { kind: "delisted", blockNumber: 400, logIndex: 0 }],
    }),
  );
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/No$/);

  assertNoOperatorRelay(urls);
});

test("no anchoring event: the pillar is Unresolved, never a pass and never an accusation", async ({
  page,
}) => {
  // `issuedAt` is a unix TIMESTAMP, so without the `RootIssued` log there is no point to sequence the
  // grant history against. That is our inability to put the question - distinct from the registry
  // answering with an empty log above, which IS evidence - so it must render as a failure to establish
  // the claim rather than as either verdict about the credential.
  const urls = await mockRoaxRpc(page, anchoredChain({ rootIssuedAt: null }));
  await openVerifyAndSubmit(page, validDoc());

  await expect(page.getByText("Verdict: fail")).toBeVisible();
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/Unresolved$/);

  assertNoOperatorRelay(urls);
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
  await expect(pillar(page, "Issuer authorised at issuance")).toHaveText(/Unresolved$/);

  assertNoOperatorRelay(urls);
  await panelCard(page).screenshot({ path: "e2e-artifacts/verify-unresolved-issuer.png" });
});
