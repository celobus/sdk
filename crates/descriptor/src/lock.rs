//! `celobus.lock`: the plugin's field numbers and the dependencies it was built against, and the breaking-change
//! check.
//!
//! No field number is in code. The SDKs print the declaration position as `number`; the lock gives every field of
//! every message a number that stays: an existing field keeps its number, a new field takes the next free one, a
//! removed field's number is reserved and never given again, and the fields of a message new to the lock are
//! numbered in declaration order. Enum values are numbered the same way from 1 (0 is `UNSPECIFIED` on the wire).
//! [`update`] computes the next lock from the previous one and a descriptor, with the changes it saw;
//! [`apply_numbers`] writes the locked numbers into the descriptor JSON that goes into the package; [`compare`]
//! is the publish check between the last published descriptor and a new one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Auth, DEFAULT_SCOPE, Descriptor, FieldType, Idempotency};

/// The lock format this crate reads and writes.
pub const LOCK_FORMAT: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    /// The format ([`LOCK_FORMAT`]).
    pub lock: u32,
    pub plugin: String,
    /// The version of the build that wrote the lock.
    #[serde(default)]
    pub version: String,
    /// The dependencies' contracts, pinned by `celobus plugin add`.
    #[serde(default, rename = "dependency", skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<LockedDependency>,
    #[serde(
        default,
        rename = "message",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub messages: BTreeMap<String, LockedMessage>,
    #[serde(default, rename = "enum", skip_serializing_if = "BTreeMap::is_empty")]
    pub enums: BTreeMap<String, LockedEnum>,
    #[serde(
        default,
        rename = "endpoint",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub endpoints: BTreeMap<String, LockedEndpoint>,
    /// How each entity is stored: what a later version cannot change without a new entity.
    #[serde(default, rename = "entity", skip_serializing_if = "BTreeMap::is_empty")]
    pub entities: BTreeMap<String, LockedEntity>,
    /// The lake tables' layers, partition and key columns and columns: what a later version cannot change
    /// ([`crate::lake::changes`]). Written only when the plugin has lake tables.
    #[serde(
        default,
        rename = "lake_table",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub lake_tables: BTreeMap<String, crate::lake::Locked>,
    /// The entities' histories: their layers and kept columns ([`crate::history::changes`]). Written only when the
    /// plugin keeps a history.
    #[serde(
        default,
        rename = "history",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub histories: BTreeMap<String, crate::history::Locked>,
}

/// An entity's storage: its key type and the options that shape its table and its rows' life.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedEntity {
    pub key: String,
    #[serde(default)]
    pub soft_delete: bool,
    #[serde(default)]
    pub version: bool,
    #[serde(default)]
    pub append_only: bool,
}

impl LockedEntity {
    fn of(e: &crate::Entity) -> Self {
        Self {
            key: e.key.clone(),
            soft_delete: e.soft_delete,
            version: e.version,
            append_only: e.append_only,
        }
    }
}

/// A dependency pinned by its descriptor's digest.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedDependency {
    /// The plugin id (what the code calls).
    pub plugin: String,
    /// The catalog name it was resolved from (`customers`, or `acme/crm` for a private plugin).
    pub name: String,
    pub version: String,
    /// `sha256:<hex>` of the dependency's canonical descriptor ([`descriptor_digest`]).
    pub digest: String,
    /// The endpoints this plugin calls; the generated client has only these.
    #[serde(default)]
    pub calls: Vec<String>,
    /// The exported entities this plugin reads (`ctx.query()`, declared SQL): entity -> field -> wire type, as the
    /// dependency's contract had them. A provider's change that breaks one of them is breaking at its publish
    /// (`E_CONTRACT_EXPORT_FIELD`), and `celobus plugin check` refuses a read the lock does not have.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reads: BTreeMap<String, BTreeMap<String, String>>,
    /// The dependency's lake tables its contract let another plugin's models copy (`export_models`), when it was
    /// pinned: `celobus plugin check` refuses a model over another of its tables (`E_LAKE_MODEL_SOURCE`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lake_models: Vec<String>,
    /// The dependency's entities stored in OLAP, when it was pinned: `celobus plugin check` refuses a ref to one
    /// (`E_OLAP_REF`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub olap_entities: Vec<String>,
}

/// The fields a dependent reads of `entity` of the provider `d`: `id` and every field with its wire type, when the
/// entity exports `read`.
pub fn read_contract(d: &Descriptor, entity: &str) -> Result<Vec<crate::ReadField>, String> {
    let e = d
        .entities
        .iter()
        .find(|e| e.name == entity)
        .ok_or_else(|| format!("{} has no entity {entity}", d.id))?;
    if !e
        .access
        .as_ref()
        .is_some_and(|a| a.export.iter().any(|p| p == "read"))
    {
        return Err(format!(
            "{}.{entity} does not export read: its provider keeps its rows to itself",
            d.id
        ));
    }
    Ok(e.fields
        .iter()
        .map(|f| crate::ReadField {
            name: f.name.clone(),
            ty: wire_type(f),
        })
        .collect())
}

