//! An entity's history in the workspace's Data Lake (`history` on an entity and on its fields): every change of the
//! kept columns of a row, each its own row with its time and its author, written by the platform into a table of the
//! lake in a layer the plugin declares, where models and reads take it.
//!
//! The entity names the layer (`history.layer`); each kept column carries `history` (a flag per field, also when the
//! plugin said `all`: the SDKs set it on every column `all` covers, and leave out the encrypted ones and, without
//! `unmasked`, the masked ones; [`left_out`] says which and why). A kept column is a scalar, an enum or a ref, never
//! a list, a message or an encrypted field, and a masked one only with `unmasked` (the lake has no masks). Who reads
//! it is a list of the plugin's roles (`read`), the plugin's own system context only with `system_read`, another
//! plugin only when it is exported (`export`) and that plugin names the entity in `dependencies[].lake`.
//!
//! In the lake the history is `<layer>.<plugin>_<entity>`: the row's `id`, `valid_from`, `valid_to` (the row's next
//! change; none for the current version, equal to `valid_from` on a delete), `change` (`baseline`, `insert`,
//! `update`, `delete`, `restore`), `changed_by` and `changed_by_kind` (`person`, `service`, `job`, `system`,
//! `platform`, `anonymous`), `changed_by_plugin` (the plugin whose call made it), and the kept columns. A source of a query or a model is the entity with `history` set;
//! with `each_day` also, one row per day the version was in force at the day's end ([`DAY`]).
//!
//! Between versions the history only grows: its layer stays, a kept column keeps its type ([`changes`]); a column no
//! longer kept stays in the lake, absent in the rows after.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Descriptor, Entity, Error, Field,
    lake::{Column, ColumnType},
    lock::Change,
};

/// How an entity's history is kept.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct EntityHistory {
    /// A layer the plugin declares.
    pub layer: String,
    /// Every column is kept (each still carries its flag): the encrypted ones and, without `unmasked`, the masked
    /// ones are left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// Masked columns may be kept: their values go to the lake as they are.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unmasked: bool,
    /// The plugin's roles whose holders read the history (every row of it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    /// The plugin's own code reads it in its system context.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system_read: bool,
    /// Other plugins that name the entity in `dependencies[].lake` read it, as callers holding one of `read`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub export: bool,
}

/// The columns every history has besides the entity's `id` and its kept columns; no kept column takes their names.
pub const VALID_FROM: &str = "valid_from";
pub const VALID_TO: &str = "valid_to";
pub const CHANGE: &str = "change";
pub const CHANGED_BY: &str = "changed_by";
pub const CHANGED_BY_KIND: &str = "changed_by_kind";
pub const CHANGED_BY_PLUGIN: &str = "changed_by_plugin";
/// The day of a history read per day (`each_day`).
pub const DAY: &str = "day";
/// The position of a change in the history (internal: never a column of a query).
pub const SEQ: &str = "seq";
pub const RESERVED: &[&str] = &[
    VALID_FROM,
    VALID_TO,
    CHANGE,
    CHANGED_BY,
    CHANGED_BY_KIND,
    CHANGED_BY_PLUGIN,
    DAY,
    SEQ,
];

/// The kinds of a change and of its author.
pub const CHANGES: &[&str] = &["baseline", "insert", "update", "delete", "restore"];
pub const AUTHORS: &[&str] = &[
    "person",
    "service",
    "job",
    "system",
    "platform",
    "anonymous",
];

/// The name of a query's or a model's source that is an entity's history (`ticket#history`), or its history per day
/// (`ticket#history_days`); `retail.ticket#history` for another plugin's.
pub fn source_name(entity: &str, each_day: bool) -> String {
    if each_day {
        format!("{entity}#history_days")
    } else {
        format!("{entity}#history")
    }
}

/// The entity (`ticket`, `retail.ticket`) and whether per day, of a source name of a history; none for any other.
pub fn parse_source(name: &str) -> Option<(&str, bool)> {
    if let Some(e) = name.strip_suffix("#history_days") {
        return Some((e, true));
    }
    name.strip_suffix("#history").map(|e| (e, false))
}

/// The fields of an entity's that are masked.
fn masked(e: &Entity) -> Vec<&str> {
    e.access
        .as_ref()
        .map(|a| a.masks.iter().map(|m| m.field.as_str()).collect())
        .unwrap_or_default()
}

