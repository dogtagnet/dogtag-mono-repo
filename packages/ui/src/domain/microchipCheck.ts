/**
 * How a microchip cross-check is PRESENTED. The decision itself is the backend's
 * (`stacks/vet/api/src/microchip.rs`); this only chooses words and tone.
 *
 * # What the check is
 *
 * A DogTag is linked to a pet by an operator typing or scanning a tag id, and nothing about that act
 * is self-checking. A credential carries the animal's microchip as a salted Merkle leaf, so recording
 * the same code on the shop's own pet gives the two sides something to compare — which turns operator
 * memory into evidence. WHICH leaf is a key-path suffix match over the four shapes real issuers emit;
 * the backend owns that list and the reason it is a suffix rather than one exact path.
 *
 * # Absent is NORMAL
 *
 * Many animals have no microchip at all; cats routinely are not chipped in Singapore. So `null` on a
 * pet is an ordinary state of an ordinary pet, never a defect to chase, and the check simply does not
 * run. That is why the copy below is neutral and never nags.
 *
 * # Four states, and the middle two are the point
 *
 * `matched` / `mismatch` / `notComparable` / `unrecognisedCredentialLeaf`, and `notComparable` must
 * never render as either of its neighbours. Not as a silent pass — a check that did not run is not a
 * check that passed. And not as a refusal either, which is the mistake this particular surface would
 * make: painting every unchipped cat red would make the field unusable and teach operators to route
 * around it.
 *
 * # Why the fourth state is not a `notComparable` reason
 *
 * This check shipped once reading a single key path no real issuer emits, so it was inert on every
 * real credential. What hid that is this file's own vocabulary: "the credential has no microchip" is
 * ordinary, quiet and benign, and a reader that cannot FIND a microchip that is present produced the
 * identical answer. So `unrecognisedCredentialLeaf` is a state of its own, gets the loudest
 * non-negative tone, says the check could not RUN rather than that there was nothing to compare, and
 * names the key paths it found so the remedy is obvious from the sentence.
 *
 * # The tone split is `isFailure`, and it comes from the wire
 *
 * A `notComparable` is either a FACT about what exists (this pet has no chip; this credential has
 * none; this shop holds no credential) or a FAILURE to look (the store could not be read; the
 * document will not parse; the tag is stored in a form the cache cannot be asked about). Facts are
 * NEUTRAL; failures get the warning treatment.
 *
 * Deliberately NOT copied from the issuer-whitelist pillar's rule that an indeterminate state renders
 * in warning colour. That rule is for failing to establish something we needed, and an unchipped cat
 * is not a failure to establish anything — amber there would over-claim in the other direction, on
 * the commonest state in the product. And `isFailure` is READ FROM THE RESPONSE rather than
 * re-derived from `reason` here, so a reason added later cannot silently default to the wrong tone.
 */

import type { MicrochipCheck } from "../api/types";

/** Same vocabulary as `bindingTone`, so the portals have one tone language. */
export type MicrochipTone = "positive" | "negative" | "warning" | "neutral";

/**
 * Every arm below is EXPLICIT and the fall-through is an exhaustiveness error, not a default.
 *
 * A `default:` here is what quietly absorbed the state this file most needs to keep separate: it
 * would have read `check.isFailure`, which `unrecognisedCredentialLeaf` does not carry, and landed
 * on `neutral` — the one outcome a reader that could not read must never get.
 */
export function microchipTone(check: MicrochipCheck): MicrochipTone {
  switch (check.state) {
    case "matched":
      return "positive";
    case "mismatch":
      // The only red. It is a statement about the LINK, not about the credential — see
      // `microchipHeadline`, whose mismatch copy says so explicitly.
      return "negative";
    case "notComparable":
      // A check that did not run is neither. Which of the two "we did not compare" tones it gets
      // depends on whether something failed or simply does not exist.
      return check.isFailure ? "warning" : "neutral";
    case "unrecognisedCredentialLeaf":
      // The loudest treatment short of the red reserved for a mismatch, and never neutral. Our
      // reader failed to read a microchip that is present — a defect on our side, so it warns; but
      // it accuses neither the animal nor the credential, so it is not negative.
      return "warning";
    default: {
      const exhaustive: never = check;
      return exhaustive;
    }
  }
}

