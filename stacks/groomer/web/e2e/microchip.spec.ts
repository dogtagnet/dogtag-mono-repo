import { expect, test, type Page, type Route } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

/**
 * The microchip cross-check, as the OPERATOR sees it on the pet page.
 *
 * Linking a DogTag is otherwise unchecked — a mistyped digit or two similar dogs in one afternoon
 * give a link that is structurally perfect and about the wrong animal. The credential carries the
 * microchip as a Merkle leaf, so when the shop has one on file too the backend compares them and
 * returns a structured verdict. This spec is about whether that verdict actually REACHES the screen.
 *
 * Three things nothing else covers:
 *
 *  1. **The refusal renders at all.** A mismatch is an HTTP 409, so the panel is reached through the
 *     api client's ERROR path — `makeError` attaching the parsed body, then `microchipCheckFromError`
 *     reading it back off the thrown error. Unit tests hand that function a hand-built object; only a
 *     real request through `createApiClient` proves the shape survives. If it ever stops, the
 *     operator gets a bare "Could not link the DogTag" and never learns the codes disagree — the
 *     verdict would be correct on the wire and lost before the screen.
 *  2. **"Not compared" renders as neither neighbour.** Not as a silent pass (a check that did not run
 *     is not one that passed) and — the mistake this surface would make — not as a refusal either.
 *     An unchipped animal is ordinary; painting it red would make the field unusable.
 *  3. **A microchip we could not READ is loud.** It is the state that looks most like the benign one
 *     and is the reason this check once shipped inert.
 *
 * Every `microchipCheck` body below is COPIED VERBATIM from the real backend's responses, captured by
 * driving `stacks/vet/api`'s router end to end. Hand-writing them would test this spec's idea of the
 * wire format rather than the format — the exact trap that let the check ship inert the first time.
 *
 * Mocked, like every groomer spec here, and NOT in CI (needs a served portal + browsers):
 *
 *   pnpm --filter @dogtag/groomer-web dev
 *   pnpm --filter @dogtag/groomer-web test:e2e -- microchip.spec.ts
 *
 * Set `MICROCHIP_SHOTS=<dir>` to also write one screenshot per state for review.
 */

const OP_TOKEN_KEY = "groomer.opToken";
const PET_ID = "pet-e2e-chip";
const CLIENT_ID = "client-e2e-chip";
const CHIP = "985141006580319";
const OTHER_CHIP = "900000000000001";

const SHOTS = process.env.MICROCHIP_SHOTS ?? "";

/** Captured from `POST /pets/{id}/dogtag` — HTTP 200. */
const MATCHED = {
  state: "matched",
  microchip: CHIP,
  detail: "The credential's microchip matches this pet's record.",
};

/** Captured from `POST /pets/{id}/dogtag` — HTTP 409 body. */
const MISMATCH = {
  state: "mismatch",
  petMicrochip: OTHER_CHIP,
  credentialMicrochip: CHIP,
  detail:
    `Microchip mismatch: this pet's record says ${OTHER_CHIP}, the credential says ${CHIP}. ` +
    "The credential is not refused and stays valid — it describes a different animal to the one on " +
    "this record. Check the DogTag id and this pet's microchip before linking.",
};

/** Captured — the commonest state in the product: an animal with no chip. */
const PET_HAS_NO_MICROCHIP = {
  state: "notComparable",
  reason: "petHasNoMicrochip",
  isFailure: false,
  detail:
    "This pet has no microchip on file, so the two could not be compared. Many animals are not " +
    "chipped; record one here if this pet has it.",
};

/** Captured with the held-credential read failing — a failure to LOOK, not an absence. */
const COULD_NOT_READ = {
  state: "notComparable",
  reason: "couldNotRead",
  isFailure: true,
  detail:
    "The held credential could not be read, so the microchip was not compared: injected client " +
    "cache read failure",
};

