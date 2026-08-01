/**
 * THE forgery guard, at the portal layer (registry-plan S-15, §3.1).
 *
 * The captain's ruling: a provider may repoint its recorded contract address ONLY to an address
 * that is a genuine clone spun off from our factory, so that "there's no way to do any false
 * contract inputs".
 *
 * The chain already enforces this for the WRITE: `ProviderRegistry.attachService` proves
 * `factory.isClone(service)` against the factory pinned to the named generation, and
 * `repointService` will not touch an address that was never attached. Those are the boundary, and
 * nothing here weakens or replaces them.
 *
 * What this module is for is the PORTAL, which is the other place a candidate address is entered by
 * a human and acted on. It answers three different questions that a single "is it genuine?" would
 * collapse:
 *
 *   1. PROVENANCE  - did OUR configured factory deploy it? Stops a hand-rolled contract.
 *   2. CONTROL     - does the caller's key own it? Provenance is not attribution: without this,
 *                    provider A can point its listing at provider B's perfectly genuine clone.
 *   3. ATTACHMENT  - is it attached to THIS provider in the core? This is the one fact chain
 *                    provenance cannot establish, and it is the registrar's to assert.
 *
 * Two ordering rules are load-bearing and each is easy to undo:
 *
 * A definite `false` from provenance is a REFUSAL, and a could-not-run on any later check does not
 * soften it. A forged address whose `owner()` read happens to fail is still forged.
 *
 * A failed READ is never a refusal. `isFactoryClone` throwing means we could not ask the factory,
 * which is a statement about our own connectivity and not about the provider's contract. Rendering
 * that as "not genuine" is an accusation from a read that never happened - the same defect that
 * declared genuine credentials forged, pointed at the provider's own property.
 */

import {
  DomainDisposition,
  Standing,
  STANDING_LABEL,
  ZERO_ADDR,
  ZERO_PROVIDER_ID,
  type Address,
  type HexWord,
  type ProviderChainReader,
} from "./readers";
import {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type CloneLifecycle,
  type ProviderCheck,
  type ProviderVerdict,
} from "./types";

export interface CloneAssessmentRequest {
  /** The address the provider typed or picked. The ONLY thing here that is caller-supplied. */
  candidate: Address;
  /** The key that would send the transaction. */
  caller: Address;
  /** The provider whose listing would move. Opaque, KYC-assigned. */
  providerId: HexWord;
}

export interface CloneAssessment {
  candidate: Address;
  checks: ProviderCheck[];
  lifecycle: CloneLifecycle;
  verdict: ProviderVerdict;
  /**
   * True only when a `repointService` transaction would be worth sending: genuine, controlled by
   * this key, attached to this provider, cleared to write, and not already the current pointer.
   * A `false` never says WHY - the checks do.
   */
  canRepoint: boolean;
  /** What the provider should do next, in their own terms. Always present. */
  nextStep: string;
  /** The record type the clone reports, once one has been read. Never taken from a form. */
  recordType?: HexWord;
}

const NOT_GENUINE =
  "This address was not deployed by the DogTag issuer factory, so it cannot be entered here.";

/**
 * Judge one candidate address.
 *
 * `reader` carries the configured factory and core. No factory, core or resolver address is a
 * parameter of this function, deliberately - see the module rule in `readers.ts`.
 */
