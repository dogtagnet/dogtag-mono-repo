// Unit coverage for the SDK's previously-untested pure logic — the TS mirror of the
// dogtag-standard-rs pure-logic suites (flatten/keyPath grammar, canonical encoding,
// field packing/hex codecs, packed-value codecs, Merkle proof round-trips). Behavior is
// pinned via invariants (round-trips, determinism, error paths) and a few well-known
// constants, so these tests are independent of the cross-language testvectors.json gate.
import {describe, it, expect} from "vitest";
import {
  flatten,
  tokenizeKeyPath,
  unflatten,
  nfc,
  canonicalInteger,
  canonicalDecimal,
  assertNotFloat,
  encodeValue,
  asString,
  bytesToHex,
  hexToBytes,
  beToBigInt,
  fieldFromScalarBytes,
  fieldFromUint,
  toHex32,
  fromHex32,
  bytesToField,
  DS_BYTES,
  FIELD_P,
  parsePacked,
  scalarFromPacked,
  leafFromPacked,
  flattenData,
  hashLeaf,
  hashNode,
  buildMerkle,
  merkleProof,
  processProof,
  TypeTag,
  type TypedScalar,
} from "../src/index.js";

describe("flatten — pinned keyPath grammar (impl §11.2 F2a)", () => {
  const nul: TypedScalar = {tag: TypeTag.Null, value: null};

  it("dotted object keys in insertion order", () => {
    const out = flatten({
      b: {tag: TypeTag.String, value: "x"},
      a: {tag: TypeTag.Integer, value: "1"},
    });
    expect(out.map((e) => e.keyPath)).toEqual(["b", "a"]); // JS preserves insertion order
  });

  it("nested objects produce a dotted path", () => {
    const out = flatten({x: {y: nul}});
    expect(out).toEqual([{keyPath: "x.y", scalar: nul}]);
  });

  it("array elements produce bracketed indices", () => {
    const out = flatten({
      arr: [
        {tag: TypeTag.Integer, value: "1"},
        {tag: TypeTag.Integer, value: "2"},
      ],
    });
    expect(out.map((e) => e.keyPath)).toEqual(["arr[0]", "arr[1]"]);
  });

  it("a root typed scalar has an empty keyPath", () => {
    expect(flatten({tag: TypeTag.String, value: "r"})).toEqual([
      {keyPath: "", scalar: {tag: TypeTag.String, value: "r"}},
    ]);
  });

  it("an empty object becomes a null-typed leaf at that path", () => {
    expect(flatten({a: {}})).toEqual([{keyPath: "a", scalar: nul}]);
  });

  it("an empty array becomes a null-typed leaf at that path", () => {
    expect(flatten({a: []})).toEqual([{keyPath: "a", scalar: nul}]);
  });

  it("a reserved char in an object key is rejected", () => {
    expect(() => flatten({"a.b": nul})).toThrow(/reserved char/);
    expect(() => flatten({"a[0]": nul})).toThrow(/reserved char/);
  });

  it("an untyped (raw) leaf is rejected — scalars must be {tag,value}", () => {
    expect(() => flatten({a: "rawstring"})).toThrow(/non-typed leaf/);
  });
});

describe("tokenizeKeyPath", () => {
  it("tokenizes a mixed key/index path", () => {
    expect(tokenizeKeyPath("a.b[0].c")).toEqual([
      {kind: "key", key: "a"},
      {kind: "key", key: "b"},
      {kind: "index", idx: 0},
      {kind: "key", key: "c"},
    ]);
  });

  it("rejects an unterminated array index", () => {
    expect(() => tokenizeKeyPath("a[")).toThrow(/unterminated array index/);
  });

  it("rejects a leading-zero array index", () => {
    expect(() => tokenizeKeyPath("a[01]")).toThrow(/bad array index/);
  });

  it("rejects a non-numeric array index", () => {
    expect(() => tokenizeKeyPath("a[x]")).toThrow(/bad array index/);
  });
});

describe("unflatten", () => {
  it("rebuilds nested objects and arrays", () => {
    expect(unflatten({"a.b": "p1", "arr[0]": "p2", "arr[1]": "p3"})).toEqual({
      a: {b: "p1"},
      arr: ["p2", "p3"],
    });
  });

  it("leaves sparse-array holes for missing indices", () => {
    const out = unflatten({"arr[2]": "p"}) as {arr: unknown[]};
    expect(out.arr.length).toBe(3);
    expect(out.arr[0]).toBeUndefined();
    expect(out.arr[2]).toBe("p");
  });

  it("propagates a bad keyPath error", () => {
    expect(() => unflatten({"a[": "p"})).toThrow(/unterminated array index/);
  });
});

