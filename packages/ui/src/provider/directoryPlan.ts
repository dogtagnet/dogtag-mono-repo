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

import { type ContactChannelRecord } from "../directory/channels";
import { parseLocationInput, type LocationInput } from "../directory/registration";
import {
  buildProfileBlob,
  logoRef,
  PROFILE_MEDIA_TYPE,
  PROVIDER_PROFILE_SCHEMA,
  type LogoPublication,
} from "../mirror/profileBlob";
import {
  Standing,
  STANDING_LABEL,
  ZERO_WORD,
  type Address,
  type DirectoryPin,
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
 * `schema` lives in {@link PROVIDER_PROFILE_SCHEMA}, beside the blob's own schema STRING, because
 * the two describe the same document and must only ever move together.
 *
 * `codec` describes how to RESOLVE the anchored digest. S-15 published 0 (unspecified), which was
 * honest while nothing served the content; S-17 serves it, so an unchanged 0 would now be a false
 * silence about content that really is resolvable.
 */
export const MULTIHASH_KECCAK_256 = 0x1b;

/**
 * The digest is a raw keccak-256 content address, resolvable from any DogTag content mirror.
 *
 * **The contenthash stays EMPTY, and that is the scheme rather than an omission.** A content address
 * makes the host irrelevant by construction - the bytes are checked against the address, so serving
 * confers nothing - and a host published here would be a provider-named fetch target buying no
 * integrity it does not already have. Locations are deployment configuration; the identifier is what
 * belongs on chain.
 */
export const CONTENTHASH_CODEC_DOGTAG_MIRROR_V1 = 1;
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
  /**
   * The logo to publish, or `null` for none.
   *
   * `null` is the ordinary contact-only case and is not a failure. The bytes are mirrored under
   * their own content address and that address is committed INSIDE the profile blob, so the chain's
   * single anchor digest covers the logo too.
   */
  logo?: LogoPublication | null;
}

/**
 * One step the provider would perform, in order.
 *
 * A discriminated union rather than a struct with optional coordinates, so "publish a pin" and
 * "publish contacts" are different shapes and no consumer can reach for a coordinate that a
 * contact-only publication does not have.
 *
 * **Not every step is a transaction.** `mirrorUpload` is an HTTP publication to the content mirror,
 * and every one of them is ordered BEFORE the `profileAnchor` transaction. That ordering is
 * load-bearing rather than tidy: the anchor is the irreversible half, so it must never name content
 * the mirror does not hold. The same rule that puts `issue(R)` before `mintCustodial` - the
 * unrepairable write goes last, so a failure of an earlier step costs a retry rather than a
 * permanently dangling reference.
 */
export type DirectoryStep =
  | {
      kind: "mirrorUpload";
      /** What this object is, so a runner can report which upload failed. */
      what: "logo" | "profile";
      /** The content address these bytes hash to. The mirror refuses them under any other. */
      address: HexWord;
      bytes: Uint8Array;
      mediaType: string;
    }
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

// The blob builder lives in `../mirror/profileBlob` as of S-17, beside the PARSER that reads it
// back. The v1 `contactBlob` that lived here is gone rather than deprecated: it emitted a document
// this build can no longer read, and a second builder is precisely how a provider comes to publish
// something no consumer understands with the digest verifying perfectly either way.

/**
 * Injected so the plan stays pure and testable; the live surface passes viem's `keccak256`.
 *
 * Takes UTF-8 TEXT or raw BYTES, because a plan digests both: the profile blob is a string and a
 * logo is an image. One function rather than two, so the two content addresses in one publication
 * cannot end up computed by different algorithms - which would make the blob's own digest cover a
 * logo address no consumer could reproduce.
 */
