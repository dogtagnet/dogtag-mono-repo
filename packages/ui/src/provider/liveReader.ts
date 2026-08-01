/**
 * The live viem implementation of {@link ProviderChainReader} (registry-plan S-15).
 *
 * Constructed with the four generation-2 addresses it speaks for, so no method takes a core,
 * factory or resolver address as a parameter - see the rule at the top of `readers.ts`. The
 * addresses come from configuration with NO fallback: unset means the surface reports itself
 * unavailable, rather than silently reading a constant that may have moved. That is the
 * `VITE_ISSUER_DOMAIN_REGISTRY_ADDR` precedent, and it is the right one here for the sharper reason
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
  type DomainClaimStanding,
  type EffectiveService,
  type HexWord,
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
  {
    type: "function",
    name: "isLiveFor",
    stateMutability: "view",
    inputs: [{ name: "providerId", type: "bytes20" }],
    outputs: [{ type: "bool" }],
  },
] as const;

export interface LiveReaderOptions {
  contracts: ProviderContracts;
  /** The holder-chosen endpoint, if any. Transport-only; it is never a trust upgrade. */
  rpcUrl?: string;
  /** Pin every read to one height so a verdict is a snapshot rather than several smeared answers. */
  blockNumber?: bigint;
  /** Injectable for tests; defaults to the guarded ROAX client. */
  client?: PublicClient;
}

/** `ProviderRegistry.PROVIDER_PERMISSION_RECORD`, mirrored so a read is not needed to ask a question. */
const PROVIDER_PERMISSION_RECORD = 1;

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
  };
}

export { CORE_ABI, DIRECTORY_ABI, DOMAIN_ABI, FACTORY_ABI, OWNABLE_ABI };