describe("encode — NFC + canonical integer/decimal (impl §1.1, A1/A2/A3)", () => {
  it("nfc composes a decomposed grapheme", () => {
    expect(nfc("é")).toBe("é"); // e + combining acute -> é
    expect(nfc("é").length).toBe(1);
  });

  it("nfc rejects unpaired surrogates", () => {
    expect(() => nfc("\ud800")).toThrow(/unpaired high surrogate/);
    expect(() => nfc("\udc00")).toThrow(/unpaired low surrogate/);
    expect(nfc("😀")).toBe("😀"); // a valid pair survives
  });

  it("canonicalInteger maps -0 to 0 and rejects leading zeros", () => {
    expect(canonicalInteger("-0")).toBe("0");
    expect(canonicalInteger("42")).toBe("42");
    expect(canonicalInteger("-42")).toBe("-42");
    expect(() => canonicalInteger("007")).toThrow(/invalid integer/);
    expect(() => canonicalInteger("1.0")).toThrow(/invalid integer/);
  });

  it("canonicalDecimal strips fractional trailing zeros and normalizes signed zero", () => {
    expect(canonicalDecimal("22.70")).toBe("22.7");
    expect(canonicalDecimal("0.50")).toBe("0.5");
    expect(canonicalDecimal("-0")).toBe("0");
    expect(canonicalDecimal("-0.000")).toBe("0");
    expect(canonicalDecimal("5.000")).toBe("5");
  });

  it("canonicalDecimal rejects exponent / sign / whitespace forms", () => {
    expect(() => canonicalDecimal("1e5")).toThrow(/invalid decimal/);
    expect(() => canonicalDecimal("+1")).toThrow(/invalid decimal/);
    expect(() => canonicalDecimal(" 1")).toThrow(/invalid decimal/);
  });

  it("assertNotFloat forbids every native number, integral or not", () => {
    expect(() => assertNotFloat(1.5)).toThrow(/floats forbidden/);
    expect(() => assertNotFloat(5)).toThrow(/native numbers forbidden/);
    expect(() => assertNotFloat("5")).not.toThrow();
    expect(() => assertNotFloat(true)).not.toThrow();
  });

  it("encodeValue produces canonical bytes per variant", () => {
    expect(encodeValue({tag: TypeTag.Null, value: null})).toEqual(new Uint8Array(0));
    expect(encodeValue({tag: TypeTag.Bool, value: true})).toEqual(new Uint8Array([1]));
    expect(encodeValue({tag: TypeTag.Bool, value: false})).toEqual(new Uint8Array([0]));
    expect(encodeValue({tag: TypeTag.String, value: "Rex"})).toEqual(
      new TextEncoder().encode("Rex"),
    );
    expect(encodeValue({tag: TypeTag.Integer, value: "-0"})).toEqual(
      new TextEncoder().encode("0"),
    );
    expect(encodeValue({tag: TypeTag.Decimal, value: "22.70"})).toEqual(
      new TextEncoder().encode("22.7"),
    );
    expect(encodeValue({tag: TypeTag.Bytes, value: new Uint8Array([1, 2, 3])})).toEqual(
      new Uint8Array([1, 2, 3]),
    );
  });

  it("asString renders the self-describing string form per variant", () => {
    expect(asString({tag: TypeTag.Null, value: null})).toBe("");
    expect(asString({tag: TypeTag.Bool, value: true})).toBe("true");
    expect(asString({tag: TypeTag.Bool, value: false})).toBe("false");
    expect(asString({tag: TypeTag.Decimal, value: "0.50"})).toBe("0.5");
    expect(asString({tag: TypeTag.Bytes, value: new Uint8Array([0xab, 0xcd])})).toBe("abcd");
  });

  it("bytesToHex / hexToBytes round-trip with and without 0x", () => {
    const b = new Uint8Array([0x00, 0x0f, 0xff]);
    expect(bytesToHex(b)).toBe("000fff");
    expect(hexToBytes("000fff")).toEqual(b);
    expect(hexToBytes("0x000fff")).toEqual(b);
    expect(() => hexToBytes("abc")).toThrow(/odd hex length/);
  });
});

