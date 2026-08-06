// What a wallet has deployed, read back from the chain (registry-plan S-15).
//
// THE INCIDENT. A captain deployed a contract from the provider self-service page. The transaction
// mined - `0x14a09008…`, block 352758, status 1, from the configured factory - and the page kept no
// record of it, so it could not tell him the address, the transaction, or what to do next. Pressing
// Deploy again is the natural response to a page that shows nothing, and it would have created a
// second contract.
//
// The fixtures below are that wallet's REAL chain state on ROAX, read with `cast` rather than
// invented, because it is a better adversary than anything hand-written would have been: one owner,
// five clones, TWO provider ids, and two contracts numbered 1 beside two numbered 0. A fixture with
// one contract per record type would let both of the two mistakes this module exists to avoid pass
// unnoticed.
import { describe, expect, it } from "vitest";
import {
  MAX_CLONE_NONCE,
  nextContractNumber,
  parseContractNumber,
  readDeploymentHistory,
  Standing,
  ZERO_PROVIDER_ID,
  type Address,
  type DeploymentHistory,
  type HexWord,
  type IssuerCreationLog,
  type ProviderChainReader,
  type ServiceRecord,
} from "../src/provider";

const OWNER: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

const VACCINATION: HexWord = "0x6510790a1a3e04db26bd73ea6246e7e8defb25eb4281f709e29decd6b8ca0561";
const TRAVEL: HexWord = "0x0ea3b61f198af15d1c1f1cd1bd926f52cb69cde62893f72fbb94e628c820321d";
const DOG_PROFILE: HexWord = "0x501883bc66249712c0662ee63b45b38088e876bff90ccfb60d6d0778a245683d";
const GROOMING: HexWord = "0x8ffc6babf10c6e34854b00b3100bd2c48dd01fec28a7c410e78c3a3a04775df3";
const EU_HEALTH: HexWord = "0xb9a3542063492a5b851ceda495dfd70430d085ab9ebf429d66077cd6ed4f0e21";

/** The provider four of the five clones were deployed for. */
const PROVIDER_A: HexWord = "0x12f607346023b299410b735818cd5a3321ecf77b";
/** The provider the captain's fifth one names. A DIFFERENT id, same wallet. */
const PROVIDER_B: HexWord = "0x7b160cb6dd3f8690247093c16d16a21e61d98eea";

const CAPTAINS_CLONE: Address = "0x14a090086a6fd747840b003a9c09521d09ddef3a";
const CAPTAINS_TX = "0x5e675ce06e9dc8cb9dbd968f10fa679e5ec4ed3fa5f00b46ae58fc5ffbb01cca";

/** `IssuerOwnerRegistered` on the ROAX factory for this owner, exactly as the chain holds them. */
const LOGS: IssuerCreationLog[] = [
  {
    clone: "0xdd1533d621ffba833738267b6b3ff58b71605d57",
    cloneNonce: 1n,
    providerId: PROVIDER_A,
    txHash: "0x2bd99e18770e7fb05bb024188e4dd40118a323ff4d93a96f6b020718977aa3d6",
    blockNumber: 339369n,
  },
  {
    clone: "0x0ca65d55d9092cac03a1981afd9a115905a526e3",
    cloneNonce: 1n,
    providerId: PROVIDER_A,
    txHash: "0x461b1276368c6af636685051ec84d4a6c8eb7e44f7997b5896b2d3845058fe23",
    blockNumber: 339382n,
  },
  {
    clone: "0xd6c312c59404e9c8b6b68a936d412273605da9f8",
    cloneNonce: 0n,
    providerId: PROVIDER_A,
    txHash: "0x15d4a66f63ec0039a91b8908f817843421a12107d012b428f09eddf73e637d36",
    blockNumber: 339692n,
  },
  {
    clone: "0xae05f415984b136934c085b7da7f3958bf8040e5",
    cloneNonce: 0n,
    providerId: PROVIDER_A,
    txHash: "0x5aa2e7ce87ef99b630740ea0ab2cb1bd3d4f85bbfc00b1042b52ef7e6202ac38",
    blockNumber: 352407n,
  },
  {
    clone: CAPTAINS_CLONE,
    cloneNonce: 1n,
    providerId: PROVIDER_B,
    txHash: CAPTAINS_TX,
    blockNumber: 352758n,
  },
];

