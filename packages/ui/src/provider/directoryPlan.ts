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
 *
 * PUBLISHING IS NOT APPEND-ONLY, and treating it as such put one provider in two places.
 * `ProviderDirectory.publishPin` issues a FRESH location number on every call, so a provider
 * correcting a mistyped coordinate by pressing Publish again left BOTH pins live and active in the
 * scan - and the mobile nearby list is built from exactly that scan, so it would have shown them at
 * two places at once. `updatePin` and `removePin` exist on the deployed contract, so the plan reads
 * what is already published and REWRITES the pin it is correcting. Where it cannot tell which pin a
 * form corresponds to, it REFUSES rather than appending a second one and says why.
 *
 * `setProfileAnchor` is read the same way and for a sharper reason: it has NO `NoChange` guard and
 * the contract is frozen, so a re-publish of unchanged contacts bumps the anchor revision, and every
 * bump makes `pinAddressProvenance.coversCurrentAddressText` false for any registrar address
 * confirmation the provider holds. The portal is the only place that no-op can be avoided.
 */

import { PROVIDER_CONTACT_CHANNELS, type ContactChannelRecord } from "../directory/channels";
import { parseLocationInput, type LocationInput } from "../directory/registration";
import { ZERO_WORD, type Address, type DirectoryPin, type HexWord, type ProviderChainReader } from "./readers";
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
      /**
       * `publish` issues a NEW location number; `update` rewrites the one named by `locationNo`.
       *
       * Modelled as two shapes rather than an optional `locationNo`, so the append case cannot
       * carry a number and the rewrite case cannot omit one. Emitting `publish` where `update` was
       * meant is the two-pins bug, and it is silent on chain.
       */
      op: "publish";
      /** Already scaled by 1e6 and rounded to the contract's int32. */
      lat: number;
      lng: number;
      locationKind: number;
      active: boolean;
    }
  | {
      kind: "pin";
      op: "update";
      /** The existing location's number, which `updatePin` keeps. Never reissued by the contract. */
      locationNo: number;
      lat: number;
      lng: number;
      locationKind: number;
      active: boolean;
    };

/**
 * What this provider has already published, when it could be established.
 *
 * `undefined` on the plan means the read FAILED - not that nothing is published. The difference is
 * the whole reason a second pin is refused rather than appended: "we could not ask what exists"
 * cannot license a write whose safety depends on knowing.
 */
export interface DirectoryListingState {
  /** How many locations are published. */
  pinCount: number;
  /** The single published location, when there is exactly one and it could be read. */
  onlyPin?: DirectoryPin;
  /** The digest currently anchored, or the all-zero word when none is. */
  anchorDigest: HexWord;
  /** How many times the anchor has been written. `0n` means never. */
  anchorRevision: bigint;
}

export interface DirectoryPublicationPlan {
  /** The provider this plan is about, captured from the request. A send addresses THIS. */
  providerId: HexWord;
  checks: ProviderCheck[];
  verdict: ProviderVerdict;
  location: LocationInput;
  /**
   * The transactions to send, in order. For a contact-only provider there is no pin step - it is
   * absent rather than skipped. It can also be EMPTY, when nothing would change.
   */
  steps: DirectoryStep[];
  /** True when the provider published no location. The state the nearby list must never show. */
  contactOnly: boolean;
  /** What is already on chain, or `undefined` when that could not be established. */
  listing?: DirectoryListingState;
  canPublish: boolean;
  /**
   * True only when exactly one location is published, it could be read, and this key's authority
   * over the provider record was established. Withdrawal is offered separately from Publish because
   * a blank coordinate field must never be read as "take my location down" - that would make a
   * typo destructive.
   */
  canWithdrawPin: boolean;
  nextStep: string;
}

/**
 * How far into a provider's location numbers this surface will scan to find its single pin.
 *
 * Numbers are issued from a monotone per-provider counter and never reissued, so a provider that has
 * published and withdrawn many times has a high counter and few pins. Past this bound the scan is
 * abandoned as a could-not-run rather than silently reporting "no pin found", which would append.
 */
export const MAX_SCANNED_LOCATION_NUMBERS = 64;

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

/**
 * Read what this provider has already published.
 *
 * THROWS on any unreadable read, and on an inconsistency it cannot resolve. The caller turns that
 * into a `could-not-run`; answering "nothing is published" from a read that failed would be exactly
 * the collapse that appends a second pin.
 *
 * The scan is skipped entirely unless there is exactly ONE pin: zero needs no scan, and more than
 * one is refused on the count alone, so neither case should pay for it.
 */
