/**
 * What this wallet has already deployed, read back from the chain (registry-plan S-15).
 *
 * THE DEFECT THIS EXISTS TO REMOVE. A captain pressed Deploy, signed, and his transaction mined -
 * contract `0x14a09008…`, block 352758, status 1, from the configured factory. The page recorded
 * nothing that survived, so it could not tell him the address he now owned, the transaction that
 * created it, or what to do next. Pressing Deploy again is the natural response to a page that shows
 * nothing, and it would have deployed a second contract.
 *
 * **THE CHAIN IS THE RECORD, and that is a correctness decision rather than a convenience.** The
 * obvious alternative - remember the deploy in `localStorage` - fails twice. A stored row is a claim
 * about the chain that the chain has not confirmed, so a transaction that reverted after the row was
 * written would leave the page asserting a contract that does not exist; this repo calls that mock
 * data and forbids it on every surface. And it is per-browser, so the operator who deploys on the
 * front desk machine and comes back on a laptop is stranded exactly as before. `IssuerOwnerRegistered`
 * exists precisely so one log filter on the factory captures a creation whole - the factory's own
 * source says so - so the honest record is already there to read.
 *
 * **FILTERED BY `owner` ALONE, NEVER BY `providerId`.** The factory salts on
 * `keccak256(abi.encode(recordType, msg.sender, cloneNonce))` - `providerId` is not in it. A contract
 * this wallet deployed while a different provider id was in the form still occupies its address and
 * therefore still spends that number, so a `providerId` filter under-reports the highest number used
 * and pre-fills one that collides. Measured on ROAX: one wallet holds five clones across two
 * provider ids, and filtering by the newer id hides four of them.
 *
 * **THE NUMBER'S NAMESPACE IS `(recordType, owner)`, so the record type scopes it.** That same
 * wallet holds two contracts numbered 1 and two numbered 0 - different record types, different
 * addresses, no collision. A highest-number-anywhere fold would answer 1 for a record type this
 * wallet has never deployed, which is not what the operator asked for and not what the salt does.
 */

import {
  Standing,
  STANDING_LABEL,
  ZERO_PROVIDER_ID,
  type Address,
  type HexWord,
  type ProviderChainReader,
} from "./readers";
import { reasonFrom } from "./types";

/**
 * One `IssuerOwnerRegistered` log, decoded and nothing more.
 *
 * `txHash` and `blockNumber` are optional because a node may return a log it considers pending with
 * neither. That costs this surface an explorer link and an ordering position - it does NOT cost the
 * number, which rides in the log's data and is the fact the collision question turns on. Dropping
 * such a log would understate the highest number used and pre-fill a colliding one, which is the
 * same "an unpositioned log may be neither placed nor dropped" rule the issuer-whitelist pillar
 * follows, reaching a different half of the record.
 */
export interface IssuerCreationLog {
  clone: Address;
  cloneNonce: bigint;
  /** The creation's STATED intent, as the factory recorded it. Not the core's attachment. */
  providerId: HexWord;
  txHash?: string;
  blockNumber?: bigint;
}

/**
 * One contract this wallet deployed, with everything the operator needs in order to act on it.
 *
 * `recordType` and `attachment` are resolved per row and degrade per row: a clone whose own reads
 * fail is still listed with its address and its transaction, because those came from the log and are
 * established. Withholding the whole row on a failed follow-up read would reproduce the defect -
 * a contract the operator owns, not shown.
 */
export interface DeployedContract extends IssuerCreationLog {
  /**
   * `DogTagIssuer.recordType()`, read off the clone itself.
   *
   * The clone's own immutable value, set in `initialize`, and deliberately NOT
   * `ProviderRegistry.service(clone).recordType` - that one is all-zero until the registrar attaches
   * the contract, which is precisely the state a freshly deployed contract is in.
   *
   * `undefined` means the read failed, never that the contract has no record type.
   */
  recordType?: HexWord;
  /** Why the record type is absent. Present if and ONLY if `recordType` is. */
  recordTypeReason?: string;
  /**
   * How far along the registrar's two steps this contract has got.
   *
   * NOT a two-value attached/not-attached, and this card shipped that way for one round and made the
   * exact over-claim the page was being fixed for: the captain's contract IS attached, so it read
   * "Attached - you can select it in step 2", and he cannot. `attachService` writes the service's
   * standing as `PENDING`, and `setServiceStanding(ACTIVE)` is a SECOND registrar call - so
   * attached-and-selectable and attached-and-waiting are different facts with different remedies and
   * one word for both is a false claim about whichever one it is not describing.
   *
   * `current` is deliberately absent: which contract is CURRENT is step 2's question, and step 2
   * assesses it in full. So is `notAClone`, which is unreachable - the address came out of the
   * factory's own creation log.
   */
  attachment: DeployedAttachment;
  /** Why the attachment is `unknown`. Present if and ONLY if it is. */
  attachmentReason?: string;
  /** The standing word, for the one state that has to name a value it cannot enumerate. */
  standingLabel?: string;
  /** The provider this contract is attached TO, when it is attached to one at all. */
  attachedProviderId?: HexWord;
}

