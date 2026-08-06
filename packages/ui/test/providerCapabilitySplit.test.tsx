/**
 * @vitest-environment jsdom
 */
// The vet/groomer capability split, mounted (registry-plan S-15).
//
// A groomer VERIFIES and does not ISSUE, so it has no `DogTagIssuer` clone, and flows 1-3 are each
// keyed BY one - they are inapplicable for a groomer rather than merely hidden. Flow 4 is keyed by
// `providerId`, so it applies to every provider.
//
// This is mounted rather than asserted on a prop because the claim is about what a provider can
// REACH. A future edit that renders the issuance sections unconditionally would type-check, pass
// every other suite, and hand a groomer three flows it cannot complete - and the browser check that
// caught it once is not repeatable.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import type { ProviderContracts } from "../src/provider";

// wagmi is the only thing here that needs a provider tree, and nothing under test reads more of it
// than these two hooks. Stubbing them keeps this a test of the capability split rather than of
// WagmiProvider.
vi.mock("wagmi", () => ({
  useAccount: () => ({ address: undefined, isConnected: false }),
  useWriteContract: () => ({ writeContractAsync: async () => "0x" }),
}));

const CONTRACTS: ProviderContracts = {
  core: "0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112",
  factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
  domainResolver: "0x7A9b0C1d2E3F4a5B6C7d8e9f0A1B2c3D4e5F6A7b",
  directory: "0x8b0c1D2E3f4a5B6c7D8E9F0A1B2C3d4E5f6A7b8C",
};

let root: Root | null = null;
let host: HTMLElement | null = null;

async function mount(capabilities: { issuance: boolean; listing: boolean }) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(
    createElement(ProviderSelfServiceFlows, {
      contracts: CONTRACTS,
      missingConfig: [],
      capabilities,
    }),
  );
  // Real macrotask turns rather than act(), which would reorder passive effects.
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  return host;
}

afterEach(() => {
  root?.unmount();
  host?.remove();
  root = null;
  host = null;
});

const sendIds = (el: HTMLElement) =>
  Array.from(el.querySelectorAll("[data-testid$='-send']")).map((n) =>
    n.getAttribute("data-testid"),
  );

describe("a vet is a provider that issues, so it gets all four flows", () => {
  it("renders every send action", async () => {
    const el = await mount({ issuance: true, listing: true });
    expect(new Set(sendIds(el))).toEqual(
      new Set(["deploy-send", "repoint-send", "domain-claim-send", "domain-none-send", "publish-send"]),
    );
    expect(el.textContent).toContain("Deploy your own contract");
    expect(el.textContent).toContain("Choose which contract is current");
    expect(el.textContent).toContain("Your domain");
    expect(el.textContent).toContain("Your listing");
  });

  it("offers no action while no wallet is connected and no plan has been run", async () => {
    // Non-vacuous: the count is asserted before the every(). An empty set satisfies `every` and
    // would make this pass against a page that rendered no buttons at all.
    const el = await mount({ issuance: true, listing: true });
    const sends = Array.from(el.querySelectorAll("[data-testid$='-send']"));
    expect(sends.length).toBe(5);
    expect(sends.every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
  });
});

describe("a groomer is a provider that does NOT issue, so three flows are inapplicable", () => {
  it("renders the listing flow and NOTHING keyed by a clone", async () => {
    const el = await mount({ issuance: false, listing: true });
    expect(sendIds(el)).toEqual(["publish-send"]);
    expect(el.textContent).not.toContain("Deploy your own contract");
    expect(el.textContent).not.toContain("Choose which contract is current");
    // "Your domain" would be a flow keyed by a clone this operator does not have.
    expect(el.textContent).not.toContain("Your domain");
    expect(el.textContent).toContain("Your listing");
  });

  it("still reaches the listing flow, which is what makes the split a split and not a removal", async () => {
    // The point of the case: a groomer publishes contacts and a location like any other provider,
    // and mounting for vets alone would have left it no way to do so at all.
    const el = await mount({ issuance: false, listing: true });
    expect(el.querySelector("[data-testid='publish-send']")).not.toBeNull();
    expect(el.textContent).toContain("A location is optional");
  });

  it("drops the record-type field, which only an issuance flow uses", async () => {
    const el = await mount({ issuance: false, listing: true });
    expect(el.querySelector("#recordType")).toBeNull();
    expect(el.querySelector("#providerId")).not.toBeNull();
  });

  it("still explains its blocked control IN FULL, although flow 1 would have carried that sentence", async () => {
    // A REGRESSION THE PAGE-WIDE REASON DEDUPE INTRODUCED, and it lands on the groomer only.
    // Reasons are said in full once and briefly after, in source order - and the first entries in
    // that order belong to flows 1-3, which a groomer does not render at all. So the full sentence
    // was assigned to a control that does not exist and the only reason on the page degraded to
    // "Unavailable while a field it needs is still empty", which names no field.
    //
    // Same rule as the withdraw-pin reason being placed last: a sentence may only be spent on a
    // control that is actually rendered. Here the gate is the capability block, one level up.
    const el = await mount({ issuance: false, listing: true });
    const reasons = [...el.querySelectorAll("[data-testid$='-reason'][data-block]")];
    expect(reasons.length).toBeGreaterThan(0);
    expect(reasons.some((r) => r.getAttribute("data-style") === "full")).toBe(true);
    // And the full one is the COMPLETE sentence, not the one-line repeat form. No wallet is
    // connected in this suite, so the obstacle here is `notConnected` and the half that matters is
    // where the button is - which the brief form does not carry.
    expect(el.textContent).toMatch(/button in the top right/i);
  });
});

describe("an unconfigured deployment checks nothing and says so", () => {
  it("renders the unconfigured state instead of any flow, whatever the capabilities", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    root.render(
      createElement(ProviderSelfServiceFlows, {
        contracts: CONTRACTS,
        missingConfig: ["VITE_PROVIDER_DIRECTORY_ADDR"],
        capabilities: { issuance: true, listing: true },
      }),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(host.textContent).toContain("Provider self-service is not configured");
    expect(host.textContent).toContain("Nothing about your provider record has been checked");
    expect(host.textContent).toContain("VITE_PROVIDER_DIRECTORY_ADDR");
    // No action is reachable, so an unconfigured page cannot send anything.
    expect(sendIds(host)).toEqual([]);
  });
});
