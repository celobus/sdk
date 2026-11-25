//! Models of the Data Lake (`lake_models` of the descriptor): how the platform computes one of the plugin's lake
//! tables, when the plugin asks (`ctx.lake().refresh::<T>()`). A model is a view's query tree ([`crate::view`]) whose
//! row is its lake table's: `sources` (the root, then joins), an optional `filter`, `group_by` and `having`, and
//! `columns`, one per field of the table's row message, of the field's type.
//!
//! Its sources are either
//!
//!  * lake tables: the plugin's own (`stock_file`), or another plugin's it declares it reads (`retail.stock`, in
//!    `dependencies[].lake`; the owner must export it); a text column of a lake table may be parsed into another type
//!    (`parse`); or
//!  * the plugin's own entities and analytic views, read as the plugin's system context: the plugin's bypass of
//!    `read` must cover every entity under the model (`capabilities[authz:bypass].on`), an encrypted column is never
//!    named, and a masked column (or a view that names one anywhere under it) only when the model says `unmasked`:
//!    the lake has no masks, and whoever reads the table reads every row of it.
//!
//! never both in one model (a model over an entity that needs a lake table's rows reads them from a table another
//! model filled first). A model never reads its own table nor, through other models, a table that reads it.
//!
//! [`validate`] runs in the descriptor's check (apart from [`crate::lake::validate`]: a model that does not hold
//! leaves the plugin's lake tables and its other models as they are). Another plugin's lake table is checked only when its fields are
//! known ([`validate_with`]): at `celobus plugin check` only its declaration is.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Descriptor, Error,
    view::{self, Column, Expr, Term, View, ViewColumn, ViewSource},
};

/// The most models of a plugin (one per lake table).
pub const MAX_MODELS: usize = crate::lake::MAX_TABLES;

/// A model of one of the plugin's lake tables.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LakeModel {
    /// The lake table it fills (one of the plugin's `lake_tables`).
    pub table: String,
    /// The root, then each joined source: a lake table, or an entity or a view of the plugin.
    pub sources: Vec<ViewSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_by: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub having: Option<Expr>,
    /// The table's columns, one per field of its row message.
    pub columns: Vec<ViewColumn>,
    /// Masked columns of the entities under it go to the lake as they are (the model reads past the masks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unmasked: bool,
}

impl LakeModel {
    /// The model as a view whose row is the table's (`message`): what the query compiler and the checks take.
    pub fn as_view(&self, message: &str) -> View {
        View {
            name: self.table.clone(),
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
        let mut out: Vec<&str> = vec![];
        for e in [&self.filter, &self.having].into_iter().flatten() {
            exists(e, &mut out);
        }
        self.sources
            .iter()
            .map(|s| s.source_name())
            .chain(out.into_iter().map(str::to_string))
            .collect()
    }

    /// The hash of the declaration as it computes its table (the model, the table's columns and their types, the views
    /// under it):
    /// what a refresh records with the table, so a changed model is first run whole.
    pub fn hash(&self, d: &Descriptor) -> String {
        use sha2::Digest as _;
        let columns = d
            .lake_tables
            .iter()
            .find(|t| t.name == self.table)
            .and_then(|t| crate::lake::columns(d, t).ok())
            .map(|c| {
                c.into_iter()
                    .map(|c| format!("{}:{}", c.name, c.wire))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // and the views under it (through views too): a view computed otherwise computes the table otherwise
        let mut names = vec![];
        for n in self.source_names() {
            views_under(d, &n, &mut names, 0);
        }
        names.sort();
        let views: Vec<&View> = names
            .iter()
            .filter_map(|n| d.views.iter().find(|v| v.name == *n))
            .collect();
        let json = serde_json::to_string(&(self, columns, views)).unwrap_or_default();
        sha2::Sha256::digest(json)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// A name of the plugin's own entity, view or lake table without its plugin (`shop.customer` in `shop` is
/// `customer`, as the query compiler reads it); another plugin's stays qualified.
pub fn own<'a>(d: &Descriptor, name: &'a str) -> &'a str {
    name.strip_prefix(d.id.as_str())
        .and_then(|r| r.strip_prefix('.'))
        .unwrap_or(name)
}

/// The plugin's views `name` is or reads through, by name.
fn views_under(d: &Descriptor, name: &str, out: &mut Vec<String>, depth: usize) {
    if depth > view::MAX_VIEW_DEPTH {
        return;
    }
    let name = own(d, name);
    let Some(v) = d.views.iter().find(|v| v.name == name) else {
        return;
    };
    if !out.iter().any(|x| x == name) {
        out.push(name.to_string());
    }
    let mut names: Vec<&str> = v.sources.iter().map(|s| s.entity.as_str()).collect();
    for e in [&v.filter, &v.having].into_iter().flatten() {
        exists(e, &mut names);
    }
    for n in names {
        views_under(d, n, out, depth + 1);
    }
}

fn exists<'a>(e: &'a Expr, out: &mut Vec<&'a str>) {
    match e {
        Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| exists(x, out)),
        Expr::Not(x) => exists(x, out),
        Expr::Compare(_) => {}
        Expr::Exists(x) => {
            out.push(&x.entity);
            if let Some(f) = &x.filter {
                exists(f, out);
            }
        }
    }
}

/// What a model's source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a lake table (the plugin's own, or a dependency's)
    Lake,
    /// an entity of the plugin
    Entity,
    /// an analytic view of the plugin
    View,
    Unknown,
}

