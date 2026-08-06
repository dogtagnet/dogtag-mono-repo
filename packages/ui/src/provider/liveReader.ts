/**
 * The live viem implementation of {@link ProviderChainReader} (registry-plan S-15).
 *
 * Constructed with the four generation-2 addresses it speaks for, so no method takes a core,
 * factory or resolver address as a parameter - see the rule at the top of `readers.ts`. The
 * addresses come from configuration with NO fallback: unset means the surface reports itself
 * unavailable, rather than silently reading a constant that may have moved. That is the
 * `VITE_PROVIDER_DIRECTORY_ADDR` precedent, and it is the right one here for the sharper reason
 * that generation 2 is DEPLOYED BUT UNWIRED - a baked default would be a repoint by accident, which
 * is exactly what C-9/C-10 exist to do deliberately.
 *
 * Reads THROW on failure. viem rejects rather than returning a sentinel, and nothing here catches -
 * the engine turns a rejection into `could-not-run` with the reason, which is the only place that
 * translation is allowed to happen.
 */

import { getAddress, type PublicClient } from "viem";
import { roaxPublicClient } from "../wallet/contracts";
import {
  DomainDisposition,
  Standing,
  type Address,
  type DirectoryPin,
  type DomainClaimStanding,
  type EffectiveService,
  type HexWord,
  type ProfileAnchorRecord,
  type ProviderChainReader,
  type ProviderRecord,
  type ServiceRecord,
} from "./readers";

/** The generation-2 addresses this surface reads. Every one is required; none has a default. */
export interface ProviderContracts {
  /** `ProviderRegistry` - the S-6 authority core. */
  core: Address;
  /** `DogTagIssuerFactoryV2` - the self-service factory. THE forgery predicate lives here. */
  factory: Address;
  /** `ServiceDomainResolver` - the S-9 typed domain resolver. */
  domainResolver: Address;
  /** `ProviderDirectory` - the S-10 typed directory resolver. */
  directory: Address;
}

const CORE_ABI = [
  {
    type: "function",
    name: "service",
    stateMutability: "view",
    inputs: [{ name: "serviceAddress", type: "address" }],
    outputs: [
      {
        type: "tuple",
        components: [
          { name: "providerId", type: "bytes20" },
          { name: "factoryGeneration", type: "bytes32" },
          { name: "recordType", type: "bytes32" },
          { name: "confirmedOwner", type: "address" },
          { name: "domainResolver", type: "address" },
          { name: "ownerEpoch", type: "uint64" },
          { name: "standing", type: "uint8" },
        ],
      },
    ],
  },
  {
    type: "function",
    name: "provider",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [
      {
        type: "tuple",
        components: [
          { name: "controller", type: "address" },
          { name: "pendingController", type: "address" },
          { name: "pendingControllerAccepted", type: "bool" },
          { name: "pendingControllerRequestedByRegistrar", type: "bool" },
          { name: "directoryResolver", type: "address" },
          { name: "controllerEpoch", type: "uint64" },
          { name: "controllerRequestNonce", type: "uint64" },
          { name: "standing", type: "uint8" },
        ],
      },
    ],
  },
  {
    type: "function",
    name: "effectiveService",
    stateMutability: "view",
    inputs: [{ name: "serviceAddress", type: "address" }],
    outputs: [
      { name: "providerStanding", type: "uint8" },
      { name: "serviceStanding", type: "uint8" },
      { name: "factoryActive", type: "bool" },
      { name: "ownerConfirmed", type: "bool" },
      { name: "hasActiveIssuer", type: "bool" },
    ],
  },
  {
    type: "function",
    name: "currentService",
    stateMutability: "view",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "recordType", type: "bytes32" },
    ],
    outputs: [{ type: "address" }],
  },
  {
    type: "function",
    name: "canCreateService",
    stateMutability: "view",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "recordType", type: "bytes32" },
      { name: "caller", type: "address" },
    ],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "canWriteProvider",
    stateMutability: "view",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "caller", type: "address" },
      { name: "permission", type: "uint32" },
    ],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "canWriteService",
    stateMutability: "view",
    inputs: [
      { name: "serviceAddress", type: "address" },
      { name: "caller", type: "address" },
      { name: "permission", type: "uint32" },
    ],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "repointService",
    stateMutability: "nonpayable",
    inputs: [{ name: "serviceAddress", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "PROVIDER_PERMISSION_RECORD",
    stateMutability: "view",
    inputs: [],
    outputs: [{ type: "uint32" }],
  },
] as const;

