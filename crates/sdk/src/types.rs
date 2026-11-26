//! Field types: `Id<E>`, `Json<T>`, `File`, `Timestamp`, `Date`, `Decimal`, `Uuid`, and the codec of the
//! language types (`String`, `bool`, `i32`, `i64`, `f64`, `Vec<u8>`, `Option<T>`, `Vec<T>`).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::str::FromStr;

use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::message::{Entity, FieldType, FieldValue, Registry, wrong_type};

pub use rust_decimal::Decimal;
pub use uuid::Uuid;

/// A point in time (UTC); JSON: RFC 3339 with `Z` (`2026-11-08T10:00:00Z`).
pub type Timestamp = chrono::DateTime<chrono::Utc>;
/// A calendar date; JSON: `YYYY-MM-DD`.
pub type Date = chrono::NaiveDate;

// --- scalars ---------------------------------------------------------------------------------------

impl FieldValue for String {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("string")
    }
    fn encode(&self) -> Value {
        Value::String(self.clone())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| wrong_type(path, "a string"))
    }
    fn is_default(&self) -> bool {
        self.is_empty()
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(String::new())
    }
}

impl FieldValue for bool {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("bool")
    }
    fn encode(&self) -> Value {
        Value::Bool(*self)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_bool()
            .ok_or_else(|| wrong_type(path, "true or false"))
    }
    fn is_default(&self) -> bool {
        !*self
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(false)
    }
}

fn integer(value: &Value) -> Option<i128> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 1e19)
                    .map(|f| f as i128)
            }),
        Value::String(s) => s.trim().parse::<i128>().ok(),
        _ => None,
    }
}

impl FieldValue for i32 {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("int32")
    }
    fn encode(&self) -> Value {
        Value::from(*self)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        integer(value)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| wrong_type(path, "a 32-bit integer"))
    }
    fn is_default(&self) -> bool {
        *self == 0
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(0)
    }
}

/// int64: a JSON string (`"42"`), so a browser never loses digits; a number is accepted too.
impl FieldValue for i64 {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("int64")
    }
    fn encode(&self) -> Value {
        Value::String(self.to_string())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        integer(value)
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(|| wrong_type(path, "a 64-bit integer"))
    }
    fn is_default(&self) -> bool {
        *self == 0
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(0)
    }
}

/// A lone `u8` is an int32 field (0–255); `Vec<u8>` is `bytes` (JSON: base64).
impl FieldValue for u8 {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("int32")
    }
    fn encode(&self) -> Value {
        Value::from(*self)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        integer(value)
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| wrong_type(path, "an integer from 0 to 255"))
    }
    fn is_default(&self) -> bool {
        *self == 0
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(0)
    }
    fn list_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("bytes")
    }
    fn encode_list(items: &[Self]) -> Value {
        Value::String(base64::engine::general_purpose::STANDARD.encode(items))
    }
    fn decode_list(value: &Value, path: &str) -> Result<Vec<Self>> {
        let s = value.as_str().ok_or_else(|| wrong_type(path, "base64"))?;
        let s = s.trim_end_matches('=');
        base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(s)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s))
            .map_err(|_| wrong_type(path, "base64"))
    }
}

/// double: a JSON number; `"NaN"`, `"Infinity"`, `"-Infinity"` as strings.
impl FieldValue for f64 {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("double")
    }
    fn encode(&self) -> Value {
        if self.is_nan() {
            Value::String("NaN".into())
        } else if self.is_infinite() {
            Value::String(if *self > 0.0 { "Infinity" } else { "-Infinity" }.into())
        } else {
            Value::from(*self)
        }
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => match s.as_str() {
                "NaN" => Some(f64::NAN),
                "Infinity" => Some(f64::INFINITY),
                "-Infinity" => Some(f64::NEG_INFINITY),
                s => s.trim().parse().ok(),
            },
            _ => None,
        }
        .ok_or_else(|| wrong_type(path, "a number"))
    }
    fn is_default(&self) -> bool {
        *self == 0.0
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(0.0)
    }
}

/// decimal: a JSON string (`"12.50"`), exact; a number is accepted too.
impl FieldValue for Decimal {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("decimal")
    }
    fn encode(&self) -> Value {
        Value::String(self.to_string())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        let text = match value {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => return Err(wrong_type(path, "a decimal string")),
        };
        Decimal::from_str(&text)
            .or_else(|_| Decimal::from_scientific(&text))
            .map_err(|_| wrong_type(path, "a decimal string"))
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(Decimal::ZERO)
    }
}

