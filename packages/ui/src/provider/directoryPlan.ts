/**
 * Flow 4: the provider publishes its pin, contact details and profile (registry-plan S-15).
 *
 * THE CAPTAIN'S RULING THIS IMPLEMENTS: **location is optional**. A provider may publish contact
 * details only - phone, WhatsApp, Telegram, email - and such a provider must not appear in the
 * mobile nearby list at all, and must never be given a placeholder coordinate. His words: "rather
 * than a false coordinate like 0 0."
 *
 * Contact-only is a FIRST-CLASS state, not a degraded one, and the shape of this module says so:
 * an absent location produces a plan with NO pin step in it. Not a pin step carrying nulls, not a
 * pin step the caller is expected to skip - the step is absent, so there is no code path on which a
 * placeholder could be written by mistake.
 *
 * WHY THE REFUSAL HAS TO LIVE HERE. `0,0` is a real coordinate off the coast of Ghana, so a pin at
 * the origin is byte-for-byte a pin anywhere else and `ProviderDirectory` cannot refuse it without
 * refusing a genuine one - `ProviderSelfService.t.sol`'s
 * `test_the_chain_cannot_tell_a_placeholder_from_a_real_coordinate` pins that absence deliberately.
 * The portal is the last place the distinction still exists, which makes this the load-bearing
 * copy rather than a belt-and-braces one.
 *
 * The mechanism the ruling guards against is not hypothetical: `Number("")` is `0`, not `NaN`, so
 * both admin register forms once turned a blank latitude field into a confident pin in the Gulf of
 * Guinea. That is why the location text goes through the SHARED {@link parseLocationInput} rather
 * than a second parser written here - a second parser is exactly how that bug came back last time.
 */

import { PROVIDER_CONTACT_CHANNELS, type ContactChannelRecord } from "../directory/channels";
import { parseLocationInput, type LocationInput } from "../directory/registration";
import type { Address, HexWord, ProviderChainReader } from "./readers";
import {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type ProviderCheck,
  type ProviderVerdict,
} from "./types";

/** `ProviderDirectory.COORDINATE_SCALE`. 1e6 is ~11cm, which is finer than a clinic pin needs. */
export const COORDINATE_SCALE = 1_000_000;

/**
 * The anchor's self-description. `schema` and `hashAlgorithm` MUST be non-zero - the contract
 * reverts `BadProfileAnchor` otherwise - so these are load-bearing values rather than filler.
 *
 * `hashAlgorithm` is the multicodec code for keccak-256 (0x1b), which is what {@link DigestFn} is
 * given (viem's `keccak256`). Naming sha2-256 here because it is the commoner constant would be a
 * false statement about which function produced the digest, and a verifier that believed it would
 * recompute the wrong hash and report a genuine blob as altered.
 *
 * `codec` describes the CONTENTHASH, and there is no contenthash yet, so 0 (unspecified) is the
 * honest value rather than a guess at what S-17 will serve.
 */
export const PROVIDER_CONTACT_SCHEMA = 1;
export const MULTIHASH_KECCAK_256 = 0x1b;
export const CONTENTHASH_CODEC_UNSPECIFIED = 0;
export const EMPTY_CONTENTHASH = "0x" as const;

export interface DirectoryPublicationRequest {
  providerId: HexWord;
  caller: Address;
  /** Raw text, exactly as typed. Blank is meaningful and must reach the parser as blank. */
  latInput: string;
  lngInput: string;
  /** Every channel, each carrying a value or the empty string. Blank means "not published". */
  contacts: Readonly<ContactChannelRecord<string>>;
  /** Opaque caller-selected code. `0` means NOT STATED and is never inferred into a real kind. */
  locationKind: number;
  /** Whether a published pin should currently be shown. */
  locationActive: boolean;
}

/**
 * One transaction the provider would send, in order.
 *
 * A discriminated union rather than a struct with optional coordinates, so "publish a pin" and
 * "publish contacts" are different shapes and no consumer can reach for a coordinate that a
 * contact-only publication does not have.
 */
