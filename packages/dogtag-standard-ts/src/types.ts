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

/**
 * M7 record-provenance block (§4.2), mirror of the Rust `ProtocolMeta`: which protocol/contract a
 * record was created on AND who issued it, carried BESIDE `signature.merkleRoot` - NEVER inside `R`
 * or the ZK proof. A routing hint only, never authority: `issuerSigner` is the envelope's *claim*,
 * validated against the on-chain `clone.issuedBy[R]` at verify time. Absent on pre-M7 records
 * (default it via `resolvedProtocol`, §4.4).
 */
export interface ProtocolMeta {
  chainId: number;
  version: string; // protocol level, e.g. "dogtag-levela/1" - NOT the envelope `version`
  verificationRegistry: string; // THE routing key
  issuerClone: string; // == issuer.documentStore; the direct isValid target
  issuerSigner: string; // the signer that issued (claim == clone.issuedBy[R]); validated, never trusted
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
  // M7 provenance block (§4.2), beside `signature.merkleRoot` - NOT inside `R`. Absent on pre-M7
  // records; default it via `resolvedProtocol`. A routing hint only, never authority.
  protocol?: ProtocolMeta;
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