/// uuid: the canonical string (`8-4-4-4-12`, lowercase).
impl FieldValue for Uuid {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("uuid")
    }
    fn encode(&self) -> Value {
        Value::String(self.hyphenated().to_string())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_str()
            .and_then(|s| Uuid::parse_str(s.trim()).ok())
            .ok_or_else(|| wrong_type(path, "a UUID"))
    }
}

impl FieldValue for Timestamp {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("timestamp")
    }
    fn encode(&self) -> Value {
        Value::String(self.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .ok_or_else(|| wrong_type(path, "an RFC 3339 timestamp"))
    }
}

impl FieldValue for Date {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("date")
    }
    fn encode(&self) -> Value {
        Value::String(self.format("%Y-%m-%d").to_string())
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        value
            .as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
            .ok_or_else(|| wrong_type(path, "a date (YYYY-MM-DD)"))
    }
}

// --- wrappers --------------------------------------------------------------------------------------

/// An optional field: omitted from the JSON when `None`.
impl<T: FieldValue> FieldValue for Option<T> {
    fn field_type(reg: &mut Registry) -> FieldType {
        let mut ty = T::field_type(reg);
        ty.optional = true;
        ty
    }
    fn encode(&self) -> Value {
        self.as_ref().map_or(Value::Null, T::encode)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        T::decode(value, path).map(Some)
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(None)
    }
    fn is_absent(&self) -> bool {
        self.is_none()
    }
}

/// A repeated field (a JSON array); `Vec<u8>` is `bytes`.
impl<T: FieldValue> FieldValue for Vec<T> {
    fn field_type(reg: &mut Registry) -> FieldType {
        T::list_type(reg)
    }
    fn encode(&self) -> Value {
        T::encode_list(self)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        T::decode_list(value, path)
    }
    fn is_default(&self) -> bool {
        self.is_empty()
    }
    fn missing(_: &str) -> Result<Self> {
        Ok(Vec::new())
    }
}

/// Any JSON value, typed by `T` in code (`json` in the descriptor). On the wire it is the JSON value
/// itself, not a string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Json<T = Value>(pub T);

impl<T> std::ops::Deref for Json<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> FieldValue for Json<T> {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("json")
    }
    fn encode(&self) -> Value {
        serde_json::to_value(&self.0).unwrap_or(Value::Null)
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        T::deserialize(value)
            .map(Json)
            .map_err(|e| wrong_type(path, &format!("valid JSON for this field ({e})")))
    }
    fn missing(path: &str) -> Result<Self> {
        Self::decode(&Value::Null, path)
    }
}

// --- files -----------------------------------------------------------------------------------------

/// A file: a typed value of a message field, an entity column or a job payload (`file` in the descriptor; `File`,
/// `Option<File>`, `Vec<File>`). It names the file, never holds its bytes: read and write them with `ctx.files()`.
/// JSON: `{"id", "name", "type", "size"}`, `size` in bytes as a string like every 64-bit integer (`"2100000"`; a
/// number is read too).
///
/// A file comes from a caller (an endpoint's `File` field: uploaded by a screen, the CLI or another client, within
/// the field's `#[field(max_size = "1 GiB", accept = ["text/csv"])]`), from another plugin (in a call or its
/// answer), or from the plugin's own code. It lives while a row, a pending job, a schedule or a live agent run of the
/// plugin holds it; a file nothing holds is collected after a grace period. A `File` in an endpoint's answer reaches
/// the client with a short-lived URL to read it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct File {
    /// The file's id.
    pub id: String,
    /// Its name as uploaded or made (`report.csv`).
    #[serde(default)]
    pub name: String,
    /// Its MIME type (`text/csv`); `application/octet-stream` when unknown.
    #[serde(default, rename = "type")]
    pub content_type: String,
    /// Its size in bytes.
    #[serde(
        default,
        serialize_with = "size_to_json",
        deserialize_with = "size_from_json"
    )]
    pub size: u64,
}

fn size_to_json<S: serde::Serializer>(size: &u64, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&size.to_string())
}