/// What a read declaration needs that a provider's fields (`have`: field -> wire type) do not give.
pub fn read_mismatches(
    read: &crate::DependencyRead,
    have: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut out = vec![];
    for f in &read.fields {
        match have.get(&f.name) {
            None => out.push(format!("{} has no field {}", read.entity, f.name)),
            Some(t) if *t != f.ty => out.push(format!(
                "{}.{} is `{t}`, the read expects `{}`",
                read.entity, f.name, f.ty
            )),
            Some(_) => {}
        }
    }
    out
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedMessage {
    #[serde(default)]
    pub fields: BTreeMap<String, LockedField>,
    /// Numbers of removed fields, never given again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedField {
    pub number: u32,
    /// The wire type: `string`, `optional int64`, `repeated message:OrderLine`.
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedEnum {
    pub values: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedEndpoint {
    pub input: String,
    pub output: String,
}

/// A difference between two contracts. `breaking` changes need a new major version at publish.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub code: &'static str,
    pub path: String,
    pub message: String,
    pub breaking: bool,
}

impl Change {
    pub fn new(
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
        breaking: bool,
    ) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
            breaking,
        }
    }

    fn breaking(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
            breaking: true,
        }
    }
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = if self.breaking { "breaking" } else { "change" };
        write!(
            f,
            "{kind}[{}] at {}: {}",
            self.code, self.path, self.message
        )
    }
}

/// `sha256:<hex>` of a descriptor's canonical JSON: how a dependent pins it.
pub fn descriptor_digest(canonical: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    let out = h.finalize();
    let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The wire type of a field as the lock records it.
pub fn wire_type(f: &crate::Field) -> String {
    if f.repeated {
        format!("repeated {}", f.ty)
    } else if f.optional {
        format!("optional {}", f.ty)
    } else {
        f.ty.clone()
    }
}

/// `repeated`-ness and the base type: what must stay for a field to keep its number.
fn identity(ty: &str) -> (&str, bool) {
    if let Some(t) = ty.strip_prefix("repeated ") {
        (t, true)
    } else if let Some(t) = ty.strip_prefix("optional ") {
        (t, false)
    } else {
        (ty, false)
    }
}

fn is_optional(ty: &str) -> bool {
    ty.starts_with("optional ") || ty.starts_with("repeated ")
}

/// A JSON reader cannot default these when they are missing, so a request without one is refused (400).
fn has_no_default(base: &str) -> bool {
    matches!(base, "uuid" | "timestamp" | "date" | "json") || base.starts_with("ref:")
}

/// The messages that are part of a request: endpoint inputs and the messages their fields use, transitively.
fn request_messages(d: &Descriptor) -> BTreeSet<String> {
    let by_name: HashMap<&str, &crate::Message> =
        d.messages.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = d.endpoints.iter().map(|e| e.input.clone()).collect();
    while let Some(n) = todo.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(m) = by_name.get(n.as_str()) {
            for f in &m.fields {
                if let Some(FieldType::Message(x)) = FieldType::parse(&f.ty) {
                    todo.push(x);
                }
            }
        }
    }
    seen
}

/// What [`update`] found.
#[derive(Clone, Debug)]
pub struct Update {
    pub lock: Lock,
    pub changes: Vec<Change>,
}

impl Update {
    pub fn breaking(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.breaking)
    }
}

/// The lock a descriptor's own numbers describe (a published descriptor carries its locked numbers).
pub fn lock_of(d: &Descriptor) -> Lock {
    let mut lock = Lock {
        lock: LOCK_FORMAT,
        plugin: d.id.clone(),
        version: d.version.clone(),
        ..Lock::default()
    };
    for m in &d.messages {
        lock.messages.insert(
            m.name.clone(),
            LockedMessage {
                fields: m
                    .fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.clone(),
                            LockedField {
                                number: f.number,
                                ty: wire_type(f),
                            },
                        )
                    })
                    .collect(),
                reserved: vec![],
            },
        );
    }
    for e in &d.enums {
        lock.enums.insert(
            e.name.clone(),
            LockedEnum {
                values: e
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (v.clone(), i as u32 + 1))
                    .collect(),
                reserved: vec![],
            },
        );
    }
    for e in &d.endpoints {
        lock.endpoints.insert(
            e.name.clone(),
            LockedEndpoint {
                input: e.input.clone(),
                output: e.output.clone(),
            },
        );
    }
    for e in &d.entities {
        lock.entities.insert(e.name.clone(), LockedEntity::of(e));
    }
    lock.lake_tables = crate::lake::locked(d);
    lock.histories = crate::history::locked(d);
    lock
}

