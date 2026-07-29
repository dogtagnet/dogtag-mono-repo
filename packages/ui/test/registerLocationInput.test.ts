/**
 * `Number("")` is `0`, and that is the whole defect.
 *
 * Both admin register paths coerced their latitude and longitude fields unconditionally, so an
 * operator who left them blank - the ordinary case for a provider with no premises - registered that
 * provider at `0, 0`. That is a legal coordinate off the coast of Ghana, so nothing rejected it and
 * every surface drew a confident pin in the Gulf of Guinea.
 */

import { describe, expect, it } from "vitest";
import { PROVIDER_CONTACT_CHANNELS } from "../src/directory/channels";
import {
  blankContactFields,
  contactRequestFields,
  locationRequestFields,
  parseLocationInput,
} from "../src/directory/registration";

describe("parseLocationInput", () => {
  it("reads blank fields as ABSENT, never as 0,0", () => {
    // The regression, stated directly: `Number("")` is 0, not NaN, so the old coercion produced a
    // valid-looking coordinate out of an empty form.
    expect(Number("")).toBe(0);

    for (const [lat, lng] of [
      ["", ""],
      ["   ", ""],
      ["", "\t"],
    ]) {
      expect(parseLocationInput(lat, lng)).toEqual({ kind: "absent" });
    }
  });

  it("omits the coordinate keys entirely for an absent location", () => {
    // Not `lat: null`, not `lat: 0`. A provider with no location carries no coordinate-shaped value.
    expect(locationRequestFields({ kind: "absent" })).toEqual({});
    expect("lat" in locationRequestFields({ kind: "absent" })).toBe(false);
  });

  it("reads a filled pair as a location, trimming and preserving a real 0,0", () => {
    expect(parseLocationInput("1.3521", "103.8198")).toEqual({
      kind: "located",
      lat: 1.3521,
      lng: 103.8198,
    });
    expect(parseLocationInput(" -33.8688 ", " 151.2093 ")).toEqual({
      kind: "located",
      lat: -33.8688,
      lng: 151.2093,
    });
    // A provider genuinely at the origin can still say so. This is exactly why an operator, not
    // code, has to answer for the rows already stored there.
    expect(parseLocationInput("0", "0")).toEqual({ kind: "located", lat: 0, lng: 0 });
    expect(locationRequestFields({ kind: "located", lat: 0, lng: 0 })).toEqual({ lat: 0, lng: 0 });
  });

  it("refuses a half-set pair rather than silently dropping the coordinate that was typed", () => {
    expect(parseLocationInput("1.3521", "")).toMatchObject({ kind: "invalid" });
    expect(parseLocationInput("", "103.8198")).toMatchObject({ kind: "invalid" });
    expect(parseLocationInput("1.3521", "  ")).toMatchObject({ kind: "invalid" });
  });

  it("refuses non-numeric and out-of-range input", () => {
    // Out of range is refused at the WRITE because the read side cannot repair it: the directory
    // seam keeps its all-or-nothing rule for a malformed coordinate, so one bad row would take the
    // whole provider list to `unavailable` for every consumer.
    for (const [lat, lng] of [
      ["north", "103.8198"],
      ["1.3521", "east"],
      ["91", "0"],
      ["-91", "0"],
      ["0", "181"],
      ["0", "-181"],
      ["Infinity", "0"],
    ]) {
      expect(parseLocationInput(lat, lng), `${lat},${lng}`).toMatchObject({ kind: "invalid" });
    }
    // The exact boundaries stay valid.
    expect(parseLocationInput("90", "180")).toMatchObject({ kind: "located" });
    expect(parseLocationInput("-90", "-180")).toMatchObject({ kind: "located" });
  });

  it("gives an invalid pair a reason an operator can act on", () => {
    const half = parseLocationInput("1.3521", "");
    expect(half.kind).toBe("invalid");
    if (half.kind !== "invalid") return;
    expect(half.reason).toMatch(/leave both blank/i);
  });
});

/**
 * The same "two forms, one rule" property applied to the contact channels.
 *
 * The channel keys were restated in six places, and `website` reached the server and the TS seam
 * while both native mirrors still read four - so a provider reachable only by website was reported
 * as having published nothing. These folds are what stop a seventh site from restating them again.
 */
describe("contact channel folds", () => {
  it("offers every channel in the shared list, in listing order", () => {
    expect(Object.keys(blankContactFields())).toEqual([...PROVIDER_CONTACT_CHANNELS]);
    expect(Object.values(blankContactFields()).every((v) => v === "")).toBe(true);
  });

  it("sends the published channels and OMITS the blank ones rather than sending empty strings", () => {
    const sent = contactRequestFields({
      ...blankContactFields(),
      phone: "  +65 6123 4567  ",
      website: "https://shop.example",
    });

    expect(sent).toEqual({ phone: "+65 6123 4567", website: "https://shop.example" });
    // Absent, not blank: a channel the provider did not publish carries no key at all.
    for (const channel of ["whatsapp", "telegram", "email"] as const) {
      expect(channel in sent).toBe(false);
    }
  });

  it("can send a provider reachable by website alone", () => {
    const sent = contactRequestFields({
      ...blankContactFields(),
      website: "https://web-only.test",
    });
    expect(sent).toEqual({ website: "https://web-only.test" });
  });

  it("treats a whitespace-only channel as unpublished", () => {
    expect(contactRequestFields({ ...blankContactFields(), telegram: "   " })).toEqual({});
  });
});
