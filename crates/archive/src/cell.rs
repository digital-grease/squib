//! Typed cell encoding for table rows in archives.
//!
//! `{"i":"123"}` integer (decimal string), `{"r":1.5}` real, `{"s":"text"}` text,
//! `{"b":"base64"}` blob, `null`. Explicit types keep 64-bit integers exact and make
//! hostile type confusion a validation error rather than a coercion.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rusqlite::types::{Value, ValueRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Cell {
    #[serde(rename = "i")]
    Int(String),
    #[serde(rename = "r")]
    Real(f64),
    #[serde(rename = "s")]
    Text(String),
    #[serde(rename = "b")]
    Blob(String),
}

pub fn from_sql(v: ValueRef<'_>) -> Option<Cell> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(Cell::Int(i.to_string())),
        ValueRef::Real(r) => Some(Cell::Real(r)),
        ValueRef::Text(t) => Some(Cell::Text(String::from_utf8_lossy(t).into_owned())),
        ValueRef::Blob(b) => Some(Cell::Blob(STANDARD.encode(b))),
    }
}

pub fn to_sql(c: &Option<Cell>) -> Result<Value, String> {
    Ok(match c {
        None => Value::Null,
        Some(Cell::Int(s)) => Value::Integer(s.parse().map_err(|_| format!("bad integer {s:?}"))?),
        Some(Cell::Real(r)) if r.is_finite() => Value::Real(*r),
        Some(Cell::Real(_)) => return Err("non-finite real".into()),
        Some(Cell::Text(t)) => Value::Text(t.clone()),
        Some(Cell::Blob(b)) => Value::Blob(STANDARD.decode(b).map_err(|_| "bad base64".to_string())?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_integers_survive_as_strings() {
        let c = from_sql(ValueRef::Integer(i64::MAX)).unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"i":"9223372036854775807"}"#);
        assert_eq!(to_sql(&Some(serde_json::from_str(&json).unwrap())).unwrap(), Value::Integer(i64::MAX));
        assert!(to_sql(&Some(Cell::Int("1e3".into()))).is_err());
        assert!(to_sql(&Some(Cell::Blob("***".into()))).is_err());
    }
}