/// The next lock: `prev` (none for a plugin's first build) moved to the descriptor `d`. Dependencies are kept
/// as they are (`celobus plugin add` changes them).
pub fn update(prev: Option<&Lock>, d: &Descriptor) -> Update {
    let empty = Lock::default();
    let prev = prev.unwrap_or(&empty);
    let requests = request_messages(d);
    let mut changes = vec![];
    let mut lock = Lock {
        lock: LOCK_FORMAT,
        plugin: d.id.clone(),
        version: d.version.clone(),
        dependencies: prev.dependencies.clone(),
        ..Lock::default()
    };

    for m in &d.messages {
        let path = format!("messages[{}]", m.name);
        let Some(old) = prev.messages.get(&m.name) else {
            // new to the lock: declaration order
            lock.messages.insert(
                m.name.clone(),
                LockedMessage {
                    fields: m
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            (
                                f.name.clone(),
                                LockedField {
                                    number: i as u32 + 1,
                                    ty: wire_type(f),
                                },
                            )
                        })
                        .collect(),
                    reserved: vec![],
                },
            );
            continue;
        };
        let mut reserved = old.reserved.clone();
        let mut next = old
            .fields
            .values()
            .map(|f| f.number)
            .chain(old.reserved.iter().copied())
            .max()
            .unwrap_or(0)
            + 1;
        let mut fields = BTreeMap::new();
        for f in &m.fields {
            let fp = format!("{path}.fields[{}]", f.name);
            let ty = wire_type(f);
            let (base, repeated) = identity(&ty);
            match old.fields.get(&f.name) {
                Some(o) if identity(&o.ty) == (base, repeated) => {
                    if requests.contains(&m.name)
                        && is_optional(&o.ty)
                        && !is_optional(&ty)
                        && has_no_default(base)
                    {
                        changes.push(Change::breaking(
                            "E_CONTRACT_REQUEST_NARROWED",
                            fp,
                            format!("{} became required in a request", f.name),
                        ));
                    }
                    fields.insert(
                        f.name.clone(),
                        LockedField {
                            number: o.number,
                            ty,
                        },
                    );
                }
                Some(o) => {
                    changes.push(Change::breaking(
                        "E_CONTRACT_FIELD_TYPE",
                        fp,
                        format!(
                            "{} changed from `{}` to `{ty}`; it takes a new number ({next}), {} is reserved",
                            f.name, o.ty, o.number
                        ),
                    ));
                    reserved.push(o.number);
                    fields.insert(f.name.clone(), LockedField { number: next, ty });
                    next += 1;
                }
                None => {
                    if requests.contains(&m.name) && !is_optional(&ty) && has_no_default(base) {
                        changes.push(Change::breaking(
                            "E_CONTRACT_REQUEST_NARROWED",
                            fp,
                            format!(
                                "{} is a new required field of a request: callers that do not send it are refused",
                                f.name
                            ),
                        ));
                    }
                    fields.insert(f.name.clone(), LockedField { number: next, ty });
                    next += 1;
                }
            }
        }
        for (name, o) in &old.fields {
            if !fields.contains_key(name) {
                changes.push(Change::breaking(
                    "E_CONTRACT_FIELD_REMOVED",
                    format!("{path}.fields[{name}]"),
                    format!("{name} was removed; number {} is reserved", o.number),
                ));
                reserved.push(o.number);
            }
        }
        reserved.sort_unstable();
        reserved.dedup();
        lock.messages
            .insert(m.name.clone(), LockedMessage { fields, reserved });
    }

    for e in &d.enums {
        let path = format!("enums[{}]", e.name);
        let old = prev.enums.get(&e.name);
        let mut reserved = old.map(|o| o.reserved.clone()).unwrap_or_default();
        let mut next = old
            .map(|o| {
                o.values
                    .values()
                    .copied()
                    .chain(o.reserved.iter().copied())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
            + 1;
        let mut values = BTreeMap::new();
        for v in &e.values {
            let n = match old.and_then(|o| o.values.get(v)) {
                Some(n) => *n,
                None => {
                    next += 1;
                    next - 1
                }
            };
            values.insert(v.clone(), n);
        }
        if let Some(o) = old {
            for (v, n) in &o.values {
                if !values.contains_key(v) {
                    changes.push(Change::breaking(
                        "E_CONTRACT_ENUM_VALUE_REMOVED",
                        format!("{path}.values[{v}]"),
                        format!("value {v} was removed; number {n} is reserved"),
                    ));
                    reserved.push(*n);
                }
            }
        }
        reserved.sort_unstable();
        reserved.dedup();
        lock.enums
            .insert(e.name.clone(), LockedEnum { values, reserved });
    }

    for e in &d.endpoints {
        let now = LockedEndpoint {
            input: e.input.clone(),
            output: e.output.clone(),
        };
        if let Some(o) = prev.endpoints.get(&e.name)
            && *o != now
        {
            changes.push(Change::breaking(
                "E_CONTRACT_ENDPOINT_TYPE",
                format!("endpoints[{}]", e.name),
                format!(
                    "{}({}) -> {} became {}({}) -> {}",
                    e.name, o.input, o.output, e.name, now.input, now.output
                ),
            ));
        }
        lock.endpoints.insert(e.name.clone(), now);
    }
    for name in prev.endpoints.keys() {
        if !lock.endpoints.contains_key(name) {
            changes.push(Change::breaking(
                "E_CONTRACT_ENDPOINT_REMOVED",
                format!("endpoints[{name}]"),
                format!("endpoint {name} was removed"),
            ));
        }
    }
    changes.extend(entity_changes(&prev.entities, d));
    lock.entities = d
        .entities
        .iter()
        .map(|e| (e.name.clone(), LockedEntity::of(e)))
        .collect();
    // a lake table only grows; one no longer declared stays locked (its table stays in the lake)
    changes.extend(crate::lake::changes(&prev.lake_tables, d));
    lock.lake_tables = prev.lake_tables.clone();
    lock.lake_tables.extend(crate::lake::locked(d));
    // a history only grows too: its layer and the types of the columns it ever kept stay locked
    changes.extend(crate::history::changes(&prev.histories, d));
    lock.histories = prev.histories.clone();
    for (name, h) in crate::history::locked(d) {
        let mut columns = lock
            .histories
            .get(&name)
            .map(|o| o.columns.clone())
            .unwrap_or_default();
        columns.extend(h.columns);
        lock.histories.insert(
            name,
            crate::history::Locked {
                layer: h.layer,
                columns,
            },
        );
    }
    Update { lock, changes }
}

/// An entity's table outlives a version: its key type and its storage options stay, and it is not removed (a new
/// entity and a backfill instead).
fn entity_changes(prev: &BTreeMap<String, LockedEntity>, d: &Descriptor) -> Vec<Change> {
    let mut changes = vec![];
    for e in &d.entities {
        let Some(o) = prev.get(&e.name) else {
            continue;
        };
        let path = format!("entities[{}]", e.name);
        if o.key != e.key {
            changes.push(Change::breaking(
                "E_CONTRACT_KEY_TYPE",
                format!("{path}.key"),
                format!("the key of {} changed from {} to {}", e.name, o.key, e.key),
            ));
        }
        for (what, was, now) in [
            ("soft_delete", o.soft_delete, e.soft_delete),
            ("version", o.version, e.version),
            ("append_only", o.append_only, e.append_only),
        ] {
            if was != now {
                changes.push(Change::breaking(
                    "E_CONTRACT_ENTITY_STORAGE",
                    format!("{path}.{what}"),
                    format!("{what} of {} changed from {was} to {now}", e.name),
                ));
            }
        }
    }
    for name in prev.keys() {
        if !d.entities.iter().any(|e| &e.name == name) {
            changes.push(Change::breaking(
                "E_CONTRACT_ENTITY_REMOVED",
                format!("entities[{name}]"),
                format!("entity {name} was removed; its table stays"),
            ));
        }
    }
    changes
}

/// Writes the locked field numbers into a descriptor's JSON (`messages[].fields[].number`, and the entities'
/// fields through their row message), so the package's `descriptor.json` carries them.
pub fn apply_numbers(descriptor: &mut Value, lock: &Lock) {
    let number = |msg: &str, field: &str| {
        lock.messages
            .get(msg)
            .and_then(|m| m.fields.get(field))
            .map(|f| f.number)
    };
    if let Some(ms) = descriptor.get_mut("messages").and_then(Value::as_array_mut) {
        for m in ms {
            let name = m["name"].as_str().unwrap_or_default().to_string();
            renumber(m, |f| number(&name, f));
        }
    }
    if let Some(es) = descriptor.get_mut("entities").and_then(Value::as_array_mut) {
        for e in es {
            let msg = e["message"].as_str().unwrap_or_default().to_string();
            renumber(e, |f| number(&msg, f));
        }
    }
}

fn renumber(holder: &mut Value, number: impl Fn(&str) -> Option<u32>) {
    if let Some(fs) = holder.get_mut("fields").and_then(Value::as_array_mut) {
        for f in fs {
            let name = f["name"].as_str().unwrap_or_default().to_string();
            if let Some(n) = number(&name) {
                f["number"] = Value::from(n);
            }
        }
    }
}

/// Whether `new` may break `old`'s contract: a higher major version (or, before 1.0, a higher minor).
pub fn is_major_bump(old: &str, new: &str) -> bool {
    match (semver::Version::parse(old), semver::Version::parse(new)) {
        (Ok(o), Ok(n)) => {
            if o.major == 0 && n.major == 0 {
                n.minor > o.minor
            } else {
                n.major > o.major
            }
        }
        _ => false,
    }
}

/// The publish check: every change from the last published descriptor `old` to `new`, wire and gates.
/// Changes with `code` `E_CONTRACT_GATE_NARROWED` are the ones `--security-fix` lets through.
pub fn compare(old: &Descriptor, new: &Descriptor) -> Vec<Change> {
    let base = lock_of(old);
    let up = update(Some(&base), new);
    let mut changes = up.changes;
    // a field that kept its name but not its number (the lock was lost or edited)
    for m in &new.messages {
        for f in &m.fields {
            if let Some(want) = up
                .lock
                .messages
                .get(&m.name)
                .and_then(|x| x.fields.get(&f.name))
                && base
                    .messages
                    .get(&m.name)
                    .is_some_and(|x| x.fields.contains_key(&f.name))
                && want.number != f.number
            {
                changes.push(Change::breaking(
                    "E_CONTRACT_FIELD_NUMBER",
                    format!("messages[{}].fields[{}]", m.name, f.name),
                    format!(
                        "{} was published as number {}, this build numbers it {}",
                        f.name, want.number, f.number
                    ),
                ));
            }
        }
    }
    changes.extend(gates(old, new));
    changes.extend(exports(old, new));
    changes.extend(crate::ui_check::ui_changes(old, new));
    changes
}

/// The permissions an entity exports for `via` are a contract with the plugins that build on them: removing one
/// is breaking; changing its grant changes what the dependents grant (a warning, named in the plan). An exported
/// `read` is also what dependents read (`ctx.query()`, declared SQL): a field of it removed or retyped is breaking, a
/// new mask on one of its fields changes what they read (a warning).
fn exports(old: &Descriptor, new: &Descriptor) -> Vec<Change> {
    let mut out = vec![];
    for o in &old.entities {
        let Some(oa) = &o.access else { continue };
        if oa.export.iter().any(|p| p == "read")
            && let Some(n) = new.entities.iter().find(|n| n.name == o.name)
        {
            for f in &o.fields {
                let path = format!("entities[{}].fields[{}]", o.name, f.name);
                match n.fields.iter().find(|x| x.name == f.name) {
                    None => out.push(Change::breaking(
                        "E_CONTRACT_EXPORT_FIELD",
                        path,
                        format!(
                            "{}.{} is gone: the plugins that read {} lose it",
                            o.name, f.name, o.name
                        ),
                    )),
                    Some(x) if wire_type(x) != wire_type(f) => out.push(Change::breaking(
                        "E_CONTRACT_EXPORT_FIELD",
                        path,
                        format!(
                            "{}.{} changed from `{}` to `{}`: the plugins that read {} read another type",
                            o.name,
                            f.name,
                            wire_type(f),
                            wire_type(x),
                            o.name
                        ),
                    )),
                    Some(_) => {}
                }
            }
            let masked = |a: Option<&crate::access::Access>, field: &str| {
                a.is_some_and(|a| a.masks.iter().any(|m| m.field == field))
            };
            for f in &n.fields {
                if !masked(Some(oa), &f.name) && masked(n.access.as_ref(), &f.name) {
                    out.push(Change::new(
                        "W_CONTRACT_EXPORT_CHANGED",
                        format!("entities[{}].access.masks", o.name),
                        format!(
                            "{}.{} is masked now: the plugins that read {} see it empty unless the reader holds the mask's roles",
                            o.name, f.name, o.name
                        ),
                        false,
                    ));
                }
            }
        }
        let na = new
            .entities
            .iter()
            .find(|n| n.name == o.name)
            .and_then(|n| n.access.as_ref());
        for perm in &oa.export {
            let path = format!("entities[{}].access.export", o.name);
            match na {
                Some(na) if na.export.contains(perm) => {
                    if na.grant(perm) != oa.grant(perm) {
                        out.push(Change::new(
                            "W_CONTRACT_EXPORT_CHANGED",
                            path,
                            format!(
                                "the exported {perm} grant of {} changed: the plugins that inherit it with via grant what it grants now",
                                o.name
                            ),
                            false,
                        ));
                    }
                }
                _ => out.push(Change::breaking(
                    "E_CONTRACT_EXPORT_REMOVED",
                    path,
                    format!(
                        "{} no longer exports {perm}: the plugins that inherit it with via lose it",
                        o.name
                    ),
                )),
            }
        }
    }
    out
}

/// The part of the version a release from `old` to `new` moves (semver).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    /// The next version after `v` for this part. Before 1.0 a breaking change moves the minor (as
    /// [`is_major_bump`] reads it); `literal` moves the major anyway (`0.3.1` → `1.0.0`).
    pub fn apply(self, v: &semver::Version, literal: bool) -> semver::Version {
        let (major, minor, patch) = match self {
            Bump::Major if v.major == 0 && !literal => (0, v.minor + 1, 0),
            Bump::Major => (v.major + 1, 0, 0),
            Bump::Minor => (v.major, v.minor + 1, 0),
            Bump::Patch => (v.major, v.minor, v.patch + 1),
        };
        semver::Version::new(major, minor, patch)
    }
}