/// Why a field cannot be kept (none: it can), the masks aside.
fn unkeepable(d: &Descriptor, f: &Field) -> Option<String> {
    if f.encrypted {
        return Some("it is encrypted: the lake keeps no encrypted field".into());
    }
    if RESERVED.contains(&f.name.as_str()) {
        return Some(format!(
            "its name is a column every history has ({})",
            RESERVED.join(", ")
        ));
    }
    if ColumnType::of(d, f).is_none() {
        return Some(format!(
            "it is `{}`: a kept column is a scalar, an enum or a ref, never a list or a message",
            crate::lock::wire_type(f)
        ));
    }
    None
}

/// The entity's fields a history may keep: every field but `id`.
fn candidates(e: &Entity) -> impl Iterator<Item = &Field> {
    e.fields.iter().filter(|f| f.name != "id")
}

/// The fields `all` leaves out of an entity's history, and why: the encrypted ones, the masked ones without
/// `unmasked`, those of a type the lake cannot hold or with a history column's name. Empty without `all`.
pub fn left_out(d: &Descriptor, e: &Entity) -> Vec<(String, String)> {
    let Some(h) = e.history.as_ref().filter(|h| h.all) else {
        return vec![];
    };
    let masks = masked(e);
    candidates(e)
        .filter_map(|f| {
            let why = unkeepable(d, f).or_else(|| {
                (!h.unmasked && masks.contains(&f.name.as_str())).then(|| {
                    "it is masked: say unmasked to keep its values as they are".to_string()
                })
            })?;
            Some((f.name.clone(), why))
        })
        .collect()
}

/// The fields `all` covers: every field `left_out` does not name (none without `all`).
pub fn covered(d: &Descriptor, e: &Entity) -> Vec<String> {
    if !e.history.as_ref().is_some_and(|h| h.all) {
        return vec![];
    }
    let out: HashSet<String> = left_out(d, e).into_iter().map(|(f, _)| f).collect();
    candidates(e)
        .filter(|f| !out.contains(&f.name))
        .map(|f| f.name.clone())
        .collect()
}

/// The kept columns of an entity (its fields marked `history`), as lake columns; a field that cannot be one is left
/// out (the check refuses such a descriptor).
pub fn columns(d: &Descriptor, e: &Entity) -> Vec<Column> {
    let Some(h) = &e.history else {
        return vec![];
    };
    // the masks again (a descriptor made by hand may mark a masked column without `unmasked`)
    let masks = masked(e);
    candidates(e)
        .filter(|f| f.history && unkeepable(d, f).is_none())
        .filter(|f| h.unmasked || !masks.contains(&f.name.as_str()))
        .filter_map(|f| {
            Some(Column {
                name: f.name.clone(),
                wire: crate::lock::wire_type(f),
                ty: ColumnType::of(d, f)?,
                optional: true,
            })
        })
        .collect()
}

/// The fields of an entity's history as a source of a model's tree (`crate::view::check_model`): `id` (a ref to the
/// entity), the kept columns (optional: a row written before a column was kept has none), `valid_from`, `valid_to`,
/// `change`, `changed_by`, `changed_by_kind`, and `day` per day.
pub fn source_fields(owner: &str, d: &Descriptor, e: &Entity, each_day: bool) -> Vec<Field> {
    let f = |name: &str, ty: String, optional: bool| Field {
        name: name.into(),
        number: 0,
        ty,
        repeated: false,
        optional,
        description: String::new(),
        unique: false,
        index: false,
        encrypted: false,
        searchable: String::new(),
        nav: String::new(),
        inverse: String::new(),
        on_delete: String::new(),
        history: false,
    };
    let mut out = vec![f("id", format!("ref:{owner}.{}", e.name), false)];
    for c in columns(d, e) {
        let base = c.wire.strip_prefix("optional ").unwrap_or(&c.wire);
        let mut x = f(&c.name, base.to_string(), true);
        x.ty = e
            .fields
            .iter()
            .find(|y| y.name == c.name)
            .map(|y| y.ty.clone())
            .unwrap_or(x.ty);
        out.push(x);
    }
    out.push(f(VALID_FROM, "timestamp".into(), false));
    out.push(f(VALID_TO, "timestamp".into(), true));
    out.push(f(CHANGE, "string".into(), false));
    out.push(f(CHANGED_BY, "ref:platform.user".into(), true));
    out.push(f(CHANGED_BY_KIND, "string".into(), false));
    out.push(f(CHANGED_BY_PLUGIN, "string".into(), true));
    if each_day {
        out.push(f(DAY, "date".into(), false));
    }
    out
}

