//! The plugin's part of the workspace's Data Lake: its layers and its lake tables (`layers`, `lake_tables`), and the
//! lake tables of other plugins it reads (`dependencies[].lake`).
//!
//! A layer is a name and a title; two plugins may declare layers of the same name, and the platform declares none. A
//! lake table belongs to one layer of its plugin and is `<layer>.<plugin>_<table>` in the lake. Its columns are the
//! fields of its row message: scalars, enums and refs, never a list, a message or an encrypted field. Its partition
//! columns and its key columns are fields of the row of types that partition and key exactly. Who reads it is a list
//! of the plugin's roles (`read`); the plugin's own code reads it in its system context only when the table says so
//! (`system_read`); another plugin reads it only when the table is exported (`export`) and that plugin declares the
//! read.
//!
//! Between versions a lake table only grows: a new optional column. Its layer, partition and key columns, a column's
//! type and the columns it has stay; a table no longer declared stays in the lake and is no longer written.
//! [`changes`] lists what a new version breaks of a previous one (`E_LAKE_*`, never allowed, not even by a new major
//! version).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Descriptor, Error, FieldType, lock::Change};

/// The most layers and lake tables a plugin declares, the columns of a table, its partition and key columns.
pub const MAX_LAYERS: usize = 16;
pub const MAX_TABLES: usize = 64;
pub const MAX_COLUMNS: usize = 256;
pub const MAX_PARTITION: usize = 4;
pub const MAX_KEY: usize = 8;

/// The rows of one write, of one chunk of it, and the bytes of a chunk's rows.
pub const WRITE_ROWS: u64 = 1_000_000;
pub const CHUNK_ROWS: usize = 10_000;
pub const CHUNK_BYTES: usize = 4 << 20;
/// The largest text, bytes or JSON value of a lake row (1 MiB): a quarter of a read's answer (4 MiB), so a row of
/// the widest values still reads back, and a read stops long before such values could crowd the memory.
pub const VALUE_BYTES: usize = 1 << 20;

/// A layer of the lake: a name (`raw`) and a title (`As it arrived`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// snake_case, at most 32 characters (`raw`, `core`, `mart`).
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

/// A lake table of the plugin.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LakeTable {
    /// snake_case, at most 32 characters, unique among the plugin's lake tables, entities and views (`stock_file`).
    pub name: String,
    /// A layer the plugin declares.
    pub layer: String,
    /// The row message (`StockFileRow`): one column per field.
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The partition columns, in order: a `replace` swaps the partitions its filter names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partition: Vec<String>,
    /// The key columns: a `merge` writes over the row with the same key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key: Vec<String>,
    /// The plugin's roles whose holders read the table (every row of it); none: no member reads it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    /// The plugin's own code reads the table in its system context.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system_read: bool,
    /// Other plugins that declare the read (`dependencies[].lake`) read it, as callers holding one of `read`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub export: bool,
    /// Another plugin that declares the read (`dependencies[].lake`) may copy the table into its own lake tables with a
    /// model (`lake_models`), where that plugin's own roles decide who reads the copy. Apart from `export`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub export_models: bool,
}

/// What a lake column holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnType {
    String,
    Bool,
    Int32,
    Int64,
    Double,
    Decimal,
    Uuid,
    Timestamp,
    Date,
    Bytes,
    Json,
    /// an enum's value by its name
    Enum(Vec<String>),
    /// a ref's key, as its text (`ref:<plugin>.<entity>` in the descriptor)
    Ref(String),
}