/// What a release from the published `old` to `new` needs: major for a breaking change ([`compare`]), minor when
/// the contract grew (an endpoint, message, field, enum value, entity, role, scope, capability or agent), else
/// patch. The second value names what decided it.
pub fn bump_needed(old: &Descriptor, new: &Descriptor) -> (Bump, Vec<String>) {
    let breaking: Vec<String> = compare(old, new)
        .into_iter()
        .filter(|c| c.breaking)
        .map(|c| c.message)
        .collect();
    if !breaking.is_empty() {
        return (Bump::Major, breaking);
    }
    let (o, n) = (contract_names(old), contract_names(new));
    let added: Vec<String> = n.difference(&o).cloned().collect();
    if !added.is_empty() {
        return (Bump::Minor, added);
    }
    (Bump::Patch, vec![])
}

/// Everything a caller can name in a contract: `endpoint GetOrder`, `field Order.total`, `value Status.SHIPPED`, …
fn contract_names(d: &Descriptor) -> BTreeSet<String> {
    let v = serde_json::to_value(d).unwrap_or_default();
    let mut out = BTreeSet::new();
    let items = |key: &str| v[key].as_array().cloned().unwrap_or_default();
    for (key, kind) in [
        ("endpoints", "endpoint"),
        ("messages", "message"),
        ("entities", "entity"),
        ("enums", "enum"),
        ("roles", "role"),
        ("scopes", "scope"),
        ("capabilities", "capability"),
        ("agents", "agent"),
    ] {
        for it in items(key) {
            let Some(name) = it["name"].as_str() else {
                continue;
            };
            out.insert(format!("{kind} {name}"));
            for f in it["fields"].as_array().into_iter().flatten() {
                if let Some(f) = f["name"].as_str() {
                    out.insert(format!("field {name}.{f}"));
                }
            }
            for x in it["values"].as_array().into_iter().flatten() {
                if let Some(x) = x.as_str() {
                    out.insert(format!("value {name}.{x}"));
                }
            }
        }
    }
    out
}

