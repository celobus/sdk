//! Messages, enums and entities: the traits the derives implement, the registry that collects their
//! declarations for the descriptor, and the JSON codec of their fields (the protobuf JSON mapping).

use std::any::TypeId;
use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::descriptor::DescribeError;
use crate::error::{Error, Result};

/// A protobuf message of the plugin: `#[derive(Message)]` on a struct, or `#[derive(Entity)]`.
pub trait Message: Sized + Send + Sync + 'static {
    /// The message name (the type name, PascalCase).
    const NAME: &'static str;
    /// Adds the message (or the entity) and the messages and enums its fields use to the registry.
    fn register(reg: &mut Registry);
    /// The protobuf JSON form: lowerCamelCase names, int64 as strings, absent optional fields omitted.
    fn to_json(&self) -> Value;
    /// Reads the protobuf JSON form (either field name form; a missing field is its default).
    fn from_json(value: &Value, path: &str) -> Result<Self>;
}

/// A protobuf enum of the plugin: `#[derive(Message)]` on a fieldless enum. Its values are the
/// snake_case variant names, numbered by declaration order; the first one is the default.
pub trait Enum: Sized + Send + Sync + 'static {
    const NAME: &'static str;
    const VALUES: &'static [&'static str];
    /// The value name (`OrderStatus::Placed` → `placed`).
    fn as_str(&self) -> &'static str;
    fn from_name(value: &str) -> Option<Self>;
    fn from_index(index: u64) -> Option<Self>;
}

/// An entity: `#[derive(Entity)]`, or a directory entity of the platform (`platform::User`).
pub trait Entity: Sized + Send + Sync + 'static {
    /// The type of `id`: `Uuid`, `i64` or `String` (`#[entity(key = ..)]`).
    type Key: crate::types::Key;
    /// The entity name (snake_case of the type name: `OrderLine` → `order_line`).
    const ENTITY: &'static str;
    /// The plugin that owns it; `None` for this plugin's own entities.
    const PLUGIN: Option<&'static str>;
}

/// A request message whose response type is known, so `ctx.call(plugin, &req)` is typed:
/// `#[message(response = Customer)]` on the request.
pub trait Call: Message {
    type Response: Message;
}

/// A type a message field may have (implemented by the SDK for the column types and by the derives).
pub trait FieldValue: Sized + Send + Sync + 'static {
    /// Its descriptor type (registers the messages and enums it uses).
    fn field_type(reg: &mut Registry) -> FieldType;
    fn encode(&self) -> Value;
    fn decode(value: &Value, path: &str) -> Result<Self>;
    /// The value of a field missing from the JSON (or `null`): the type's default where protobuf has
    /// one (`""`, `0`, `false`, the first enum value, an empty list, an empty message), `None` for an
    /// `Option`, otherwise an `invalid` error naming the field.
    fn missing(path: &str) -> Result<Self> {
        Err(Error::invalid(format!("{} is required", display_path(path))).field(path, "required"))
    }
    /// Left out of the JSON (an `Option` that is `None`).
    fn is_absent(&self) -> bool {
        false
    }
    /// The protobuf default of a scalar or a list that has one (`""`, `0`, `false`, an empty list): what
    /// `#[message(omit_defaults)]` leaves out. Never for an `Option` (presence is its value) or an enum.
    fn is_default(&self) -> bool {
        false
    }
    /// The JSON of a field of an `omit_defaults` message: `None` when it is left out (absent, or its default; a
    /// message whose JSON is `{}`).
    #[doc(hidden)]
    fn encode_trimmed(&self) -> Option<Value> {
        (!self.is_absent() && !self.is_default()).then(|| self.encode())
    }
    #[doc(hidden)]
    fn list_type(reg: &mut Registry) -> FieldType {
        let mut ty = Self::field_type(reg);
        ty.repeated = true;
        ty
    }
    #[doc(hidden)]
    fn encode_list(items: &[Self]) -> Value {
        Value::Array(items.iter().map(Self::encode).collect())
    }
    #[doc(hidden)]
    fn decode_list(value: &Value, path: &str) -> Result<Vec<Self>> {
        let Value::Array(items) = value else {
            return Err(wrong_type(path, "a list"));
        };
        items
            .iter()
            .enumerate()
            .map(|(i, v)| Self::decode(v, &format!("{path}[{i}]")))
            .collect()
    }
}

