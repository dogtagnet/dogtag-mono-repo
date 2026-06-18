//! Shared types (mirror of packages/dogtag-standard-ts/src/types.ts).

/// Mandatory type tag so `"5"` (string) != `5` (integer). impl §1.1 / §3.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeTag {
    Null = 0,
    Bool = 1,
    String = 2,
    Integer = 3,
    Decimal = 4,
    Bytes = 5,
}

impl TypeTag {
    pub fn from_u8(t: u8) -> Option<Self> {
        Some(match t {
            0 => TypeTag::Null,
            1 => TypeTag::Bool,
            2 => TypeTag::String,
            3 => TypeTag::Integer,
            4 => TypeTag::Decimal,
            5 => TypeTag::Bytes,
            _ => return None,
        })
    }
}

/// A single typed scalar entering the wrap boundary (typed input — never a native float).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedScalar {
    Null,
    Bool(bool),
    /// NFC-normalized string.
    Str(String),
    /// decimal-string big integer.
    Integer(String),
    /// fixed-point decimal string.
    Decimal(String),
    Bytes(Vec<u8>),
}

impl TypedScalar {
    pub fn tag(&self) -> TypeTag {
        match self {
            TypedScalar::Null => TypeTag::Null,
            TypedScalar::Bool(_) => TypeTag::Bool,
            TypedScalar::Str(_) => TypeTag::String,
            TypedScalar::Integer(_) => TypeTag::Integer,
            TypedScalar::Decimal(_) => TypeTag::Decimal,
            TypedScalar::Bytes(_) => TypeTag::Bytes,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DogTagError {
    #[error("invalid integer: {0}")]
    InvalidInteger(String),
    #[error("invalid decimal: {0}")]
    InvalidDecimal(String),
    #[error("floats forbidden; pass INTEGER or DECIMAL as a string")]
    FloatForbidden,
    #[error("{0}")]
    Other(String),
}
