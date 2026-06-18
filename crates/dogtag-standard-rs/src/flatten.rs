//! Pinned flatten/keyPath grammar (impl §11.2 F2a — load-bearing, keyPath is hashed) —
//! mirror of packages/dogtag-standard-ts/src/flatten.ts.
//!
//!   object key -> ".key"   (key NFC; reserved chars . [ ] rejected)
//!   array elem -> "[i]"    (i base-10, no leading zeros)
//!   root has no leading "."; an empty object/array -> a null-typed leaf at that path.
use serde_json::{Map, Value};

use crate::encode::nfc;
use crate::types::{DogTagError, TypeTag, TypedScalar};

pub struct FlatEntry {
    pub key_path: String,
    pub scalar: TypedScalar,
}

/// A reserved char (`.`, `[`, `]`) in an object key is rejected.
fn has_reserved(key: &str) -> bool {
    key.contains('.') || key.contains('[') || key.contains(']')
}

/// Detect the `{tag, value}` leaf shape (mirror of TS `isTypedScalar`).
fn is_typed_scalar(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.contains_key("value") && matches!(m.get("tag"), Some(Value::Number(_))),
        _ => false,
    }
}

/// Build a `TypedScalar` from a `{tag, value}` leaf object.
fn scalar_of(m: &Map<String, Value>) -> Result<TypedScalar, DogTagError> {
    let tag_n = m
        .get("tag")
        .and_then(|t| t.as_u64())
        .ok_or_else(|| DogTagError::Other("typed scalar: tag must be a number".to_string()))?;
    let tag = TypeTag::from_u8(tag_n as u8)
        .ok_or_else(|| DogTagError::Other(format!("unknown tag {tag_n}")))?;
    let value = m.get("value").unwrap_or(&Value::Null);
    Ok(match tag {
        TypeTag::Null => TypedScalar::Null,
        TypeTag::Bool => TypedScalar::Bool(value.as_bool().ok_or_else(|| {
            DogTagError::Other("bool scalar: value must be a boolean".to_string())
        })?),
        TypeTag::String => TypedScalar::Str(
            value
                .as_str()
                .ok_or_else(|| DogTagError::Other("string scalar: value must be a string".to_string()))?
                .to_string(),
        ),
        TypeTag::Integer => TypedScalar::Integer(
            value
                .as_str()
                .ok_or_else(|| DogTagError::Other("integer scalar: value must be a string".to_string()))?
                .to_string(),
        ),
        TypeTag::Decimal => TypedScalar::Decimal(
            value
                .as_str()
                .ok_or_else(|| DogTagError::Other("decimal scalar: value must be a string".to_string()))?
                .to_string(),
        ),
        TypeTag::Bytes => TypedScalar::Bytes(
            hex::decode(
                value
                    .as_str()
                    .ok_or_else(|| DogTagError::Other("bytes scalar: value must be hex string".to_string()))?,
            )
            .map_err(|e| DogTagError::Other(format!("bytes scalar: bad hex: {e}")))?,
        ),
    })
}

/// Flatten a nested typed credential into pinned (keyPath, scalar) pairs.
pub fn flatten(credential: &Value) -> Result<Vec<FlatEntry>, DogTagError> {
    let mut out = Vec::new();
    walk(credential, "", &mut out)?;
    Ok(out)
}

