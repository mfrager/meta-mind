//! Primitive ↔ XSD literal mapping (§7.4, adopted from nexus
//! `path_to_xsd_datatype` / `value_to_literal`). Encoding uses the exact
//! datatype per Rust type; decoding accepts the whole integer / float /
//! string / boolean families and rejects anything else.

use math_core::oxigraph::model::{Literal, NamedNode};

use crate::CodecError;

pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

pub fn xsd(local: &str) -> NamedNode {
    NamedNode::new(format!("{XSD}{local}")).expect("xsd datatype IRI")
}

fn typed(value: impl Into<String>, local: &str) -> Literal {
    Literal::new_typed_literal(value.into(), xsd(local))
}

fn finite(v: f64) -> Result<(), CodecError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(CodecError::Encode(
            "non-finite float has no faithful xsd encoding".into(),
        ))
    }
}

// ── encoders ──────────────────────────────────────────────────────────────

pub fn bool_literal(v: bool) -> Literal {
    typed(v.to_string(), "boolean")
}
pub fn i8_literal(v: i8) -> Literal {
    typed(v.to_string(), "byte")
}
pub fn i16_literal(v: i16) -> Literal {
    typed(v.to_string(), "short")
}
pub fn i32_literal(v: i32) -> Literal {
    typed(v.to_string(), "int")
}
pub fn i64_literal(v: i64) -> Literal {
    // §7.4 of the RDF interface plan: i64 maps to xsd:integer.
    typed(v.to_string(), "integer")
}
pub fn isize_literal(v: isize) -> Literal {
    typed(v.to_string(), "integer")
}
pub fn u8_literal(v: u8) -> Literal {
    typed(v.to_string(), "unsignedByte")
}
pub fn u16_literal(v: u16) -> Literal {
    typed(v.to_string(), "unsignedShort")
}
pub fn u32_literal(v: u32) -> Literal {
    typed(v.to_string(), "unsignedInt")
}
pub fn u64_literal(v: u64) -> Literal {
    typed(v.to_string(), "unsignedLong")
}
pub fn usize_literal(v: usize) -> Literal {
    typed(v.to_string(), "nonNegativeInteger")
}
pub fn f32_literal(v: f32) -> Result<Literal, CodecError> {
    finite(f64::from(v))?;
    Ok(typed(v.to_string(), "float"))
}
pub fn f64_literal(v: f64) -> Result<Literal, CodecError> {
    finite(v)?;
    Ok(typed(v.to_string(), "double"))
}
pub fn string_literal(v: &str) -> Literal {
    Literal::new_simple_literal(v.to_string())
}

// ── decoders ──────────────────────────────────────────────────────────────

fn datatype_local(l: &Literal) -> String {
    let dt = l.datatype().as_str().to_string();
    let local = dt.rsplit(['#', '/']).next().unwrap_or(&dt);
    local.to_lowercase()
}

pub fn bool_from_literal(l: &Literal) -> Result<bool, CodecError> {
    match l.value() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(CodecError::Decode(format!(
            "`{other}` is not an xsd:boolean"
        ))),
    }
}

fn integer_family() -> &'static [&'static str] {
    &[
        "integer",
        "long",
        "int",
        "short",
        "byte",
        "nonnegativeinteger",
        "unsignedlong",
        "unsignedint",
        "unsignedshort",
        "unsignedbyte",
    ]
}

pub fn i64_from_literal(l: &Literal) -> Result<i64, CodecError> {
    let name = datatype_local(l);
    if !integer_family().contains(&name.as_str()) {
        return Err(CodecError::Decode(format!(
            "expected an integer datatype, got `{name}`"
        )));
    }
    l.value()
        .parse::<i64>()
        .map_err(|_| CodecError::Decode(format!("`{}` does not fit in i64", l.value())))
}

pub fn u64_from_literal(l: &Literal) -> Result<u64, CodecError> {
    let name = datatype_local(l);
    if !matches!(
        name.as_str(),
        "nonnegativeinteger"
            | "unsignedlong"
            | "unsignedint"
            | "unsignedshort"
            | "unsignedbyte"
            | "integer"
            | "long"
            | "int"
    ) {
        return Err(CodecError::Decode(format!(
            "expected an unsigned integer datatype, got `{name}`"
        )));
    }
    l.value()
        .parse::<u64>()
        .map_err(|_| CodecError::Decode(format!("`{}` does not fit in u64", l.value())))
}

pub fn f64_from_literal(l: &Literal) -> Result<f64, CodecError> {
    let name = datatype_local(l);
    if !matches!(name.as_str(), "float" | "double" | "decimal") {
        return Err(CodecError::Decode(format!(
            "expected a numeric datatype, got `{name}`"
        )));
    }
    l.value()
        .parse::<f64>()
        .map_err(|_| CodecError::Decode(format!("`{}` is not a number", l.value())))
}

pub fn string_from_literal(l: &Literal) -> Result<String, CodecError> {
    let name = datatype_local(l);
    if name != "string" {
        return Err(CodecError::Decode(format!(
            "expected xsd:string, got `{name}`"
        )));
    }
    Ok(l.value().to_string())
}

// ── generic primitive ─────────────────────────────────────────────────────

/// A decoded primitive value, for generic round trips and collections.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
}

impl Prim {
    pub fn to_literal(&self) -> Result<Literal, CodecError> {
        match self {
            Prim::Bool(b) => Ok(bool_literal(*b)),
            Prim::Int(i) => Ok(i64_literal(*i)),
            Prim::UInt(u) => Ok(u64_literal(*u)),
            Prim::Float(f) => f64_literal(*f),
            Prim::Text(s) => Ok(string_literal(s)),
        }
    }

    pub fn from_literal(l: &Literal) -> Result<Self, CodecError> {
        match datatype_local(l).as_str() {
            "boolean" => Ok(Prim::Bool(bool_from_literal(l)?)),
            "integer" | "long" | "int" | "short" | "byte" => Ok(Prim::Int(i64_from_literal(l)?)),
            "nonnegativeinteger" | "unsignedlong" | "unsignedint" | "unsignedshort"
            | "unsignedbyte" => Ok(Prim::UInt(u64_from_literal(l)?)),
            "float" | "double" | "decimal" => Ok(Prim::Float(f64_from_literal(l)?)),
            "string" => Ok(Prim::Text(string_from_literal(l)?)),
            other => Err(CodecError::Decode(format!(
                "unsupported literal datatype `{other}`"
            ))),
        }
    }

    /// The float value of a numeric primitive (`Int`/`UInt`/`Float`), so
    /// whole-number literals written by the SEM lane (`xsd:integer`) decode
    /// into float-typed engine fields without requiring a `double` literal.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Prim::Int(i) => Some(*i as f64),
            Prim::UInt(u) => Some(*u as f64),
            Prim::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The unsigned value of a numeric primitive when it is a whole number
    /// (`Int`/`UInt`, or an integral `Float`), so SEM-authored counts decode
    /// into uint-typed engine fields regardless of literal datatype.
    pub fn as_uint(&self) -> Option<u64> {
        match self {
            Prim::UInt(u) => Some(*u),
            Prim::Int(i) => u64::try_from(*i).ok(),
            Prim::Float(f) => {
                if f.is_finite() && f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64 {
                    Some(*f as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