const FACTORY_ABI = [
  {
    type: "function",
    name: "isClone",
    stateMutability: "view",
    inputs: [{ name: "", type: "address" }],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "createIssuer",
    stateMutability: "nonpayable",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "recordType", type: "bytes32" },
      { name: "cloneNonce", type: "uint96" },
    ],
    outputs: [{ name: "clone", type: "address" }],
  },
  {
    type: "function",
    name: "predictIssuer",
    stateMutability: "view",
    inputs: [
      { name: "recordType", type: "bytes32" },
      { name: "business", type: "address" },
      { name: "cloneNonce", type: "uint96" },
    ],
    outputs: [{ type: "address" }],
  },
] as const;

/**
 * The factory's own record of a creation, whole: `DogTagIssuerFactory.sol` emits this beside
 * `IssuerCreated` for exactly that reason.
 *
 * `cloneNonce` is UNINDEXED and rides in the data, which is what lets a log with no block position
 * still answer the contract-number question. Named rather than indexed out of `FACTORY_ABI`, so
 * reordering that array cannot silently point the log filter at a different event - a wrong event
 * here matches nothing, which renders as "you have deployed nothing".
 */
const ISSUER_OWNER_REGISTERED_EVENT = {
  type: "event",
  name: "IssuerOwnerRegistered",
  inputs: [
    { name: "clone", type: "address", indexed: true },
    { name: "owner", type: "address", indexed: true },
    { name: "providerId", type: "bytes20", indexed: true },
    { name: "cloneNonce", type: "uint96", indexed: false },
  ],
} as const;

/** The clone's own immutable record type. See `ProviderChainReader.cloneRecordType` for why. */
const ISSUER_ABI = [
  {
    type: "function",
    name: "recordType",
    stateMutability: "view",
    inputs: [],
    outputs: [{ type: "bytes32" }],
  },
] as const;

const OWNABLE_ABI = [
  { type: "function", name: "owner", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] },
] as const;