/** Captured with a microchip at a key path this build does not recognise. */
const UNRECOGNISED = {
  state: "unrecognisedCredentialLeaf",
  keyPaths: ["credentialSubject.chipDetails.microchipIdentifier"],
  detail:
    "The microchip could NOT be checked: this credential carries microchip data at " +
    "credentialSubject.chipDetails.microchipIdentifier, which this system does not know how to " +
    "read, so nothing was compared. This is a defect in this system's reader, not a statement " +
    "about the animal or the credential — report it so the key path can be added.",
};

function petRow(microchipCode: string | null, name = "Rex") {
  return {
    petId: PET_ID,
    clientId: CLIENT_ID,
    clientName: "Alice Tan",
    name,
    species: "dog",
    breed: "Standard Poodle",
    sex: "",
    dateOfBirth: "",
    notes: "",
    dogTagId: null as string | null,
    microchipCode,
    updatedAt: 0,
  };
}

const emptyPage = { rows: [], total: 0, limit: 25, offset: 0 };

/**
 * @param link what `POST /pets/{id}/dogtag` answers — status plus the body the backend really sends.
 */
async function mockBackend(
  page: Page,
  pet: ReturnType<typeof petRow>,
  link: { status: number; json: unknown },
) {
  // The page RE-READS the pet after a successful link, so the stored row has to move the way the
  // backend's would. A static row would leave the record looking unlinked after a 200 and make the
  // "an absent microchip never blocks a link" assertion untestable.
  let stored = pet;
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    // The portal's on-load custody probe. A catch-all `{}` here is read as LOCKED and redirects to
    // /unlock instead of the page under test — `activeSigner` must be keyed on PRESENCE.
    if (path === "/issuer/signers") {
      return route.fulfill({ json: { signers: [] } });
    }
    if (path === `/pets/${PET_ID}` && request.method() === "GET") {
      return route.fulfill({ json: stored });
    }
    if (path === `/pets/${PET_ID}/dogtag` && request.method() === "POST") {
      if (link.status === 200) {
        stored = { ...stored, dogTagId: (request.postDataJSON() ?? {}).dogTagId ?? null };
      }
      return route.fulfill({ status: link.status, json: link.json });
    }
    return route.fulfill({ json: emptyPage });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
  if (SHOTS) mkdirSync(SHOTS, { recursive: true });
});

/** Type a tag id, submit, and wait for the verdict panel to render. */
async function linkTag(page: Page) {
  await page.goto(`/pets/${PET_ID}`);
  await page.getByPlaceholder("The number the owner's app shows").fill("4");
  await page.getByRole("button", { name: "Add DogTag" }).click();
  await expect(page.getByTestId("microchip-check")).toBeVisible();
}

async function shot(page: Page, name: string) {
  if (!SHOTS) return;
  // Let the toast expire first: it floats over the verdict panel and would hide the half of the
  // sentence that says WHY the two could not be compared — the part worth reviewing. Asserted
  // present BEFORE waiting for it to go, so a selector that matches nothing cannot make the wait a
  // silent no-op that leaves the toast in every capture.
  const toast = page.getByRole("status");
  await expect(toast).toHaveCount(1);
  await expect(toast).toHaveCount(0, { timeout: 10_000 });
  await page.getByTestId("microchip-check").scrollIntoViewIfNeeded();
  await page.screenshot({ path: join(SHOTS, `${name}.png`), fullPage: true });
}

test("a matching microchip is shown as positive evidence the tag belongs to this animal", async ({
  page,
}) => {
  await mockBackend(page, petRow(CHIP), {
    status: 200,
    json: { ...petRow(CHIP), dogTagId: "4", microchipCheck: MATCHED },
  });

  await linkTag(page);
  const panel = page.getByTestId("microchip-check");
  await expect(page.getByTestId("microchip-check-headline")).toHaveText(
    "Microchip matches the credential",
  );
  await expect(panel).toContainText(CHIP);
  // The one state that may read green.
  await expect(panel).toHaveClass(/border-success/);
  await shot(page, "01-matched");
});

