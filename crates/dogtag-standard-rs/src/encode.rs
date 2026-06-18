//! Canonical value encoding (impl §1.1, §11.2) — byte-identical to dogtag-standard-ts/src/encode.ts.
use unicode_normalization::UnicodeNormalization;

use crate::types::{DogTagError, TypedScalar};

/// Pinned Unicode version target (A3) — must match the TS SDK's runtime ICU major version.
pub const UNICODE_VERSION: &str = "15.1";

/// NFC-normalize. Rust `String`s are always well-formed UTF-8, so unpaired surrogates are
/// unrepresentable (the TS SDK rejects them explicitly because of UTF-16).
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn is_integer(s: &str) -> bool {
    let b = s.strip_prefix('-').unwrap_or(s);
    if b.is_empty() {
        return false;
    }
    if b == "0" {
        return true;
    }
    let bytes = b.as_bytes();
    bytes[0] != b'0' && bytes.iter().all(|c| c.is_ascii_digit())
}

/// Canonical integer string: no leading zeros, "-0" -> "0" (A1).
pub fn canonical_integer(s: &str) -> Result<String, DogTagError> {
    if !is_integer(s) {
        return Err(DogTagError::InvalidInteger(s.to_string()));
    }
    Ok(if s == "-0" { "0".to_string() } else { s.to_string() })
}

/// Canonical decimal over the INPUT STRING, never a float (A1/A2).
pub fn canonical_decimal(s: &str) -> Result<String, DogTagError> {
    // grammar: ^-?(0|[1-9][0-9]*)(\.[0-9]+)?$
    let body = s.strip_prefix('-').unwrap_or(s);
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let int_ok = int_part == "0" || (!int_part.is_empty() && int_part.as_bytes()[0] != b'0' && int_part.bytes().all(|c| c.is_ascii_digit()));
    let frac_ok = match frac_part {
        Some(f) => !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit()),
        None => true,
    };
    if !int_ok || !frac_ok {
        return Err(DogTagError::InvalidDecimal(s.to_string()));
    }
    let mut out = s.to_string();
    if out.contains('.') {
        while out.ends_with('0') {
            out.pop();
        }
        if out.ends_with('.') {
            out.pop();
        }
    }
    if out == "-0" {
        out = "0".to_string();
    }
    Ok(out)
}

/// encodeValue(typeTag, value) -> canonical bytes (impl §1.1).
pub fn encode_value(s: &TypedScalar) -> Result<Vec<u8>, DogTagError> {
    Ok(match s {
        TypedScalar::Null => Vec::new(),
        TypedScalar::Bool(b) => vec![if *b { 0x01 } else { 0x00 }],
        TypedScalar::Str(v) => nfc(v).into_bytes(),
        TypedScalar::Integer(v) => canonical_integer(v)?.into_bytes(),
        TypedScalar::Decimal(v) => canonical_decimal(v)?.into_bytes(),
        TypedScalar::Bytes(v) => v.clone(),
    })
}

/// Canonical string form stored in `data`.
pub fn as_string(s: &TypedScalar) -> Result<String, DogTagError> {
    Ok(match s {
        TypedScalar::Null => String::new(),
        TypedScalar::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        TypedScalar::Str(v) => nfc(v),
        TypedScalar::Integer(v) => canonical_integer(v)?,
        TypedScalar::Decimal(v) => canonical_decimal(v)?,
        TypedScalar::Bytes(v) => hex::encode(v),
    })
}