/// What `name` is to a model of `d` (an entity's history is a source of the lake: the plugin's own entity that keeps
/// one, or another plugin's it names in `dependencies[].lake`).
pub fn kind(d: &Descriptor, name: &str) -> Kind {
    let name = own(d, name);
    // an entity's history: the plugin's own only, of an entity that keeps one (another plugin's history is no
    // source of a model: copying it under this plugin's tables needs a consent no history declares)
    if let Some((entity, _)) = crate::history::parse_source(name) {
        let known = !entity.contains('.')
            && d.entities
                .iter()
                .any(|e| e.name == entity && e.history.is_some());
        return if known { Kind::Lake } else { Kind::Unknown };
    }
    match name.split_once('.') {
        Some((p, t)) => {
            let declared = d
                .dependencies
                .iter()
                .any(|x| x.plugin == p && x.lake.iter().any(|l| l == t));
            if declared { Kind::Lake } else { Kind::Unknown }
        }
        None if d.lake_tables.iter().any(|t| t.name == name) => Kind::Lake,
        None if d.entities.iter().any(|e| e.name == name) => Kind::Entity,
        None if d.views.iter().any(|v| v.name == name) => Kind::View,
        None => Kind::Unknown,
    }
}

/// The plugin's entities a model reads: its own entity sources, and those under its views (through views too).
pub fn entities(d: &Descriptor, m: &LakeModel) -> Vec<String> {
    let mut out = vec![];
    for name in m.source_names() {
        under(d, &name, &mut out, 0);
    }
    out
}

fn under(d: &Descriptor, name: &str, out: &mut Vec<String>, depth: usize) {
    if depth > view::MAX_VIEW_DEPTH {
        return;
    }
    let name = own(d, name);
    if d.entities.iter().any(|e| e.name == name) {
        if !out.iter().any(|x| x == name) {
            out.push(name.to_string());
        }
        return;
    }
    if let Some(v) = d.views.iter().find(|v| v.name == name) {
        for s in &v.sources {
            under(d, &s.entity, out, depth + 1);
        }
        let mut names = vec![];
        for e in [&v.filter, &v.having].into_iter().flatten() {
            exists(e, &mut names);
        }
        for n in names {
            under(d, n, out, depth + 1);
        }
    }
}