describe("field — packing + hex codecs (impl §3.3/§3.4, §11.2(a))", () => {
  it("beToBigInt big-endian decodes", () => {
    expect(beToBigInt(new Uint8Array([0x01, 0x00]))).toBe(256n);
    expect(beToBigInt(new Uint8Array([0xff]))).toBe(255n);
    expect(beToBigInt(new Uint8Array(0))).toBe(0n);
  });

  it("fieldFromScalarBytes accepts <=31 bytes and rejects more", () => {
    expect(fieldFromScalarBytes(new Uint8Array([0x01, 0x02]))).toBe(258n);
    expect(() => fieldFromScalarBytes(new Uint8Array(32))).toThrow(/<= 31/);
  });

  it("fieldFromUint reduces mod P and rejects negatives", () => {
    expect(fieldFromUint(3)).toBe(3n);
    expect(fieldFromUint(FIELD_P + 5n)).toBe(5n);
    expect(() => fieldFromUint(-1n)).toThrow(/negative/);
  });

  it("toHex32 zero-pads to 64 hex and range-checks", () => {
    expect(toHex32(0n)).toBe("0x" + "0".repeat(64));
    expect(toHex32(255n)).toBe("0x" + "0".repeat(62) + "ff");
    expect(() => toHex32(FIELD_P)).toThrow(/out of range/);
    expect(() => toHex32(-1n)).toThrow(/out of range/);
  });

  it("fromHex32 parses with/without 0x and rejects >= field", () => {
    expect(fromHex32("0x" + "0".repeat(62) + "ff")).toBe(255n);
    expect(fromHex32("00ff")).toBe(255n);
    expect(() => fromHex32("0x" + FIELD_P.toString(16))).toThrow(/exceeds field/);
  });

  it("toHex32 / fromHex32 round-trip a mid-range element", () => {
    const x = 123456789012345678901234567890n;
    expect(fromHex32(toHex32(x))).toBe(x);
  });

  it("bytesToField is deterministic and never returns the raw DS tag", () => {
    expect(bytesToField(new Uint8Array(0))).toBe(bytesToField(new Uint8Array(0)));
    // the 8-byte length prefix forces at least one fold, so the empty input != DS_BYTES
    expect(bytesToField(new Uint8Array(0))).not.toBe(DS_BYTES);
    const a = bytesToField(new TextEncoder().encode("a"));
    const b = bytesToField(new TextEncoder().encode("b"));
    expect(a).not.toBe(b);
  });
});

describe("wrap — packed-value codecs (impl §11.2 F2b)", () => {
  it("parsePacked splits on the first two colons; the value may contain ':'", () => {
    expect(parsePacked("abcd:2:hello:world")).toEqual({
      saltHex: "abcd",
      tag: TypeTag.String,
      valueRest: "hello:world",
    });
  });

  it("parsePacked rejects a value missing a colon", () => {
    expect(() => parsePacked("noColon")).toThrow(/bad packed value/);
    expect(() => parsePacked("only:one")).toThrow(/bad packed value/);
  });

  it("scalarFromPacked reconstructs each variant; Bool is exact-'true'", () => {
    expect(scalarFromPacked(TypeTag.Null, "")).toEqual({tag: TypeTag.Null, value: null});
    expect(scalarFromPacked(TypeTag.Bool, "true")).toEqual({tag: TypeTag.Bool, value: true});
    expect(scalarFromPacked(TypeTag.Bool, "false")).toEqual({tag: TypeTag.Bool, value: false});
    expect(scalarFromPacked(TypeTag.Bool, "TRUE")).toEqual({tag: TypeTag.Bool, value: false});
    expect(scalarFromPacked(TypeTag.Integer, "42")).toEqual({tag: TypeTag.Integer, value: "42"});
    expect(scalarFromPacked(TypeTag.Bytes, "00ff")).toEqual({
      tag: TypeTag.Bytes,
      value: new Uint8Array([0x00, 0xff]),
    });
  });

  it("scalarFromPacked rejects an unknown tag", () => {
    expect(() => scalarFromPacked(99 as TypeTag, "x")).toThrow(/unknown tag/);
  });

  it("leafFromPacked recomputes the same hash as hashLeaf", () => {
    const salt = new Uint8Array(16).fill(7);
    const scalar: TypedScalar = {tag: TypeTag.String, value: "Rex"};
    const packed = `${bytesToHex(salt)}:${TypeTag.String}:Rex`;
    expect(leafFromPacked("name", packed)).toBe(hashLeaf("name", salt, scalar));
  });

  it("flattenData walks objects/arrays, skips non-strings, and handles a root string", () => {
    expect(flattenData({a: "p1", b: {c: "p2"}, arr: ["p3", "p4"]})).toEqual([
      ["a", "p1"],
      ["b.c", "p2"],
      ["arr[0]", "p3"],
      ["arr[1]", "p4"],
    ]);
    expect(flattenData({a: "p1", n: 42})).toEqual([["a", "p1"]]); // non-string skipped
    expect(flattenData("justastring")).toEqual([["", "justastring"]]);
  });
});

describe("merkle — proof round-trips (impl §1.3, §11.2)", () => {
  it("hashNode is commutative", () => {
    expect(hashNode(3n, 7n)).toBe(hashNode(7n, 3n));
  });

  it("a single leaf is its own root", () => {
    expect(buildMerkle([42n]).root).toBe(42n);
  });

  it("build -> prove -> process recovers the root for sizes 1..8 (incl. odd promotion)", () => {
    for (let n = 1; n <= 8; n++) {
      const leaves = Array.from({length: n}, (_, i) => BigInt(i + 1) * 1000n + 1n);
      const tree = buildMerkle(leaves);
      for (const leaf of leaves) {
        const proof = merkleProof(tree.layers, leaf);
        expect(processProof(proof, leaf)).toBe(tree.root);
      }
    }
  });
});