export type DigestFn = (content: string | Uint8Array) => HexWord;

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

  // THE PROVIDER'S OWN STANDING, read BEFORE its authority - the same order and the same reason as
  // the repoint and domain flows. `ProviderRegistry.canWriteProvider` returns false on
  // `p.standing != Standing.ACTIVE` BEFORE it looks at the caller, so for a provider record that is
  // not ACTIVE it answers `false` for the controller, for every delegate and for everybody else.
  //
  // THIS IS THE STATE EVERY NEW PROVIDER STARTS IN. `registerProvider` writes `PENDING`, and
  // `setProviderStanding(ACTIVE)` is a separate registrar call - so the very first time a provider
  // opens this page, the row read that shared `false` and told them their key was not allowed to
  // publish into their own record. This flow did not read the standing at all, so nothing on screen
  // named the real cause.
  let providerActive: boolean | undefined;
  try {
    const record = await reader.provider(providerId);
    providerActive = record.standing === Standing.ACTIVE;
    checks.push(
      providerCheck(
        "provider-standing",
        "Is your provider record cleared to act?",
        providerActive ? "pass" : "fail",
        providerActive
          ? "Your provider record is active."
          : record.standing === Standing.PENDING
            ? "Your provider record is registered and its standing is still pending review. DogTag "
              + "sets it to active as the next step - being registered leaves it pending by design. "
              + "There is nothing for you to change here."
            : `Your provider record is ${STANDING_LABEL[record.standing]}, so it cannot publish.`,
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

  try {
    const canWrite = await reader.canWriteProviderRecord(providerId, caller);
    mayWriteRecord = canWrite;
    if (canWrite) {
      checks.push(
        providerCheck(
          "domain-write-authority",
          "May your key publish into your provider record?",
          "pass",
          "Your key may publish into your provider record.",
        ),
      );
    } else if (providerActive === true) {
      checks.push(
        providerCheck(
          "domain-write-authority",
          "May your key publish into your provider record?",
          "fail",
          "Your key may not publish into your provider record. The provider's controller, or a "
            + "delegate they authorised, may.",
        ),
      );
    } else {
      checks.push(
        providerCheck(
          "domain-write-authority",
          "May your key publish into your provider record?",
          "could-not-run",
          "Whether your key may publish into your provider record is not established yet.",
          providerActive === false
            ? "the registry refuses a write from ANY key while your provider record's standing is not "
              + "active, so this answer says nothing about your key - the standing row above is the "
              + "one to act on"
            : "your provider record's standing could not be read, and the registry folds it into this "
              + "same answer, so a refusal here cannot be attributed to your key",
        ),
      );
    }
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
  // The logo is addressed FIRST, because its address is a field of the blob and therefore an input
  // to the blob's own digest. That is what puts the logo under the chain's single anchor digest.
  const logo = request.logo ?? null;
  const logoAddress = logo ? digest(logo.bytes) : null;
  const { blob, channelsPublished } = buildProfileBlob(
    contacts,
    logo && logoAddress ? logoRef(logoAddress, logo) : null,
  );
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

  // THE UPLOADS ARE UNCONDITIONAL, and the anchor transaction is not. Two different concerns that
  // shared one branch until they were separated here.
  //
  // An upload is idempotent BY CONSTRUCTION - the same bytes have the same address, and the mirror
  // overwrites identical content - so re-uploading costs one request. Omitting it when the anchor
  // already matches made a lost object unrecoverable: the mirror is ephemeral, so a restart empties
  // it while the chain still holds the digest, and the only publisher surface would then answer
  // "nothing to publish" about content it is holding the sole remaining copy of. The remaining
  // workaround - editing a contact to move the digest - bumps the anchor revision and invalidates
  // `coversCurrentAddressText` on any registrar address confirmation, which is precisely the harm
  // `anchorUnchanged` exists to prevent.
  //
  // Both uploads still precede the anchor. A mirror that does not hold what the anchor names would
  // leave every consumer resolving `absent` for a digest the chain says is published - which is
  // indistinguishable, from the outside, from a provider who published nothing.
  if (logo && logoAddress) {
    steps.push({
      kind: "mirrorUpload",
      what: "logo",
      address: logoAddress,
      bytes: logo.bytes,
      mediaType: logo.mediaType,
    });
  }
  steps.push({
    kind: "mirrorUpload",
    what: "profile",
    address: anchorDigest,
    bytes: new TextEncoder().encode(blob),
    mediaType: PROFILE_MEDIA_TYPE,
  });

  if (!anchorUnchanged) {
    steps.push({
      kind: "profileAnchor",
      digest: anchorDigest,
      blob,
      channelsPublished,
      schema: PROVIDER_PROFILE_SCHEMA,
      codec: CONTENTHASH_CODEC_DOGTAG_MIRROR_V1,
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
  //
  // This is now the ONLY reachable refusal here, and deliberately so. The profile document is
  // always uploaded, so `steps` is never empty and the old "everything is already published, so
  // there is nothing to send" branch could not fire - an unreachable refusal is a claim nobody can
  // check, so it is gone rather than left as reassuring dead code. A publication whose only steps
  // are uploads IS offerable: it re-publishes the document to the mirror and sends no transaction,
  // which is exactly the repair path after the mirror has lost it.
  if (channelsPublished === 0 && location.kind !== "located") {
    checks.push(
      providerCheck(
        "directory-contents",
        "Is there anything to publish?",
        "fail",
        "You are publishing neither contact details nor a location. Add at least one so people can find you.",
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

/**
 * Why this publication cannot be sent with the mirror as configured, or `null` when it can.
 *
 * THE REFUSAL LIVES HERE, ONE LEVEL ABOVE THE SEND LOOP, and that placement is the fix rather than a
 * tidy-up. Both of these were once discovered from INSIDE the loop, on the first upload: the loop
 * had already begun, so the publication aborted mid-sequence, and the missing-token case surfaced as
 * a bare "missing bearer token" from the HTTP layer with nothing an operator could act on. A step
 * sequence must be refused before its first step, not during it.
 *
 * Both call sites use this - the pre-flight throw AND the button's own disabled state - because a
 * helper consulted only at the throw site still leaves a button offering an action that cannot
 * succeed.
 *
 * A CHECKED plan with no uploads needs neither setting and is never refused here. That case is real:
 * a pin-only correction sends transactions and publishes no content.
 *
 * `undefined` steps means no plan has been checked yet, and is answered as though there WILL be
 * uploads - which is true by construction, since every publication uploads its profile document.
 * That is what lets the surface state a configuration problem before the provider fills the form in
 * rather than after they press Publish.
 */
export function mirrorPublicationRefusal(
  steps: readonly DirectoryStep[] | undefined,
  mirrorBase: string | undefined,
  mirrorToken: string | undefined,
): string | null {
  if (steps && !steps.some((step) => step.kind === "mirrorUpload")) return null;
  if (!mirrorBase) {
    return (
      "No content mirror is configured, so the profile document cannot be published and the anchor "
      + "would name content nothing holds. Set VITE_CONTENT_MIRROR_BASE before publishing."
    );
  }
  if (!mirrorToken) {
    return (
      "No content mirror token is configured, so the mirror would refuse this publication. Set "
      + "VITE_CONTENT_MIRROR_TOKEN before publishing."
    );
  }
  return null;
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

/**
 * What would actually be sent, so "Publish" is never a button whose effect is unstated.
 *
 * The mirror uploads and the transactions are counted SEPARATELY, because they are different acts
 * with different costs and different reversibility: an upload can be repeated freely, while a
 * `setProfileAnchor` bumps a revision that invalidates any registrar address confirmation. Rolling
 * them into one "5 transactions" would overstate what reaches the chain.
 */
function describeSteps(steps: readonly DirectoryStep[]): string {
  const uploads = steps.filter((step) => step.kind === "mirrorUpload");
  const transactions = steps.filter((step) => step.kind !== "mirrorUpload");
  const parts = transactions.map((step) => {
    if (step.kind === "profileAnchor") {
      return step.channelsPublished === 0
        ? "your contact details (none - this records that you publish none)"
        : `your contact details (${step.channelsPublished} channel${step.channelsPublished === 1 ? "" : "s"})`;
    }
    return step.op === "publish"
      ? "a new location"
      : `a replacement for your published location (number ${step.locationNo})`;
  });
  const what = uploads.map((step) => (step.kind === "mirrorUpload" ? step.what : "")).join(" and your ");
  // Uploads with NO transaction is an ordinary, offerable publication - the repair path after the
  // mirror has lost content the chain still names - so it is described by what it DOES rather than
  // through the empty transaction sentence, which rendered as a dangling "0 transactions: ." and
  // read as nothing to do.
  if (parts.length === 0) {
    return `This would send no transaction. Your ${what} would be re-published to the content mirror, `
      + "which is what restores content the mirror no longer holds while the chain still names it.";
  }
  const sent = `This would send ${parts.length} transaction${parts.length === 1 ? "" : "s"}: ${parts.join(", then ")}.`;
  if (uploads.length === 0) return sent;
  return `${sent} Your ${what} would be published to the content mirror first, so the anchor never names content the mirror does not hold.`;
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
 * S-17 built the mirror that serves the document that digest names, so this no longer says the
 * service does not exist - that sentence became FALSE the moment the mirror shipped, and a surface
 * that kept it would be understating what a provider had just published.
 *
 * What it must still not do is overstate it. The anchor is a fingerprint, not a hosting guarantee:
 * a reader fetches the document from a mirror and CHECKS it against that fingerprint, so what the
 * chain guarantees is that altered content is detectable, never that the content is reachable.
 */
export const CONTACTS_ARE_ANCHORED_NOT_SERVED =
  "Your contact details are recorded on chain as a fingerprint, and the details themselves are published to the content mirror. Anyone reading them checks them against that fingerprint, so altered details are detectable rather than merely unlikely.";

/**
 * Taking a published location down, stated as a value the surface must render.
 *
 * Withdrawal is its own deliberate action and is never inferred from a blank coordinate field: a
 * provider clearing a typo would otherwise silently remove their pin. The provider stays listed and
 * contactable - withdrawing content is not leaving the directory.
 */
/**
 * The two-step dependency this flow sits behind, said BEFORE a provider fills the form in.
 *
 * The directory twin of {@link DOMAIN_REGISTER_NEEDS_TURNING_ON}, and permanent for the same
 * reason: `setResolverApproved` is `onlyOwner` and `setDirectoryResolver` reverts
 * `ResolverNotApproved` until it has run, so the first half is DogTag's however the chain reads
 * today. Stated here rather than left to the check, because a provider who has typed five contact
 * fields and a coordinate before being refused has already paid for the surprise.
 *
 * **THE SECOND HALF IS NOW THE PROVIDER'S OWN, AND THIS SENTENCE SAID OTHERWISE FOR ITS WHOLE LIFE.**
 * It read "Both are DogTag's - ask them", which is the harm this flow exists to remove pointed at the
 * provider: it sent them to ask for the one step nobody else could take. `setDirectoryResolver` is
 * gated by `canWriteProvider(providerId, caller, PROVIDER_PERMISSION_DIRECTORY_RESOLVER)`, which
 * admits the provider's controller and their delegates and NOT the registrar. The selection now has
 * a surface in this very flow, so the sentence names the step and where it is. Check a claim like
 * this against what ships before repeating it.
 */
export const DIRECTORY_NEEDS_TURNING_ON =
  "Publishing a listing needs the provider directory switched on for your provider record, in two steps. DogTag approves the directory fleet-wide - that half is theirs, and if this stops at the directory check what you have filled in is not the problem. Selecting it for your record is YOURS, right below, signed by your controller key: DogTag cannot make that choice for you.";

export const WITHDRAW_LOCATION_NOTICE =
  "Taking your location down leaves your listing and your contact details in place. You stop appearing in the nearby list, and you can publish a location again later. Clearing the latitude and longitude fields does NOT do this - it only means this publication carries no location.";

export { parseLocationInput, type LocationInput, type DirectoryPin };