/**
 * How far along a deployed contract is, as one explicit state.
 *
 * Every member is somebody's next move, which is what makes this worth modelling rather than
 * folding: `notAttached` and `pendingStanding` are both DogTag's, `active` is the provider's, and
 * `unknown` is nobody's until a read succeeds.
 */
export type DeployedAttachment =
  /** A read failed. Nothing about where this stands has been established. */
  | "unknown"
  /** Attached to no provider at all. DogTag attaches it next. */
  | "notAttached"
  /** Attached, and its standing is still PENDING - the state `attachService` leaves it in. */
  | "pendingStanding"
  /** Attached and active. This is the one that can be selected in step 2. */
  | "active"
  /** Attached, and in some other standing - suspended or retired. Named rather than guessed at. */
  | "otherStanding";

/**
 * What this wallet has deployed, or an honest statement that we could not find out.
 *
 * Two states and no third. An empty `read` is a FACT - this wallet has deployed nothing - and may be
 * said out loud. `couldNotRead` may not: rendering a failed log read as "you have deployed nothing"
 * is the could-not-check-as-an-answer collapse, and on this page it would tell an operator his
 * contract does not exist.
 */
export type DeploymentHistory =
  | { state: "read"; deployments: readonly DeployedContract[] }
  | { state: "couldNotRead"; reason: string };

/** The largest number `createIssuer`'s `uint96 cloneNonce` can carry. */
export const MAX_CLONE_NONCE = 2n ** 96n - 1n;

/**
 * Read every contract this wallet has deployed, and resolve what the operator needs to act on each.
 *
 * One log filter plus two reads per contract. The log filter is the whole record; the per-clone
 * reads only annotate it, so their failures narrow a row rather than emptying the list.
 */
export async function readDeploymentHistory(
  owner: Address,
  reader: ProviderChainReader,
): Promise<DeploymentHistory> {
  let logs: readonly IssuerCreationLog[];
  try {
    logs = await reader.issuerCreations(owner);
  } catch (error) {
    return {
      state: "couldNotRead",
      reason: reasonFrom(error, "the factory's creation log could not be read"),
    };
  }

  const deployments = await Promise.all(
    logs.map(async (log): Promise<DeployedContract> => ({
      ...log,
      ...(await resolveRecordType(log.clone, reader)),
      ...(await resolveAttachment(log.clone, reader)),
    })),
  );
  return { state: "read", deployments: [...deployments].sort(newestFirst) };
}

async function resolveRecordType(
  clone: Address,
  reader: ProviderChainReader,
): Promise<Pick<DeployedContract, "recordType" | "recordTypeReason">> {
  try {
    return { recordType: await reader.cloneRecordType(clone) };
  } catch (error) {
    return { recordTypeReason: reasonFrom(error, "the recordType() read failed") };
  }
}

async function resolveAttachment(
  clone: Address,
  reader: ProviderChainReader,
): Promise<
  Pick<
    DeployedContract,
    "attachment" | "attachmentReason" | "attachedProviderId" | "standingLabel"
  >
> {
  try {
    const service = await reader.service(clone);
    // An all-zero provider id is the core's own "attached to nobody", not a failed read - `service`
    // returns a zeroed struct for an address it has never been told about.
    if (service.providerId.toLowerCase() === ZERO_PROVIDER_ID) return { attachment: "notAttached" };
    const attachedProviderId = service.providerId;
    if (service.standing === Standing.ACTIVE) return { attachment: "active", attachedProviderId };
    if (service.standing === Standing.PENDING) {
      return { attachment: "pendingStanding", attachedProviderId };
    }
    // Suspended, retired, or a value this build does not know: the word is carried rather than
    // folded into a sentence, so a standing added later is reported instead of silently rendering as
    // one of its neighbours.
    return {
      attachment: "otherStanding",
      attachedProviderId,
      standingLabel: STANDING_LABEL[service.standing] ?? `standing ${service.standing}`,
    };
  } catch (error) {
    return {
      attachment: "unknown",
      attachmentReason: reasonFrom(error, "the service() read failed"),
    };
  }
}