export async function assessCandidateClone(
  request: CloneAssessmentRequest,
  reader: ProviderChainReader,
): Promise<CloneAssessment> {
  const { candidate, caller, providerId } = request;
  const checks: ProviderCheck[] = [];

  // ---- 1. Provenance. The only question whose "no" is a refusal on its own. -------------------
  let genuine: boolean | undefined;
  try {
    genuine = await reader.isFactoryClone(candidate);
    checks.push(
      providerCheck(
        "clone-provenance",
        "Did the DogTag issuer factory deploy this contract?",
        genuine ? "pass" : "fail",
        genuine
          ? "The factory's own record confirms it deployed this address."
          : `The factory has no record of deploying ${candidate}.`,
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "clone-provenance",
        "Did the DogTag issuer factory deploy this contract?",
        "could-not-run",
        "The factory could not be reached, so this address was neither confirmed nor refused.",
        reasonFrom(error, "the isClone read failed"),
      ),
    );
  }

  // A definite refusal is final. Reading on would spend calls to decorate an answer that cannot
  // change, and - worse - a could-not-run below could be mistaken for the reason.
  if (genuine === false) {
    return {
      candidate,
      checks,
      lifecycle: "notAClone",
      verdict: "refused",
      canRepoint: false,
      nextStep: NOT_GENUINE,
    };
  }

  // ---- 2. Control. Ownership is live, and is re-resolved rather than remembered. --------------
  let owner: Address | undefined;
  try {
    owner = await reader.cloneOwner(candidate);
    const owned = owner.toLowerCase() === caller.toLowerCase();
    checks.push(
      providerCheck(
        "clone-control",
        "Does your key own this contract?",
        owned ? "pass" : "fail",
        owned
          ? "The contract reports your key as its owner."
          : `The contract is owned by ${owner}, not by ${caller}.`,
      ),
    );
  } catch (error) {
    // An address with no code lands here: a staticcall to an EOA succeeds with empty returndata,
    // which a decoder rejects rather than reading as the zero address.
    checks.push(
      providerCheck(
        "clone-control",
        "Does your key own this contract?",
        "could-not-run",
        "Ownership could not be read from this address.",
        reasonFrom(error, "the owner() read failed"),
      ),
    );
  }

  // ---- 3. Attachment. The registrar's assertion, and the step that is not self-service. -------
  let lifecycle: CloneLifecycle = "unknown";
  let recordType: HexWord | undefined;
  let attachedHere = false;
  try {
    const service = await reader.service(candidate);
    const attachedToNobody = service.providerId.toLowerCase() === ZERO_PROVIDER_ID;
    attachedHere = !attachedToNobody && service.providerId.toLowerCase() === providerId.toLowerCase();
    recordType = attachedToNobody ? undefined : service.recordType;

    if (attachedToNobody) {
      lifecycle = "deployed";
      checks.push(
        providerCheck(
          "clone-attachment",
          "Has DogTag attached this contract to your provider record?",
          "fail",
          "Not yet attached. A contract you have deployed is not listed until DogTag attaches it.",
        ),
      );
    } else if (!attachedHere) {
      lifecycle = "foreign";
      checks.push(
        providerCheck(
          "clone-attachment",
          "Has DogTag attached this contract to your provider record?",
          "fail",
          "This contract is genuine, but it is attached to a different provider.",
        ),
      );
    } else {
      checks.push(
        providerCheck(
          "clone-attachment",
          "Has DogTag attached this contract to your provider record?",
          "pass",
          "Attached to your provider record.",
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "clone-attachment",
        "Has DogTag attached this contract to your provider record?",
        "could-not-run",
        "The registry record for this address could not be read.",
        reasonFrom(error, "the service() read failed"),
      ),
    );
  }

  // ---- 4. Standing, and whether it is already the current pointer. ----------------------------
  if (attachedHere) {
    try {
      const effective = await reader.effectiveService(candidate);
      const frozen =
        effective.serviceStanding !== Standing.ACTIVE ||
        effective.providerStanding !== Standing.ACTIVE ||
        !effective.factoryActive;
      // `ownerConfirmed` is reported alongside rather than folded into the standing sentence: a
      // freeze is cleared by nothing, an unconfirmed handover is cleared by the registrar
      // confirming it, and one message for both would send the provider to the wrong remedy.
      if (frozen) {
        checks.push(
          providerCheck(
            "clone-standing",
            "Does this contract's standing still allow changes?",
            "fail",
            `Frozen: provider ${STANDING_LABEL[effective.providerStanding]}, contract ` +
              `${STANDING_LABEL[effective.serviceStanding]}` +
              `${effective.factoryActive ? "" : ", factory generation deprecated"}.`,
          ),
        );
      } else if (!effective.ownerConfirmed) {
        checks.push(
          providerCheck(
            "clone-standing",
            "Does this contract's standing still allow changes?",
            "fail",
            "A change of owner is waiting for DogTag to confirm it. Changes are paused until then.",
          ),
        );
      } else {
        checks.push(
          providerCheck(
            "clone-standing",
            "Does this contract's standing still allow changes?",
            "pass",
            "Active, and the owner on file matches the contract.",
          ),
        );
      }
    } catch (error) {
      checks.push(
        providerCheck(
          "clone-standing",
          "Does this contract's standing still allow changes?",
          "could-not-run",
          "Standing could not be read.",
          reasonFrom(error, "the effectiveService() read failed"),
        ),
      );
    }

    if (recordType) {
      try {
        const current = await reader.currentService(providerId, recordType);
        lifecycle = current.toLowerCase() === candidate.toLowerCase() ? "current" : "attached";
      } catch {
        // The pointer is a display fact, not a gate. Failing to read it leaves the lifecycle at
        // `attached`, which is what has already been established, rather than dropping the whole
        // assessment to `unknown` over a question no check depends on.
        lifecycle = "attached";
      }
    } else {
      lifecycle = "attached";
    }
  }

  const verdict = foldVerdict(checks);
  const canRepoint = verdict === "ready" && lifecycle === "attached";

  return {
    candidate,
    checks,
    lifecycle,
    verdict,
    canRepoint,
    nextStep: nextStepFor(lifecycle, verdict),
    ...(recordType ? { recordType } : {}),
  };
}

/**
 * What to tell the provider to do next.
 *
 * `unknown` deliberately does not offer a remedy that would fix a refusal - the honest instruction
 * when a read failed is to try again, not to go and change something that may be perfectly fine.
 */
export function nextStepFor(lifecycle: CloneLifecycle, verdict: ProviderVerdict): string {
  if (lifecycle === "notAClone") return NOT_GENUINE;
  if (verdict === "indeterminate") {
    return "Some checks could not run, so nothing about this address has been established. Try again once the connection is back.";
  }
  switch (lifecycle) {
    case "deployed":
      return "Send this address to DogTag to attach it to your provider record. You can select it here once that is done.";
    case "foreign":
      return "This contract belongs to another provider. Deploy your own, or ask DogTag to correct the attachment.";
    case "attached":
      return verdict === "refused"
        ? "This contract cannot be selected right now. The failed checks above say why."
        : "Ready to select. New credentials of this record type will be anchored here.";
    case "current":
      return "Already selected. New credentials of this record type are anchored here.";
    case "unknown":
      return "Nothing about this address has been established yet.";
  }
}

/**
 * A repoint moves where NEW credentials anchor and nothing else.
 *
 * Stated as a value the surface must render rather than as prose in a comment, because a provider
 * who reads "repoint" as "move my credentials" will believe their history followed them.
 * `rootIssuer` is write-once, so everything the old contract issued keeps resolving to the old
 * contract and stays revocable there - which is correct rather than a limitation: re-attributing
 * issued credentials to a contract that did not issue them is exactly the misattribution the
 * control check exists to prevent.
 */
export const REPOINT_SCOPE_NOTICE =
  "Selecting a different contract changes where NEW credentials are anchored. Credentials you have already issued stay with the contract that issued them, and stay revocable there.";

/** Re-exported so a renderer can name a disposition without importing the reader module. */
export { DomainDisposition, ZERO_ADDR };