/// A field's type in the descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldType {
    pub(crate) name: TypeName,
    pub(crate) repeated: bool,
    pub(crate) optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeName {
    Scalar(&'static str),
    Message(&'static str),
    Enum(&'static str),
    /// `ref:<plugin>.<entity>`; `None` is the plugin being described.
    Ref(Option<&'static str>, &'static str),
}

impl FieldType {
    fn of(name: TypeName) -> Self {
        Self {
            name,
            repeated: false,
            optional: false,
        }
    }

    /// `string`, `bool`, `int32`, `int64`, `double`, `decimal`, `uuid`, `timestamp`, `date`, `bytes`, `json`.
    pub fn scalar(name: &'static str) -> Self {
        Self::of(TypeName::Scalar(name))
    }

    /// `message:<name>`.
    pub fn message(name: &'static str) -> Self {
        Self::of(TypeName::Message(name))
    }

    /// `enum:<name>`.
    pub fn enumeration(name: &'static str) -> Self {
        Self::of(TypeName::Enum(name))
    }

    /// `ref:<plugin>.<entity>`; `plugin` `None` is the plugin being described.
    pub fn reference(plugin: Option<&'static str>, entity: &'static str) -> Self {
        Self::of(TypeName::Ref(plugin, entity))
    }

    /// The descriptor spelling, refs to own entities resolved to `plugin`.
    pub(crate) fn spelled(&self, plugin: &str) -> String {
        match &self.name {
            TypeName::Scalar(s) => (*s).to_string(),
            TypeName::Message(m) => format!("message:{m}"),
            TypeName::Enum(e) => format!("enum:{e}"),
            TypeName::Ref(p, e) => format!("ref:{}.{e}", p.unwrap_or(plugin)),
        }
    }
}

/// A field as a derive declares it.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub(crate) name: &'static str,
    pub(crate) ty: FieldType,
    pub(crate) description: &'static str,
    pub(crate) unique: bool,
    pub(crate) index: bool,
    pub(crate) encrypted: bool,
    /// `searchable`: its language ("" when not searchable)
    pub(crate) searchable: &'static str,
    /// a ref's declared forward navigation and inverse navigation ("" when not declared)
    pub(crate) nav: &'static str,
    pub(crate) inverse: &'static str,
    /// a ref's `on_delete` ("" when not declared: `restrict` within the plugin, `keep` across)
    pub(crate) on_delete: &'static str,
}

impl FieldInfo {
    /// The column modifiers of an entity's field (`#[col(unique, index, encrypted)]`).
    pub fn column(mut self, unique: bool, index: bool, encrypted: bool) -> Self {
        self.unique = unique;
        self.index = index;
        self.encrypted = encrypted;
        self
    }

    /// `#[col(searchable)]` (`"simple"`) or `#[col(searchable(lang = "english"))]`; `""`: not searchable.
    pub fn searchable(mut self, language: &'static str) -> Self {
        self.searchable = language;
        self
    }

    /// `#[col(nav = customer, inverse = orders)]` on a ref (`""`: not declared).
    pub fn navigation(mut self, nav: &'static str, inverse: &'static str) -> Self {
        self.nav = nav;
        self.inverse = inverse;
        self
    }

    /// `#[col(on_delete = cascade)]` on a ref (`""`: not declared).
    pub fn on_delete(mut self, on_delete: &'static str) -> Self {
        self.on_delete = on_delete;
        self
    }
}

/// An index over several columns of an entity: `(column, desc)` in order.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    pub columns: Vec<(&'static str, bool)>,
    pub unique: bool,
}

/// A message as `#[derive(Message)]` declares it.
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub name: &'static str,
    pub kind: &'static str,
    pub description: &'static str,
    pub fields: Vec<FieldInfo>,
}

/// An entity as `#[derive(Entity)]` declares it.
#[derive(Debug, Clone)]
pub struct EntityInfo {
    pub name: &'static str,
    pub message: &'static str,
    pub description: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub soft_delete: bool,
    pub version: bool,
    pub append_only: bool,
    pub fields: Vec<FieldInfo>,
    pub indexes: Vec<IndexInfo>,
}

