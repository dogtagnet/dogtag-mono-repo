// Shared types for the DogTag standard (mirror of dogtag-standard-rs).

/** Mandatory type tag so `"5"` (string) != `5` (integer). impl §1.1 / §3.2. */
export enum TypeTag {
  Null = 0,
  Bool = 1,
  String = 2,
  Integer = 3,
  Decimal = 4,
  Bytes = 5,
}

/** A single typed scalar entering the wrap boundary (typed input — A2; never a native float). */
export type TypedScalar =
  | {tag: TypeTag.Null; value: null}
  | {tag: TypeTag.Bool; value: boolean}
  | {tag: TypeTag.String; value: string}
  | {tag: TypeTag.Integer; value: string} // decimal-string big integer
  | {tag: TypeTag.Decimal; value: string} // fixed-point decimal string
  | {tag: TypeTag.Bytes; value: Uint8Array};

export interface IssuerMeta {
  name: string;
  domain: string;
  documentStore: string; // issuer clone address (0x..)
  recordType: string; // human label, e.g. "VACCINATION"
}

export interface WrappedDoc {
  version: "dogtag/1.0";
  data: unknown; // nested, salted, type-tagged scalars (self-describing)
  signature: {
    type: "DogTagMerkleProof";
    targetHash: string; // 0x.. merkle root of THIS doc's leaves
    proof: string[]; // sibling hashes to the batch root (empty for single-doc)
    merkleRoot: string; // anchored on-chain (== targetHash when proof empty)
  };
  privacy: {obfuscated: string[]}; // leaf hashes of redacted fields
  issuer: IssuerMeta;
}

/** 4-state fragment result (impl §11.3). */
export type FragmentState = "VALID" | "INVALID" | "ERROR" | "NOT_APPLICABLE";

export interface Verdict {
  valid: boolean;
  fragments: {
    integrity: FragmentState;
    issuance: FragmentState;
    identity: FragmentState;
    ownership: FragmentState;
  };
}
