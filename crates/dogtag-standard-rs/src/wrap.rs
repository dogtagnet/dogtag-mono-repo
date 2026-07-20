//! Wrap / selective-disclosure / packed-value parsing (impl §1.4, §1.5, §11.2 F2b) —
//! mirror of packages/dogtag-standard-ts/src/wrap.ts.
use ark_bn254::Fr;
use ark_ff::PrimeField;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::poseidon::to_be_bytes32;

use crate::encode::as_string;
use crate::field::to_hex32;
use crate::flatten::{flatten, unflatten};
use crate::leaf::hash_leaf;
use crate::merkle::build_merkle;
use crate::types::{DogTagError, TypeTag, TypedScalar};

/// A salt provider yields 16 fresh bytes per leaf.
pub type SaltProvider<'a> = dyn FnMut() -> [u8; 16] + 'a;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssuerMeta {
    pub name: String,
    pub domain: String,
    #[serde(rename = "documentStore")]
    pub document_store: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
}

/// The Level-A protocol version string (M7 §4.4). This is the *protocol level* stamped in the
/// `protocol` block, distinct from the envelope schema `WrappedDoc.version` (`"dogtag/1.0"`).
pub const LEVEL_A_VERSION: &str = "dogtag-levela/1";

/// The Level-B protocol version string - the **on-chain `ContractSet` axis** of the two-axis
/// `ProtocolRegistry` (R-5). Its keccak is the `contractSetId` that keys the trio + verifier +
/// circuitId (`contracts/script/ProtocolVersions.sol:44`, `ProtocolRegistry.sol` §"ON-CHAIN axis").
///
/// This is the axis an *issuance* stamps, because minting binds a tag to deployed contracts - the
/// SBT it is sealed into and the registry that will verify it. It is deliberately NOT the artifact
/// axis (`dogtag-levelb-artifacts/1`, keyed by `artifactSetId` in a separate keyspace): a zkey
/// rotation re-points `activeArtifactSetOf` and must NOT change what an already-minted tag claims.
/// Resolve the two independently; never collapse them back into one version.
///
/// Adding this constant does **not** flip any Level-A producer. Level-A issuance keeps stamping
/// [`LEVEL_A_VERSION`] (that cutover is a separate milestone); this is stamped only by the Level-B
/// custodial issuance path, so an owner-hidden tag never claims to be a Level-A record.
pub const LEVEL_B_VERSION: &str = "dogtag-levelb/1";

/// M7 record-provenance block (§4.2): which protocol/contract a record was created on **and who
/// issued it**, carried BESIDE `signature.merkleRoot` - NEVER inside `R` or the ZK proof.
///
/// It is a **routing hint only, never authority**: the on-chain re-derivation stays authoritative
/// (`clone = rootIssuer[R]`, `R == profileRoot(id)`, `isValid(R)`). In particular `issuerSigner` is
/// the envelope's *claim* of who issued; a verifier validates it against the on-chain
/// `clone.issuedBy[R]` (see `verify`) and a wrong/forged claim fails closed. Absent on pre-M7
/// records - consumers default it via [`WrappedDoc::resolved_protocol`] (§4.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolMeta {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    /// protocol level, e.g. `"dogtag-levela/1"` / `"dogtag-levelb/1"` - resolves circuit/VK/artifact
    /// via the discovery anchor. NOT the envelope `WrappedDoc.version`.
    pub version: String,
    /// THE routing key -> derives `sbt()` + `rootIndex()` after a trio migration.
    #[serde(rename = "verificationRegistry")]
    pub verification_registry: String,
    /// == today's `issuer.documentStore`; the direct `isValid` target.
    #[serde(rename = "issuerClone")]
    pub issuer_clone: String,
    /// the signer that issued (claim, == on-chain `clone.issuedBy[R]`); validated, never trusted.
    #[serde(rename = "issuerSigner")]
    pub issuer_signer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "targetHash")]
    pub target_hash: String,
    pub proof: Vec<String>,
    #[serde(rename = "merkleRoot")]
    pub merkle_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Privacy {
    pub obfuscated: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedDoc {
    pub version: String,
    /// nested, salted, type-tagged scalars (self-describing).
    pub data: Value,
    pub signature: Signature,
    pub privacy: Privacy,
    pub issuer: IssuerMeta,
    /// M7 provenance block (§4.2), beside `signature.merkleRoot` - NOT inside `R`. Absent on pre-M7
    /// records; default it with [`WrappedDoc::resolved_protocol`]. A routing hint only, never authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<ProtocolMeta>,
}

impl WrappedDoc {
    /// The effective provenance for routing (§4.4 back-compat). A stamped block is returned as-is
    /// (a routing hint only - never authority). An **absent** block defaults to Level-A:
    /// `verificationRegistry` = the Level-A registry, `version` = [`LEVEL_A_VERSION`],
    /// `issuerClone` = `IssuerMeta.documentStore`, `issuerSigner` = the on-chain `clone.issuedBy[R]`
    /// (supplied by the caller, which reads it on-chain - it exists for Level-A too). Existing
    /// records self-route to the old trio and keep verifying unchanged.
    pub fn resolved_protocol(
        &self,
        chain_id: u64,
        level_a_verification_registry: &str,
        onchain_issued_by: &str,
    ) -> ProtocolMeta {
        self.protocol.clone().unwrap_or_else(|| ProtocolMeta {
            chain_id,
            version: LEVEL_A_VERSION.to_string(),
            verification_registry: level_a_verification_registry.to_string(),
            issuer_clone: self.issuer.document_store.clone(),
            issuer_signer: onchain_issued_by.to_string(),
        })
    }
}

/// Parse a 0x.. 32-byte hex back into a field element (mirror of TS `fromHex32`).
pub fn from_hex32(h: &str) -> Result<Fr, DogTagError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| DogTagError::Other(format!("bad hex32: {e}")))?;
    if bytes.len() != 32 {
        return Err(DogTagError::Other(format!("hex32 must be 32 bytes (got {})", bytes.len())));
    }
    // Hashes are always < p; reject anything >= p to mirror the TS guard.
    let v = Fr::from_be_bytes_mod_order(&bytes);
    // Re-encode to detect reduction (i.e. input was >= p): canonical 32-byte BE must round-trip.
    if to_be_bytes32(&v) != bytes.as_slice() {
        return Err(DogTagError::Other("hex exceeds field".to_string()));
    }
    Ok(v)
}