#[derive(Debug, Clone)]
pub(crate) struct EnumInfo {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

/// Collects the messages, enums and entities a plugin uses, each once, in the order they are first met
/// (a message before the messages and enums its fields use: depth first, in field order).
#[derive(Default)]
pub struct Registry {
    names: HashMap<&'static str, TypeId>,
    /// message and entity message names, in the order they were first met
    pub(crate) order: Vec<&'static str>,
    pub(crate) messages: Vec<MessageInfo>,
    pub(crate) enums: Vec<EnumInfo>,
    pub(crate) entities: Vec<EntityInfo>,
    pub(crate) errors: Vec<DescribeError>,
}

impl Registry {
    /// Claims `name` for `T`: true the first time (the caller then declares it), false when `T` is
    /// already registered or another type has the name (`E_MSG_DUP`).
    pub fn begin<T: 'static>(&mut self, name: &'static str) -> bool {
        match self.names.get(name) {
            Some(id) if *id == TypeId::of::<T>() => false,
            Some(_) => {
                self.errors.push(DescribeError::new(
                    "E_MSG_DUP",
                    format!("messages.{name}"),
                    format!("two different types are named {name}"),
                ));
                false
            }
            None => {
                self.names.insert(name, TypeId::of::<T>());
                self.order.push(name);
                true
            }
        }
    }

    pub fn field<T: FieldValue>(
        &mut self,
        name: &'static str,
        description: &'static str,
    ) -> FieldInfo {
        FieldInfo {
            name,
            ty: T::field_type(self),
            description,
            unique: false,
            index: false,
            encrypted: false,
            searchable: "",
            nav: "",
            inverse: "",
            on_delete: "",
        }
    }

    pub fn message<T: 'static>(&mut self, info: MessageInfo) {
        self.messages.push(info);
    }

    pub fn entity<T: 'static>(&mut self, info: EntityInfo) {
        self.entities.push(info);
    }

    pub fn enumeration<T: Enum>(&mut self) {
        if self.names.get(T::NAME) == Some(&TypeId::of::<T>()) {
            return;
        }
        if self.names.contains_key(T::NAME) {
            self.errors.push(DescribeError::new(
                "E_ENUM_DUP",
                format!("enums.{}", T::NAME),
                format!("two different types are named {}", T::NAME),
            ));
            return;
        }
        self.names.insert(T::NAME, TypeId::of::<T>());
        self.enums.push(EnumInfo {
            name: T::NAME,
            values: T::VALUES,
        });
    }
}

// --- the codec's helpers, used by the derives ------------------------------------------------------

pub(crate) fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

fn display_path(path: &str) -> &str {
    if path.is_empty() { "the body" } else { path }
}

pub(crate) fn wrong_type(path: &str, expected: &str) -> Error {
    let err = Error::invalid(format!("{} must be {expected}", display_path(path)));
    if path.is_empty() {
        err
    } else {
        err.field(path, format!("must be {expected}"))
    }
}

/// The JSON object of a message.
pub fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| wrong_type(path, "an object"))
}

/// Writes a field under its JSON name, unless it is absent.
pub fn encode_field<T: FieldValue>(map: &mut Map<String, Value>, json_name: &str, value: &T) {
    if !value.is_absent() {
        map.insert(json_name.to_string(), value.encode());
    }
}

/// Writes a field under its JSON name, unless it is absent or its protobuf default (`#[message(omit_defaults)]`).
pub fn encode_field_trimmed<T: FieldValue>(
    map: &mut Map<String, Value>,
    json_name: &str,
    value: &T,
) {
    if let Some(v) = value.encode_trimmed() {
        map.insert(json_name.to_string(), v);
    }
}

/// Reads a field by its JSON name (or its declared name); missing or `null` is [`FieldValue::missing`].
pub fn decode_field<T: FieldValue>(
    obj: &Map<String, Value>,
    json_name: &str,
    name: &str,
    path: &str,
) -> Result<T> {
    let path = join(path, name);
    match obj.get(json_name).or_else(|| obj.get(name)) {
        None | Some(Value::Null) => T::missing(&path),
        Some(v) => T::decode(v, &path),
    }
}

/// An enum value: its name, or its number (the declaration order, from 0).
pub fn decode_enum<T: Enum>(value: &Value, path: &str) -> Result<T> {
    let found = match value {
        Value::String(s) => T::from_name(s),
        Value::Number(n) => n.as_u64().and_then(T::from_index),
        _ => None,
    };
    found.ok_or_else(|| wrong_type(path, &format!("one of {}", T::VALUES.join(", "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelled_types() {
        assert_eq!(FieldType::scalar("int64").spelled("x"), "int64");
        assert_eq!(FieldType::message("Note").spelled("x"), "message:Note");
        assert_eq!(FieldType::enumeration("Mood").spelled("x"), "enum:Mood");
        assert_eq!(
            FieldType::reference(None, "note").spelled("hello"),
            "ref:hello.note"
        );
        assert_eq!(
            FieldType::reference(Some("platform"), "user").spelled("hello"),
            "ref:platform.user"
        );
    }
}