impl ColumnType {
    /// The type of a field's wire type (`decimal`, `enum:Status`, `ref:shop.order`); none for a list, a message or
    /// an unknown type.
    pub fn of(d: &Descriptor, f: &crate::Field) -> Option<ColumnType> {
        if f.repeated {
            return None;
        }
        Some(match FieldType::parse(&f.ty)? {
            FieldType::Scalar(s) => match s {
                "string" => ColumnType::String,
                "bool" => ColumnType::Bool,
                "int32" => ColumnType::Int32,
                "int64" => ColumnType::Int64,
                "double" => ColumnType::Double,
                "decimal" => ColumnType::Decimal,
                "uuid" => ColumnType::Uuid,
                "timestamp" => ColumnType::Timestamp,
                "date" => ColumnType::Date,
                "bytes" => ColumnType::Bytes,
                "json" => ColumnType::Json,
                _ => return None,
            },
            FieldType::Enum(e) => ColumnType::Enum(
                d.enums
                    .iter()
                    .find(|x| x.name == e)
                    .map(|x| x.values.clone())?,
            ),
            FieldType::Ref { plugin, entity } => ColumnType::Ref(format!("{plugin}.{entity}")),
            FieldType::Message(_) => return None,
        })
    }

    /// A partition column's type: exact values of a small domain.
    pub fn partitions(&self) -> bool {
        matches!(
            self,
            ColumnType::String
                | ColumnType::Bool
                | ColumnType::Int32
                | ColumnType::Int64
                | ColumnType::Uuid
                | ColumnType::Date
                | ColumnType::Enum(_)
        )
    }

    /// A key column's type: compared exactly.
    pub fn keys(&self) -> bool {
        self.partitions() || matches!(self, ColumnType::Timestamp | ColumnType::Ref(_))
    }
}

/// A column of a lake table: a field of its row message.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// The field's wire type as the lock writes it (`optional decimal`).
    pub wire: String,
    pub ty: ColumnType,
    pub optional: bool,
}

/// The columns of a lake table, from its row message; an error names the first field that cannot be a column.
pub fn columns(d: &Descriptor, t: &LakeTable) -> Result<Vec<Column>, String> {
    let m = d
        .messages
        .iter()
        .find(|m| m.name == t.message)
        .ok_or_else(|| format!("{} declares no message {:?}", d.id, t.message))?;
    m.fields
        .iter()
        .map(|f| {
            if f.encrypted {
                return Err(format!("{} is encrypted: the lake keeps no encrypted field", f.name));
            }
            let ty = ColumnType::of(d, f).ok_or_else(|| {
                format!(
                    "{} is `{}`: a lake column is a scalar, an enum or a ref, never a list or a message",
                    f.name,
                    crate::lock::wire_type(f)
                )
            })?;
            Ok(Column {
                name: f.name.clone(),
                wire: crate::lock::wire_type(f),
                ty,
                optional: f.optional,
            })
        })
        .collect()
}

/// A layer's or a table's name: snake_case, at most 32 characters.
fn name_ok(s: &str) -> bool {
    s.len() <= 32 && crate::validate::snake(s)
}