/// How widely an endpoint can be called; a narrower kind refuses callers that passed before.
fn auth_admits(a: Auth) -> &'static [Auth] {
    match a {
        Auth::Public => &[Auth::Public, Auth::Any, Auth::User, Auth::Service],
        Auth::Any => &[Auth::Any, Auth::User, Auth::Service],
        Auth::User => &[Auth::User],
        Auth::Service => &[Auth::Service],
    }
}

fn gates(old: &Descriptor, new: &Descriptor) -> Vec<Change> {
    let mut out = vec![];
    let scopes = |e: &crate::Endpoint| -> BTreeSet<String> {
        if e.scopes.is_empty() {
            [DEFAULT_SCOPE.to_string()].into()
        } else {
            e.scopes.iter().cloned().collect()
        }
    };
    for n in &new.endpoints {
        let Some(o) = old.endpoints.iter().find(|o| o.name == n.name) else {
            continue;
        };
        let path = format!("endpoints[{}]", n.name);
        let mut why = vec![];
        let added: Vec<String> = scopes(n).difference(&scopes(o)).cloned().collect();
        if !added.is_empty() {
            why.push(format!("needs scope {}", added.join(", ")));
        }
        if o.roles.is_empty() && !n.roles.is_empty() {
            why.push(format!("needs a role ({})", n.roles.join(" or ")));
        } else {
            let gone: Vec<&String> = o.roles.iter().filter(|r| !n.roles.contains(r)).collect();
            if !gone.is_empty() && !n.roles.is_empty() {
                why.push(format!(
                    "no longer admits role {}",
                    gone.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if !auth_admits(o.auth)
            .iter()
            .all(|a| auth_admits(n.auth).contains(a))
        {
            why.push(format!("auth {:?} became {:?}", o.auth, n.auth).to_lowercase());
        }
        if n.idempotency == Idempotency::Required && o.idempotency != Idempotency::Required {
            why.push("requires an idempotency key".into());
        }
        if !why.is_empty() {
            out.push(Change::breaking(
                "E_CONTRACT_GATE_NARROWED",
                path,
                format!(
                    "{} {}: callers that passed may now be refused",
                    n.name,
                    why.join("; ")
                ),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn desc(v: Value) -> Descriptor {
        let mut base = json!({ "schema": 1, "id": "orders", "version": "1.0.0" });
        for (k, x) in v.as_object().unwrap() {
            base[k] = x.clone();
        }
        crate::parse(base.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn bump_needed_reads_breaks_additions_and_the_rest() {
        let old =
            desc(json!({ "messages": [{ "name": "Order", "fields": [field("id", "uuid")] }] }));
        let same = desc(json!({ "version": "1.0.1", "description": "now with docs",
            "messages": [{ "name": "Order", "fields": [field("id", "uuid")] }] }));
        let grown = desc(
            json!({ "messages": [{ "name": "Order", "fields": [field("id", "uuid"), field("note", "string")] }] }),
        );
        let broken = desc(json!({ "messages": [{ "name": "Order", "fields": [] }] }));
        assert_eq!(bump_needed(&old, &same).0, Bump::Patch);
        let (b, why) = bump_needed(&old, &grown);
        assert_eq!(
            (b, why),
            (Bump::Minor, vec!["field Order.note".to_string()])
        );
        assert_eq!(bump_needed(&old, &broken).0, Bump::Major);
        let v = |s: &str| semver::Version::parse(s).unwrap();
        assert_eq!(Bump::Major.apply(&v("0.3.1"), false), v("0.4.0"));
        assert_eq!(Bump::Major.apply(&v("0.3.1"), true), v("1.0.0"));
        assert_eq!(Bump::Minor.apply(&v("1.2.3"), false), v("1.3.0"));
        assert_eq!(Bump::Patch.apply(&v("1.2.3"), false), v("1.2.4"));
    }

    fn field(name: &str, ty: &str) -> Value {
        json!({ "name": name, "number": 0, "type": ty })
    }

    fn orders(fields: Vec<Value>) -> Descriptor {
        desc(json!({
            "messages": [
                { "name": "PlaceOrderRequest", "fields": fields },
                { "name": "Order", "fields": [field("id", "string")] }
            ],
            "endpoints": [{ "name": "PlaceOrder", "input": "PlaceOrderRequest", "output": "Order" }]
        }))
    }

    fn numbers(l: &Lock, msg: &str) -> Vec<(String, u32)> {
        l.messages[msg]
            .fields
            .iter()
            .map(|(k, f)| (k.clone(), f.number))
            .collect()
    }

    #[test]
    fn numbers_stay_and_removed_ones_are_reserved() {
        let first = update(
            None,
            &orders(vec![
                field("customer_id", "string"),
                field("quantity", "int32"),
            ]),
        );
        assert!(first.changes.is_empty());
        assert_eq!(
            numbers(&first.lock, "PlaceOrderRequest"),
            [("customer_id".into(), 1), ("quantity".into(), 2)]
        );
        // a field inserted before the others takes the next free number; the old ones keep theirs
        let second = update(
            Some(&first.lock),
            &orders(vec![
                field("note", "string"),
                field("customer_id", "string"),
                field("quantity", "int32"),
            ]),
        );
        assert!(second.changes.is_empty(), "{:?}", second.changes);
        assert_eq!(
            numbers(&second.lock, "PlaceOrderRequest")[1],
            ("note".into(), 3)
        );
        // a removed field: breaking, its number reserved and never given again
        let third = update(
            Some(&second.lock),
            &orders(vec![field("note", "string"), field("sku", "string")]),
        );
        let codes: Vec<&str> = third.changes.iter().map(|c| c.code).collect();
        assert_eq!(
            codes,
            ["E_CONTRACT_FIELD_REMOVED", "E_CONTRACT_FIELD_REMOVED"]
        );
        assert_eq!(third.lock.messages["PlaceOrderRequest"].reserved, [1, 2]);
        assert_eq!(
            numbers(&third.lock, "PlaceOrderRequest"),
            [("note".into(), 3), ("sku".into(), 4)]
        );
    }

    #[test]
    fn type_changes_and_narrowed_requests_are_breaking() {
        let first = update(None, &orders(vec![field("quantity", "int32")]));
        let up = update(
            Some(&first.lock),
            &orders(vec![
                field("quantity", "int64"),
                field("delivery_on", "date"),
                field("note", "string"),
            ]),
        );
        let codes: Vec<&str> = up.changes.iter().map(|c| c.code).collect();
        // int32 → int64 takes a new number; a new required date in a request refuses old callers; a string
        // defaults to "" and is compatible
        assert_eq!(
            codes,
            ["E_CONTRACT_FIELD_TYPE", "E_CONTRACT_REQUEST_NARROWED"]
        );
        assert_eq!(
            up.lock.messages["PlaceOrderRequest"].fields["quantity"].number,
            2
        );
        assert_eq!(up.lock.messages["PlaceOrderRequest"].reserved, [1]);
        // optional ↔ required keeps the number
        let opt = update(
            Some(&up.lock),
            &orders(vec![
                json!({ "name": "quantity", "number": 0, "type": "int64", "optional": true }),
                field("delivery_on", "date"),
                field("note", "string"),
            ]),
        );
        assert!(opt.changes.is_empty(), "{:?}", opt.changes);
        assert_eq!(
            opt.lock.messages["PlaceOrderRequest"].fields["quantity"].number,
            2
        );
    }

    #[test]
    fn enums_and_endpoints() {
        let d1 = desc(json!({
            "enums": [{ "name": "Status", "values": ["placed", "paid"] }],
            "messages": [{ "name": "R", "fields": [] }],
            "endpoints": [{ "name": "A", "input": "R", "output": "R" }, { "name": "B", "input": "R", "output": "R" }]
        }));
        let l1 = update(None, &d1).lock;
        assert_eq!(l1.enums["Status"].values["placed"], 1);
        let d2 = desc(json!({
            "enums": [{ "name": "Status", "values": ["shipped", "placed"] }],
            "messages": [{ "name": "R", "fields": [] }, { "name": "S", "fields": [] }],
            "endpoints": [{ "name": "A", "input": "S", "output": "R" }]
        }));
        let up = update(Some(&l1), &d2);
        let codes: Vec<&str> = up.changes.iter().map(|c| c.code).collect();
        assert_eq!(
            codes,
            [
                "E_CONTRACT_ENUM_VALUE_REMOVED",
                "E_CONTRACT_ENDPOINT_TYPE",
                "E_CONTRACT_ENDPOINT_REMOVED"
            ]
        );
        assert_eq!(up.lock.enums["Status"].values["shipped"], 3);
        assert_eq!(up.lock.enums["Status"].reserved, [2]);
    }

    #[test]
    fn apply_numbers_rewrites_the_package_descriptor() {
        let first = update(
            None,
            &orders(vec![field("a", "string"), field("b", "string")]),
        );
        let second = update(
            Some(&first.lock),
            &orders(vec![field("b", "string"), field("c", "string")]),
        );
        let mut v: Value =
            serde_json::to_value(orders(vec![field("b", "string"), field("c", "string")])).unwrap();
        apply_numbers(&mut v, &second.lock);
        let fs = &v["messages"][0]["fields"];
        assert_eq!(
            (fs[0]["number"].as_u64(), fs[1]["number"].as_u64()),
            (Some(2), Some(3))
        );
        // and the result is still a valid descriptor
        assert!(crate::check(v.to_string().as_bytes()).is_ok());
    }

    #[test]
    fn publish_compare_gates_and_versions() {
        let ep = |roles: Value, scopes: Value, auth: &str| {
            desc(json!({
                "messages": [{ "name": "R", "fields": [] }],
                "endpoints": [{ "name": "A", "input": "R", "output": "R", "roles": roles, "scopes": scopes, "auth": auth }]
            }))
        };
        let old = ep(json!(["clerk", "manager"]), json!([]), "any");
        // a role alternative added and the same default scope: compatible
        assert!(
            compare(
                &old,
                &ep(
                    json!(["clerk", "manager", "admin"]),
                    json!(["write"]),
                    "any"
                )
            )
            .is_empty()
        );
        let c = compare(&old, &ep(json!(["clerk"]), json!(["orders_write"]), "user"));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].code, "E_CONTRACT_GATE_NARROWED");
        assert!(c[0].message.contains("orders_write") && c[0].message.contains("manager"));
        assert!(
            compare(
                &ep(json!([]), json!([]), "any"),
                &ep(json!(["clerk"]), json!([]), "public")
            )
            .len()
                == 1
        );

        assert!(is_major_bump("1.4.2", "2.0.0"));
        assert!(!is_major_bump("1.4.2", "1.5.0"));
        assert!(is_major_bump("0.3.1", "0.4.0"));
        assert!(!is_major_bump("0.3.1", "0.3.2"));
    }

    #[test]
    fn publish_compare_sees_a_lost_lock() {
        let mut old = orders(vec![field("a", "string"), field("b", "string")]);
        old.messages[0].fields[0].number = 1;
        old.messages[0].fields[1].number = 2;
        let mut new = orders(vec![field("b", "string")]);
        new.messages[0].fields[0].number = 1; // renumbered from the declaration position
        let codes: Vec<&str> = compare(&old, &new).iter().map(|c| c.code).collect();
        assert_eq!(
            codes,
            ["E_CONTRACT_FIELD_REMOVED", "E_CONTRACT_FIELD_NUMBER"]
        );
    }

    #[test]
    fn proto_view() {
        let d = desc(json!({
            "description": "Orders.",
            "enums": [{ "name": "OrderStatus", "values": ["placed", "paid"] }],
            "messages": [
                { "name": "PlaceOrderRequest", "fields": [
                    { "name": "customer_id", "number": 3, "type": "ref:customers.customer" },
                    { "name": "lines", "number": 4, "type": "message:Line", "repeated": true },
                    { "name": "placed_at", "number": 5, "type": "timestamp", "optional": true } ] },
                { "name": "Line", "fields": [{ "name": "total", "number": 1, "type": "decimal" }] },
                { "name": "Order", "fields": [{ "name": "status", "number": 1, "type": "enum:OrderStatus" }] }
            ],
            "dependencies": [{ "plugin": "customers", "calls": [] }],
            "endpoints": [{ "name": "PlaceOrder", "input": "PlaceOrderRequest", "output": "Order", "roles": ["clerk"] }]
        }));
        let mut l = update(None, &d).lock;
        l.messages.get_mut("PlaceOrderRequest").unwrap().reserved = vec![1, 2];
        let p = crate::to_proto(&d, Some(&l));
        assert!(p.contains("package orders.v1;"));
        assert!(p.contains("import \"google/protobuf/timestamp.proto\";"));
        assert!(p.contains("rpc PlaceOrder(PlaceOrderRequest) returns (Order);   // /orders.v1.Orders/PlaceOrder · scope write · role clerk"));
        assert!(p.contains("  string customer_id = 3;   // ref customers.customer"));
        assert!(p.contains("  repeated Line lines = 4;"));
        assert!(p.contains("  optional google.protobuf.Timestamp placed_at = 5;"));
        assert!(p.contains("  reserved 1, 2;"));
        assert!(p.contains("  ORDER_STATUS_UNSPECIFIED = 0;\n  ORDER_STATUS_PLACED = 1;"));
        assert!(p.contains("  string total = 1;   // decimal"));
    }

    fn entity(key: &str, soft_delete: bool, version: bool, append_only: bool) -> Descriptor {
        let id = if key == "i64" { "int64" } else { key };
        desc(
            json!({ "entities": [{ "name": "note", "message": "Note", "key": key,
            "soft_delete": soft_delete, "version": version, "append_only": append_only,
            "fields": [field("id", id), field("title", "string")] }] }),
        )
    }

    #[test]
    fn entities_are_locked_and_their_storage_mode_stays() {
        let d = entity("uuid", true, true, false);
        let l = update(None, &d).lock;
        assert_eq!(
            l.entities["note"],
            LockedEntity {
                key: "uuid".into(),
                soft_delete: true,
                version: true,
                append_only: false
            }
        );
        let text = toml::to_string(&l).unwrap();
        assert!(text.contains("[entity.note]"), "{text}");
        assert_eq!(toml::from_str::<Lock>(&text).unwrap(), l);
        // a lock written before entities were locked reads as one without them
        let old: Lock = toml::from_str("lock = 1\nplugin = \"orders\"\n").unwrap();
        assert!(old.entities.is_empty());
        assert!(update(Some(&old), &d).changes.is_empty());

        assert!(update(Some(&l), &d).changes.is_empty());
        for (changed, code) in [
            (entity("i64", true, true, false), "E_CONTRACT_KEY_TYPE"),
            (
                entity("uuid", false, true, false),
                "E_CONTRACT_ENTITY_STORAGE",
            ),
            (
                entity("uuid", true, false, false),
                "E_CONTRACT_ENTITY_STORAGE",
            ),
            (desc(json!({})), "E_CONTRACT_ENTITY_REMOVED"),
        ] {
            let up = update(Some(&l), &changed);
            let codes: Vec<&str> = up.breaking().map(|c| c.code).collect();
            assert_eq!(codes, [code]);
        }
        // at publish: the published descriptor against the new one
        let codes: Vec<&str> = compare(&d, &entity("string", true, true, false))
            .iter()
            .map(|c| c.code)
            .collect();
        assert_eq!(codes, ["E_CONTRACT_KEY_TYPE"]);
    }

    fn provider(export: bool, fields: Vec<Value>, masks: Value) -> Descriptor {
        let mut all = vec![field("id", "uuid")];
        all.extend(fields);
        desc(
            json!({ "id": "crm", "entities": [{ "name": "customer", "message": "Customer", "fields": all,
            "access": { "relations": [], "read": "public", "masks": masks,
                        "export": if export { json!(["read"]) } else { json!([]) } } }] }),
        )
    }

    #[test]
    fn what_a_dependent_reads_is_an_exported_entity_s_fields() {
        let p = provider(
            true,
            vec![
                field("name", "string"),
                json!({ "name": "email", "number": 3, "type": "string", "optional": true }),
            ],
            json!([]),
        );
        let r = read_contract(&p, "customer").unwrap();
        let got: Vec<(&str, &str)> = r.iter().map(|f| (f.name.as_str(), f.ty.as_str())).collect();
        assert_eq!(
            got,
            [
                ("id", "uuid"),
                ("name", "string"),
                ("email", "optional string")
            ]
        );
        assert!(read_contract(&p, "nope").unwrap_err().contains("no entity"));
        let hidden = provider(false, vec![field("name", "string")], json!([]));
        assert!(
            read_contract(&hidden, "customer")
                .unwrap_err()
                .contains("does not export read")
        );

        // a read checked against what a provider has
        let read = crate::DependencyRead {
            entity: "customer".into(),
            fields: r.clone(),
        };
        let have: BTreeMap<String, String> =
            r.iter().map(|f| (f.name.clone(), f.ty.clone())).collect();
        assert!(read_mismatches(&read, &have).is_empty());
        let mut less = have.clone();
        less.remove("email");
        less.insert("name".into(), "int64".into());
        let m = read_mismatches(&read, &less);
        assert_eq!(m.len(), 2, "{m:?}");
        assert!(m[0].contains("email") || m[1].contains("email"));
    }

    #[test]
    fn a_change_to_an_exported_read_is_a_contract_change() {
        let old = provider(
            true,
            vec![field("name", "string"), field("rank", "int32")],
            json!([]),
        );
        // a field removed or retyped breaks the dependents that read it
        let new = provider(true, vec![field("name", "string")], json!([]));
        let codes: Vec<&str> = compare(&old, &new)
            .iter()
            .filter(|c| c.breaking)
            .map(|c| c.code)
            .collect();
        assert!(codes.contains(&"E_CONTRACT_EXPORT_FIELD"), "{codes:?}");
        let new = provider(
            true,
            vec![field("name", "string"), field("rank", "int64")],
            json!([]),
        );
        let codes: Vec<&str> = compare(&old, &new)
            .iter()
            .filter(|c| c.breaking)
            .map(|c| c.code)
            .collect();
        assert!(codes.contains(&"E_CONTRACT_EXPORT_FIELD"), "{codes:?}");
        // a new mask changes what they read: a warning
        let new = provider(
            true,
            vec![field("name", "string"), field("rank", "int32")],
            json!([{ "field": "name", "roles": ["admin"] }]),
        );
        let c = compare(&old, &new);
        assert!(
            c.iter()
                .any(|c| c.code == "W_CONTRACT_EXPORT_CHANGED" && !c.breaking),
            "{c:?}"
        );
        // a field added is not breaking
        let new = provider(
            true,
            vec![
                field("name", "string"),
                field("rank", "int32"),
                field("note", "string"),
            ],
            json!([]),
        );
        assert!(compare(&old, &new).iter().all(|c| !c.breaking));
    }
}