fn size_from_json<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(0),
        v => integer(&v)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| serde::de::Error::custom("size must be a number of bytes")),
    }
}

impl FieldValue for File {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::scalar("file")
    }
    fn encode(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "type": self.content_type,
            "size": self.size.to_string(),
        })
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        let obj = crate::message::object(value, path)?;
        let text = |k: &str| -> Result<String> {
            match obj.get(k) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.clone()),
                Some(_) => Err(wrong_type(&format!("{path}.{k}"), "a string")),
            }
        };
        let id = text("id")?;
        if id.is_empty() {
            return Err(wrong_type(&format!("{path}.id"), "a file id"));
        }
        let size = match obj.get("size") {
            None | Some(Value::Null) => 0,
            Some(v) => integer(v)
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| wrong_type(&format!("{path}.size"), "a number of bytes"))?,
        };
        Ok(File {
            id,
            name: text("name")?,
            content_type: text("type")?,
            size,
        })
    }
}

// --- ids -------------------------------------------------------------------------------------------

/// The key type of an entity: `Uuid`, `i64` or `String`.
pub trait Key: Clone + Eq + Ord + Hash + fmt::Debug + fmt::Display + Send + Sync + 'static {
    #[doc(hidden)]
    fn encode_key(&self) -> Value;
    #[doc(hidden)]
    fn decode_key(value: &Value, path: &str) -> Result<Self>;
}

impl Key for Uuid {
    fn encode_key(&self) -> Value {
        FieldValue::encode(self)
    }
    fn decode_key(value: &Value, path: &str) -> Result<Self> {
        <Uuid as FieldValue>::decode(value, path)
    }
}

impl Key for i64 {
    fn encode_key(&self) -> Value {
        FieldValue::encode(self)
    }
    fn decode_key(value: &Value, path: &str) -> Result<Self> {
        <i64 as FieldValue>::decode(value, path)
    }
}

impl Key for String {
    fn encode_key(&self) -> Value {
        FieldValue::encode(self)
    }
    fn decode_key(value: &Value, path: &str) -> Result<Self> {
        let s = <String as FieldValue>::decode(value, path)?;
        if s.is_empty() {
            return Err(wrong_type(path, "a non-empty id"));
        }
        Ok(s)
    }
}

/// The id of an entity `E`: its key, typed by the entity, so an id of one entity is never passed
/// where another's is expected. As a field of another message or entity it is a reference
/// (`ref:<plugin>.<entity>` in the descriptor); on the wire it is the key (an `i64` key as a string).
pub struct Id<E: Entity> {
    key: E::Key,
    _entity: PhantomData<fn() -> E>,
}

impl<E: Entity> Id<E> {
    pub fn new(key: E::Key) -> Self {
        Self {
            key,
            _entity: PhantomData,
        }
    }

    pub fn key(&self) -> &E::Key {
        &self.key
    }

    pub fn into_key(self) -> E::Key {
        self.key
    }
}

impl<E: Entity> Clone for Id<E> {
    fn clone(&self) -> Self {
        Self::new(self.key.clone())
    }
}

impl<E: Entity> Copy for Id<E> where E::Key: Copy {}

impl<E: Entity> PartialEq for Id<E> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<E: Entity> Eq for Id<E> {}

impl<E: Entity> PartialOrd for Id<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E: Entity> Ord for Id<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

impl<E: Entity> Hash for Id<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state)
    }
}

impl<E: Entity> fmt::Debug for Id<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id<{}>({:?})", E::ENTITY, self.key)
    }
}

impl<E: Entity> fmt::Display for Id<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.key, f)
    }
}

impl<E: Entity> FieldValue for Id<E> {
    fn field_type(_: &mut Registry) -> FieldType {
        FieldType::reference(E::PLUGIN, E::ENTITY)
    }
    fn encode(&self) -> Value {
        self.key.encode_key()
    }
    fn decode(value: &Value, path: &str) -> Result<Self> {
        E::Key::decode_key(value, path).map(Self::new)
    }
}

impl<E: Entity> Serialize for Id<E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.key.encode_key().serialize(s)
    }
}

impl<'de, E: Entity> Deserialize<'de> for Id<E> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        E::Key::decode_key(&value, "")
            .map(Self::new)
            .map_err(serde::de::Error::custom)
    }
}