/// Every problem of the descriptor's lake declarations (`E_LAKE_*`).
pub fn validate(d: &Descriptor) -> Vec<Error> {
    let mut errors = vec![];
    let mut err = |code: &'static str, path: String, message: String| {
        errors.push(Error::new(code, path, message))
    };
    if d.layers.len() > MAX_LAYERS || d.lake_tables.len() > MAX_TABLES {
        err(
            "E_LAKE_SIZE",
            "lake_tables".into(),
            format!(
                "the plugin declares {} layers and {} lake tables: at most {MAX_LAYERS} and {MAX_TABLES}",
                d.layers.len(),
                d.lake_tables.len()
            ),
        );
        return errors;
    }
    let mut layers = HashSet::new();
    for (i, l) in d.layers.iter().enumerate() {
        let p = format!(
            "layers[{}]",
            if l.name.is_empty() {
                i.to_string()
            } else {
                l.name.clone()
            }
        );
        if !name_ok(&l.name) {
            err(
                "E_LAKE_LAYER_NAME",
                format!("{p}.name"),
                format!(
                    "{:?} is not a layer name (snake_case, at most 32 characters)",
                    l.name
                ),
            );
        }
        if !layers.insert(l.name.as_str()) {
            err(
                "E_LAKE_LAYER_DUP",
                p,
                format!("layer {:?} is declared twice", l.name),
            );
        }
    }
    let roles: HashSet<&str> = d.roles.iter().map(|r| r.name.as_str()).collect();
    let mut seen = HashSet::new();
    for (i, t) in d.lake_tables.iter().enumerate() {
        let p = format!(
            "lake_tables[{}]",
            if t.name.is_empty() {
                i.to_string()
            } else {
                t.name.clone()
            }
        );
        if !name_ok(&t.name) || t.name.contains("__") || t.name.ends_with('_') {
            err(
                "E_LAKE_TABLE_NAME",
                format!("{p}.name"),
                format!(
                    "{:?} is not a lake table name (snake_case, at most 32 characters, no `__`, not ending in `_`)",
                    t.name
                ),
            );
        }
        if !seen.insert(t.name.as_str()) {
            err(
                "E_LAKE_TABLE_DUP",
                p.clone(),
                format!("lake table {:?} is declared twice", t.name),
            );
        }
        if d.entities.iter().any(|e| e.name == t.name) || d.views.iter().any(|v| v.name == t.name) {
            err(
                "E_LAKE_TABLE_CLASH",
                p.clone(),
                format!(
                    "{:?} is the name of an entity or a view of the plugin",
                    t.name
                ),
            );
        }
        if !layers.contains(t.layer.as_str()) {
            err(
                "E_LAKE_TABLE_LAYER",
                format!("{p}.layer"),
                format!(
                    "{:?} is not a layer of the plugin: declare it with the plugin's layers",
                    t.layer
                ),
            );
        }
        for (j, r) in t.read.iter().enumerate() {
            if !roles.contains(r.as_str()) {
                err(
                    "E_LAKE_ROLE",
                    format!("{p}.read[{j}]"),
                    format!("{r:?} is not a role of the plugin"),
                );
            }
        }
        let cols = match columns(d, t) {
            Ok(c) => c,
            Err(m) => {
                let code = if m.contains("encrypted") {
                    "E_LAKE_ENCRYPTED"
                } else if m.contains("declares no message") {
                    "E_LAKE_TABLE_MESSAGE"
                } else {
                    "E_LAKE_FIELD_TYPE"
                };
                err(code, format!("{p}.message"), m);
                continue;
            }
        };
        if cols.is_empty() || cols.len() > MAX_COLUMNS {
            err(
                "E_LAKE_SIZE",
                format!("{p}.message"),
                format!(
                    "{} has {} fields: a lake table has 1 to {MAX_COLUMNS} columns",
                    t.message,
                    cols.len()
                ),
            );
        }
        for (what, list, max, fits) in [
            (
                "partition",
                &t.partition,
                MAX_PARTITION,
                ColumnType::partitions as fn(&ColumnType) -> bool,
            ),
            (
                "key",
                &t.key,
                MAX_KEY,
                ColumnType::keys as fn(&ColumnType) -> bool,
            ),
        ] {
            let code = if what == "partition" {
                "E_LAKE_PARTITION"
            } else {
                "E_LAKE_KEY"
            };
            if list.len() > max {
                err(
                    code,
                    format!("{p}.{what}"),
                    format!("at most {max} {what} columns"),
                );
            }
            let mut once = HashSet::new();
            for (j, name) in list.iter().enumerate() {
                let at = format!("{p}.{what}[{j}]");
                if !once.insert(name.as_str()) {
                    err(code, at, format!("{name} is named twice"));
                    continue;
                }
                match cols.iter().find(|c| c.name == *name) {
                    None => err(code, at, format!("{} has no field {name}", t.message)),
                    Some(c) if c.optional => err(
                        code,
                        at,
                        format!("{name} is optional: a {what} column always has a value"),
                    ),
                    Some(c) if !fits(&c.ty) => err(
                        code,
                        at,
                        format!(
                            "{name} is `{}`: a {what} column is {}",
                            c.wire,
                            if what == "partition" {
                                "a string, bool, int32, int64, uuid, date or enum"
                            } else {
                                "a string, bool, int32, int64, uuid, date, timestamp, enum or ref"
                            }
                        ),
                    ),
                    Some(_) => {}
                }
            }
        }
    }
    for (i, dep) in d.dependencies.iter().enumerate() {
        let mut once = HashSet::new();
        for (j, t) in dep.lake.iter().enumerate() {
            if !name_ok(t) || !once.insert(t.as_str()) {
                err(
                    "E_LAKE_DEPENDENCY",
                    format!("dependencies[{i}].lake[{j}]"),
                    format!("{t:?} is not a lake table name, or is named twice"),
                );
            }
        }
    }
    errors
}

