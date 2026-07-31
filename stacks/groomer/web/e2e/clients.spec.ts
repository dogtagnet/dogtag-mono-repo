import { expect, test, type Page, type Route } from "@playwright/test";

/**
 * The client form must NOT be a way to link a DogTag - and must not silently unlink one either.
 *
 * Linking happens on the pet page, which is where the meaning lives: that it records this shop's own
 * note of which tag the pet holds, that it mints nothing and writes nothing on chain, and that
 * removing it is a local reversible disassociation rather than a revocation. A bare optional field
 * on the client form carried none of that, so it is gone.
 *
 * Removing an input from a form that submits a WHOLE-DOCUMENT replace is the risky half, and it is
 * what the second test exists for: `PUT /clients/{id}` replaces the pet array outright, so the form
 * has to keep ECHOING each pet's stored `dogTagId` even though nothing renders it. If it ever stops,
 * every save quietly unlinks every tag the client holds - invisible in the UI until someone tries to
 * verify a pet.
 */

const OP_TOKEN_KEY = "groomer.opToken";
const CLIENT_ID = "client-e2e-1";
const PET_ID = "pet-e2e-1";
const DOG_TAG = "4";
const MICROCHIP = "985141006580319";

interface MockState {
  puts: Record<string, unknown>[];
}

const storedClient = {
  clientId: CLIENT_ID,
  name: "Alice Tan",
  email: "",
  phone: "+65 9123 4567",
  address: "",
  notes: "",
  pets: [
    {
      petId: PET_ID,
      name: "Rex",
      species: "dog",
      breed: "Standard Poodle",
      sex: "",
      dateOfBirth: "",
      notes: "",
      dogTagId: DOG_TAG,
      microchipCode: MICROCHIP,
    },
  ],
  createdAt: 0,
  updatedAt: 0,
};

async function mockBackend(page: Page, state: MockState) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    // Paged empty lists for the reads ClientDetail issues alongside the client itself: it fetches the
    // client, its appointments and its verifications in ONE Promise.all, so a catch-all `{}` leaves
    // `rows` undefined and the page throws before it can render.
    if (path === "/appointments" || path === "/verifications") {
      return route.fulfill({ json: { rows: [], total: 0, limit: 25, offset: 0 } });
    }
    if (path === "/clients" && request.method() === "GET") {
      return route.fulfill({ json: { rows: [storedClient], total: 1, limit: 20, offset: 0 } });
    }
    if (path === `/clients/${CLIENT_ID}` && request.method() === "GET") {
      return route.fulfill({ json: storedClient });
    }
    if (path === `/clients/${CLIENT_ID}` && request.method() === "PUT") {
      state.puts.push((request.postDataJSON() ?? {}) as Record<string, unknown>);
      return route.fulfill({ json: storedClient });
    }
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

test("offers no DogTag field on the client form", async ({ page }) => {
  await mockBackend(page, { puts: [] });

  await page.goto(`/clients/${CLIENT_ID}`);
  await page.getByRole("button", { name: "Edit" }).first().click();

  await expect(page.getByLabel("Pet name")).toBeVisible();
  // Linking is the pet page's job, and the form says so rather than leaving a silent gap.
  await expect(page.getByText(/DogTag id/i)).toHaveCount(0);
  await expect(page.getByText(/linked from the pet's own page/i)).toBeVisible();
});

test("still echoes a pet's stored DogTag so a client edit cannot unlink it", async ({ page }) => {
  const state: MockState = { puts: [] };
  await mockBackend(page, state);

  await page.goto(`/clients/${CLIENT_ID}`);
  await page.getByRole("button", { name: "Edit" }).first().click();

  // Change something entirely unrelated to the tag.
  await page.getByLabel("Pet name").fill("Rex II");
  await page.getByRole("button", { name: "Save changes" }).click();

  await expect.poll(() => state.puts.length).toBe(1);
  const pets = (state.puts[0] as { pets: Array<Record<string, unknown>> }).pets;
  expect(pets[0].name).toBe("Rex II");
  expect(pets[0].petId).toBe(PET_ID);
  // The whole point: absent here, the replace would drop the tag on the floor.
  expect(pets[0].dogTagId).toBe(DOG_TAG);
  // Same hazard, second field. The microchip is what lets a DogTag link be cross-checked against the
  // credential, and losing it fails SILENTLY — no error, the check simply stops firing on every
  // future link. So an unrelated edit must carry it through exactly as it carries the tag.
  expect(pets[0].microchipCode).toBe(MICROCHIP);
});