export type DirectoryStep =
  | {
      kind: "profileAnchor";
      /** keccak256 of the canonical contact blob. The blob itself is off chain. */
      digest: HexWord;
      /** The blob whose digest that is, so a caller can host it rather than re-deriving it. */
      blob: string;
      channelsPublished: number;
      /**
       * `setProfileAnchor`'s remaining arguments, carried HERE rather than filled in at the call
       * site. `ProviderDirectory` reverts `BadProfileAnchor` on a zero `schema` or a zero
       * `hashAlgorithm`, so these are not decoration - and a second call site would otherwise be
       * free to invent different values for the same blob, which is how one document acquires two
       * on-chain descriptions of what it is.
       */
      schema: number;
      codec: number;
      hashAlgorithm: number;
      /**
       * Where the blob can be fetched. EMPTY while S-17 (the content mirror) does not exist: the
       * anchor's job is integrity, and publishing a location nothing serves would be a worse claim
       * than publishing none. The contract permits an empty contenthash; only digest, schema and
       * hashAlgorithm are required non-zero.
       */
      contenthash: HexWord;
    }
  | {
      kind: "pin";
      /** Already scaled by 1e6 and truncated to the contract's int32. */
      lat: number;
      lng: number;
      locationKind: number;
      active: boolean;
    };

export interface DirectoryPublicationPlan {
  checks: ProviderCheck[];
  verdict: ProviderVerdict;
  location: LocationInput;
  /**
   * The transactions to send, in order. For a contact-only provider this contains exactly ONE
   * step and it is the anchor - there is no pin step to skip.
   */
  steps: DirectoryStep[];
  /** True when the provider published no location. The state the nearby list must never show. */
  contactOnly: boolean;
  canPublish: boolean;
  nextStep: string;
}

/**
 * Turn scaled-by-1e6 into the contract's `int32`.
 *
 * `Math.round`, not `Math.trunc`: truncation biases every southern and western coordinate toward
 * the equator and the prime meridian, which is a systematic error rather than a rounding one. At
 * 1e6 the residual is ~5.5cm either way.
 */
export function toContractCoordinate(degrees: number): number {
  return Math.round(degrees * COORDINATE_SCALE);
}

/**
 * The canonical publication-safe contact blob.
 *
 * Channels are emitted in the shared list's order and a blank channel is OMITTED rather than
 * carried as `""` - an omitted channel is absent, a present-but-empty one is a published emptiness,
 * and the digest must not make those the same document. Folded over
 * {@link PROVIDER_CONTACT_CHANNELS} so a channel added there is picked up here without an edit,
 * which is the property that list exists for.
 *
 * Content addressing gives integrity, not privacy: anyone can fetch what the digest names, so only
 * publication-safe values belong in it.
 */
export function contactBlob(contacts: Readonly<ContactChannelRecord<string>>): {
  blob: string;
  channelsPublished: number;
} {
  const published: [string, string][] = [];
  for (const channel of PROVIDER_CONTACT_CHANNELS) {
    const value = (contacts[channel] ?? "").trim();
    if (value) published.push([channel, value]);
  }
  return {
    blob: JSON.stringify({ schema: "dogtag/provider-contact/1", contact: Object.fromEntries(published) }),
    channelsPublished: published.length,
  };
}

/** Injected so the plan stays pure and testable; the live surface passes viem's `keccak256`. */
export type DigestFn = (utf8: string) => HexWord;

