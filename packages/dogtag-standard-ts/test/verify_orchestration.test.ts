// Unit coverage for the three-pillar verify() orchestration (verify.ts), the TS mirror of the
// Rust dogtag-standard-rs verify.rs tests. The pillars are exercised with deterministic mock
// adapters: RpcAdapter/DnsAdapter/RegistryAdapter whose throws model the transient-ERROR arm.
import {describe, it, expect} from "vitest";
import {TypeTag, wrapDocument, type IssuerMeta} from "../src/index.js";
import {
  verify,
  type RpcAdapter,
  type DnsAdapter,
  type RegistryAdapter,
  type VerifyOpts,
} from "../src/verify.js";
import type {WrappedDoc} from "../src/types.js";

const issuer: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};

const OWNER = "0xabc0000000000000000000000000000000000abc";

/** A wrapped doc with dogTagId == "42"; deterministic salts so the root is stable across runs. */
function validDoc(): WrappedDoc {
  let seq = 0;
  const fixedSalt = () => new Uint8Array(16).fill(++seq);
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: {tag: TypeTag.Integer, value: "42"},
        name: {tag: TypeTag.String, value: "Rex"},
      },
    },
    issuer,
    fixedSalt,
  );
}

/** Tamper a packed value while preserving salt/tag so integrity recomputes to INVALID. */
function tamperIntegrity(doc: WrappedDoc): WrappedDoc {
  const data = JSON.parse(JSON.stringify(doc.data));
  const packed: string = data.credentialSubject.name;
  const [salt, tag] = packed.split(":");
  data.credentialSubject.name = `${salt}:${tag}:Fido`;
  return {...doc, data};
}

interface MockCfg {
  isValid?: boolean | "throw";
  owner?: string | "throw";
  txt?: boolean | "throw";
  knows?: boolean;
  /** on-chain `issuedBy[R]` the RpcAdapter reports; omit to leave the adapter method UNWIRED. */
  issuedBy?: string | "throw";
}

function opts(mode: "self-import" | "third-party", cfg: MockCfg, userWalletAddress?: string): VerifyOpts {
  const rpc: RpcAdapter = {
    async isValid() {
      if (cfg.isValid === "throw") throw new Error("rpc down");
      return cfg.isValid ?? true;
    },
    async ownerOf() {
      if (cfg.owner === "throw") throw new Error("rpc down");
      return cfg.owner ?? OWNER;
    },
    // Only present when the test wires it - mirrors the optional/unwired Rust default.
    ...(cfg.issuedBy !== undefined
      ? {
          async issuedBy() {
            if (cfg.issuedBy === "throw") throw new Error("rpc down");
            return cfg.issuedBy as string;
          },
        }
      : {}),
  };
  const dns: DnsAdapter = {
    async txtMatches() {
      if (cfg.txt === "throw") throw new Error("dns down");
      return cfg.txt ?? true;
    },
  };
  const registry: RegistryAdapter = {
    async knows() {
      return cfg.knows ?? true;
    },
  };
  return {rpc, dns, registry, mode, userWalletAddress};
}

