import {describe, it, expect} from "vitest";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {
  TypeTag,
  buildMerkle,
  canonicalDecimal,
  hashLeaf,
  obfuscate,
  toHex32,
  fromHex32,
  wrapDocument,
  bytesToField,
  type IssuerMeta,
} from "../src/index.js";
import {hexToBytes} from "../src/encode.js";
import {checkIntegrity} from "../src/verify.js";

const VECS = JSON.parse(readFileSync(resolve(__dirname, "..", "testvectors.json"), "utf8"));

function scalarOf(tag: number, value: string | null) {
  switch (tag) {
    case TypeTag.Null:
      return {tag, value: null};
    case TypeTag.Bool:
      return {tag, value: value === "true"};
    case TypeTag.Bytes:
      return {tag, value: hexToBytes(value!)};
    default:
      return {tag, value: value!};
  }
}

const issuer: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};

describe("Poseidon leaf vectors (testvectors.json)", () => {
  for (const v of VECS.leaves) {
    it(`leaf ${v.name}`, () => {
      const got = toHex32(hashLeaf(v.keyPath, hexToBytes(v.saltHex), scalarOf(v.tag, v.value) as never));
      expect(got).toBe(v.expected_hex);
    });
  }

  it('tag 2 "5" != tag 3 5 (typeTag is load-bearing)', () => {
    const s2 = VECS.leaves.find((l: {name: string}) => l.name === "string_five");
    const s3 = VECS.leaves.find((l: {name: string}) => l.name === "integer_five");
    expect(s2.expected_hex).not.toBe(s3.expected_hex);
  });

  it("decimal 22.70 canonicalizes to 22.7", () => {
    expect(canonicalDecimal("22.70")).toBe("22.7");
    expect(canonicalDecimal("-0")).toBe("0");
    expect(canonicalDecimal("0.50")).toBe("0.5");
  });
});

describe("bytesToField edge vectors", () => {
  for (const v of VECS.bytesToField) {
    it(`btf ${v.name}`, () => {
      expect(toHex32(bytesToField(hexToBytes(v.inputHex)))).toBe(v.expected_hex);
    });
  }
});

describe("Merkle vectors", () => {
  for (const v of VECS.merkle) {
    it(`merkle ${v.name}`, () => {
      const leaves = v.leaf_hexes.map(fromHex32);
      expect(toHex32(buildMerkle(leaves).root)).toBe(v.root_hex);
      if (v.reversed_root_hex) {
        expect(toHex32(buildMerkle([...leaves].reverse()).root)).toBe(v.reversed_root_hex);
        expect(v.reversed_root_hex).toBe(v.root_hex); // commutativity
      }
    });
  }
});

describe("wrap + obfuscation + tamper", () => {
  const credential = {
    credentialSubject: {
      dogTagId: {tag: TypeTag.Integer, value: "42"},
      name: {tag: TypeTag.String, value: "Rex"},
      microchip: {code: {tag: TypeTag.String, value: "985141006580311"}},
    },
    vaccinationDate: {tag: TypeTag.String, value: "2026-06-17T14:46:29Z"},
    weightHistory: [{value: {tag: TypeTag.Decimal, value: "22.7"}}],
  };
  let seq = 0;
  const fixedSalt = () => new Uint8Array(16).fill(++seq);

  it("wrap produces a single root R; integrity VALID", () => {
    seq = 0;
    const doc = wrapDocument(credential, issuer, fixedSalt);
    expect(doc.signature.merkleRoot).toBe(doc.signature.targetHash);
    expect(checkIntegrity(doc).state).toBe("VALID");
  });

  it("obfuscating a field preserves the root", () => {
    seq = 0;
    const doc = wrapDocument(credential, issuer, fixedSalt);
    const before = doc.signature.targetHash;
    const obf = obfuscate(doc, ["credentialSubject.name"]);
    expect(obf.signature.targetHash).toBe(before);
    expect(obf.privacy.obfuscated.length).toBe(1);
    expect(checkIntegrity(obf).state).toBe("VALID");
    // the cleartext is gone
    expect(JSON.stringify(obf.data)).not.toContain("Rex");
  });

  it("tampering a value breaks integrity (cannot swap a value and keep the root)", () => {
    seq = 0;
    const doc = wrapDocument(credential, issuer, fixedSalt);
    // mutate the packed name value while keeping salt/tag
    const data = JSON.parse(JSON.stringify(doc.data));
    const packed: string = data.credentialSubject.name;
    const [salt, tag] = packed.split(":");
    data.credentialSubject.name = `${salt}:${tag}:Fido`;
    const tampered = {...doc, data};
    expect(checkIntegrity(tampered).state).toBe("INVALID");
  });

  it("dropping the non-obfuscatable dogTagId fails integrity", () => {
    seq = 0;
    const doc = wrapDocument(credential, issuer, fixedSalt);
    const data = JSON.parse(JSON.stringify(doc.data));
    delete data.credentialSubject.dogTagId;
    expect(checkIntegrity({...doc, data}).state).toBe("INVALID");
  });
});
