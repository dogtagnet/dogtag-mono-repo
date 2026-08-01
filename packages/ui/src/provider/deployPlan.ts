/**
 * Flow 1: the provider deploys its own clone (registry-plan S-15).
 *
 * The captain's framing is that the provider deploys and DogTag does not deploy for them. On
 * generation 1 that was not merely unbuilt but impossible - `DogTagIssuerFactory.createIssuer` is
 * `onlyOwner`, so no provider key could send it. Generation 2's factory replaces that with the
 * core's `canCreateService` predicate, which is what makes this flow exist at all.
 *
 * This module is a PREFLIGHT, not a gate. The factory re-asks the core on every `createIssuer`, so
 * what is here only avoids sending a transaction that cannot succeed, and says which term is
 * missing when one is.
 */

import {
  Standing,
  STANDING_LABEL,
  type Address,
  type HexWord,
  type ProviderChainReader,
} from "./readers";
import {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type ProviderCheck,
  type ProviderVerdict,
} from "./types";

export interface DeployPlanRequest {
  providerId: HexWord;
  recordType: HexWord;
  /** The key that would send `createIssuer` and would own the result. */
  caller: Address;
  /**
   * Which clone of this record type. The salt carries it, so a provider has more than one possible
   * address per record type - without it there is exactly one and nothing to repoint TO.
   */
  cloneNonce: bigint;
}

export interface DeployPlan {
  /**
   * The exact inputs this plan was computed from, carried back out.
   *
   * A send addresses THESE, never whatever the form holds when the button is pressed. The free-nonce
   * and `canCreateService` answers below are about this record type and this number; sending a
   * different pair would be deploying something nobody checked.
   */
  request: DeployPlanRequest;
  checks: ProviderCheck[];
  verdict: ProviderVerdict;
  /**
   * The exact address `createIssuer` would deploy, when it could be computed.
   *
   * `undefined` means the prediction read failed - NOT that the address is unknowable. A surface
   * must render the difference: showing a blank where a provider expects an address reads as "no
   * address yet", which is a different and wrong fact.
   */
  predictedAddress?: Address;
  /** True only when every check passed. Never a substitute for reading the checks. */
  canDeploy: boolean;
  nextStep: string;
}

export async function planCloneDeployment(
  request: DeployPlanRequest,
  reader: ProviderChainReader,
): Promise<DeployPlan> {
  const { providerId, recordType, caller, cloneNonce } = request;
  const checks: ProviderCheck[] = [];

  // ---- Provider standing --------------------------------------------------------------------
  try {
    const provider = await reader.provider(providerId);
    const active = provider.standing === Standing.ACTIVE;
    checks.push(
      providerCheck(
        "provider-standing",
        "Is your provider record cleared to act?",
        active ? "pass" : "fail",
        active
          ? "Your provider record is active."
          : `Your provider record is ${STANDING_LABEL[provider.standing]}.`,
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "provider-standing",
        "Is your provider record cleared to act?",
        "could-not-run",
        "Your provider record could not be read.",
        reasonFrom(error, "the provider() read failed"),
      ),
    );
  }

  // ---- Deploy authority ----------------------------------------------------------------------
  // See `ProviderChainReader.canCreateService`: this read is only meaningful when it is made AS
  // the factory. Asked with no `from` it answers `false` for every provider on earth, which would
  // tell an eligible provider it may not deploy.
  try {
    const approved = await reader.canCreateService(providerId, recordType, caller);
    checks.push(
      providerCheck(
        "deploy-authority",
        "May this key deploy a contract for your provider and record type?",
        approved ? "pass" : "fail",
        approved
          ? "Approved: your key may deploy this record type for your provider."
          : "Not approved. DogTag approves each record type a provider may issue, and your key must "
            + "hold the create permission on the provider record.",
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "deploy-authority",
        "May this key deploy a contract for your provider and record type?",
        "could-not-run",
        "The approval could not be read.",
        reasonFrom(error, "the canCreateService read failed"),
      ),
    );
  }

  // ---- The address itself --------------------------------------------------------------------
  // Predicted BEFORE the transaction and exactly equal to what gets deployed, so the provider is
  // shown what they are about to create rather than asked to trust a receipt.
  let predictedAddress: Address | undefined;
  let nonceTaken = false;
  try {
    predictedAddress = await reader.predictIssuer(recordType, caller, cloneNonce);
    nonceTaken = await reader.isFactoryClone(predictedAddress);
    if (nonceTaken) {
      checks.push(
        providerCheck(
          "clone-provenance",
          "Is this contract number still free?",
          "fail",
          `Contract number ${cloneNonce} already exists at ${predictedAddress}. Choose another number.`,
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "clone-provenance",
        "Is this contract number still free?",
        "could-not-run",
        "The address this would deploy to could not be computed.",
        reasonFrom(error, "the predictIssuer read failed"),
      ),
    );
  }

  const verdict = foldVerdict(checks);
  return {
    request: { providerId, recordType, caller, cloneNonce },
    checks,
    verdict,
    ...(predictedAddress ? { predictedAddress } : {}),
    canDeploy: verdict === "ready",
    nextStep: deployNextStep(verdict, nonceTaken),
  };
}

function deployNextStep(verdict: ProviderVerdict, nonceTaken: boolean): string {
  if (verdict === "indeterminate") {
    return "Some checks could not run, so it is not known whether this would succeed. Try again once the connection is back.";
  }
  if (verdict === "refused") {
    return nonceTaken
      ? "Choose a different contract number."
      : "This cannot be deployed yet. The failed checks above say why.";
  }
  return "Ready to deploy. You will own the contract. DogTag then attaches it to your provider record before you can select it.";
}

/**
 * What happens AFTER a successful deploy, stated as a value the surface must render.
 *
 * The step people model wrong: `attachService` is `onlyOwner` on the core, so a freshly deployed
 * contract is genuine, owned by the provider, and attached to nobody. A surface that presented
 * deploy-and-select as one continuous self-service action would be describing a journey no provider
 * can complete.
 */
export const ATTACHMENT_IS_NOT_SELF_SERVICE =
  "A contract you deploy is yours immediately, but it is not part of your listing until DogTag attaches it to your provider record. That step is DogTag's, not yours.";
