/**
 * Turning two text inputs into a location, or into the absence of one.
 *
 * This exists because `Number("")` is `0`, not `NaN`. Every register form coerced its latitude and
 * longitude fields unconditionally, so an operator who left them blank registered a provider at
 * `0, 0` - a legal coordinate off the coast of Ghana - and it rendered as a confident pin in the
 * Gulf of Guinea. The blank field was the ordinary case for a provider with no premises.
 *
 * There are TWO register forms in the admin portal (the Businesses dialog and the setup Wizard), and
 * the whole point of this module is that they share one rule. Fixing one and not the other is how
 * this defect comes back on the path nobody looked at.
 *
 * The server enforces the same rules independently (`register_business` in
 * `stacks/admin/api/src/routes.rs`, 400 on a half-set or out-of-range pair). This is not that
 * boundary - it is here so the operator gets the message beside the fields.
 */

import { PROVIDER_CONTACT_CHANNELS, type ContactChannelRecord } from "./channels";

/** What a pair of location inputs means. */
export type LocationInput =
  | { kind: "absent" }
  | { kind: "located"; lat: number; lng: number }
  | { kind: "invalid"; reason: string };

const HALF_SET =
  "Latitude and longitude go together: fill in both, or leave both blank for a provider with no location.";
const NOT_NUMERIC = "Latitude and longitude must be numbers.";
const OUT_OF_RANGE = "Latitude must be within ±90 and longitude within ±180.";

/**
 * Read a register form's location fields.
 *
 * - both blank → `absent`. NOT `0, 0`, and not an error: a provider may have no location.
 * - both filled and usable → `located`.
 * - one filled → `invalid`. One coordinate is not a place, and silently dropping the filled one
 *   would discard something the operator deliberately typed.
 * - out of range or non-numeric → `invalid`, because the read side cannot repair it: the directory
 *   seam keeps its all-or-nothing rule for a malformed coordinate, so one bad row would take the
 *   whole provider list to `unavailable` for every consumer.
 */
export function parseLocationInput(latInput: string, lngInput: string): LocationInput {
  const lat = latInput.trim();
  const lng = lngInput.trim();

  if (!lat && !lng) return { kind: "absent" };
  if (!lat || !lng) return { kind: "invalid", reason: HALF_SET };

  const latNum = Number(lat);
  const lngNum = Number(lng);
  if (!Number.isFinite(latNum) || !Number.isFinite(lngNum)) {
    return { kind: "invalid", reason: NOT_NUMERIC };
  }
  if (latNum < -90 || latNum > 90 || lngNum < -180 || lngNum > 180) {
    return { kind: "invalid", reason: OUT_OF_RANGE };
  }
  return { kind: "located", lat: latNum, lng: lngNum };
}

/**
 * The `{lat, lng}` half of a `RegisterBusinessReq` body, spread at the call site.
 *
 * `{}` for an absent location - the keys are OMITTED rather than sent as `null`, so a provider with
 * no location never carries a coordinate-shaped value at all.
 */
export function locationRequestFields(
  location: Extract<LocationInput, { kind: "absent" | "located" }>,
): { lat: number; lng: number } | Record<string, never> {
  return location.kind === "located" ? { lat: location.lat, lng: location.lng } : {};
}

/** Every channel as an empty form field, so a channel added to the shared list appears in a form. */
export function blankContactFields(): ContactChannelRecord<string> {
  return Object.fromEntries(
    PROVIDER_CONTACT_CHANNELS.map((channel) => [channel, ""]),
  ) as ContactChannelRecord<string>;
}

/**
 * The `contact` half of a `RegisterBusinessReq` body, with the channels left blank omitted.
 *
 * Folded over the shared channel list for the same reason `parseLocationInput` exists: both register
 * forms restated the channel keys, so a channel added to one form and not the other is offered to
 * the operator, saved-looking, and never sent. An omitted channel is absent rather than blank.
 */
export function contactRequestFields(
  values: Readonly<ContactChannelRecord<string>>,
): Partial<ContactChannelRecord<string>> {
  return Object.fromEntries(
    PROVIDER_CONTACT_CHANNELS.flatMap((channel) => {
      const value = values[channel].trim();
      return value ? [[channel, value] as const] : [];
    }),
  );
}