/// Every column a tree names (joins, conditions, groups, columns, aggregates), as (alias, field).
fn named<'a>(
    sources: &'a [ViewSource],
    filter: &'a Option<Expr>,
    group_by: &'a [Column],
    having: &'a Option<Expr>,
    columns: &'a [ViewColumn],
) -> Vec<&'a Column> {
    fn term<'a>(t: &'a Term, out: &mut Vec<&'a Column>) {
        match t {
            Term::Column(c) => out.push(c),
            Term::Aggregate(a) => out.extend(a.column.as_ref()),
            Term::DaysAgo(_) => {}
        }
    }
    fn expr<'a>(e: &'a Expr, out: &mut Vec<&'a Column>) {
        match e {
            Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| expr(x, out)),
            Expr::Not(x) => expr(x, out),
            Expr::Compare(c) => {
                term(&c.left, out);
                if let Some(r) = &c.right {
                    term(r, out);
                }
            }
            Expr::Exists(x) => {
                if let Some(f) = &x.filter {
                    expr(f, out);
                }
            }
        }
    }
    let mut out = vec![];
    for s in sources {
        out.extend(s.left.as_ref());
        out.extend(s.right.as_ref());
    }
    for e in [filter, having].into_iter().flatten() {
        expr(e, &mut out);
    }
    out.extend(group_by);
    for c in columns {
        term(&c.value, &mut out);
    }
    out
}

/// The aliases of a tree's sources (joined and in `exists`) and what each names.
fn aliases<'a>(
    sources: &'a [ViewSource],
    filter: &'a Option<Expr>,
    having: &'a Option<Expr>,
) -> BTreeMap<&'a str, &'a str> {
    fn walk<'a>(e: &'a Expr, out: &mut BTreeMap<&'a str, &'a str>) {
        match e {
            Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| walk(x, out)),
            Expr::Not(x) => walk(x, out),
            Expr::Compare(_) => {}
            Expr::Exists(x) => {
                out.insert(&x.alias, &x.entity);
                if let Some(f) = &x.filter {
                    walk(f, out);
                }
            }
        }
    }
    let mut out: BTreeMap<&str, &str> = sources
        .iter()
        .map(|s| (s.alias.as_str(), s.entity.as_str()))
        .collect();
    for e in [filter, having].into_iter().flatten() {
        walk(e, &mut out);
    }
    out
}

/// An alias a tree gives twice (its sources and every `exists`, nested too): the first such.
pub(crate) fn twice(
    sources: &[ViewSource],
    filter: &Option<Expr>,
    having: &Option<Expr>,
) -> Option<String> {
    fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a str>) {
        match e {
            Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| walk(x, out)),
            Expr::Not(x) => walk(x, out),
            Expr::Compare(_) => {}
            Expr::Exists(x) => {
                out.push(&x.alias);
                if let Some(f) = &x.filter {
                    walk(f, out);
                }
            }
        }
    }
    let mut all: Vec<&str> = sources.iter().map(|s| s.alias.as_str()).collect();
    for e in [filter, having].into_iter().flatten() {
        walk(e, &mut all);
    }
    let mut seen = HashSet::new();
    all.into_iter()
        .find(|a| !seen.insert(*a))
        .map(str::to_string)
}

/// What a model refuses under it, in its own tree and in every view it reads through: an alias given twice (a
/// column could not be told to its source), a dependency's entity or the directory (read as the caller who refreshes
/// it, they would freeze that caller's view of another plugin's rows into the table).
fn refused_under(d: &Descriptor, m: &LakeModel) -> Option<String> {
    if let Some(a) = twice(&m.sources, &m.filter, &m.having) {
        return Some(format!(
            "the model gives the alias {a:?} twice: one alias, one source"
        ));
    }
    let mut views = vec![];
    for n in m.source_names() {
        views_under(d, &n, &mut views, 0);
    }
    for name in views {
        let Some(v) = d.views.iter().find(|v| v.name == name) else {
            continue;
        };
        if let Some(a) = twice(&v.sources, &v.filter, &v.having) {
            return Some(format!(
                "the view {name} under the model gives the alias {a:?} twice: one alias, one source"
            ));
        }
        let mut names: Vec<&str> = v.sources.iter().map(|s| s.entity.as_str()).collect();
        for e in [&v.filter, &v.having].into_iter().flatten() {
            exists(e, &mut names);
        }
        if let Some(n) = names.into_iter().find(|n| own(d, n).contains('.')) {
            return Some(format!(
                "the view {name} under the model reads {n}: a model reads the plugin's own entities and views only, \
                 never another plugin's entity or the directory (it would be read as whoever refreshes it)"
            ));
        }
    }
    None
}