const RECORD_TYPE_OF: Readonly<Record<string, HexWord>> = {
  "0xdd1533d621ffba833738267b6b3ff58b71605d57": VACCINATION,
  "0x0ca65d55d9092cac03a1981afd9a115905a526e3": TRAVEL,
  "0xd6c312c59404e9c8b6b68a936d412273605da9f8": DOG_PROFILE,
  "0xae05f415984b136934c085b7da7f3958bf8040e5": GROOMING,
  [CAPTAINS_CLONE]: DOG_PROFILE,
};

function service(providerId: HexWord, standing = Standing.ACTIVE): ServiceRecord {
  return {
    providerId,
    factoryGeneration: `0x${"4a".repeat(32)}`,
    recordType: DOG_PROFILE,
    confirmedOwner: OWNER,
    domainResolver: `0x${"0".repeat(40)}`,
    ownerEpoch: 1n,
    standing,
  };
}

/** Every read a case does not script THROWS, so nothing passes by leaning on a default. */
function reader(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  const unscripted = (name: string) => async () => {
    throw new Error(`the test did not script ${name}`);
  };
  return {
    isFactoryClone: unscripted("isFactoryClone"),
    cloneOwner: unscripted("cloneOwner"),
    service: unscripted("service"),
    effectiveService: unscripted("effectiveService"),
    provider: unscripted("provider"),
    currentService: unscripted("currentService"),
    canCreateService: unscripted("canCreateService"),
    predictIssuer: unscripted("predictIssuer"),
    issuerCreations: unscripted("issuerCreations"),
    cloneRecordType: unscripted("cloneRecordType"),
    canWriteServiceRepoint: unscripted("canWriteServiceRepoint"),
    domainClaimStanding: unscripted("domainClaimStanding"),
    canWriteDomain: unscripted("canWriteDomain"),
    directoryIsLiveFor: unscripted("directoryIsLiveFor"),
    canWriteProviderRecord: unscripted("canWriteProviderRecord"),
    providerProfileAnchor: unscripted("providerProfileAnchor"),
    providerPinCount: unscripted("providerPinCount"),
    providerNextLocationNumber: unscripted("providerNextLocationNumber"),
    providerHasPin: unscripted("providerHasPin"),
    providerPin: unscripted("providerPin"),
    ...overrides,
  } as ProviderChainReader;
}

/** The chain as it stood when the captain came back to the page: his contract not yet attached. */
function liveChain(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  return reader({
    issuerCreations: async () => LOGS,
    cloneRecordType: async (clone) => RECORD_TYPE_OF[clone.toLowerCase()]!,
    service: async (clone) =>
      clone.toLowerCase() === CAPTAINS_CLONE
        ? service(ZERO_PROVIDER_ID as HexWord)
        : service(PROVIDER_A),
    ...overrides,
  });
}

function read(history: DeploymentHistory) {
  if (history.state !== "read") throw new Error(`expected a read, got ${history.state}`);
  return history.deployments;
}