/// A lake table as the lock keeps it: what a later version may not change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Locked {
    pub layer: String,
    #[serde(default)]
    pub partition: Vec<String>,
    #[serde(default)]
    pub key: Vec<String>,
    /// column -> its wire type
    #[serde(default)]
    pub columns: BTreeMap<String, String>,
}

/// The lake tables of a descriptor as the lock keeps them (a table whose message is missing has no columns).
pub fn locked(d: &Descriptor) -> BTreeMap<String, Locked> {
    d.lake_tables
        .iter()
        .map(|t| {
            let columns = columns(d, t)
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.name, c.wire))
                .collect();
            (
                t.name.clone(),
                Locked {
                    layer: t.layer.clone(),
                    partition: t.partition.clone(),
                    key: t.key.clone(),
                    columns,
                },
            )
        })
        .collect()
}

/// The base of a wire type (`optional decimal` -> `decimal`).
fn base(wire: &str) -> &str {
    wire.strip_prefix("optional ").unwrap_or(wire)
}

/// What `new` breaks of the lake tables `prev` locked (`E_LAKE_*`): a table moved to another layer, other partition
/// or key columns, a column removed (or renamed) or of another type, a new column that is not optional. A table no
/// longer declared is no change (it stays in the lake). Each is breaking, and no version may make it.
pub fn changes(prev: &BTreeMap<String, Locked>, new: &Descriptor) -> Vec<Change> {
    let now = locked(new);
    let mut out = vec![];
    for (name, n) in &now {
        let Some(o) = prev.get(name) else { continue };
        let p = format!("lake_tables[{name}]");
        if o.layer != n.layer {
            out.push(Change::new(
                "E_LAKE_TABLE_LAYER_CHANGED",
                format!("{p}.layer"),
                format!("{name} moved from the layer {} to {}: a lake table stays in its layer (declare a new table)", o.layer, n.layer),
                true,
            ));
        }
        if o.partition != n.partition {
            out.push(Change::new(
                "E_LAKE_PARTITION_CHANGED",
                format!("{p}.partition"),
                format!("the partition columns of {name} changed from {:?} to {:?}: they stay (declare a new table)", o.partition, n.partition),
                true,
            ));
        }
        if o.key != n.key {
            out.push(Change::new(
                "E_LAKE_KEY_CHANGED",
                format!("{p}.key"),
                format!(
                    "the key of {name} changed from {:?} to {:?}: it stays (declare a new table)",
                    o.key, n.key
                ),
                true,
            ));
        }
        for (col, ty) in &o.columns {
            match n.columns.get(col) {
                None => out.push(Change::new(
                    "E_LAKE_COLUMN_REMOVED",
                    format!("{p}.columns[{col}]"),
                    format!("{name}.{col} is gone: a lake column stays (a rename is a new column; keep the old field)"),
                    true,
                )),
                Some(t) if base(t) != base(ty) => out.push(Change::new(
                    "E_LAKE_COLUMN_TYPE",
                    format!("{p}.columns[{col}]"),
                    format!("{name}.{col} changed from `{ty}` to `{t}`: a lake column keeps its type (add a new column)"),
                    true,
                )),
                Some(_) => {}
            }
        }
        for (col, ty) in &n.columns {
            if !o.columns.contains_key(col) && !ty.starts_with("optional ") {
                out.push(Change::new(
                    "E_LAKE_COLUMN_REQUIRED",
                    format!("{p}.columns[{col}]"),
                    format!("{name}.{col} is new and not optional: the rows written before have no value for it (make it Option)"),
                    true,
                ));
            }
        }
    }
    out
}