test("a mismatch refuses the link, names BOTH codes, and does not accuse the credential", async ({
  page,
}) => {
  // The whole error path: 409 -> the api client attaches the parsed body -> the page reads the
  // structured verdict back off the thrown error. Hand-built error objects cannot prove this.
  await mockBackend(page, petRow(OTHER_CHIP, "Bella"), {
    status: 409,
    json: { error: MISMATCH.detail, microchipCheck: MISMATCH },
  });

  await linkTag(page);
  const panel = page.getByTestId("microchip-check");
  await expect(page.getByTestId("microchip-check-headline")).toHaveText(
    "Microchip does not match this credential",
  );
  await expect(panel).toContainText(OTHER_CHIP);
  await expect(panel).toContainText(CHIP);
  // A statement about the LINK, never about the credential.
  await expect(panel).toContainText("stays valid");
  await expect(panel).toHaveClass(/border-danger/);
  await shot(page, "02-mismatch");
});

test("an unchipped animal links freely and reads as neither a pass nor a refusal", async ({
  page,
}) => {
  // Cats routinely are not chipped. This is the commonest state in the product, so it must be
  // neutral: no red, no green, and copy that says plainly that the two could not be compared.
  await mockBackend(page, petRow(null, "Mittens"), {
    status: 200,
    json: { ...petRow(null, "Mittens"), dogTagId: "4", microchipCheck: PET_HAS_NO_MICROCHIP },
  });

  await linkTag(page);
  const panel = page.getByTestId("microchip-check");
  await expect(page.getByTestId("microchip-check-headline")).toHaveText("Microchip not compared");
  await expect(panel).toContainText("Many animals are not chipped");
  await expect(panel).not.toHaveClass(/border-success|border-danger|border-warning/);
  // The link went through: the record now shows the tag as linked, beside an untouched blank
  // microchip — an absent code must never block, and must never be filled in from the credential.
  await expect(page.getByText("linked to this pet's record")).toBeVisible();
  await shot(page, "03-not-comparable-pet-has-no-microchip");
});

test("a read that did not resolve warns, rather than passing for an ordinary absence", async ({
  page,
}) => {
  await mockBackend(page, petRow(CHIP), {
    status: 200,
    json: { ...petRow(CHIP), dogTagId: "4", microchipCheck: COULD_NOT_READ },
  });

  await linkTag(page);
  const panel = page.getByTestId("microchip-check");
  await expect(page.getByTestId("microchip-check-headline")).toHaveText("Microchip not compared");
  await expect(panel).toContainText("could not be read");
  // The tone comes from the wire's own fact/failure flag — same headline as the case above, a
  // different colour, because one is an ordinary absence and this is a failure to look.
  await expect(panel).toHaveClass(/border-warning/);
  await shot(page, "04-not-comparable-could-not-read");
});

test("a microchip this build cannot READ is loud and never reads as nothing to compare", async ({
  page,
}) => {
  await mockBackend(page, petRow(CHIP), {
    status: 200,
    json: { ...petRow(CHIP), dogTagId: "4", microchipCheck: UNRECOGNISED },
  });

  await linkTag(page);
  const panel = page.getByTestId("microchip-check");
  // NOT "Microchip not compared" — that sentence belongs to the unchipped animal, and reading it
  // over a credential that IS carrying a microchip is exactly what let this check ship inert.
  await expect(page.getByTestId("microchip-check-headline")).toHaveText(
    "Microchip check could not run",
  );
  await expect(panel).toContainText("credentialSubject.chipDetails.microchipIdentifier");
  await expect(panel).toHaveClass(/border-warning/);
  await shot(page, "05-unrecognised-credential-leaf");
});

test("the pet form offers the microchip and says leaving it blank is fine", async ({ page }) => {
  // Silence beside a field the portal uses to cross-check a credential reads as "required", and
  // many animals genuinely have no chip.
  await mockBackend(page, petRow(CHIP), { status: 200, json: petRow(CHIP) });

  await page.goto(`/pets/${PET_ID}`);
  await page.getByRole("button", { name: "Edit" }).first().click();
  await expect(page.getByLabel("Microchip")).toHaveValue(CHIP);
  await expect(page.getByText("Optional. If recorded, it is checked against the credential")).toBeVisible();
  if (SHOTS) await page.screenshot({ path: join(SHOTS, "06-pet-form-microchip-field.png"), fullPage: true });
});