describe("what this wallet has deployed", () => {
  it("shows the captain his contract, its transaction and that it is not yet attached", async () => {
    // THE WHOLE INCIDENT, as one assertion. Before this the page held nothing after a reload, so
    // every one of these was unavailable to him.
    const found = read(await readDeploymentHistory(OWNER, liveChain())).find(
      (d) => d.clone === CAPTAINS_CLONE,
    )!;
    expect(found).toBeDefined();
    expect(found.txHash).toBe(CAPTAINS_TX);
    expect(found.blockNumber).toBe(352758n);
    expect(found.cloneNonce).toBe(1n);
    expect(found.recordType).toBe(DOG_PROFILE);
    expect(found.providerId).toBe(PROVIDER_B);
    expect(found.attachment).toBe("notAttached");
  });

  it("lists a contract deployed under a DIFFERENT provider id, because the wallet still owns it", async () => {
    // THE SHARPEST CORRECTNESS POINT. The factory salts on (recordType, msg.sender, cloneNonce) -
    // `providerId` is not in it - so a contract deployed while another provider id was in the form
    // still occupies its address and still spends its number. Filtering the log by provider id would
    // hide four of these five, and would then pre-fill a number that collides.
    const all = read(await readDeploymentHistory(OWNER, liveChain()));
    expect(all).toHaveLength(5);
    expect(new Set(all.map((d) => d.providerId))).toEqual(new Set([PROVIDER_A, PROVIDER_B]));
  });

  it("newest first, so the contract just deployed is at the top", async () => {
    const all = read(await readDeploymentHistory(OWNER, liveChain()));
    expect(all.map((d) => d.blockNumber)).toEqual([352758n, 352407n, 339692n, 339382n, 339369n]);
  });

  it("keeps a log with no block position, and puts it last rather than dropping it", async () => {
    // A dropped log is a contract the operator owns and cannot see, AND a number missing from the
    // fold. Position is the only thing this log is short of, and ordering is the only thing that
    // costs it.
    const unpositioned: IssuerCreationLog = {
      clone: "0x9999999999999999999999999999999999999999",
      cloneNonce: 7n,
      providerId: PROVIDER_B,
    };
    const all = read(
      await readDeploymentHistory(
        OWNER,
        liveChain({
          issuerCreations: async () => [unpositioned, ...LOGS],
          cloneRecordType: async (c) =>
            c.toLowerCase() === unpositioned.clone ? DOG_PROFILE : RECORD_TYPE_OF[c.toLowerCase()]!,
          service: async () => service(ZERO_PROVIDER_ID as HexWord),
        }),
      ),
    );
    expect(all).toHaveLength(6);
    expect(all[all.length - 1]!.clone).toBe(unpositioned.clone);
    expect(all[all.length - 1]!.txHash).toBeUndefined();
  });

  it("reports a failed log read as unread, never as nothing deployed", async () => {
    // The collapse this page exists not to make: rendered as an empty list it tells an operator his
    // contract does not exist.
    const history = await readDeploymentHistory(
      OWNER,
      reader({
        issuerCreations: async () => {
          throw new Error("eth_getLogs: query returned more than 10000 results");
        },
      }),
    );
    expect(history.state).toBe("couldNotRead");
    if (history.state !== "couldNotRead") throw new Error("unreachable");
    expect(history.reason).toMatch(/10000 results/);
  });

  it("keeps a contract listed when its own follow-up reads fail, and says which answer is missing", async () => {
    // The address and the transaction came from the LOG and are established. Withholding the whole
    // row over a failed annotation would be the original defect - a contract the operator owns, not
    // shown - reached from a smaller cause.
    const all = read(
      await readDeploymentHistory(
        OWNER,
        liveChain({
          cloneRecordType: async () => {
            throw new Error("execution reverted");
          },
          service: async () => {
            throw new Error("connection reset");
          },
        }),
      ),
    );
    expect(all).toHaveLength(5);
    const found = all.find((d) => d.clone === CAPTAINS_CLONE)!;
    expect(found.txHash).toBe(CAPTAINS_TX);
    expect(found.recordType).toBeUndefined();
    expect(found.recordTypeReason).toMatch(/execution reverted/);
    expect(found.attachment).toBe("unknown");
    expect(found.attachmentReason).toMatch(/connection reset/);
  });

  it("does NOT report an attached-but-pending contract as one you can select", async () => {
    // The card shipped a two-value attached/not-attached for one round and made the captain's own
    // defect in a new place: his contract IS attached, so it read "Attached - you can select it in
    // step 2", and he could not. `attachService` writes PENDING and `setServiceStanding(ACTIVE)` is
    // a second registrar call, so these are two states with two different people's next moves.
    const all = read(
      await readDeploymentHistory(
        OWNER,
        liveChain({ service: async () => service(PROVIDER_B, Standing.PENDING) }),
      ),
    );
    const found = all.find((d) => d.clone === CAPTAINS_CLONE)!;
    expect(found.attachment).toBe("pendingStanding");
    expect(found.attachedProviderId).toBe(PROVIDER_B);
  });

  it("tells not-attached, pending, active and some-other-standing apart", async () => {
    const at = async (standing: Standing) =>
      read(
        await readDeploymentHistory(
          OWNER,
          liveChain({ service: async () => service(PROVIDER_A, standing) }),
        ),
      )[0]!;
    expect((await at(Standing.ACTIVE)).attachment).toBe("active");
    expect((await at(Standing.PENDING)).attachment).toBe("pendingStanding");
    const suspended = await at(Standing.SUSPENDED);
    expect(suspended.attachment).toBe("otherStanding");
    // The word is carried rather than folded into a sentence, so a standing added later is reported
    // instead of silently rendering as one of its neighbours.
    expect(suspended.standingLabel).toBe("suspended");
    const none = read(
      await readDeploymentHistory(
        OWNER,
        liveChain({ service: async () => service(ZERO_PROVIDER_ID as HexWord) }),
      ),
    )[0]!;
    expect(none.attachment).toBe("notAttached");
  });
});