/// parse(packed): split on the FIRST TWO ":" only (value may contain ":"). impl §11.2 F2b.
pub fn parse_packed(packed: &str) -> Result<(String, TypeTag, String), DogTagError> {
    let first = packed
        .find(':')
        .ok_or_else(|| DogTagError::Other(format!("bad packed value: {packed}")))?;
    let second_rel = packed[first + 1..]
        .find(':')
        .ok_or_else(|| DogTagError::Other(format!("bad packed value: {packed}")))?;
    let second = first + 1 + second_rel;
    let salt_hex = packed[..first].to_string();
    let tag_n: u8 = packed[first + 1..second]
        .parse()
        .map_err(|_| DogTagError::Other(format!("bad packed value: {packed}")))?;
    let tag = TypeTag::from_u8(tag_n)
        .ok_or_else(|| DogTagError::Other(format!("unknown tag {tag_n}")))?;
    let value_rest = packed[second + 1..].to_string();
    Ok((salt_hex, tag, value_rest))
}

/// Reconstruct a TypedScalar from a packed `tag:valueRest`.
pub fn scalar_from_packed(tag: TypeTag, value_rest: &str) -> Result<TypedScalar, DogTagError> {
    Ok(match tag {
        TypeTag::Null => TypedScalar::Null,
        TypeTag::Bool => TypedScalar::Bool(value_rest == "true"),
        TypeTag::String => TypedScalar::Str(value_rest.to_string()),
        TypeTag::Integer => TypedScalar::Integer(value_rest.to_string()),
        TypeTag::Decimal => TypedScalar::Decimal(value_rest.to_string()),
        TypeTag::Bytes => TypedScalar::Bytes(
            hex::decode(value_rest).map_err(|e| DogTagError::Other(format!("bad bytes hex: {e}")))?,
        ),
    })
}

/// Recompute the leaf hash for one packed entry (used by verify + obfuscate).
pub fn leaf_from_packed(key_path: &str, packed: &str) -> Result<Fr, DogTagError> {
    let (salt_hex, tag, value_rest) = parse_packed(packed)?;
    let salt = hex::decode(&salt_hex).map_err(|e| DogTagError::Other(format!("bad salt hex: {e}")))?;
    hash_leaf(key_path, &salt, &scalar_from_packed(tag, &value_rest)?)
}

/// Collect every (keyPath, packed) pair from a nested `data` object (ordered).
pub fn flatten_data(data: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk_data(data, "", &mut out);
    out
}

fn walk_data(node: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match node {
        Value::String(s) => out.push((path.to_string(), s.clone())),
        Value::Array(arr) => {
            for (i, el) in arr.iter().enumerate() {
                walk_data(el, &format!("{path}[{i}]"), out);
            }
        }
        Value::Object(m) => {
            for (k, v) in m.iter() {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                walk_data(v, &child_path, out);
            }
        }
        _ => {}
    }
}

