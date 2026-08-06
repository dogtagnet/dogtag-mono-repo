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
 *   2. AUTHORITY   - would the CHAIN accept a repoint from this key? Provenance is not attribution:
 *                    without this, provider A can point its listing at provider B's genuine clone.
 *   3. ATTACHMENT  - is it attached to THIS provider in the core? This is the one fact chain
 *                    provenance cannot establish, and it is the registrar's to assert.
 *
 * THE AUTHORITY QUESTION IS COMPOSED, NEVER RE-DERIVED. `repointService` is gated by
 * `ProviderRegistry.canWriteService(service, caller, SERVICE_PERMISSION_REPOINT)`, which admits the
 * confirmed live owner OR an owner-epoch-scoped delegate holding that bit. This module used to ask
 * `owner() == caller` instead and refuse anything else - a preflight STRICTER than the chain, which
 * told a legitimate delegate its key may not select a contract the chain would have let it select.
 * That is the exact mirror of the `canCreateService` trap recorded in `readers.ts`, and it is the
 * failure mode `ServiceDomainResolver` avoids by composing `isAuthoritativeFor` rather than
 * re-listing its terms.
 *
 * `owner()` and the `effectiveService` terms are still READ and still shown, because "you are a
 * delegate, not the owner" and "your provider is suspended" send a provider to different remedies -
 * but `clone-control` is now a REPORT of who owns the contract and can no longer refuse on its own.
 * The standing rows keep their refusals because they do NOT disagree with the chain:
 * `canWriteService` itself requires `_serviceStandingIsEffective` (provider ACTIVE, service ACTIVE,
 * generation active) and `liveOwner == confirmedOwner`, which is exactly what those rows fold.
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

  // ---- 2. Control, REPORTED. Ownership is live, and is re-resolved rather than remembered. ----
  // Deliberately incapable of refusing: the question is WHO owns the contract, and the answer is
  // established as soon as the read succeeds. Whether this key may act is `clone-write-authority`
  // below, which asks the chain. Making this row fail for a non-owner is precisely the regression
  // this module's header describes, so it is not a stylistic choice.
  let owner: Address | undefined;
  try {
    owner = await reader.cloneOwner(candidate);
    const owned = owner.toLowerCase() === caller.toLowerCase();
    checks.push(
      providerCheck(
        "clone-control",
        "Who owns this contract?",
        "pass",
        owned
          ? "The contract reports your key as its owner."
          : `The contract is owned by ${owner}, not by ${caller}. Your key can still act if that `
            + "owner has authorised it as a delegate - the next check asks the registry whether it has.",
      ),
    );
  } catch (error) {
    // An address with no code lands here: a staticcall to an EOA succeeds with empty returndata,
    // which a decoder rejects rather than reading as the zero address.
    checks.push(
      providerCheck(
        "clone-control",
        "Who owns this contract?",
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

  // Declared at function scope because the SUMMARY line needs it too: this is the state where the
  // one-line answer has to say "wait for DogTag" rather than "read the rows to find out".
  let awaitingRegistrar = false;

  // ---- 4. Authority, standing, and whether it is already the current pointer. -----------------
  if (attachedHere) {
    // STANDING IS READ BEFORE AUTHORITY, and the order is load-bearing rather than tidy.
    // `canWriteService` returns false on `!_serviceStandingIsEffective(...)` BEFORE it ever looks at
    // the caller (`ProviderRegistry.sol:703`), so for a service that is not in effective standing it
    // answers `false` for the owner, for every delegate, and for everybody else alike. The authority
    // row used to read that shared `false` as a fact about the key and say the owner on file could
    // do this instead - to a captain who WAS the owner on file, about a contract DogTag had just
    // attached. Knowing the standing first is what lets that row decline to make a claim it is in no
    // position to make.
    let standingEffective: boolean | undefined;
    try {
      const effective = await reader.effectiveService(candidate);
      // NOT YET ACTIVE IS NOT FROZEN, and calling it frozen stranded a captain who had done
      // everything right. `attachService` writes the service's standing as `PENDING`, so this is the
      // ORDINARY state of a contract DogTag has just attached - the single likeliest state a
      // provider is in the first time they check. "Frozen" describes the opposite situation: a
      // `RETIRED` standing is terminal and a deprecated generation cannot be undeprecated, so both
      // really are permanent, and there is nothing to wait for. Reporting a step that is pending as
      // one that will never come tells a provider their contract is dead when it is queued, and it
      // sends them looking for a fault instead of at DogTag.
      awaitingRegistrar =
        effective.providerStanding === Standing.PENDING
        || effective.serviceStanding === Standing.PENDING;
      const frozen =
        !awaitingRegistrar
        && (effective.serviceStanding !== Standing.ACTIVE
          || effective.providerStanding !== Standing.ACTIVE
          || !effective.factoryActive);
      // `ownerConfirmed` is reported alongside rather than folded into the standing sentence: a
      // freeze is cleared by nothing, an unconfirmed handover is cleared by the registrar
      // confirming it, and one message for both would send the provider to the wrong remedy.
      standingEffective = !awaitingRegistrar && !frozen;
      if (awaitingRegistrar) {
        checks.push(
          providerCheck(
            "clone-standing",
            "Does this contract's standing still allow changes?",
            // STILL a `fail`, because the chain would refuse a selection right now and this row is
            // what gates the send. What changes is the sentence, not the verdict: reporting it as a
            // pass would offer a transaction that cannot succeed, and reporting it as could-not-run
            // would claim we failed to read something we read perfectly well.
            "fail",
            `Waiting on DogTag: your ${
              effective.providerStanding === Standing.PENDING ? "provider record" : "contract"
            } is registered and its standing is still pending review. Attaching a contract leaves it `
              + `pending by design - DogTag sets its standing to active as the next step, on the `
              + `Providers page under Attached services. Nothing is wrong with what you deployed, and `
              + `there is nothing for you to change here.`,
          ),
        );
      } else if (frozen) {
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

    // THE authorization row, and the only one that may refuse on the authority axis. Asked of the
    // core rather than derived from `owner()`, so a delegate the chain admits is admitted here.
    // Scoped to an attached clone because `repointService` is meaningless for an unattached one and
    // the attachment row already carries that refusal.
    try {
      const mayRepoint = await reader.canWriteServiceRepoint(candidate, caller);
      if (mayRepoint) {
        checks.push(
          providerCheck(
            "clone-write-authority",
            "May your key select this contract?",
            "pass",
            "The registry accepts a selection from your key for this contract.",
          ),
        );
      } else if (standingEffective === true) {
        // Standing is established as fine, so the refusal really is about the key and this row is
        // entitled to say so.
        checks.push(
          providerCheck(
            "clone-write-authority",
            "May your key select this contract?",
            "fail",
            "The registry does not accept a selection from your key for this contract. The owner "
              + "on file, or a delegate they authorised for this, may.",
          ),
        );
      } else {
        // The chain gave one `false` for two possible reasons and this row cannot separate them, so
        // it declines to blame the key. `could-not-run` rather than `fail`: nothing about the key
        // has been ESTABLISHED, and a page that accuses an operator's key of being wrong when it is
        // the right key sends them to change the one thing that was never the problem.
        checks.push(
          providerCheck(
            "clone-write-authority",
            "May your key select this contract?",
            "could-not-run",
            "Whether your key may select this contract is not established yet.",
            standingEffective === false
              ? "the registry refuses a selection for ANY key while this contract's standing is not "
                + "yet active, so this answer says nothing about your key - the standing row above is "
                + "the one to act on"
              : "the contract's standing could not be read, and the registry folds standing into this "
                + "same answer, so a refusal here cannot be attributed to your key",
          ),
        );
      }
    } catch (error) {
      checks.push(
        providerCheck(
          "clone-write-authority",
          "May your key select this contract?",
          "could-not-run",
          "Whether the registry would accept a selection from your key could not be read.",
          reasonFrom(error, "the canWriteService read failed"),
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
    nextStep: nextStepFor(lifecycle, verdict, awaitingRegistrar),
    ...(recordType ? { recordType } : {}),
  };
}

/**
 * What to tell the provider to do next.
 *
 * `unknown` deliberately does not offer a remedy that would fix a refusal - the honest instruction
 * when a read failed is to try again, not to go and change something that may be perfectly fine.
 */
export function nextStepFor(
  lifecycle: CloneLifecycle,
  verdict: ProviderVerdict,
  /**
   * The refusal is a standing DogTag has still to set, rather than anything the provider can change.
   *
   * Passed in rather than inferred, and it changes the SUMMARY line specifically because that is the
   * line a first-time provider reads and stops at. "The failed checks above say why" is true and
   * sends them to read five rows to discover the answer is "wait" - which, for the single most
   * common state a provider is in right after DogTag attaches their contract, is a wall.
   */
  awaitingRegistrar = false,
): string {
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
      if (awaitingRegistrar) {
        return "Nothing for you to do here yet: ask DogTag to set this contract's standing to active. You can select it here once they have.";
      }
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

/**
 * That this flow depends on steps the provider cannot take, said BEFORE they try it.
 *
 * A DEPENDENCY, not a status. `attachService` and `setServiceStanding` are both `onlyOwner` on the
 * core, so "DogTag does this, not you" is a permanent property of the design and stays true after
 * any given contract gets through; a sentence claiming the flow is currently blocked would go stale
 * silently, and deriving one from a live read would mean rendering a could-not-run state as an
 * accusation. The checks report what is true now; this reports what this flow needs.
 *
 * **IT SAYS TWO STEPS BECAUSE THERE ARE TWO, and naming only the first stranded a captain.**
 * `attachService` writes the service's standing as `PENDING` - so the ordinary state immediately
 * after an attach is a contract that is attached and still cannot be selected, which is precisely
 * the state a provider is in the first time they come back to check. A provider told about one step
 * has no way to read that as anything but a failure of the step they were told about.
 *
 * **AND IT NO LONGER CLAIMS THERE IS NOWHERE TO DO IT.** It used to end "there is no page for it
 * yet", which was true when it was written and is now false: the admin portal's Providers page
 * carries both controls, under Attached services. A captain read that sentence, went looking anyway,
 * found the control, used it - and the sentence had already told him the thing he had just done was
 * impossible. A claim about what exists elsewhere in the product is exactly the kind that rots
 * without anything failing, so it is stated as what DogTag does rather than as what does not exist.
 */
export const ATTACHMENT_IS_A_DOGTAG_STEP =
  "Before you can select a contract here, DogTag has to do two things to it: attach it to your provider record, and then set that contract's standing to active. Both are theirs, not yours - so if this stops at the attachment or the standing check, nothing is wrong with what you deployed.";

/** Re-exported so a renderer can name a disposition without importing the reader module. */
export { DomainDisposition, ZERO_ADDR };