describe("the next contract number", () => {
  const history = async () => readDeploymentHistory(OWNER, liveChain());

  it("is scoped BY RECORD TYPE, because that is what the address salt is scoped by", async () => {
    // The case a highest-anywhere fold gets wrong, and this wallet really is in it: two contracts
    // numbered 1 and two numbered 0, all under one owner, no collision between them.
    const h = await history();
    expect(nextContractNumber(h, DOG_PROFILE)).toEqual({
      state: "known",
      highestUsed: 1n,
      next: 2n,
    });
    expect(nextContractNumber(h, GROOMING)).toEqual({ state: "known", highestUsed: 0n, next: 1n });
  });

  it("starts at 0 for a record type this wallet has never deployed", async () => {
    expect(nextContractNumber(await history(), EU_HEALTH)).toEqual({
      state: "known",
      highestUsed: null,
      next: 0n,
    });
  });

  it("refuses to guess when the log could not be read", () => {
    const answer = nextContractNumber(
      { state: "couldNotRead", reason: "the node refused the range" },
      DOG_PROFILE,
    );
    expect(answer.state).toBe("unknown");
    if (answer.state !== "unknown") throw new Error("unreachable");
    expect(answer.reason).toMatch(/the node refused the range/);
  });

  it("refuses to guess when ANY contract's record type is unresolved, even though the rest fold", async () => {
    // `max` over the rows we CAN read is collision-safe and still wrong: it answers "the latest
    // contract number" with a number that is not the latest, for the one record type in question.
    const h = await readDeploymentHistory(
      OWNER,
      liveChain({
        cloneRecordType: async (c) => {
          if (c.toLowerCase() === CAPTAINS_CLONE) throw new Error("recordType() reverted");
          return RECORD_TYPE_OF[c.toLowerCase()]!;
        },
      }),
    );
    const answer = nextContractNumber(h, GROOMING);
    expect(answer.state).toBe("unknown");
    if (answer.state !== "unknown") throw new Error("unreachable");
    expect(answer.reason).toContain(CAPTAINS_CLONE);
  });
});

describe("the contract number field", () => {
  it("accepts a whole number", () => {
    expect(parseContractNumber("0")).toEqual({ state: "ok", value: 0n });
    expect(parseContractNumber(" 12 ")).toEqual({ state: "ok", value: 12n });
  });

  it("refuses what BigInt would have thrown on, rather than letting it reach the wallet notice", () => {
    // `BigInt("2a")` throws a SyntaxError, and this page's catch renders wallet faults - so an
    // unguarded field put a surface fault where a verdict about a typed value belongs. It matters
    // more now that the field is pre-filled and the operator is invited to edit it.
    for (const bad of ["2a", "-1", "1.5", "", "  ", "1e3", "0x2"]) {
      expect(parseContractNumber(bad).state, bad).toBe("invalid");
    }
  });

  it("refuses a number the uint96 argument cannot carry", () => {
    expect(parseContractNumber(String(MAX_CLONE_NONCE)).state).toBe("ok");
    expect(parseContractNumber(String(MAX_CLONE_NONCE + 1n)).state).toBe("invalid");
  });
});
