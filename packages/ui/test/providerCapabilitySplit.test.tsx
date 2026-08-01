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
  core: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9",
  factory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6",
  domainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33",
  directory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0",
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