export async function readListingState(
  reader: ProviderChainReader,
  providerId: HexWord,
): Promise<DirectoryListingState> {
  const anchor = await reader.providerProfileAnchor(providerId);
  const pinCount = await reader.providerPinCount(providerId);
  const base = {
    pinCount,
    anchorDigest: anchor.digest,
    anchorRevision: anchor.revision,
  };
  if (pinCount !== 1) return base;

  const next = await reader.providerNextLocationNumber(providerId);
  if (next > MAX_SCANNED_LOCATION_NUMBERS) {
    throw new Error(
      `this provider's location numbers run to ${next}, past the ${MAX_SCANNED_LOCATION_NUMBERS} this page will scan`,
    );
  }
  for (let locationNo = 0; locationNo < next; locationNo++) {
    if (await reader.providerHasPin(providerId, locationNo)) {
      return { ...base, onlyPin: await reader.providerPin(providerId, locationNo) };
    }
  }
  throw new Error("the directory reports one published location but none could be found");
}

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
  let directoryLive: boolean | undefined;
  let mayWriteRecord: boolean | undefined;
  try {
    const live = await reader.directoryIsLiveFor(providerId);
    directoryLive = live;
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
    mayWriteRecord = canWrite;
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

  // ---- What is already published -------------------------------------------------------------
  let listing: DirectoryListingState | undefined;
  try {
    listing = await readListingState(reader, providerId);
    const tooMany = listing.pinCount > 1;
    if (tooMany && location.kind === "located") {
      // The refusal the captain's ruling requires, and it is deliberately scoped to the case where
      // a pin would actually be written. A provider with several legacy pins must still be able to
      // update its CONTACTS, so an unconditional refusal here would be over-broad in the other
      // direction.
      checks.push(
        providerCheck(
          "directory-listing-state",
          "What is already published for your provider record?",
          "fail",
          `You have ${listing.pinCount} published locations. This page publishes one, and cannot tell `
            + "which of them you mean, so it will not add another - two live locations would put you in "
            + "two places at once in the nearby list. Ask DogTag to tidy them up first.",
        ),
      );
    } else {
      checks.push(
        providerCheck(
          "directory-listing-state",
          "What is already published for your provider record?",
          "pass",
          describeListing(listing, location.kind === "located"),
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "directory-listing-state",
        "What is already published for your provider record?",
        "could-not-run",
        "What you have already published could not be read, so it is not known whether this would "
          + "replace a location or add a second one.",
        reasonFrom(error, "the directory read failed"),
      ),
    );
  }

  // ---- The steps ------------------------------------------------------------------------------
  const { blob, channelsPublished } = contactBlob(contacts);
  const anchorDigest = digest(blob);
  const steps: DirectoryStep[] = [];

  // Omitted when it would change nothing. `setProfileAnchor` has no `NoChange` guard, so a needless
  // write bumps the revision, and every bump invalidates `coversCurrentAddressText` for a registrar
  // address confirmation. With the listing state unknown the anchor IS sent: that is what the
  // provider asked for, and the unknown state has already made the verdict indeterminate.
  const anchorUnchanged =
    !!listing &&
    listing.anchorDigest.toLowerCase() === anchorDigest.toLowerCase() &&
    listing.anchorDigest.toLowerCase() !== ZERO_WORD;
  if (!anchorUnchanged) {
    steps.push({
      kind: "profileAnchor",
      digest: anchorDigest,
      blob,
      channelsPublished,
      schema: PROVIDER_CONTACT_SCHEMA,
      codec: CONTENTHASH_CODEC_UNSPECIFIED,
      hashAlgorithm: MULTIHASH_KECCAK_256,
      contenthash: EMPTY_CONTENTHASH,
    });
  }

  // The whole ruling, in one branch. `located` is the ONLY case that emits a pin step - `absent`
  // emits nothing, and `invalid` emits nothing either because a refused location must not be
  // published as if it had been understood. A `located` pin whose existing state is UNKNOWN emits
  // nothing too: appending is only safe when we know there is nothing to replace.
  if (location.kind === "located" && listing) {
    const lat = toContractCoordinate(location.lat);
    const lng = toContractCoordinate(location.lng);
    const existing = listing.onlyPin;
    if (listing.pinCount === 0) {
      steps.push({ kind: "pin", op: "publish", lat, lng, locationKind, active: locationActive });
    } else if (existing) {
      const same =
        existing.lat === lat &&
        existing.lng === lng &&
        existing.kind === locationKind &&
        existing.active === locationActive;
      // `updatePin` reverts `NoChange` on an identical word, so sending it would be an opaque
      // failure rather than a no-op.
      if (!same) {
        steps.push({
          kind: "pin",
          op: "update",
          locationNo: existing.locationNo,
          lat,
          lng,
          locationKind,
          active: locationActive,
        });
      }
    }
  }

  // ---- Is there anything to publish at all? ---------------------------------------------------
  // Closes the contradiction where a plan reported itself ready while its own next step said "add
  // at least one so people can find you". An empty anchor is a published emptiness, and the listing
  // sequence is append-only, so it is worth refusing rather than recording.
  if (channelsPublished === 0 && location.kind !== "located") {
    checks.push(
      providerCheck(
        "directory-contents",
        "Is there anything to publish?",
        "fail",
        "You are publishing neither contact details nor a location. Add at least one so people can find you.",
      ),
    );
  } else if (steps.length === 0) {
    checks.push(
      providerCheck(
        "directory-contents",
        "Is there anything to publish?",
        "fail",
        "Everything here is already published exactly as it stands, so there is nothing to send.",
      ),
    );
  } else {
    checks.push(
      providerCheck(
        "directory-contents",
        "Is there anything to publish?",
        "pass",
        describeSteps(steps),
      ),
    );
  }

  const verdict = foldVerdict(checks);
  const contactOnly = location.kind !== "located";
  return {
    providerId,
    checks,
    verdict,
    location,
    steps,
    contactOnly,
    ...(listing ? { listing } : {}),
    canPublish: verdict === "ready",
    // Deliberately NOT gated on the whole verdict: an unusable latitude typo must not block taking
    // a published location down. It needs the two authority facts and a pin that was actually read.
    canWithdrawPin:
      directoryLive === true && mayWriteRecord === true && !!listing?.onlyPin,
    nextStep: directoryNextStep(verdict, location.kind, channelsPublished),
  };
}

/** What is already on chain, in the provider's own terms. */
function describeListing(listing: DirectoryListingState, publishingLocation: boolean): string {
  const anchored =
    listing.anchorRevision === 0n
      ? "You have never published contact details."
      : listing.anchorDigest.toLowerCase() === ZERO_WORD
        ? "Your contact details have been withdrawn."
        : "You have contact details published.";
  if (listing.pinCount === 0) {
    return `${anchored} You have no published location${publishingLocation ? ", so this would publish your first" : ""}.`;
  }
  if (listing.pinCount === 1 && listing.onlyPin) {
    return `${anchored} You have one published location (number ${listing.onlyPin.locationNo})`
      + `${publishingLocation ? ", and this would replace it rather than add a second" : ", which leaving the location fields blank does not withdraw"}.`;
  }
  return `${anchored} You have ${listing.pinCount} published locations.`;
}

/** What would actually be sent, so "Publish" is never a button whose effect is unstated. */
function describeSteps(steps: readonly DirectoryStep[]): string {
  const parts = steps.map((step) => {
    if (step.kind === "profileAnchor") {
      return step.channelsPublished === 0
        ? "your contact details (none - this records that you publish none)"
        : `your contact details (${step.channelsPublished} channel${step.channelsPublished === 1 ? "" : "s"})`;
    }
    return step.op === "publish"
      ? "a new location"
      : `a replacement for your published location (number ${step.locationNo})`;
  });
  return `This would send ${parts.length} transaction${parts.length === 1 ? "" : "s"}: ${parts.join(", then ")}.`;
}

function directoryNextStep(
  verdict: ProviderVerdict,
  locationKind: LocationInput["kind"],
  channelsPublished: number,
): string {
  if (verdict === "indeterminate") {
    return "Some checks could not run, so it is not known whether this would succeed. Try again once the connection is back.";
  }
  // Stated BEFORE the generic refusal, because this is a refusal whose specific remedy is known and
  // "the failed checks say why" would send the provider to read what this sentence already says.
  // Keyed on `absent` rather than on `contactOnly`, so an UNUSABLE location is not diagnosed as a
  // missing one - those have different remedies.
  if (channelsPublished === 0 && locationKind === "absent") {
    return "You are publishing neither contact details nor a location. Add at least one so people can find you.";
  }
  if (verdict === "refused") return "This cannot be published yet. The failed checks above say why.";
  return locationKind !== "located"
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

/**
 * Taking a published location down, stated as a value the surface must render.
 *
 * Withdrawal is its own deliberate action and is never inferred from a blank coordinate field: a
 * provider clearing a typo would otherwise silently remove their pin. The provider stays listed and
 * contactable - withdrawing content is not leaving the directory.
 */
export const WITHDRAW_LOCATION_NOTICE =
  "Taking your location down leaves your listing and your contact details in place. You stop appearing in the nearby list, and you can publish a location again later. Clearing the latitude and longitude fields does NOT do this - it only means this publication carries no location.";

export { parseLocationInput, type LocationInput, type DirectoryPin };
