/**
 * How a microchip cross-check is PRESENTED. The decision itself is the backend's
 * (`stacks/vet/api/src/microchip.rs`); this only chooses words and tone.
 *
 * # What the check is
 *
 * A DogTag is linked to a pet by an operator typing or scanning a tag id, and nothing about that act
 * is self-checking. A credential carries `credentialSubject.microchip.code` as a salted Merkle leaf,
 * so recording the same code on the shop's own pet gives the two sides something to compare — which
 * turns operator memory into evidence.
 *
 * # Absent is NORMAL
 *
 * Many animals have no microchip at all; cats routinely are not chipped in Singapore. So `null` on a
 * pet is an ordinary state of an ordinary pet, never a defect to chase, and the check simply does not
 * run. That is why the copy below is neutral and never nags.
 *
 * # Three states, and the middle one is the point
 *
 * `matched` / `mismatch` / `notComparable`, and `notComparable` must never render as either
 * neighbour. Not as a silent pass — a check that did not run is not a check that passed. And not as
 * a refusal either, which is the mistake this particular surface would make: painting every
 * unchipped cat red would make the field unusable and teach operators to route around it.
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

export function microchipTone(check: MicrochipCheck): MicrochipTone {
  switch (check.state) {
    case "matched":
      return "positive";
    case "mismatch":
      // The only red. It is a statement about the LINK, not about the credential — see
      // `microchipHeadline`, whose mismatch copy says so explicitly.
      return "negative";
    default:
      // A check that did not run is neither. Which of the two "we did not compare" tones it gets
      // depends on whether something failed or simply does not exist.
      return check.isFailure ? "warning" : "neutral";
  }
}

/** The short label beside the state. Never a verdict about the credential. */
export function microchipHeadline(check: MicrochipCheck): string {
  switch (check.state) {
    case "matched":
      return "Microchip matches the credential";
    case "mismatch":
      return "Microchip does not match this credential";
    default:
      // ONE headline for every not-comparable reason, because the distinction that matters at a
      // glance is "compared" vs "not compared"; WHICH side was missing is the detail sentence's job.
      return "Microchip not compared";
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
    default:
      return check.detail;
  }
}

/**
 * Whether this state is positive evidence that the tag and the animal belong together.
 *
 * ONLY `matched`. Exists so a caller cannot express the question as `!== "mismatch"`, which quietly
 * counts every not-comparable state as a pass — the exact collapse the three states exist to prevent.
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
  if (state !== "matched" && state !== "mismatch" && state !== "notComparable") return null;
  return check as MicrochipCheck;
}