describe("verify() three-pillar orchestration", () => {
  it("self-import, all pillars valid + owner matches -> valid, all fragments VALID", async () => {
    const v = await verify(validDoc(), opts("self-import", {}, OWNER));
    expect(v.valid).toBe(true);
    expect(v.fragments).toEqual({
      integrity: "VALID",
      issuance: "VALID",
      identity: "VALID",
      ownership: "VALID",
    });
  });

  it("self-import, owner mismatch -> ownership INVALID gates validity", async () => {
    const v = await verify(validDoc(), opts("self-import", {owner: "0xdead000000000000000000000000000000000000"}, OWNER));
    expect(v.valid).toBe(false);
    expect(v.fragments.ownership).toBe("INVALID");
  });

  it("self-import, ownership compare is case-insensitive", async () => {
    const v = await verify(validDoc(), opts("self-import", {owner: OWNER.toUpperCase()}, OWNER.toLowerCase()));
    expect(v.fragments.ownership).toBe("VALID");
    expect(v.valid).toBe(true);
  });

  it("self-import, ownerOf throws -> ownership ERROR, not valid", async () => {
    const v = await verify(validDoc(), opts("self-import", {owner: "throw"}, OWNER));
    expect(v.fragments.ownership).toBe("ERROR");
    expect(v.valid).toBe(false);
  });

  it("self-import without userWalletAddress throws (contract precondition)", async () => {
    await expect(verify(validDoc(), opts("self-import", {}))).rejects.toThrow(/userWalletAddress/);
  });

  it("third-party, no wallet -> ownership NOT_APPLICABLE, validity from the 3 pillars", async () => {
    const v = await verify(validDoc(), opts("third-party", {}));
    expect(v.fragments.ownership).toBe("NOT_APPLICABLE");
    expect(v.valid).toBe(true);
  });

  it("third-party, owner mismatch is reported but does NOT gate validity", async () => {
    const v = await verify(
      validDoc(),
      opts("third-party", {owner: "0xdead000000000000000000000000000000000000"}, OWNER),
    );
    expect(v.fragments.ownership).toBe("INVALID");
    expect(v.valid).toBe(true); // ownership never gates third-party
  });

  it("issuance false -> issuance INVALID, not valid", async () => {
    const v = await verify(validDoc(), opts("third-party", {isValid: false}));
    expect(v.fragments.issuance).toBe("INVALID");
    expect(v.valid).toBe(false);
  });

  it("issuance RPC throws -> issuance ERROR, not valid", async () => {
    const v = await verify(validDoc(), opts("third-party", {isValid: "throw"}));
    expect(v.fragments.issuance).toBe("ERROR");
    expect(v.valid).toBe(false);
  });

  it("identity needs BOTH dns TXT and registry knowledge", async () => {
    const noTxt = await verify(validDoc(), opts("third-party", {txt: false}));
    expect(noTxt.fragments.identity).toBe("INVALID");
    const noKnow = await verify(validDoc(), opts("third-party", {knows: false}));
    expect(noKnow.fragments.identity).toBe("INVALID");
  });

  it("identity DNS throws -> identity ERROR, not valid", async () => {
    const v = await verify(validDoc(), opts("third-party", {txt: "throw"}));
    expect(v.fragments.identity).toBe("ERROR");
    expect(v.valid).toBe(false);
  });

  it("tampered integrity gates validity in both modes", async () => {
    const tampered = tamperIntegrity(validDoc());
    const self = await verify(tampered, opts("self-import", {}, OWNER));
    expect(self.fragments.integrity).toBe("INVALID");
    expect(self.valid).toBe(false);
    const third = await verify(tampered, opts("third-party", {}));
    expect(third.fragments.integrity).toBe("INVALID");
    expect(third.valid).toBe(false);
  });
});

// M7 provenance: the `protocol` block is a routing hint, NEVER authority (§4.2/§4.3). Mirror of the
// Rust `verify::tests::provenance_*` cases. `issuerSigner` is only the envelope's CLAIM of who issued;
// verify() validates it against the on-chain `clone.issuedBy[R]` (the injected `issuedBy`).
const SIGNER = "0x00000000000000000000000000000000000515e6";

function withProtocol(doc: WrappedDoc, issuerSigner: string): WrappedDoc {
  return {
    ...doc,
    protocol: {
      chainId: 135,
      version: "dogtag-levelb/1",
      verificationRegistry: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87",
      issuerClone: doc.issuer.documentStore,
      issuerSigner,
    },
  };
}

describe("verify() M7 provenance issuer-signer check", () => {
  it("matching issuerSigner claim verifies", async () => {
    const doc = withProtocol(validDoc(), SIGNER);
    const v = await verify(doc, opts("third-party", {issuedBy: SIGNER}));
    expect(v.fragments.issuance).toBe("VALID");
    expect(v.valid).toBe(true);
  });

  it("wrong/forged issuerSigner does NOT verify (routing hint, not authority)", async () => {
    const doc = withProtocol(validDoc(), "0x00000000000000000000000000000000deadbeef");
    const v = await verify(doc, opts("third-party", {issuedBy: SIGNER})); // on-chain != claim
    expect(v.fragments.issuance).toBe("INVALID");
    expect(v.valid).toBe(false);
  });

  it("a matching block cannot rescue an on-chain-invalid record", async () => {
    // The literal acceptance property: a present block must NOT make an INVALID record verify, even
    // when its issuerSigner matches on-chain - the additive check only ever tightens.
    const doc = withProtocol(validDoc(), SIGNER);
    const v = await verify(doc, opts("third-party", {isValid: false, issuedBy: SIGNER}));
    expect(v.fragments.issuance).toBe("INVALID");
    expect(v.valid).toBe(false);
  });

  it("absent block verifies unchanged (§4.4 back-compat)", async () => {
    const v = await verify(validDoc(), opts("third-party", {issuedBy: SIGNER}));
    expect(v.fragments.issuance).toBe("VALID");
    expect(v.valid).toBe(true);
  });

  it("unwired adapter (no issuedBy) skips the additive check, never regresses", async () => {
    const doc = withProtocol(validDoc(), "0xanything");
    const v = await verify(doc, opts("third-party", {})); // issuedBy omitted -> unwired
    expect(v.fragments.issuance).toBe("VALID");
    expect(v.valid).toBe(true);
  });
});