/// The masked fields of an entity of the plugin.
fn masks<'a>(d: &'a Descriptor, entity: &str) -> Vec<&'a str> {
    let entity = own(d, entity);
    d.entities
        .iter()
        .find(|e| e.name == entity)
        .and_then(|e| e.access.as_ref())
        .map(|a| a.masks.iter().map(|m| m.field.as_str()).collect())
        .unwrap_or_default()
}

/// The first masked column a view names, anywhere under it (through other views too): `entity.field`.
fn masked_in_view(d: &Descriptor, v: &View, depth: usize) -> Option<String> {
    if depth > view::MAX_VIEW_DEPTH {
        return None;
    }
    let names = aliases(&v.sources, &v.filter, &v.having);
    for c in named(&v.sources, &v.filter, &v.group_by, &v.having, &v.columns) {
        if let Some(entity) = names.get(c.source.as_str())
            && masks(d, entity).contains(&c.field.as_str())
        {
            return Some(format!("{}.{}", own(d, entity), c.field));
        }
    }
    for name in names.values() {
        if let Some(w) = d.views.iter().find(|w| w.name == own(d, name))
            && let Some(m) = masked_in_view(d, w, depth + 1)
        {
            return Some(m);
        }
    }
    None
}

/// The first masked column a model names: of an entity it reads, or anywhere under a view it reads.
pub fn masked(d: &Descriptor, m: &LakeModel) -> Option<String> {
    let names = aliases(&m.sources, &m.filter, &m.having);
    for c in named(&m.sources, &m.filter, &m.group_by, &m.having, &m.columns) {
        if let Some(entity) = names.get(c.source.as_str())
            && masks(d, entity).contains(&c.field.as_str())
        {
            return Some(format!("{}.{}", own(d, entity), c.field));
        }
    }
    for name in names.values() {
        if let Some(v) = d.views.iter().find(|v| v.name == own(d, name))
            && let Some(x) = masked_in_view(d, v, 0)
        {
            return Some(x);
        }
    }
    None
}

/// Whether the plugin's bypass covers `read` of the entity.
pub fn bypasses_read(d: &Descriptor, entity: &str) -> bool {
    d.capabilities
        .iter()
        .filter(|c| c.name == "authz:bypass")
        .flat_map(|c| c.on.iter())
        .any(|s| s.entity == entity && s.ops.iter().any(|o| o == "read"))
}

