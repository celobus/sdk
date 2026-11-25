//! Entities kept in the analytic copy only, and their loads (`storage` and `olap_key` of an entity, `loads` of the
//! descriptor).
//!
//! **An entity stored in OLAP** (`"storage": "olap"`) has a table in the workspace's database that stays empty: its
//! rows are in the analytic copy, put there by its **load** when the plugin asks for a snapshot. Reports read it with
//! the analytic engine like any entity, under its own access rules and masks; nothing writes it, and the
//! transactional engine refuses to read it (it would answer an empty table).
//!
//! Such an entity
//!
//!  * is keyed by some of its fields (`olap_key`, at least one): plain, required values (no list, message, json, bytes
//!    or encrypted field) that are unique among the rows a load gives. Its key is `uuid` and its `id` is made from
//!    the key's values (the same values, the same id, in every snapshot);
//!  * has fields a lake column can hold (a scalar, an enum or a ref; never a list, a message or an encrypted field:
//!    the copy never holds an encrypted value it could not open, and a load reads none);
//!  * keeps no history, is not `soft_delete`, `version` nor `append_only`;
//!  * has an access declaration of the kinds that need no grants kept per row: roles, `authenticated`, `public`,
//!    `via` through a ref, conditions on its own columns (`when`), masks; a `read` grant only (no `create`, `update`,
//!    `delete`, custom action or guard), and no relation but `column` (what `via` follows);
//!  * is never the target of a ref of an ordinary entity of the plugin (its database table is always empty, so a key
//!    to it would refuse every row);
//!  * has exactly one load.
//!
//! **A load** is a query tree as a model's ([`crate::lake_model`]), whose row is the entity's (every field but `id`, by
//! name and type; the optional ones may be left out and read absent). Its sources are lake relations: the plugin's own
//! lake tables and histories (read as the plugin's system context: each declared so, `system_read` or
//! `history_read_as_system`), and another plugin's lake table the plugin declares it reads (`dependencies[].lake`)
//! when its owner lets other plugins copy it (`export_models`: the same consent a model's copy needs, checked against
//! the owner's installed descriptor at every load). Never an entity or an analytic view (those are in the copy
//! already: a view joins them with the loaded entity).
//!
//! [`validate`] runs in the descriptor's check (`E_OLAP_*`, `E_LOAD_*`); another plugin's lake table is checked only
//! when its fields are known ([`validate_with`]).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    Descriptor, Entity, Error, Field, FieldType, Message, MessageKind,
    access::{Grant, RelationKind},
    lake::ColumnType,
    lake_model::{Kind, kind, own, twice},
    view::{self, Column, Expr, View, ViewColumn, ViewSource},
};

/// How one of the plugin's entities stored in OLAP is filled from the lake.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Load {
    /// The entity it fills (one of the plugin's entities stored in OLAP).
    pub entity: String,
    /// The root, then each joined source: lake relations only.
    pub sources: Vec<ViewSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_by: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub having: Option<Expr>,
    /// The entity's fields, one each (an optional one may be left out).
    pub columns: Vec<ViewColumn>,
}

/// The name of the row message a load is checked against: the entity's fields but `id` (and the optional ones it
/// leaves out).
pub fn row_message(e: &Entity) -> String {
    format!("{}#load", e.message)
}

impl Load {
    /// The load as a view whose row is `message` (the compiler's and the checks' input).
    pub fn as_view(&self, message: &str) -> View {
        View {
            name: self.entity.clone(),
            message: message.into(),
            description: String::new(),
            sources: self.sources.clone(),
            filter: self.filter.clone(),
            group_by: self.group_by.clone(),
            having: self.having.clone(),
            columns: self.columns.clone(),
        }
    }

    /// Every source it names: joined, and in `exists`.
    pub fn source_names(&self) -> Vec<String> {
        crate::lake_model::LakeModel {
            table: String::new(),
            sources: self.sources.clone(),
            filter: self.filter.clone(),
            group_by: vec![],
            having: self.having.clone(),
            columns: vec![],
            unmasked: false,
        }
        .source_names()
    }

