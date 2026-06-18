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