export async function planDirectoryPublication(
  request: DirectoryPublicationRequest,
  reader: ProviderChainReader,
  digest: DigestFn,
): Promise<DirectoryPublicationPlan> {
  const { providerId, caller, latInput, lngInput, contacts, locationKind, locationActive } = request;
  const checks: ProviderCheck[] = [];

  // ---- The location, through the shared parser and nothing else ------------------------------
  const location = parseLocationInput(latInput, lngInput);
  checks.push(
    location.kind === "invalid"
      ? providerCheck(
          "directory-location",
          "Is the location usable, or deliberately left out?",
          "fail",
          location.reason,
        )
      : providerCheck(
          "directory-location",
          "Is the location usable, or deliberately left out?",
          "pass",
          location.kind === "absent"
            ? "No location given. Your contact details will be published and you will not appear in "
              + "the nearby list - that is a normal listing, not an incomplete one."
            : `Location ${location.lat}, ${location.lng} will be published.`,
        ),
  );

  // ---- The directory register, and the caller's authority over the provider record ------------
  try {
    const live = await reader.directoryIsLiveFor(providerId);
    checks.push(
      providerCheck(
        "directory-resolver-live",
        "Is the provider directory live for your provider record?",
        live ? "pass" : "fail",
        live
          ? "The directory is approved and selected for your provider record."
          : "The directory is either not approved by DogTag or not selected by your provider record.",
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "directory-resolver-live",
        "Is the provider directory live for your provider record?",
        "could-not-run",
        "The directory's status could not be read.",
        reasonFrom(error, "the isLiveFor read failed"),
      ),
    );
  }

  try {
    const canWrite = await reader.canWriteProviderRecord(providerId, caller);
    checks.push(
      providerCheck(
        "domain-write-authority",
        "May your key publish into your provider record?",
        canWrite ? "pass" : "fail",
        canWrite
          ? "Your key may publish into your provider record."
          : "Your key may not publish into your provider record. The provider's controller, or a "
            + "delegate they authorised, may.",
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "domain-write-authority",
        "May your key publish into your provider record?",
        "could-not-run",
        "Write authority could not be read.",
        reasonFrom(error, "the canWriteProvider read failed"),
      ),
    );
  }

  // ---- The steps ------------------------------------------------------------------------------
  const { blob, channelsPublished } = contactBlob(contacts);
  const steps: DirectoryStep[] = [
    {
      kind: "profileAnchor",
      digest: digest(blob),
      blob,
      channelsPublished,
      schema: PROVIDER_CONTACT_SCHEMA,
      codec: CONTENTHASH_CODEC_UNSPECIFIED,
      hashAlgorithm: MULTIHASH_KECCAK_256,
      contenthash: EMPTY_CONTENTHASH,
    },
  ];

  // The whole ruling, in one branch. `located` is the ONLY case that appends a pin - `absent` adds
  // nothing, and `invalid` adds nothing either because a refused location must not be published as
  // if it had been understood.
  if (location.kind === "located") {
    steps.push({
      kind: "pin",
      lat: toContractCoordinate(location.lat),
      lng: toContractCoordinate(location.lng),
      locationKind,
      active: locationActive,
    });
  }

  const verdict = foldVerdict(checks);
  const contactOnly = location.kind !== "located";
  return {
    checks,
    verdict,
    location,
    steps,
    contactOnly,
    canPublish: verdict === "ready",
    nextStep: directoryNextStep(verdict, contactOnly, channelsPublished),
  };
}

function directoryNextStep(
  verdict: ProviderVerdict,
  contactOnly: boolean,
  channelsPublished: number,
): string {
  if (verdict === "indeterminate") {
    return "Some checks could not run, so it is not known whether this would succeed. Try again once the connection is back.";
  }
  if (verdict === "refused") return "This cannot be published yet. The failed checks above say why.";
  if (channelsPublished === 0 && contactOnly) {
    return "You are publishing neither contact details nor a location. Add at least one so people can find you.";
  }
  return contactOnly
    ? "Ready to publish your contact details. You will be listed, and you will not appear in the nearby list because you have published no location."
    : "Ready to publish your contact details and your location.";
}

/**
 * What a contact-only listing is and is not, stated as a value the surface must render.
 *
 * A provider who publishes no location needs to know they are still listed and searchable by name,
 * because "you will not appear in nearby" reads as "you will not appear" otherwise - and a provider
 * who believes that is exactly the provider who invents a coordinate to fix it.
 */
export const CONTACT_ONLY_NOTICE =
  "Publishing without a location is a normal listing. People can find you by name and reach you through the contact details you publish. You will not appear in the nearby list, because that list is built from published locations - and an invented coordinate would put you somewhere you are not.";

/**
 * The blob is composed here and hosted nowhere.
 *
 * The chain carries only the integrity anchor - digest, schema, codec, hash algorithm, contenthash.
 * The mirror that actually serves the blob is S-17, which depends on this slice. So a provider's
 * contacts are ANCHORED by this flow and are not yet FETCHABLE from chain data alone, and a surface
 * must say that rather than implying the details are already readable by anyone.
 */
export const CONTACTS_ARE_ANCHORED_NOT_SERVED =
  "Your contact details are recorded on chain as a fingerprint, so anyone can check they have not been altered. The service that publishes the details themselves is not built yet.";

export { parseLocationInput, type LocationInput };
