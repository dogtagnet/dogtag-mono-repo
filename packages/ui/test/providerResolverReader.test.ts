// The LIVE reader's resolver reads, asserted on what reaches `readContract`.
//
// WHY THIS FILE EXISTS. The engine's suite injects a fake reader, so it cannot see any of this - and a
// mutation run proved it: making the live reader call every listed register "approved", and making it
// ask about the wrong permission bit, both survived the whole engine suite untouched. Neither is a
// decision the engine makes; both are the binding between a decision and the chain, and that binding
// had no test at all.
//
// Every assertion here is on the ARGUMENTS, never on a returned value, for the reason
// `providerDomainAndDeploy.test.ts` already gives about `canCreateService`: a fake that answers `true`
// satisfies any assertion about the result while the thing under test is silently dropped.
import { describe, expect, it, vi } from "vitest";
import {
  createLiveProviderReader,
  PROVIDER_PERMISSION_DIRECTORY_RESOLVER,
  ResolverKind,
  SERVICE_PERMISSION_DOMAIN_RESOLVER,
  type Address,
  type HexWord,
  type ProviderContracts,
} from "../src/provider";

const CONTRACTS: ProviderContracts = {
  core: "0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112",
  factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
  domainResolver: "0x7A9b0C1d2E3F4a5B6C7d8e9f0A1B2c3D4e5F6A7b",
  directory: "0x8b0c1D2E3f4a5B6c7D8E9F0A1B2C3d4E5f6A7b8C",
};
const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const SERVICE: Address = "0x1111111111111111111111111111111111111111";
const CALLER: Address = "0x2222222222222222222222222222222222222222";
const A: Address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: Address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

type Call = { functionName: string; args?: readonly unknown[]; address: string; account?: string };

/**
 * A chain that answers by function name, and records every call.
 *
 * `approvals` maps a lowercased address to what `isResolverApproved` says about it, so a case can
 * script "listed and pulled" - the state `resolverPage` keeps forever and the one an unfiltered client
 * would offer as a choice.
 */
function chain(options: {
  page?: readonly Address[];
  count?: bigint;
  approvals?: Readonly<Record<string, boolean>>;
  /** Force `resolverPage` to answer a cursor that does not advance, as a broken peer would. */
  stuckCursor?: boolean;
}) {
  const page = options.page ?? [A];
  const calls: Call[] = [];
  const readContract = vi.fn(async (call: Call) => {
    calls.push(call);
    switch (call.functionName) {
      case "resolverCount":
        return options.count ?? BigInt(page.length);
      case "resolverPage": {
        const cursor = call.args![1] as bigint;
        const limit = call.args![2] as bigint;
        const end = Math.min(Number(cursor) + Number(limit), page.length);
        return [page.slice(Number(cursor), end), options.stuckCursor ? cursor : BigInt(end)];
      }
      case "isResolverApproved":
        return options.approvals?.[(call.args![1] as string).toLowerCase()] ?? true;
      default:
        return true;
    }
  });
  return {
    calls,
    readContract,
    reader: createLiveProviderReader({ contracts: CONTRACTS, client: { readContract } as never }),
  };
}

const of = (calls: readonly Call[], name: string) => calls.filter((c) => c.functionName === name);