/// Whether an entity's rows are read under rules of the row (relations, conditions) beyond a role.
fn row_rules(e: &Entity) -> bool {
    e.access.as_ref().is_some_and(|a| {
        !a.relations.is_empty()
            || a.read.as_ref().is_some_and(|g| {
                let text = serde_json::to_string(g).unwrap_or_default();
                ["\"rel\"", "\"via\"", "\"when\"", "\"holds\""]
                    .iter()
                    .any(|k| text.contains(k))
            })
    })
}

/// What `celobus plugin check` notes (not errors): a history, or a lake table a model fills from an entity, is read
/// by the holders of its roles, every row, without the entity's row rules or masks: (path, note).
pub fn notes(d: &Descriptor) -> Vec<(String, String)> {
    let roles = |r: &[String]| {
        if r.is_empty() {
            "no member (only the plugin's system context, if it says so)".to_string()
        } else {
            r.join(", ")
        }
    };
    let mut out = vec![];
    for e in &d.entities {
        let Some(h) = &e.history else { continue };
        let masked: Vec<String> = masked(e)
            .into_iter()
            .filter(|m| columns(d, e).iter().any(|c| c.name == *m))
            .map(str::to_string)
            .collect();
        if row_rules(e) || !masked.is_empty() {
            out.push((
                format!("entities[{}].history", e.name),
                format!(
                    "the history of {} is read by {} without the entity's rules{}: every row, every version",
                    e.name,
                    roles(&h.read),
                    if masked.is_empty() {
                        String::new()
                    } else {
                        format!(" and with the masked columns {} as they are", masked.join(", "))
                    }
                ),
            ));
        }
    }
    for m in &d.lake_models {
        let ruled: Vec<String> = crate::lake_model::entities(d, m)
            .into_iter()
            .filter(|n| d.entities.iter().any(|e| e.name == *n && row_rules(e)))
            .collect();
        if ruled.is_empty() {
            continue;
        }
        let read = d
            .lake_tables
            .iter()
            .find(|t| t.name == m.table)
            .map(|t| t.read.clone())
            .unwrap_or_default();
        out.push((
            format!("lake_models[{}]", m.table),
            format!(
                "the lake table {} is computed from {} and read by {} without their rules: every row",
                m.table,
                ruled.join(", "),
                roles(&read)
            ),
        ));
    }
    out
}