/** The short label beside the state. Never a verdict about the credential. */
export function microchipHeadline(check: MicrochipCheck): string {
  switch (check.state) {
    case "matched":
      return "Microchip matches the credential";
    case "mismatch":
      return "Microchip does not match this credential";
    case "notComparable":
      // ONE headline for every not-comparable reason, because the distinction that matters at a
      // glance is "compared" vs "not compared"; WHICH side was missing is the detail sentence's job.
      return "Microchip not compared";
    case "unrecognisedCredentialLeaf":
      // NOT "not compared". That wording is the camouflage: it is the sentence an unchipped cat
      // gets, and reading it over a credential that IS carrying a microchip is what let this check
      // ship inert. It says the check could not RUN.
      return "Microchip check could not run";
    default: {
      const exhaustive: never = check;
      return exhaustive;
    }
  }
}

/**
 * The sentence explaining the state.
 *
 * For `notComparable` this is the backend's own `detail`, passed through rather than re-worded here:
 * a second copy of seven reason strings in the client is a second thing to keep in step, and the one
 * that drifts is always the one the operator reads.
 */
export function microchipExplanation(check: MicrochipCheck): string {
  switch (check.state) {
    case "matched":
      return `The credential's microchip is ${check.microchip}, the same as this pet's record.`;
    case "mismatch":
      return (
        `This pet's record says ${check.petMicrochip}; the credential says ` +
        `${check.credentialMicrochip}. The credential itself is not in question — it stays valid — ` +
        `but it describes a different animal to the one on this record. Check the DogTag id and this ` +
        `pet's microchip before linking.`
      );
    case "notComparable":
    case "unrecognisedCredentialLeaf":
      // Both pass the backend's own sentence through. For the unrecognised state that sentence names
      // the key paths, which is the whole remedy, so re-wording it here would drop the actionable
      // half of the message.
      return check.detail;
    default: {
      const exhaustive: never = check;
      return exhaustive;
    }
  }
}

/**
 * Whether this state is positive evidence that the tag and the animal belong together.
 *
 * ONLY `matched`. Exists so a caller cannot express the question as `!== "mismatch"`, which quietly
 * counts every other state as a pass — the exact collapse the four states exist to prevent, and the
 * one that would swallow `unrecognisedCredentialLeaf` most quietly of all.
 */
export function microchipConfirmsAnimal(check: MicrochipCheck): boolean {
  return check.state === "matched";
}

/**
 * Pull the structured check out of a rejected request's body.
 *
 * The refusal carries the same `microchipCheck` object as a success, so the UI has ONE vocabulary
 * rather than a rendered state on the happy path and a parsed sentence on the sad one. Returns null
 * for any other failure, so an unrelated 409 (a tag already held by another pet, say) is not
 * mislabelled as a microchip problem.
 */
export function microchipCheckFromError(err: unknown): MicrochipCheck | null {
  const body = (err as { body?: unknown } | null | undefined)?.body;
  if (!body || typeof body !== "object") return null;
  const check = (body as { microchipCheck?: unknown }).microchipCheck;
  if (!check || typeof check !== "object") return null;
  const state = (check as { state?: unknown }).state;
  // Every state the backend can emit. A state missing from this list is DROPPED, so it must move
  // with `MicrochipCheck` — the same reason `microchipTone` refuses a `default:` arm.
  if (
    state !== "matched" &&
    state !== "mismatch" &&
    state !== "notComparable" &&
    state !== "unrecognisedCredentialLeaf"
  ) {
    return null;
  }
  return check as MicrochipCheck;
}