describe("the register list is read from the chain, with approval asked per entry", () => {
  it("asks isResolverApproved for EVERY listed address and reports each answer", async () => {
    // THE APPEND-ONLY TRAP, at the reader. `setResolverApproved` pushes an address the first time it
    // sees one and never removes it, so the page carries entries whose approval has been withdrawn -
    // and a reader that returned a bare address list, or hard-coded `approved: true`, would hand the
    // engine nothing to filter on. The engine's filter would then be dead code that still looked right.
    const { reader, calls } = chain({
      page: [A, B],
      approvals: { [A.toLowerCase()]: true, [B.toLowerCase()]: false },
    });
    const listings = await reader.approvedResolvers(ResolverKind.DIRECTORY);
    expect(listings).toEqual([
      { resolver: A, approved: true },
      { resolver: B, approved: false },
    ]);
    // One approval read per entry, each naming its own address - not one read reused for the page.
    expect(of(calls, "isResolverApproved").map((c) => c.args![1])).toEqual([A, B]);
  });

  it("reads through the CORE, and passes the kind it was asked about", async () => {
    // The two kinds are separate keyspaces. Asking DIRECTORY and getting DOMAIN's allowlist would make
    // every choice refused for a reason nothing on screen could explain.
    const { reader, calls } = chain({ page: [A] });
    await reader.approvedResolvers(ResolverKind.DOMAIN);
    for (const c of calls) expect(c.address).toBe(CONTRACTS.core);
    expect(of(calls, "resolverCount")[0]!.args).toEqual([ResolverKind.DOMAIN]);
    expect(of(calls, "resolverPage")[0]!.args![0]).toBe(ResolverKind.DOMAIN);
    expect(of(calls, "isResolverApproved")[0]!.args![0]).toBe(ResolverKind.DOMAIN);
  });

  it("pages to COMPLETION rather than returning the first page", async () => {
    // A resolver missing from a half-read list is indistinguishable from one the registrar never
    // approved - so on a deployment with one approved register per kind, stopping early would tell the
    // provider their only legitimate choice does not exist.
    const page = Array.from(
      { length: 250 },
      (_, i) => `0x${i.toString(16).padStart(40, "0")}` as Address,
    );
    const { reader, calls } = chain({ page });
    const listings = await reader.approvedResolvers(ResolverKind.DIRECTORY);
    expect(listings).toHaveLength(250);
    // Three pages at the contract's MAX_PAGE_SIZE of 100, and no page asked for more - `_checkPage`
    // reverts `BadPage()` above that, so a larger limit would fail the whole read.
    const pages = of(calls, "resolverPage");
    expect(pages).toHaveLength(3);
    for (const p of pages) expect(p.args![2]).toBe(100n);
    expect(pages.map((p) => p.args![1])).toEqual([0n, 100n, 200n]);
  });

  it("reads no page at all when the count is zero", async () => {
    // `resolverPage(kind, 0, limit)` is legal on an empty list, but asking is pointless - and a loop
    // that entered anyway on a zero count would need the cursor guard to save it.
    const { reader, calls } = chain({ page: [], count: 0n });
    expect(await reader.approvedResolvers(ResolverKind.DIRECTORY)).toEqual([]);
    expect(of(calls, "resolverPage")).toHaveLength(0);
  });

  it("THROWS rather than looping or truncating when a page does not advance", async () => {
    // A peer that answers a non-advancing cursor would spin forever, and stopping quietly would return
    // the partial list this method exists not to return. The contract cannot produce this state; a
    // broken or hostile peer can.
    const { reader } = chain({ page: [A, B], count: 2n, stuckCursor: true });
    await expect(reader.approvedResolvers(ResolverKind.DIRECTORY)).rejects.toThrow(
      /did not advance past cursor/i,
    );
  });

  it("propagates a failed read rather than answering an empty list", async () => {
    // The engine reports could-not-run from the throw. A reader that swallowed the fault and answered
    // `[]` would turn "we could not ask" into "DogTag has approved nothing".
    const readContract = vi.fn(async () => {
      throw new Error("rate limited");
    });
    const reader = createLiveProviderReader({
      contracts: CONTRACTS,
      client: { readContract } as never,
    });
    await expect(reader.approvedResolvers(ResolverKind.DIRECTORY)).rejects.toThrow(/rate limited/);
  });
});

describe("each write's authority is asked with ITS OWN permission bit", () => {
  it("asks canWriteProvider with PROVIDER_PERMISSION_DIRECTORY_RESOLVER", async () => {
    // A wrong bit is as silent as a wrong selector and worse to diagnose: the call succeeds and
    // answers a confident `false` about a permission nobody asked about, which a provider reads as
    // "you may not choose a register". The record bit (1) and the repoint bit (4) are both already
    // mirrored in this module, so a copy-paste from either is the realistic mistake.
    const { reader, calls } = chain({});
    await reader.canWriteProviderDirectoryResolver(PROVIDER, CALLER);
    const call = of(calls, "canWriteProvider")[0]!;
    expect(call.address).toBe(CONTRACTS.core);
    expect(call.args).toEqual([PROVIDER, CALLER, PROVIDER_PERMISSION_DIRECTORY_RESOLVER]);
    // Spelled out, so the assertion above cannot pass by comparing the constant with itself after
    // somebody redefines it.
    expect(call.args![2]).toBe(2);
  });

  it("asks canWriteService with SERVICE_PERMISSION_DOMAIN_RESOLVER, not the repoint bit", async () => {
    const { reader, calls } = chain({});
    await reader.canWriteServiceDomainResolver(SERVICE, CALLER);
    const call = of(calls, "canWriteService")[0]!;
    expect(call.args).toEqual([SERVICE, CALLER, SERVICE_PERMISSION_DOMAIN_RESOLVER]);
    expect(call.args![2]).toBe(2);
    expect(call.args![2]).not.toBe(4);
  });

  it("sends no `account` on either - neither predicate branches on msg.sender", async () => {
    // Unlike `canCreateService`, whose first term IS `generationOfFactory[msg.sender]`, both of these
    // take the caller as an explicit argument. Sending an account would be harmless here and is
    // asserted absent anyway, so the one read that genuinely needs it stays distinguishable.
    const { reader, calls } = chain({});
    await reader.canWriteProviderDirectoryResolver(PROVIDER, CALLER);
    await reader.canWriteServiceDomainResolver(SERVICE, CALLER);
    for (const c of calls) expect(c.account).toBeUndefined();
  });
});