fn bytes_to_hex(b: &[u8]) -> String {
    hex::encode(b)
}

/// wrapDocument — typed input -> single Poseidon root R (impl §1.4).
pub fn wrap_document(
    typed_credential: &Value,
    issuer: IssuerMeta,
    salt_provider: &mut SaltProvider,
) -> Result<WrappedDoc, DogTagError> {
    let flat = flatten(typed_credential)?;
    let mut data_flat: Vec<(String, String)> = Vec::with_capacity(flat.len());
    let mut leaves: Vec<Fr> = Vec::with_capacity(flat.len());
    for entry in &flat {
        let salt = salt_provider();
        data_flat.push((
            entry.key_path.clone(),
            format!(
                "{}:{}:{}",
                bytes_to_hex(&salt),
                entry.scalar.tag() as u8,
                as_string(&entry.scalar)?
            ),
        ));
        leaves.push(hash_leaf(&entry.key_path, &salt, &entry.scalar)?);
    }
    let root = build_merkle(&leaves).root;
    let r = to_hex32(&root);
    Ok(WrappedDoc {
        version: "dogtag/1.0".to_string(),
        data: unflatten(&data_flat)?,
        signature: Signature {
            type_: "DogTagMerkleProof".to_string(),
            target_hash: r.clone(),
            proof: Vec::new(),
            merkle_root: r,
        },
        privacy: Privacy { obfuscated: Vec::new() },
        issuer,
        // The pure wrap knows the leaves, not the deployment: the issuing stack attaches the
        // `protocol` block (§4.2) after wrapping, from its chain/registry/signer config.
        protocol: None,
    })
}

/// obfuscate — move a field's leaf hash into privacy.obfuscated[] and drop its cleartext.
/// Root unchanged.
pub fn obfuscate(doc: &WrappedDoc, key_paths: &[String]) -> Result<WrappedDoc, DogTagError> {
    let mut data_flat = flatten_data(&doc.data);
    let mut obfuscated = doc.privacy.obfuscated.clone();
    for kp in key_paths {
        let packed = data_flat
            .iter()
            .find(|(k, _)| k == kp)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| DogTagError::Other(format!("cannot obfuscate missing field: {kp}")))?;
        obfuscated.push(to_hex32(&leaf_from_packed(kp, &packed)?));
        data_flat.retain(|(k, _)| k != kp);
    }
    let mut out = doc.clone();
    out.data = unflatten(&data_flat)?;
    out.privacy = Privacy { obfuscated };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Deterministic salt: each call returns [n, n, ... 16x], n increments.
    fn fixed_salts() -> impl FnMut() -> [u8; 16] {
        let mut n: u8 = 1;
        move || {
            let s = [n; 16];
            n = n.wrapping_add(1);
            s
        }
    }

    fn sample_credential() -> Value {
        json!({
            "credentialSubject": {
                "dogTagId": {"tag": 3, "value": "42"},
                "name": {"tag": 2, "value": "Rex"},
                "microchip": {"code": {"tag": 2, "value": "985141006580311"}},
                "weightHistory": [{"value": {"tag": 4, "value": "22.7"}}]
            }
        })
    }

    fn issuer() -> IssuerMeta {
        IssuerMeta {
            name: "Acme Vet".to_string(),
            domain: "acme.example".to_string(),
            document_store: "0x0000000000000000000000000000000000000001".to_string(),
            record_type: "VACCINATION".to_string(),
        }
    }

    #[test]
    fn wrap_roundtrip_and_data_shape() {
        let mut sp = fixed_salts();
        let doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        // data must contain the packed cleartext for dogTagId
        let flat = flatten_data(&doc.data);
        let dog = flat.iter().find(|(k, _)| k == "credentialSubject.dogTagId").unwrap();
        let (_, tag, val) = parse_packed(&dog.1).unwrap();
        assert_eq!(tag, TypeTag::Integer);
        assert_eq!(val, "42");
        // name present
        assert!(flat.iter().any(|(k, v)| k == "credentialSubject.name" && v.ends_with(":Rex")));
        assert_eq!(doc.signature.target_hash, doc.signature.merkle_root);
    }

    #[test]
    fn parse_packed_splits_first_two_colons() {
        let (salt, tag, rest) = parse_packed("aabb:2:2026-01-01T00:00:00Z").unwrap();
        assert_eq!(salt, "aabb");
        assert_eq!(tag, TypeTag::String);
        assert_eq!(rest, "2026-01-01T00:00:00Z");
    }
}