fn walk(node: &Value, path: &str, out: &mut Vec<FlatEntry>) -> Result<(), DogTagError> {
    if is_typed_scalar(node) {
        let m = node.as_object().unwrap();
        out.push(FlatEntry {
            key_path: path.to_string(),
            scalar: scalar_of(m)?,
        });
        return Ok(());
    }
    if let Value::Array(arr) = node {
        if arr.is_empty() {
            out.push(FlatEntry {
                key_path: path.to_string(),
                scalar: TypedScalar::Null,
            });
            return Ok(());
        }
        for (i, el) in arr.iter().enumerate() {
            walk(el, &format!("{path}[{i}]"), out)?;
        }
        return Ok(());
    }
    if let Value::Object(m) = node {
        if m.is_empty() {
            out.push(FlatEntry {
                key_path: path.to_string(),
                scalar: TypedScalar::Null,
            });
            return Ok(());
        }
        // serde_json::Map preserves insertion order with the (default) preserve_order feature off
        // it is a BTreeMap; either way we iterate keys in their map order, matching JS Object.keys
        // for the string keys produced by our wrap pipeline.
        for (k, v) in m.iter() {
            let key = nfc(k);
            if has_reserved(&key) {
                return Err(DogTagError::Other(format!("reserved char in object key: {key:?}")));
            }
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            walk(v, &child_path, out)?;
        }
        return Ok(());
    }
    Err(DogTagError::Other(format!(
        "non-typed leaf at {path:?} — wrap scalars as {{tag,value}}"
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Key(String),
    Index(usize),
}

/// Tokenize a pinned keyPath into segments (reserved chars make this unambiguous).
pub fn tokenize_key_path(key_path: &str) -> Result<Vec<Token>, DogTagError> {
    let mut tokens = Vec::new();
    let bytes: Vec<char> = key_path.chars().collect();
    let mut i = 0usize;
    let mut cur = String::new();
    let flush_key = |cur: &mut String, tokens: &mut Vec<Token>| {
        if !cur.is_empty() {
            tokens.push(Token::Key(std::mem::take(cur)));
        }
    };
    while i < bytes.len() {
        let c = bytes[i];
        if c == '.' {
            flush_key(&mut cur, &mut tokens);
            i += 1;
        } else if c == '[' {
            flush_key(&mut cur, &mut tokens);
            let end = bytes[i..]
                .iter()
                .position(|&x| x == ']')
                .map(|p| p + i)
                .ok_or_else(|| DogTagError::Other("unterminated array index".to_string()))?;
            let num: String = bytes[i + 1..end].iter().collect();
            if !is_array_index(&num) {
                return Err(DogTagError::Other(format!("bad array index: {num}")));
            }
            let idx: usize = num
                .parse()
                .map_err(|_| DogTagError::Other(format!("bad array index: {num}")))?;
            tokens.push(Token::Index(idx));
            i = end + 1;
        } else {
            cur.push(c);
            i += 1;
        }
    }
    flush_key(&mut cur, &mut tokens);
    Ok(tokens)
}

/// `^(0|[1-9][0-9]*)$`
fn is_array_index(s: &str) -> bool {
    if s == "0" {
        return true;
    }
    let b = s.as_bytes();
    !b.is_empty() && b[0] != b'0' && b.iter().all(|c| c.is_ascii_digit())
}

/// Rebuild a nested object/array of packed strings from flat (keyPath -> packed) pairs.
/// `entries` is an ordered list of (keyPath, packed) so insertion order matches the TS Object.
pub fn unflatten(entries: &[(String, String)]) -> Result<Value, DogTagError> {
    let mut root = Value::Object(Map::new());
    for (key_path, packed) in entries {
        let tokens = tokenize_key_path(key_path)?;
        let mut cursor: &mut Value = &mut root;
        let n = tokens.len();
        for (t, tok) in tokens.iter().enumerate() {
            let last = t == n - 1;
            let next_is_index = tokens.get(t + 1).map(|nt| matches!(nt, Token::Index(_)));
            let fresh = || {
                if next_is_index == Some(true) {
                    Value::Array(Vec::new())
                } else {
                    Value::Object(Map::new())
                }
            };
            match tok {
                Token::Key(key) => {
                    let obj = ensure_object(cursor);
                    if last {
                        obj.insert(key.clone(), Value::String(packed.clone()));
                        break;
                    }
                    if !obj.contains_key(key) {
                        obj.insert(key.clone(), fresh());
                    }
                    cursor = obj.get_mut(key).unwrap();
                }
                Token::Index(idx) => {
                    let arr = ensure_array(cursor);
                    ensure_len(arr, *idx + 1);
                    if last {
                        arr[*idx] = Value::String(packed.clone());
                        break;
                    }
                    if matches!(arr[*idx], Value::Null) {
                        arr[*idx] = fresh();
                    }
                    cursor = &mut arr[*idx];
                }
            }
        }
    }
    Ok(root)
}

fn ensure_object(v: &mut Value) -> &mut Map<String, Value> {
    if !matches!(v, Value::Object(_)) {
        *v = Value::Object(Map::new());
    }
    v.as_object_mut().unwrap()
}

fn ensure_array(v: &mut Value) -> &mut Vec<Value> {
    if !matches!(v, Value::Array(_)) {
        *v = Value::Array(Vec::new());
    }
    v.as_array_mut().unwrap()
}

/// Grow a JSON array to `len`, filling holes with `null` (mirrors JS sparse-array holes).
fn ensure_len(arr: &mut Vec<Value>, len: usize) {
    while arr.len() < len {
        arr.push(Value::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_then_unflatten_preserves_keypaths() {
        // packed-string entries keyed by pinned keyPaths
        let entries: Vec<(String, String)> = vec![
            ("credentialSubject.dogTagId".to_string(), "aabb:3:42".to_string()),
            ("credentialSubject.name".to_string(), "ccdd:2:Rex".to_string()),
            (
                "credentialSubject.microchip.code".to_string(),
                "eeff:2:985141006580311".to_string(),
            ),
            (
                "credentialSubject.weightHistory[0].value".to_string(),
                "0011:4:22.7".to_string(),
            ),
        ];
        let tree = unflatten(&entries).unwrap();
        // re-collect (keyPath, packed) pairs from the rebuilt tree
        let mut out: Vec<(String, String)> = Vec::new();
        collect(&tree, "", &mut out);
        out.sort();
        let mut want: Vec<(String, String)> = entries.clone();
        want.sort();
        assert_eq!(out, want);
    }

    /// Mirror of wrap::flatten_data for the round-trip assertion.
    fn collect(node: &Value, path: &str, out: &mut Vec<(String, String)>) {
        match node {
            Value::String(s) => out.push((path.to_string(), s.clone())),
            Value::Array(arr) => {
                for (i, el) in arr.iter().enumerate() {
                    collect(el, &format!("{path}[{i}]"), out);
                }
            }
            Value::Object(m) => {
                for (k, v) in m.iter() {
                    let child = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    collect(v, &child, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn tokenize_key_path_segments() {
        let toks = tokenize_key_path("credentialSubject.weightHistory[0].value").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Key("credentialSubject".to_string()),
                Token::Key("weightHistory".to_string()),
                Token::Index(0),
                Token::Key("value".to_string()),
            ]
        );
    }
}