/**
 * Newest first, so the contract just deployed is at the top.
 *
 * A log with no position sorts to the END rather than being dropped - it is still the operator's
 * contract and still spends its number. It says so on its own row; ordering is the only thing its
 * missing position actually costs.
 */
function newestFirst(a: DeployedContract, b: DeployedContract): number {
  if (a.blockNumber === undefined || b.blockNumber === undefined) {
    if (a.blockNumber !== b.blockNumber) return a.blockNumber === undefined ? 1 : -1;
  } else if (a.blockNumber !== b.blockNumber) {
    return a.blockNumber > b.blockNumber ? -1 : 1;
  }
  if (a.cloneNonce !== b.cloneNonce) return a.cloneNonce > b.cloneNonce ? -1 : 1;
  return a.clone < b.clone ? -1 : a.clone > b.clone ? 1 : 0;
}

/**
 * The next free contract number for a record type, and the highest this wallet has already used.
 *
 * `highestUsed` is `null` when this wallet has deployed nothing of this record type - which is a
 * fact, and the reason the first contract is number 0.
 */
export type NextContractNumber =
  | { state: "known"; highestUsed: bigint | null; next: bigint }
  /** The number could not be worked out. NEVER pre-fill from this - say so instead. */
  | { state: "unknown"; reason: string };

/**
 * Derive the next free number from what actually exists.
 *
 * TWO WAYS THIS MUST REFUSE TO GUESS, and both produce a colliding or misleading number if they are
 * softened into a fallback:
 *
 *  * the log read failed, so nothing about what exists is established; and
 *  * some row's record type could not be read, so it is not known whether that contract belongs to
 *    this number's namespace. `max` over the rest is collision-SAFE and still wrong - it would
 *    answer "the latest contract number" with a number that is not the latest.
 *
 * A number derived here is still only a suggestion: `planCloneDeployment` re-asks the factory
 * whether the chosen number's address is already taken, so a stale or incomplete log is caught
 * before anything is sent.
 */
export function nextContractNumber(
  history: DeploymentHistory,
  recordType: HexWord,
): NextContractNumber {
  if (history.state === "couldNotRead") {
    return {
      state: "unknown",
      reason: `what you have already deployed could not be read (${history.reason})`,
    };
  }
  const unresolved = history.deployments.find((d) => d.recordType === undefined);
  if (unresolved) {
    return {
      state: "unknown",
      reason:
        `the record type of your contract at ${unresolved.clone} could not be read `
        + `(${unresolved.recordTypeReason ?? "no reason given"}), so it is not known whether it uses `
        + `one of these numbers`,
    };
  }
  const used = history.deployments
    .filter((d) => d.recordType === recordType)
    .map((d) => d.cloneNonce);
  const highestUsed = used.length ? used.reduce((a, b) => (a > b ? a : b)) : null;
  return { state: "known", highestUsed, next: highestUsed === null ? 0n : highestUsed + 1n };
}

/** A contract number the operator typed, or why it cannot be used. */
export type ContractNumberInput =
  | { state: "ok"; value: bigint }
  | { state: "invalid"; reason: string };

/**
 * Parse the contract-number field.
 *
 * Guarded because the field is now PRE-FILLED and the operator is invited to override it, so a
 * non-numeric value stopped being hypothetical. Unguarded, `BigInt("2a")` throws a `SyntaxError`
 * into the flow's surface-fault handler and renders as a wallet fault - a fault notice where a
 * verdict about a typed field belongs, which is the misattribution this whole surface is built not
 * to make.
 */
export function parseContractNumber(input: string): ContractNumberInput {
  const trimmed = input.trim();
  if (!trimmed) return { state: "invalid", reason: "Enter a contract number. Your first one is 0." };
  if (!/^\d+$/.test(trimmed)) {
    return {
      state: "invalid",
      reason: "A contract number is a whole number: digits only, no sign and no decimal point.",
    };
  }
  const value = BigInt(trimmed);
  if (value > MAX_CLONE_NONCE) {
    return { state: "invalid", reason: `The largest contract number is ${MAX_CLONE_NONCE}.` };
  }
  return { state: "ok", value };
}

/**
 * What happens to a contract once it is deployed, stated as a value the surface must render.
 *
 * Deliberately a DEPENDENCY rather than a status: `attachService` is `onlyOwner` on the core, so
 * this is true whether or not the step has happened yet, and it does not go stale the day it does.
 * The per-row attachment state is what says where a particular contract has got to.
 */
export const DEPLOYED_CONTRACT_NEXT_STEP =
  "Send this address to DogTag. Until DogTag attaches it to your provider record you cannot select it in step 2, and nothing anchors to it.";