    /// The hash of the declaration as it fills its entity (the load, and the entity's fields, key and types): what a
    /// load records with its rows.
    pub fn hash(&self, d: &Descriptor) -> String {
        use sha2::Digest as _;
        let entity = d.entities.iter().find(|e| e.name == self.entity).map(|e| {
            (
                e.fields
                    .iter()
                    .map(|f| format!("{}:{}", f.name, crate::lock::wire_type(f)))
                    .collect::<Vec<_>>(),
                e.olap_key.clone(),
            )
        });
        let json = serde_json::to_string(&(self, entity)).unwrap_or_default();
        sha2::Sha256::digest(json)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The columns of an entity stored in OLAP that a load fills: every field but `id`, with its lake type.
pub fn columns(d: &Descriptor, e: &Entity) -> Result<Vec<crate::lake::Column>, String> {
    e.fields
        .iter()
        .filter(|f| f.name != "id")
        .map(|f| {
            if f.encrypted {
                return Err(format!(
                    "{}.{} is encrypted: an entity stored in OLAP holds no encrypted field",
                    e.name, f.name
                ));
            }
            let ty = ColumnType::of(d, f).ok_or_else(|| {
                format!(
                    "{}.{} is `{}`: a field of an entity stored in OLAP is a scalar, an enum or a ref, never a list \
                     or a message",
                    e.name,
                    f.name,
                    crate::lock::wire_type(f)
                )
            })?;
            Ok(crate::lake::Column {
                name: f.name.clone(),
                wire: crate::lock::wire_type(f),
                ty,
                optional: f.optional,
            })
        })
        .collect()
}

/// The plugin's entities stored in OLAP.
pub fn olap_entities(d: &Descriptor) -> impl Iterator<Item = &Entity> {
    d.entities.iter().filter(|e| e.olap())
}

/// The check codes of a view's tree as a load's.
fn code(c: &'static str) -> &'static str {
    match c {
        "E_VIEW_COLUMN" => "E_LOAD_COLUMN",
        "E_VIEW_ENCRYPTED" => "E_LOAD_COLUMN",
        "E_VIEW_BUCKET" => "E_LOAD_BUCKET",
        "E_VIEW_TERM" => "E_LOAD_TERM",
        "E_VIEW_FILTER" => "E_LOAD_FILTER",
        "E_VIEW_TYPE" => "E_LOAD_TYPE",
        "E_VIEW_SOURCE" => "E_LOAD_SOURCE",
        "E_VIEW_JOIN" => "E_LOAD_JOIN",
        "E_VIEW_SIZE" => "E_LOAD_SIZE",
        "E_VIEW_MESSAGE" => "E_LOAD_ENTITY",
        "E_VIEW_GROUP" => "E_LOAD_GROUP",
        "E_VIEW_PARSE" => "E_LOAD_PARSE",
        _ => "E_LOAD",
    }
}

/// The refs of `d`'s entities to another plugin's entity stored in OLAP (`E_OLAP_REF`): its rows are not in the
/// workspace's database, so a rule or a read that follows the ref finds none. `olap(plugin, entity)`: whether that
/// plugin's entity is stored in OLAP, as far as it is known (a dependency's lock, the installed descriptors).
pub fn foreign_refs(d: &Descriptor, olap: &dyn Fn(&str, &str) -> bool) -> Vec<Error> {
    let mut errors = vec![];
    for e in &d.entities {
        for f in &e.fields {
            if let Some(FieldType::Ref { plugin, entity }) = FieldType::parse(&f.ty)
                && plugin != d.id
                && plugin != "platform"
                && olap(&plugin, &entity)
            {
                errors.push(Error::new(
                    "E_OLAP_REF",
                    format!("entities[{}].fields[{}]", e.name, f.name),
                    format!(
                        "{}.{} refers to {plugin}.{entity}, which is stored in OLAP: its rows are not in the \
                         workspace's database, so no other plugin's entity refers to them (keep its key fields as plain \
                         values)",
                        e.name, f.name
                    ),
                ));
            }
        }
    }
    errors
}

/// Every problem of the descriptor's entities stored in OLAP and their loads, another plugin's lake table checked by
/// its name only.
pub fn validate(d: &Descriptor) -> Vec<Error> {
    validate_with(d, &|_, _| None)
}

/// Every problem of the descriptor's entities stored in OLAP (`E_OLAP_*`) and their loads (`E_LOAD_*`);
/// `foreign(plugin, table)`: the fields of another plugin's lake table's row, when known.
pub fn validate_with(
    d: &Descriptor,
    foreign: &dyn Fn(&str, &str) -> Option<Vec<Field>>,
) -> Vec<Error> {
    let mut errors = vec![];
    entities(d, &mut errors);
    loads(d, foreign, &mut errors);
    errors
}

fn entities(d: &Descriptor, errors: &mut Vec<Error>) {
    let mut err = |code: &'static str, path: String, message: String| {
        errors.push(Error::new(code, path, message))
    };
    let olap: HashSet<&str> = olap_entities(d).map(|e| e.name.as_str()).collect();
    for (i, e) in d.entities.iter().enumerate() {
        let p = format!(
            "entities[{}]",
            if e.name.is_empty() {
                i.to_string()
            } else {
                e.name.clone()
            }
        );
        if !e.storage.is_empty() && !e.olap() {
            err(
                "E_OLAP_STORAGE",
                format!("{p}.storage"),
                format!(
                    "{:?} is not a storage: an entity is kept in the workspace's database (the default) or in the \
                     analytic copy only (olap)",
                    e.storage
                ),
            );
            continue;
        }
        if !e.olap() {
            if !e.olap_key.is_empty() {
                err(
                    "E_OLAP_KEY",
                    format!("{p}.olap_key"),
                    "a key of fields is declared by an entity stored in OLAP only (storage = olap)"
                        .into(),
                );
            }
            // an ordinary entity's ref to one stored in OLAP: its key would refuse every row
            for f in &e.fields {
                if let Some(FieldType::Ref { plugin, entity }) = FieldType::parse(&f.ty)
                    && plugin == d.id
                    && olap.contains(entity.as_str())
                {
                    err(
                        "E_OLAP_REF",
                        format!("{p}.fields[{}]", f.name),
                        format!(
                            "{}.{} refers to {entity}, which is stored in OLAP: its rows are not in the workspace's \
                             database, so no ordinary entity refers to them (keep its key fields as plain values)",
                            e.name, f.name
                        ),
                    );
                }
            }
            continue;
        }
        if e.key != "uuid" {
            err(
                "E_OLAP_KEY",
                format!("{p}.key"),
                format!(
                    "{} is keyed by {}: an entity stored in OLAP has a uuid made from its key fields",
                    e.name, e.key
                ),
            );
        }
        for (what, on) in [
            ("soft_delete", e.soft_delete),
            ("version", e.version),
            ("append_only", e.append_only),
        ] {
            if on {
                err(
                    "E_OLAP_ENTITY",
                    format!("{p}.{what}"),
                    format!(
                        "{} is stored in OLAP: its rows are replaced by its load, so it has no {what}",
                        e.name
                    ),
                );
            }
        }
        if e.history.is_some() || e.fields.iter().any(|f| f.history) {
            err(
                "E_OLAP_HISTORY",
                format!("{p}.history"),
                format!(
                    "{} is stored in OLAP: its rows are replaced by its load, so it keeps no history",
                    e.name
                ),
            );
        }
        for f in e.fields.iter().filter(|f| f.name != "id") {
            if f.encrypted {
                err(
                    "E_OLAP_ENCRYPTED",
                    format!("{p}.fields[{}]", f.name),
                    format!(
                        "{}.{} is encrypted: an entity stored in OLAP holds no encrypted field (the lake has none)",
                        e.name, f.name
                    ),
                );
            } else if ColumnType::of(d, f).is_none() {
                err(
                    "E_OLAP_FIELD",
                    format!("{p}.fields[{}]", f.name),
                    format!(
                        "{}.{} is `{}`: a field of an entity stored in OLAP is a scalar, an enum or a ref, never a \
                         list or a message",
                        e.name,
                        f.name,
                        crate::lock::wire_type(f)
                    ),
                );
            }
        }
        // its key
        if e.olap_key.is_empty() {
            err(
                "E_OLAP_KEY",
                format!("{p}.olap_key"),
                format!(
                    "{} is stored in OLAP: declare the fields that key a row (key(..)), unique among the rows its load \
                     gives",
                    e.name
                ),
            );
        }
        let mut seen = HashSet::new();
        for k in &e.olap_key {
            let Some(f) = e.fields.iter().find(|f| f.name == *k && f.name != "id") else {
                err(
                    "E_OLAP_KEY",
                    format!("{p}.olap_key"),
                    format!("{} has no field {k:?} to key its rows", e.name),
                );
                continue;
            };
            if !seen.insert(k.as_str()) {
                err(
                    "E_OLAP_KEY",
                    format!("{p}.olap_key"),
                    format!("{k} is named twice in the key of {}", e.name),
                );
            }
            let plain = !f.repeated
                && !f.encrypted
                && !f.optional
                && !matches!(
                    ColumnType::of(d, f),
                    None | Some(ColumnType::Bytes | ColumnType::Json)
                );
            if !plain {
                err(
                    "E_OLAP_KEY",
                    format!("{p}.olap_key"),
                    format!(
                        "{}.{k} cannot key a row: a key field is a required plain value (no list, message, json, \
                         bytes, optional or encrypted field)",
                        e.name
                    ),
                );
            }
        }
        // its access: what needs no grants kept per row, read only
        if let Some(a) = &e.access {
            let ap = format!("{p}.access");
            for (op, g) in [
                ("create", &a.create),
                ("update", &a.update),
                ("delete", &a.delete),
            ] {
                if g.is_some() {
                    err(
                        "E_OLAP_ACCESS",
                        format!("{ap}.{op}"),
                        format!(
                            "{} is stored in OLAP: its rows come from its load only, so nobody may {op} one",
                            e.name
                        ),
                    );
                }
            }
            if !a.permits.is_empty() || !a.guards.is_empty() {
                err(
                    "E_OLAP_ACCESS",
                    ap.clone(),
                    format!(
                        "{} is stored in OLAP: it has no custom actions and no guards (nothing changes its rows)",
                        e.name
                    ),
                );
            }
            for r in &a.relations {
                if r.kind != RelationKind::Column {
                    err(
                        "E_OLAP_ACCESS",
                        format!("{ap}.relations[{}]", r.name),
                        format!(
                            "{}: an entity stored in OLAP has only relations of a ref column (what via follows): \
                             users, group and rows relations need grants kept per row, which its rows never get",
                            r.name
                        ),
                    );
                }
            }
            if let Some(g) = &a.read {
                let mut all = vec![];
                g.walk(&mut all);
                for leaf in all {
                    let why = match leaf {
                        Grant::Rel(r) => Some(format!(
                            "rel {r}: a relation that names the caller needs grants kept per row, which the rows of an \
                             entity stored in OLAP never get (use via, a role or a condition on its columns)"
                        )),
                        Grant::Holds(h) => Some(format!(
                            "holds {h}: an entity stored in OLAP has no custom actions"
                        )),
                        _ => None,
                    };
                    if let Some(why) = why {
                        err("E_OLAP_ACCESS", format!("{ap}.read"), why);
                    }
                }
            }
        }
        match d.loads.iter().filter(|l| l.entity == e.name).count() {
            1 => {}
            0 => err(
                "E_OLAP_LOAD",
                p.clone(),
                format!(
                    "{} is stored in OLAP: declare its load (Load::<{}>::from::<..>(), Plugin::load), what fills it \
                     from the lake",
                    e.name, e.message
                ),
            ),
            _ => err(
                "E_OLAP_LOAD",
                p.clone(),
                format!("{} has more than one load: an entity has one", e.name),
            ),
        }
    }
}

fn loads(
    d: &Descriptor,
    foreign: &dyn Fn(&str, &str) -> Option<Vec<Field>>,
    errors: &mut Vec<Error>,
) {
    let mut seen = HashSet::new();
    for (i, l) in d.loads.iter().enumerate() {
        let p = format!(
            "loads[{}]",
            if l.entity.is_empty() {
                i.to_string()
            } else {
                l.entity.clone()
            }
        );
        let mut err = |code: &'static str, path: String, message: String| {
            errors.push(Error::new(code, path, message))
        };
        let Some(e) = d.entities.iter().find(|e| e.name == l.entity) else {
            err(
                "E_LOAD_ENTITY",
                format!("{p}.entity"),
                format!("{} declares no entity {:?}", d.id, l.entity),
            );
            continue;
        };
        if !e.olap() {
            err(
                "E_LOAD_ENTITY",
                format!("{p}.entity"),
                format!(
                    "{} is kept in the workspace's database: a load fills an entity stored in OLAP only (storage = \
                     olap)",
                    e.name
                ),
            );
            continue;
        }
        if !seen.insert(l.entity.as_str()) {
            continue;
        }
        if l.sources.is_empty() {
            err(
                "E_LOAD_SOURCE",
                format!("{p}.sources"),
                "a load has a root source".into(),
            );
            continue;
        }
        if let Some(a) = twice(&l.sources, &l.filter, &l.having) {
            err(
                "E_LOAD_SOURCE",
                format!("{p}.sources"),
                format!("the load gives the alias {a:?} twice: one alias, one source"),
            );
            continue;
        }
        // lake relations only: the plugin's own lake tables and histories, read as its system context, and another
        // plugin's lake table it declares it reads
        let mut bad = false;
        for name in l.source_names() {
            let n = own(d, &name);
            if kind(d, &name) != Kind::Lake {
                bad = true;
                let why = match n.split_once('.') {
                    _ if crate::history::parse_source(n).is_some() => format!(
                        "{name}: a load reads the history of one of the plugin's own entities that keeps one, never \
                         another plugin's"
                    ),
                    Some((owner, table)) => format!(
                        "{owner}.{table}: a load reads another plugin's lake table it declares (dependencies[].lake), \
                         never another plugin's entity or the directory"
                    ),
                    None if d.entities.iter().any(|x| x.name == n)
                        || d.views.iter().any(|v| v.name == n) =>
                    {
                        format!(
                            "{n}: a load reads the lake (lake tables and histories), never an entity or a view: they \
                             are in the analytic copy already, where a view joins them with {}",
                            e.name
                        )
                    }
                    None => format!("{} declares no lake table {name:?}", d.id),
                };
                err("E_LOAD_SOURCE", format!("{p}.sources"), why);
                continue;
            }
            if let Some((entity, _)) = crate::history::parse_source(n)
                && d.entities
                    .iter()
                    .any(|x| x.name == entity && x.history.as_ref().is_some_and(|h| !h.system_read))
            {
                bad = true;
                err(
                    "E_LOAD_SOURCE",
                    format!("{p}.sources"),
                    format!(
                        "the load reads the history of {entity} as the plugin's system context: declare that it is \
                         read so (history_read_as_system)"
                    ),
                );
            }
            if let Some(t) = d.lake_tables.iter().find(|t| t.name == n)
                && !t.system_read
            {
                bad = true;
                err(
                    "E_LOAD_SOURCE",
                    format!("{p}.sources"),
                    format!(
                        "the load reads the lake table {n} as the plugin's system context: declare that it is read so \
                         (system_read)"
                    ),
                );
            }
        }
        if bad {
            continue;
        }
        if let Some(c) = l.columns.iter().find(|c| c.name == "id") {
            err(
                "E_LOAD_COLUMN",
                format!("{p}.columns[{}]", c.name),
                format!(
                    "the id of {} is made from its key fields ({}): a load gives the other fields",
                    e.name,
                    e.olap_key.join(", ")
                ),
            );
            continue;
        }
        // another plugin's table whose fields are not known here: checked by its name only
        if l.source_names().iter().any(|n| {
            own(d, n)
                .split_once('.')
                .is_some_and(|(o, t)| foreign(o, t).is_none())
        }) {
            continue;
        }
        // the tree, as a view whose row is the entity's (its optional fields may be left out)
        let fields: Vec<Field> = e
            .fields
            .iter()
            .filter(|f| f.name != "id")
            .filter(|f| !f.optional || l.columns.iter().any(|c| c.name == f.name))
            .cloned()
            .collect();
        let mut with = d.clone();
        with.messages.push(Message {
            name: row_message(e),
            kind: MessageKind::Message,
            description: String::new(),
            fields,
        });
        let resolve = |name: &str| -> Option<Result<Vec<Field>, String>> {
            let name = own(d, name);
            if let Some((entity, days)) = crate::history::parse_source(name)
                && !entity.contains('.')
            {
                let x = d.entities.iter().find(|x| x.name == entity)?;
                return Some(Ok(crate::history::source_fields(&d.id, d, x, days)));
            }
            match name.split_once('.') {
                Some((o, t)) => foreign(o, t).map(Ok),
                None => {
                    let t = d.lake_tables.iter().find(|t| t.name == name)?;
                    Some(
                        d.messages
                            .iter()
                            .find(|x| x.name == t.message)
                            .map(|x| x.fields.clone())
                            .ok_or_else(|| format!("{} declares no message {:?}", d.id, t.message)),
                    )
                }
            }
        };
        for x in view::check_model(&with, &l.as_view(&row_message(e)), &p, &resolve, true) {
            errors.push(Error {
                code: code(x.code),
                ..x
            });
        }
    }
}