/// The check codes of a view's tree as a model's.
fn code(c: &'static str) -> &'static str {
    match c {
        "E_VIEW_COLUMN" => "E_LAKE_MODEL_COLUMN",
        "E_VIEW_ENCRYPTED" => "E_LAKE_MODEL_ENCRYPTED",
        "E_VIEW_BUCKET" => "E_LAKE_MODEL_BUCKET",
        "E_VIEW_TERM" => "E_LAKE_MODEL_TERM",
        "E_VIEW_FILTER" => "E_LAKE_MODEL_FILTER",
        "E_VIEW_TYPE" => "E_LAKE_MODEL_TYPE",
        "E_VIEW_SOURCE" => "E_LAKE_MODEL_SOURCE",
        "E_VIEW_JOIN" => "E_LAKE_MODEL_JOIN",
        "E_VIEW_SIZE" => "E_LAKE_MODEL_SIZE",
        "E_VIEW_MESSAGE" => "E_LAKE_MODEL_TABLE",
        "E_VIEW_GROUP" => "E_LAKE_MODEL_GROUP",
        "E_VIEW_PARSE" => "E_LAKE_MODEL_PARSE",
        _ => "E_LAKE_MODEL",
    }
}

/// Every problem of the descriptor's models (`E_LAKE_MODEL_*`), another plugin's lake table checked by its name only.
pub fn validate(d: &Descriptor) -> Vec<Error> {
    validate_with(d, &|_, _| None)
}

/// Every problem of the descriptor's models; `foreign(plugin, table)`: the fields of another plugin's lake table's row,
/// when known (none: a model reading it is checked by the names it declares only).
pub fn validate_with(
    d: &Descriptor,
    foreign: &dyn Fn(&str, &str) -> Option<Vec<crate::Field>>,
) -> Vec<Error> {
    let mut errors = vec![];
    let mut trees = vec![];
    let mut err = |code: &'static str, path: String, message: String| {
        errors.push(Error::new(code, path, message))
    };
    if d.lake_models.len() > MAX_MODELS {
        err(
            "E_LAKE_MODEL_SIZE",
            "lake_models".into(),
            format!(
                "the plugin declares {} models: at most {MAX_MODELS}",
                d.lake_models.len()
            ),
        );
        return errors;
    }
    let mut seen = HashSet::new();
    for (i, m) in d.lake_models.iter().enumerate() {
        let p = format!(
            "lake_models[{}]",
            if m.table.is_empty() {
                i.to_string()
            } else {
                m.table.clone()
            }
        );
        let Some(t) = d.lake_tables.iter().find(|t| t.name == m.table) else {
            err(
                "E_LAKE_MODEL_TABLE",
                format!("{p}.table"),
                format!(
                    "{:?} is not a lake table of the plugin: a model fills one of its own",
                    m.table
                ),
            );
            continue;
        };
        if !seen.insert(m.table.as_str()) {
            err(
                "E_LAKE_MODEL_TABLE",
                p.clone(),
                format!("{} has two models: a table has one", m.table),
            );
            continue;
        }
        if m.sources.is_empty() {
            err(
                "E_LAKE_MODEL_SOURCE",
                format!("{p}.sources"),
                "a model has a root source".into(),
            );
            continue;
        }
        // what it reads: lake tables, or entities and views, never both
        let mut kinds = vec![];
        let mut unknown = false;
        for name in m.source_names() {
            match kind(d, &name) {
                Kind::Unknown => {
                    unknown = true;
                    let why = match name.split_once('.') {
                        _ if crate::history::parse_source(own(d, &name)).is_some() => format!(
                            "{name}: a model reads the history of one of the plugin's own entities that keeps one, never \
                             another plugin's"
                        ),
                        Some((owner, table)) => format!(
                            "{owner}.{table}: a model reads another plugin's lake table it declares \
                             (dependencies[].lake), never another plugin's entity or the directory"
                        ),
                        None => format!("{} declares no lake table, entity or view {name:?}", d.id),
                    };
                    err("E_LAKE_MODEL_SOURCE", format!("{p}.sources"), why);
                }
                k => kinds.push(k),
            }
        }
        if unknown {
            continue;
        }
        // its own lake tables and histories are read as its system context: each must say so
        for name in m.source_names() {
            let name = own(d, &name);
            if let Some((entity, _)) = crate::history::parse_source(name)
                && d.entities
                    .iter()
                    .any(|e| e.name == entity && e.history.as_ref().is_some_and(|h| !h.system_read))
            {
                err(
                    "E_LAKE_MODEL_SOURCE",
                    format!("{p}.sources"),
                    format!(
                        "the model reads the history of {entity} as the plugin's system context: declare that it is \
                         read so (history_read_as_system)"
                    ),
                );
            }
            if let Some(t) = d.lake_tables.iter().find(|t| t.name == name)
                && !t.system_read
            {
                err(
                    "E_LAKE_MODEL_SOURCE",
                    format!("{p}.sources"),
                    format!(
                        "the model reads the lake table {name} as the plugin's system context: declare that it is read \
                         so (system_read)"
                    ),
                );
            }
        }
        if let Some(why) = refused_under(d, m) {
            err("E_LAKE_MODEL_SOURCE", format!("{p}.sources"), why);
            continue;
        }
        let lake = kinds.iter().filter(|k| **k == Kind::Lake).count();
        if lake > 0 && lake < kinds.len() {
            err(
                "E_LAKE_MODEL_MIXED",
                format!("{p}.sources"),
                "a model reads lake tables, or the plugin's entities and views, never both: fill a lake table from \
                 the entities with a model of its own first, and read that table"
                    .into(),
            );
            continue;
        }
        if lake == 0 {
            for e in entities(d, m) {
                if !bypasses_read(d, &e) {
                    err(
                        "E_LAKE_MODEL_BYPASS",
                        p.clone(),
                        format!(
                            "the model reads {e} as the plugin's system context: the plugin's authz:bypass must \
                             cover read of {e}"
                        ),
                    );
                }
            }
            if !m.unmasked
                && let Some(x) = masked(d, m)
            {
                err(
                    "E_LAKE_MODEL_MASKED",
                    p.clone(),
                    format!(
                        "the model names the masked column {x}: the lake has no masks, so say that its values go \
                         there as they are (unmasked)"
                    ),
                );
            }
        } else if m.unmasked {
            err(
                "E_LAKE_MODEL_MASKED",
                format!("{p}.unmasked"),
                "unmasked is said of a model over entities or views: a lake table has no masks"
                    .into(),
            );
        }
        // the tree, as a view whose row is the table's
        let foreign_unknown = m.source_names().iter().any(|n| {
            own(d, n)
                .split_once('.')
                .is_some_and(|(o, t)| foreign(o, t).is_none())
        });
        if foreign_unknown {
            continue;
        }
        let resolve = |name: &str| -> Option<Result<Vec<crate::Field>, String>> {
            let name = own(d, name);
            if let Some((entity, days)) = crate::history::parse_source(name)
                && !entity.contains('.')
            {
                let e = d.entities.iter().find(|e| e.name == entity)?;
                return Some(Ok(crate::history::source_fields(&d.id, d, e, days)));
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
        for e in view::check_model(d, &m.as_view(&t.message), &p, &resolve, lake > 0) {
            trees.push(Error {
                code: code(e.code),
                ..e
            });
        }
    }
    errors.extend(trees);
    errors.extend(cycles(d));
    errors
}

/// Models that read their own table, or each other's in a cycle (`E_LAKE_MODEL_CYCLE`).
fn cycles(d: &Descriptor) -> Vec<Error> {
    let reads = |table: &str| -> Vec<String> {
        d.lake_models
            .iter()
            .find(|m| m.table == table)
            .map(|m| {
                m.source_names()
                    .into_iter()
                    .map(|n| own(d, &n).to_string())
                    .filter(|n| !n.contains('.') && !n.contains('#') && kind(d, n) == Kind::Lake)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = vec![];
    for m in &d.lake_models {
        // a walk from the model's table along what the models read: back at its table, a cycle
        let mut stack = reads(&m.table);
        let mut seen = HashSet::new();
        while let Some(t) = stack.pop() {
            if t == m.table {
                out.push(Error::new(
                    "E_LAKE_MODEL_CYCLE",
                    format!("lake_models[{}]", m.table),
                    format!(
                        "the model of {} reads its own table, directly or through other models",
                        m.table
                    ),
                ));
                break;
            }
            if seen.insert(t.clone()) {
                stack.extend(reads(&t));
            }
        }
    }
    out
}