const DOMAIN_ABI = [
  {
    type: "function",
    name: "claimStanding",
    stateMutability: "view",
    inputs: [{ name: "service", type: "address" }],
    outputs: [
      { name: "disposition", type: "uint8" },
      { name: "domain", type: "string" },
      { name: "lineageRecognizesService", type: "bool" },
      { name: "registryApprovesThisResolver", type: "bool" },
      { name: "coreSelectsThisResolver", type: "bool" },
      { name: "serviceStandingEffective", type: "bool" },
    ],
  },
  {
    type: "function",
    name: "claimDomain",
    stateMutability: "nonpayable",
    inputs: [
      { name: "service", type: "address" },
      { name: "domain", type: "string" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "declareNoDomain",
    stateMutability: "nonpayable",
    inputs: [{ name: "service", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "clearDomain",
    stateMutability: "nonpayable",
    inputs: [{ name: "service", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "canWriteDomain",
    stateMutability: "view",
    inputs: [
      { name: "service", type: "address" },
      { name: "who", type: "address" },
    ],
    outputs: [{ type: "bool" }],
  },
] as const;

const DIRECTORY_ABI = [
  {
    type: "function",
    name: "setProfileAnchor",
    stateMutability: "nonpayable",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "digest", type: "bytes32" },
      { name: "schema", type: "uint32" },
      { name: "codec", type: "uint16" },
      { name: "hashAlgorithm", type: "uint8" },
      { name: "contenthash", type: "bytes" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "publishPin",
    stateMutability: "nonpayable",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "lat", type: "int32" },
      { name: "lng", type: "int32" },
      { name: "kind", type: "uint8" },
      { name: "active", type: "bool" },
    ],
    outputs: [{ name: "locationNo", type: "uint16" }],
  },
  // `updatePin` and `removePin` are what stop this surface being append-only. `publishPin` issues a
  // FRESH location number on every call, so a provider correcting a mistyped coordinate by pressing
  // Publish again would leave BOTH pins live in the scan and appear at two places in the mobile
  // nearby list. Confirmed callable on the deployed ProviderDirectory: an `eth_call` to either
  // selector reverts with the NAMED `UnknownProvider()` (`0xf2b51dfc`) against an unregistered
  // provider id, whereas a deliberately nonexistent selector on the same contract returns empty
  // data - so a named error is positive evidence of dispatch rather than an absence.
  {
    type: "function",
    name: "updatePin",
    stateMutability: "nonpayable",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "locationNo", type: "uint16" },
      { name: "lat", type: "int32" },
      { name: "lng", type: "int32" },
      { name: "kind", type: "uint8" },
      { name: "active", type: "bool" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "removePin",
    stateMutability: "nonpayable",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "locationNo", type: "uint16" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "isLiveFor",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "profileAnchor",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [
      {
        type: "tuple",
        components: [
          { name: "digest", type: "bytes32" },
          { name: "schema", type: "uint32" },
          { name: "codec", type: "uint16" },
          { name: "hashAlgorithm", type: "uint8" },
          { name: "revision", type: "uint64" },
          { name: "updatedAtBlock", type: "uint64" },
          { name: "setBy", type: "address" },
          { name: "contenthash", type: "bytes" },
        ],
      },
    ],
  },
  {
    type: "function",
    name: "pinCount",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [{ type: "uint16" }],
  },
  {
    type: "function",
    name: "nextLocationNumber",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [{ type: "uint16" }],
  },
  {
    type: "function",
    name: "hasPin",
    stateMutability: "view",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "locationNo", type: "uint16" },
    ],
    outputs: [{ type: "bool" }],
  },
  {
    type: "function",
    name: "pin",
    stateMutability: "view",
    inputs: [
      { name: "providerId", type: "bytes20" },
      { name: "locationNo", type: "uint16" },
    ],
    outputs: [
      {
        type: "tuple",
        components: [
          { name: "providerId", type: "bytes20" },
          { name: "lat", type: "int32" },
          { name: "lng", type: "int32" },
          { name: "locationNo", type: "uint16" },
          { name: "kind", type: "uint8" },
          { name: "flags", type: "uint8" },
        ],
      },
    ],
  },
] as const;

/** `ProviderDirectory.PIN_FLAG_ACTIVE` - bit 0 of the packed pin's flags byte. */
const PIN_FLAG_ACTIVE = 1;

export interface LiveReaderOptions {
  contracts: ProviderContracts;
  /** The holder-chosen endpoint, if any. Transport-only; it is never a trust upgrade. */
  rpcUrl?: string;
  /** Pin every read to one height so a verdict is a snapshot rather than several smeared answers. */
  blockNumber?: bigint;
  /** Injectable for tests; defaults to the guarded ROAX client. */
  client?: PublicClient;
}

/**
 * `ProviderRegistry.PROVIDER_PERMISSION_RECORD`, mirrored so a read is not needed to ask a question.
 *
 * Exported so `providerWriteAbi.test.ts` can pin it against the contract's own declaration. A
 * permission bit is as silent as a selector when it is wrong: the read answers a confident `false`
 * about a permission nobody asked about, which reads as "you may not" rather than as a mistake.
 */
export const PROVIDER_PERMISSION_RECORD = 1;
/**
 * `ProviderRegistry.SERVICE_PERMISSION_REPOINT` (`1 << 2`), the bit `repointService` demands.
 *
 * A DIFFERENT bit from `SERVICE_PERMISSION_RECORD` (`1 << 0`) and `SERVICE_PERMISSION_DOMAIN_RESOLVER`
 * (`1 << 1`), and the difference is the whole grant model: a delegate trusted to publish content is
 * not thereby trusted to move where new credentials anchor. Pinned by `providerWriteAbi.test.ts`
 * against the constant's own declaration.
 */
export const SERVICE_PERMISSION_REPOINT = 4;

export function createLiveProviderReader(options: LiveReaderOptions): ProviderChainReader {
  const { contracts, rpcUrl, blockNumber } = options;
  const client = options.client ?? roaxPublicClient(rpcUrl);
  const at = blockNumber === undefined ? {} : { blockNumber };

  return {
    async isFactoryClone(candidate) {
      return (await client.readContract({
        address: contracts.factory,
        abi: FACTORY_ABI,
        functionName: "isClone",
        args: [getAddress(candidate)],
        ...at,
      })) as boolean;
    },

    async cloneOwner(candidate) {
      // Deliberately unguarded against an address with no code: viem rejects empty returndata, and
      // that rejection is the honest answer. Decoding it as the zero address would report an EOA as
      // "owned by nobody" rather than "not answerable" - a definite claim from a read that returned
      // nothing.
      return (await client.readContract({
        address: getAddress(candidate),
        abi: OWNABLE_ABI,
        functionName: "owner",
        ...at,
      })) as Address;
    },

    async service(candidate) {
      const s = (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "service",
        args: [getAddress(candidate)],
        ...at,
      })) as {
        providerId: HexWord;
        factoryGeneration: HexWord;
        recordType: HexWord;
        confirmedOwner: Address;
        domainResolver: Address;
        ownerEpoch: bigint;
        standing: number;
      };
      return { ...s, standing: s.standing as Standing } satisfies ServiceRecord;
    },

    async effectiveService(candidate) {
      const r = (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "effectiveService",
        args: [getAddress(candidate)],
        ...at,
      })) as readonly [number, number, boolean, boolean, boolean];
      return {
        providerStanding: r[0] as Standing,
        serviceStanding: r[1] as Standing,
        factoryActive: r[2],
        ownerConfirmed: r[3],
        hasActiveIssuer: r[4],
      } satisfies EffectiveService;
    },

    async provider(providerId) {
      const p = (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "provider",
        args: [providerId],
        ...at,
      })) as { controller: Address; directoryResolver: Address; standing: number };
      return {
        controller: p.controller,
        directoryResolver: p.directoryResolver,
        standing: p.standing as Standing,
      } satisfies ProviderRecord;
    },

    async currentService(providerId, recordType) {
      return (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "currentService",
        args: [providerId, recordType],
        ...at,
      })) as Address;
    },

    async canCreateService(providerId, recordType, caller) {
      // THE TRAP, and the whole reason this method exists rather than a bare readContract at the
      // call site. `canCreateService`'s first term is `generationOfFactory[msg.sender]`, because the
      // core is designed to be asked this BY a factory during `createIssuer`. An `eth_call` with no
      // `from` carries `msg.sender == address(0)`, no generation matches, and the answer is `false`
      // for every provider on earth - so a preflight without this `account` would tell a perfectly
      // eligible provider that it may not deploy.
      //
      // Same shape as `ProviderRegistry.isWhitelistedFor`'s `msg.sender` branch, which
      // docs/CLIENT_REPOINT.md records as the reason ISSUER_REGISTRY_ADDR cannot move: every
      // production read is a plain eth_call with no `from`, so that branch always runs.
      //
      // It cannot be decomposed instead - the service-creation approval it folds lives in a PRIVATE
      // mapping with no getter, so this aggregate is the only way to observe that term at all.
      return (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "canCreateService",
        args: [providerId, recordType, getAddress(caller)],
        account: contracts.factory,
        ...at,
      })) as boolean;
    },

    async predictIssuer(recordType, business, cloneNonce) {
      return (await client.readContract({
        address: contracts.factory,
        abi: FACTORY_ABI,
        functionName: "predictIssuer",
        args: [recordType, getAddress(business), cloneNonce],
        ...at,
      })) as Address;
    },

    async issuerCreations(owner) {
      // Genesis to head, deliberately. A floor would have to come from somewhere, and every
      // candidate is wrong in the same direction: too high silently hides a contract the operator
      // owns, which is the whole defect. The same unbounded shape the issuer-whitelist pillar
      // already uses against this chain, on an operator-gated page rather than a public route.
      //
      // `toBlock` follows any pin so a pinned reader stays one snapshot rather than mixing a pinned
      // call with a head-of-chain log read.
      const logs = await client.getLogs({
        address: contracts.factory,
        event: ISSUER_OWNER_REGISTERED_EVENT,
        args: { owner: getAddress(owner) },
        fromBlock: 0n,
        ...(blockNumber === undefined ? { toBlock: "latest" as const } : { toBlock: blockNumber }),
      });
      return logs.map((log) => ({
        clone: log.args.clone as Address,
        cloneNonce: log.args.cloneNonce as bigint,
        providerId: log.args.providerId as HexWord,
        // A log a node considers pending carries neither. Both are OMITTED rather than defaulted:
        // `0` is a real block and a zeroed hash is a real-looking transaction id, and either would
        // be a claim about a position nobody reported.
        ...(log.transactionHash ? { txHash: log.transactionHash } : {}),
        ...(log.blockNumber === null || log.blockNumber === undefined
          ? {}
          : { blockNumber: log.blockNumber }),
      }));
    },

    async cloneRecordType(clone) {
      return (await client.readContract({
        address: getAddress(clone),
        abi: ISSUER_ABI,
        functionName: "recordType",
        ...at,
      })) as HexWord;
    },

    async domainClaimStanding(service) {
      const r = (await client.readContract({
        address: contracts.domainResolver,
        abi: DOMAIN_ABI,
        functionName: "claimStanding",
        args: [getAddress(service)],
        ...at,
      })) as readonly [number, string, boolean, boolean, boolean, boolean];
      return {
        disposition: r[0] as DomainDisposition,
        domain: r[1],
        lineageRecognizesService: r[2],
        registryApprovesThisResolver: r[3],
        coreSelectsThisResolver: r[4],
        serviceStandingEffective: r[5],
      } satisfies DomainClaimStanding;
    },

    async canWriteDomain(service, who) {
      return (await client.readContract({
        address: contracts.domainResolver,
        abi: DOMAIN_ABI,
        functionName: "canWriteDomain",
        args: [getAddress(service), getAddress(who)],
        ...at,
      })) as boolean;
    },

    async directoryIsLiveFor(providerId) {
      return (await client.readContract({
        address: contracts.directory,
        abi: DIRECTORY_ABI,
        functionName: "isLiveFor",
        args: [providerId],
        ...at,
      })) as boolean;
    },

    async canWriteProviderRecord(providerId, caller) {
      return (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "canWriteProvider",
        args: [providerId, getAddress(caller), PROVIDER_PERMISSION_RECORD],
        ...at,
      })) as boolean;
    },

    async canWriteServiceRepoint(service, caller) {
      // The chain's own predicate, asked rather than re-derived. Unlike `canCreateService` this one
      // does NOT branch on `msg.sender` - the caller is an explicit argument - so a plain `eth_call`
      // with no `from` is the right shape here.
      return (await client.readContract({
        address: contracts.core,
        abi: CORE_ABI,
        functionName: "canWriteService",
        args: [getAddress(service), getAddress(caller), SERVICE_PERMISSION_REPOINT],
        ...at,
      })) as boolean;
    },

    async providerProfileAnchor(providerId) {
      const a = (await client.readContract({
        address: contracts.directory,
        abi: DIRECTORY_ABI,
        functionName: "profileAnchor",
        args: [providerId],
        ...at,
      })) as {
        digest: HexWord;
        schema: number;
        codec: number;
        hashAlgorithm: number;
        revision: bigint;
      };
      return {
        digest: a.digest,
        schema: a.schema,
        codec: a.codec,
        hashAlgorithm: a.hashAlgorithm,
        revision: a.revision,
      } satisfies ProfileAnchorRecord;
    },

    async providerPinCount(providerId) {
      return Number(
        (await client.readContract({
          address: contracts.directory,
          abi: DIRECTORY_ABI,
          functionName: "pinCount",
          args: [providerId],
          ...at,
        })) as number,
      );
    },

    async providerNextLocationNumber(providerId) {
      return Number(
        (await client.readContract({
          address: contracts.directory,
          abi: DIRECTORY_ABI,
          functionName: "nextLocationNumber",
          args: [providerId],
          ...at,
        })) as number,
      );
    },

    async providerHasPin(providerId, locationNo) {
      return (await client.readContract({
        address: contracts.directory,
        abi: DIRECTORY_ABI,
        functionName: "hasPin",
        args: [providerId, locationNo],
        ...at,
      })) as boolean;
    },

    async providerPin(providerId, locationNo) {
      const p = (await client.readContract({
        address: contracts.directory,
        abi: DIRECTORY_ABI,
        functionName: "pin",
        args: [providerId, locationNo],
        ...at,
      })) as { lat: number; lng: number; locationNo: number; kind: number; flags: number };
      return {
        locationNo: Number(p.locationNo),
        lat: Number(p.lat),
        lng: Number(p.lng),
        kind: Number(p.kind),
        active: (Number(p.flags) & PIN_FLAG_ACTIVE) !== 0,
      } satisfies DirectoryPin;
    },
  };
}

export { CORE_ABI, DIRECTORY_ABI, DOMAIN_ABI, FACTORY_ABI, OWNABLE_ABI };
