import {describe, it, expect} from "vitest";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {
  TypeTag,
  buildMerkle,
  canonicalDecimal,
  hashLeaf,
  merkleProof,
  obfuscate,
  processProof,
  verifyInclusion,
  toHex32,
  fromHex32,
  wrapDocument,
  bytesToField,
  type ProofStep,
  type TypedScalar,
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

describe("Inclusion proofs — DSDP §2.3 Sibling|Promote (testvectors.json)", () => {
  function stepsOf(raw: ({sibling: string} | {promote: true})[]): ProofStep[] {
    return raw.map((s) => ("promote" in s ? {promote: true} : {sibling: fromHex32(s.sibling)}));
  }

  it("has the shared inclusion vector set", () => {
    expect(Array.isArray(VECS.inclusion)).toBe(true);
    expect(VECS.inclusion.length).toBeGreaterThanOrEqual(100);
    // multi-level promotion must be present (sorted-last leaf of an odd tree promotes repeatedly)
    expect(VECS.inclusion.some((v: {valid: boolean; promotes: number}) => v.valid && v.promotes >= 2)).toBe(true);
    // negative (reject) vectors must be present
    expect(VECS.inclusion.some((v: {valid: boolean}) => !v.valid)).toBe(true);
  });

  for (const v of VECS.inclusion) {
    it(`inclusion ${v.name} -> ${v.valid ? "accept" : "reject"}`, () => {
      const scalar = scalarOf(v.tag, v.value) as TypedScalar;
      const got = verifyInclusion(v.keyPath, hexToBytes(v.saltHex), scalar, stepsOf(v.steps), fromHex32(v.root));
      expect(got).toBe(v.valid);
    });
  }

  it("processProof is a fold primitive, NOT a membership check (arity/domain split blocks it)", () => {
    // 4-leaf tree: sorted layer0 [a,b,c,d] -> [ab,cd] -> root.
    const recs: {keyPath: string; salt: Uint8Array; scalar: TypedScalar}[] = Array.from({length: 4}, (_, i) => ({
      keyPath: `leaf${i}`,
      salt: new Uint8Array(16).fill((100 + i) & 0xff),
      scalar: {tag: TypeTag.Integer, value: String(i)},
    }));
    const hashes = recs.map((r) => hashLeaf(r.keyPath, r.salt, r.scalar));
    const {root, layers} = buildMerkle(hashes);
    const node = layers[1]![0]!; // internal node (Poseidon3 image under DS_NODE)
    const sibling = layers[1]![1]!;
    // the permissive fold folds the internal node straight to the root — the opaque-leaf hole
    expect(processProof([{sibling}], node)).toBe(root);
    // but recompute-from-fields can never make a leaf equal an internal node (arity split)
    const atk: TypedScalar = {tag: TypeTag.Integer, value: "99"};
    expect(hashLeaf("leaf99", new Uint8Array(16).fill(0xaa), atk)).not.toBe(node);
    expect(verifyInclusion("leaf99", new Uint8Array(16).fill(0xaa), atk, [{sibling}], root)).toBe(false);
  });

  it("generate∘verify round-trips for every leaf across the audit leaf counts", () => {
    for (const k of [1, 2, 3, 5, 6, 7, 13, 24, 34]) {
      const recs = Array.from({length: k}, (_, i) => ({
        keyPath: `leaf${i}`,
        salt: new Uint8Array(16).fill((100 + i) & 0xff),
        scalar: {tag: TypeTag.Integer, value: String(i)} as TypedScalar,
      }));
      const hashes = recs.map((r) => hashLeaf(r.keyPath, r.salt, r.scalar));
      const {root, layers} = buildMerkle(hashes);
      recs.forEach((r, i) => {
        const proof = merkleProof(layers, hashes[i]!);
        expect(proof.length).toBe(layers.length - 1); // one step per level
        expect(verifyInclusion(r.keyPath, r.salt, r.scalar, proof, root)).toBe(true);
      });
    }
  });
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

  it("a non-empty signature.proof is now INVALID (batching unsupported; C1 tightening)", () => {
    seq = 0;
    const doc = wrapDocument(credential, issuer, fixedSalt);
    // Doc→batch-root inclusion never shipped; a non-empty proof is rejected outright rather than
    // folded through the permissive commutative primitive, even if it names the root itself.
    const tampered = {...doc, signature: {...doc.signature, proof: [doc.signature.targetHash]}};
    expect(checkIntegrity(tampered).state).toBe("INVALID");
  });
});