/// Every problem of the descriptor's histories (`E_HISTORY_*`).
pub fn validate(d: &Descriptor) -> Vec<Error> {
    let mut errors = vec![];
    let layers: HashSet<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
    let roles: HashSet<&str> = d.roles.iter().map(|r| r.name.as_str()).collect();
    for e in &d.entities {
        let p = format!("entities[{}]", e.name);
        let Some(h) = &e.history else {
            if let Some(f) = candidates(e).find(|f| f.history) {
                errors.push(Error::new(
                    "E_HISTORY_LAYER",
                    format!("{p}.fields[{}].history", f.name),
                    format!(
                        "{}.{} is kept, but the entity keeps no history: declare history(layer = ..) on it",
                        e.name, f.name
                    ),
                ));
            }
            continue;
        };
        // its relations in the lake (`<plugin>_<entity>` and `<plugin>_<entity>__history`) beside the plugin's lake
        // tables (`<plugin>_<table>`, `<plugin>_<table>__writes`): a name with `__` could take one of theirs
        if e.name.contains("__") || e.name.ends_with('_') {
            errors.push(Error::new(
                "E_HISTORY_NAME",
                format!("{p}.name"),
                format!(
                    "{} keeps a history: its name has no `__` and does not end in `_` (its history's tables in the \
                     lake would take another table's name)",
                    e.name
                ),
            ));
        }
        if !layers.contains(h.layer.as_str()) {
            errors.push(Error::new(
                "E_HISTORY_LAYER",
                format!("{p}.history.layer"),
                format!(
                    "{:?} is not a layer of the plugin: declare it with the plugin's layers",
                    h.layer
                ),
            ));
        }
        if !matches!(e.key.as_str(), "uuid" | "i64" | "string") {
            errors.push(Error::new(
                "E_HISTORY_KEY",
                format!("{p}.key"),
                format!(
                    "{} is keyed by {:?}: a history keys its rows by an entity's uuid, i64 or string id",
                    e.name, e.key
                ),
            ));
        }
        for (j, r) in h.read.iter().enumerate() {
            if !roles.contains(r.as_str()) {
                errors.push(Error::new(
                    "E_HISTORY_ROLE",
                    format!("{p}.history.read[{j}]"),
                    format!("{r:?} is not a role of the plugin"),
                ));
            }
        }
        let masks = masked(e);
        for f in candidates(e).filter(|f| f.history) {
            let at = format!("{p}.fields[{}].history", f.name);
            if let Some(why) = unkeepable(d, f) {
                let code = if f.encrypted {
                    "E_HISTORY_ENCRYPTED"
                } else if RESERVED.contains(&f.name.as_str()) {
                    "E_HISTORY_COLUMN"
                } else {
                    "E_HISTORY_TYPE"
                };
                errors.push(Error::new(
                    code,
                    at,
                    format!("{}.{} cannot be kept: {why}", e.name, f.name),
                ));
            } else if masks.contains(&f.name.as_str()) && !h.unmasked {
                errors.push(Error::new(
                    "E_HISTORY_MASKED",
                    at,
                    format!(
                        "{}.{} is masked: the lake has no masks, so say that its values go there as they are \
                         (history(.., unmasked))",
                        e.name, f.name
                    ),
                ));
            }
        }
        if h.all {
            let flagged: HashSet<&str> = candidates(e)
                .filter(|f| f.history)
                .map(|f| f.name.as_str())
                .collect();
            for f in covered(d, e) {
                if !flagged.contains(f.as_str()) {
                    errors.push(Error::new(
                        "E_HISTORY_ALL",
                        format!("{p}.fields[{f}].history"),
                        format!(
                            "the history of {} keeps every column (all), but {f} does not carry its flag",
                            e.name
                        ),
                    ));
                }
            }
        }
    }
    errors
}

/// An entity's history as the lock keeps it: what a later version may not change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Locked {
    pub layer: String,
    /// column -> its wire type, every column ever kept
    #[serde(default)]
    pub columns: BTreeMap<String, String>,
}

/// The histories of a descriptor as the lock keeps them.
pub fn locked(d: &Descriptor) -> BTreeMap<String, Locked> {
    d.entities
        .iter()
        .filter_map(|e| {
            let h = e.history.as_ref()?;
            Some((
                e.name.clone(),
                Locked {
                    layer: h.layer.clone(),
                    columns: columns(d, e)
                        .into_iter()
                        .map(|c| {
                            let base = c.wire.strip_prefix("optional ").unwrap_or(&c.wire);
                            (c.name, base.to_string())
                        })
                        .collect(),
                },
            ))
        })
        .collect()
}

/// What `new` breaks of the histories `prev` locked (`E_HISTORY_*`, never allowed): a history moved to another layer,
/// a column kept again (or still) with another type. A history no longer kept, or a column no longer kept, is no
/// change (it stays in the lake).
pub fn changes(prev: &BTreeMap<String, Locked>, new: &Descriptor) -> Vec<Change> {
    let mut out = vec![];
    for (name, n) in locked(new) {
        let Some(o) = prev.get(&name) else { continue };
        let p = format!("entities[{name}].history");
        if o.layer != n.layer {
            out.push(Change::new(
                "E_HISTORY_LAYER_CHANGED",
                format!("{p}.layer"),
                format!(
                    "the history of {name} moved from the layer {} to {}: a history stays in its layer",
                    o.layer, n.layer
                ),
                true,
            ));
        }
        for (col, ty) in &n.columns {
            if let Some(was) = o.columns.get(col)
                && was != ty
            {
                out.push(Change::new(
                    "E_HISTORY_COLUMN_TYPE",
                    format!("{p}.columns[{col}]"),
                    format!(
                        "{name}.{col} was kept as `{was}` and is `{ty}` now: a kept column keeps its type in the lake \
                         (keep a new field)"
                    ),
                    true,
                ));
            }
        }
    }
    out
}
