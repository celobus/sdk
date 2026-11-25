//! Analytic views (`views` of the descriptor): a named, typed query over the plugin's entities, its dependencies'
//! exported entities, the directory and its other views, whose rows are the view's columns. The platform creates
//! each view in both engines over the entities' access rules, so every row under a view passes its own entity's
//! rules for the caller; a view has no rules of its own.
//!
//! A view is the query tree of `ctx.query()` (`celobus.v1.Query` in `plugin.proto`) written as JSON: its `sources`
//! (the root, then joins), an optional `filter`, `group_by` and `having`, and its `columns`, each a name of the row
//! message's fields and the term it is (a column, possibly bucketed, an aggregate). [`validate`] checks every name
//! and type it can see in the descriptor: sources, columns, joins, buckets, the terms against the row message's
//! fields, cycles between views, a name taken by an entity.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Descriptor, Error, FieldType, MessageKind};

/// The most views a view reads through (a view of a view of a view ...).
pub const MAX_VIEW_DEPTH: usize = 4;

/// The most days `days_ago(n)` reaches back or ahead (about a hundred years): past it a date falls out of the engines'
/// range and every read fails.
pub const MAX_DAYS_AGO: i32 = 36_500;

/// The most sources of one view (joined and in `exists`).
pub const MAX_VIEW_SOURCES: usize = 8;

/// The most views of a plugin.
pub const MAX_VIEWS: usize = 32;

/// The most nodes of one view (sources, groups, columns, conditions and their terms), as a query's.
pub const MAX_VIEW_NODES: usize = 500;

/// The most bytes of one view's JSON.
pub const MAX_VIEW_BYTES: usize = 64 << 10;

/// A view's size: its sources (joined and in `exists`) and its nodes, counted as a query's are.
pub fn size(v: &View) -> (usize, usize) {
    fn walk(e: &Expr, sources: &mut usize, nodes: &mut usize) {
        *nodes += 1;
        match e {
            Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| walk(x, sources, nodes)),
            Expr::Not(x) => walk(x, sources, nodes),
            Expr::Compare(c) => *nodes += 1 + usize::from(c.right.is_some()),
            Expr::Exists(x) => {
                *sources += 1;
                *nodes += 1;
                if let Some(f) = &x.filter {
                    walk(f, sources, nodes);
                }
            }
        }
    }
    let mut sources = v.sources.len();
    let mut nodes = v.sources.len() + v.group_by.len() + v.columns.len();
    for e in [&v.filter, &v.having].into_iter().flatten() {
        walk(e, &mut sources, &mut nodes);
    }
    (sources, nodes)
}

/// An analytic view of the plugin.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// snake_case (`sales`), unique among the plugin's entities and views.
    pub name: String,
    /// The message of a row (`Sales`): one field per column, of the column's type.
    pub message: String,
    #[serde(default)]
    pub description: String,
    /// The root, then each joined source.
    pub sources: Vec<ViewSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_by: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub having: Option<Expr>,
    /// The row's columns, in the order of the row message's fields.
    pub columns: Vec<ViewColumn>,
}

/// A source of a view: an entity (`order`, `crm.customer`, `platform.user`) or another view of the plugin, under its
/// alias; every source but the first is joined (`inner` or `left`) on `left` (a column of a source before it) equal
/// to `right` (a column of this one).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewSource {
    pub entity: String,
    pub alias: String,
    /// `inner` | `left`; absent on the root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub join: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Column>,
    /// The entity's history instead of its rows (a model of the lake only: [`crate::history`]); written only when
    /// set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub history: bool,
    /// With `history`: one row per day each version was in force at the day's end.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub each_day: bool,
}

impl ViewSource {
    /// The name the source is resolved by: the entity's, or its history's (`ticket#history`,
    /// [`crate::history::source_name`]).
    pub fn source_name(&self) -> String {
        if self.history || self.each_day {
            crate::history::source_name(&self.entity, self.each_day)
        } else {
            self.entity.clone()
        }
    }
}

/// A column of a source, or (`bucket`: `day`, `week`, `month`, `quarter`, `year`) the period a `date` or `timestamp`
/// column falls in, as a `date`; or (`parse`: one of [`PARSES`]) a `string` column of a lake table read as that type,
/// only in a model of the lake (`crate::lake_model`), never in a view.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub source: String,
    pub field: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bucket: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parse: String,
}

/// The types a text column of a lake table is parsed as (`parse`).
pub const PARSES: &[&str] = &[
    "decimal",
    "int32",
    "int64",
    "double",
    "bool",
    "date",
    "timestamp",
    "uuid",
];

/// The buckets of a column.
pub const BUCKETS: &[&str] = &["day", "week", "month", "quarter", "year"];

/// A column of the view: its name (a field of the row message) and its value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewColumn {
    pub name: String,
    pub value: Term,
}

/// A term: a column, an aggregate, a day relative to the day the query runs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Term {
    Column(Column),
    Aggregate(Aggregate),
    DaysAgo(i32),
}

/// `count` (no column), `count_distinct`, `sum`, `avg`, `min`, `max`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Aggregate {
    #[serde(rename = "fn")]
    pub func: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<Column>,
}

/// The aggregate functions.
pub const AGGREGATES: &[&str] = &["count", "count_distinct", "sum", "avg", "min", "max"];

/// A condition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Compare(Compare),
    Exists(Exists),
}

/// `left op values` or `left op right` (`op`: `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `in`, `is_null`, `is_not_null`,
/// `contains`); values are JSON strings, numbers and booleans as a filter's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    pub left: Term,
    pub op: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Term>,
}

/// The operators of a comparison.
pub const OPS: &[&str] = &[
    "eq",
    "ne",
    "lt",
    "le",
    "gt",
    "ge",
    "in",
    "is_null",
    "is_not_null",
    "search",
    "contains",
];

/// A row of `entity` (under `alias`) for which `filter` holds exists.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Exists {
    pub entity: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Box<Expr>>,
}

impl View {
    /// The views of the plugin this one reads (its sources and `exists` that name no entity).
    pub fn reads<'a>(&'a self, views: &HashSet<&str>) -> Vec<&'a str> {
        let mut out: Vec<&str> = self
            .sources
            .iter()
            .map(|s| s.entity.as_str())
            .filter(|e| views.contains(e))
            .collect();
        fn walk<'a>(e: &'a Expr, views: &HashSet<&str>, out: &mut Vec<&'a str>) {
            match e {
                Expr::And(l) | Expr::Or(l) => l.iter().for_each(|x| walk(x, views, out)),
                Expr::Not(x) => walk(x, views, out),
                Expr::Compare(_) => {}
                Expr::Exists(x) => {
                    if views.contains(x.entity.as_str()) {
                        out.push(&x.entity);
                    }
                    if let Some(f) = &x.filter {
                        walk(f, views, out);
                    }
                }
            }
        }
        for e in [&self.filter, &self.having].into_iter().flatten() {
            walk(e, views, &mut out);
        }
        out
    }

    /// Whether its rows are groups (a `group_by`, a `having` or an aggregate column).
    pub fn grouped(&self) -> bool {
        !self.group_by.is_empty()
            || self.having.is_some()
            || self
                .columns
                .iter()
                .any(|c| matches!(c.value, Term::Aggregate(_)))
    }
}

/// The plugin's views in an order each comes after those it reads (`None`: a cycle, with a view on it).
pub fn ordered(views: &[View]) -> Result<Vec<&View>, String> {
    let names: HashSet<&str> = views.iter().map(|v| v.name.as_str()).collect();
    let mut out: Vec<&View> = vec![];
    let mut done: HashSet<&str> = HashSet::new();
    fn visit<'a>(
        v: &'a View,
        all: &'a [View],
        names: &HashSet<&str>,
        path: &mut Vec<&'a str>,
        done: &mut HashSet<&'a str>,
        out: &mut Vec<&'a View>,
    ) -> Result<(), String> {
        if done.contains(v.name.as_str()) {
            return Ok(());
        }
        if path.contains(&v.name.as_str()) {
            return Err(v.name.clone());
        }
        path.push(&v.name);
        for r in v.reads(names) {
            if let Some(w) = all.iter().find(|w| w.name == r) {
                visit(w, all, names, path, done, out)?;
            }
        }
        path.pop();
        done.insert(&v.name);
        out.push(v);
        Ok(())
    }
    for v in views {
        visit(v, views, &names, &mut vec![], &mut done, &mut out)?;
    }
    Ok(out)
}

/// Whether a value of a decimal comparison holds its number exactly: a string, or a whole JSON number (a number with a
/// fraction or an exponent, or past a signed 64-bit integer, is read as a binary floating point and may stand for another number).
pub fn exact_decimal(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.is_i64(),
        _ => true,
    }
}

/// What a column is to the checks: its type as a field's (`string`, `decimal`, `ref:shop.order`, `enum:State`), and
/// whether it is a list, encrypted, or kept as json or a message.
#[derive(Clone, Debug, PartialEq)]
struct Ty {
    base: String,
    repeated: bool,
    encrypted: bool,
}

impl Ty {
    fn of(base: &str) -> Self {
        Ty {
            base: base.into(),
            repeated: false,
            encrypted: false,
        }
    }

    /// Compared, grouped, ordered, joined: not a list, json, a message or encrypted.
    fn plain(&self) -> bool {
        !self.repeated
            && !self.encrypted
            && self.base != "json"
            && !self.base.starts_with("message:")
    }

    fn is_ref(&self) -> bool {
        self.base.starts_with("ref:")
    }
}

/// The columns of a source, by name.
type Columns = BTreeMap<String, Ty>;

/// The directory's columns (what `ctx.query()` reads of `platform.*`).
fn directory(entity: &str) -> Option<Columns> {
    let user = || Ty::of("ref:platform.user");
    let group = || Ty::of("ref:platform.group");
    let text = || Ty::of("string");
    let cols: Vec<(&str, Ty)> = match entity {
        "user" => vec![
            ("id", user()),
            ("display_name", text()),
            ("avatar_url", text()),
            ("email", text()),
            ("kind", Ty::of("enum:platform.UserKind")),
            ("base_role", Ty::of("enum:platform.BaseRole")),
            ("status", Ty::of("enum:platform.UserStatus")),
            ("locale", text()),
            ("joined_at", Ty::of("timestamp")),
            ("left_at", Ty::of("timestamp")),
        ],
        "group" => vec![("id", group()), ("name", text())],
        "group_member" => vec![("group_id", group()), ("user_id", user())],
        "role_assignment" => vec![("user_id", user()), ("role", text())],
        _ => return None,
    };
    Some(cols.into_iter().map(|(n, t)| (n.to_string(), t)).collect())
}

/// A read field's wire type (`optional string`, `repeated ref:x.y`) as a column's.
fn wire(t: &str) -> Ty {
    let t = t.strip_prefix("optional ").unwrap_or(t);
    match t.strip_prefix("repeated ") {
        Some(rest) => Ty {
            base: rest.into(),
            repeated: true,
            encrypted: false,
        },
        None => Ty::of(t),
    }
}

/// What a model's source is when it is a lake table: its row's fields (none: not a lake table).
pub(crate) type LakeSources<'a> = dyn Fn(&str) -> Option<Result<Vec<crate::Field>, String>> + 'a;

struct Check<'a> {
    d: &'a Descriptor,
    /// the columns of each view checked so far
    views: BTreeMap<String, Columns>,
    errors: Vec<Error>,
    /// a model of the lake is checked: its sources are resolved by this (a lake table's row fields), then as a view's
    /// (the plugin's own entities and views; never the directory nor a dependency's entity)
    model: Option<&'a LakeSources<'a>>,
    /// `parse` is allowed (a model over lake tables)
    parse: bool,
    /// the aliases whose source is an entity's history, and that entity (a column it does not keep is named so)
    histories: BTreeMap<String, String>,
}

/// The scope of a view's terms: (alias, columns).
type Scope = Vec<(String, Columns)>;

impl Check<'_> {
    fn err(&mut self, code: &'static str, path: &str, message: String) {
        self.errors.push(Error::new(code, path, message));
    }

    /// The columns of an entity or a view as a source names it.
    fn source(&self, name: &str) -> Result<Columns, String> {
        if self.model.is_none()
            && let Some((entity, _)) = crate::history::parse_source(name)
        {
            return Err(format!(
                "the history of {entity} is read from the lake only (a model of a lake table, or a lake query): a \
                 view reads entities"
            ));
        }
        let (plugin, entity) = match name.split_once('.') {
            Some((p, e)) => (p, e),
            None => (self.d.id.as_str(), name),
        };
        if let Some(lake) = self.model {
            if let Some(fields) = lake(name) {
                return Ok(fields?
                    .iter()
                    .map(|f| {
                        (
                            f.name.clone(),
                            Ty {
                                base: f.ty.clone(),
                                repeated: f.repeated,
                                encrypted: f.encrypted,
                            },
                        )
                    })
                    .collect());
            }
            if plugin != self.d.id {
                return Err(format!(
                    "a model reads the plugin's own lake tables, entities and views, and the lake tables of other \
                     plugins it declares (dependencies[].lake): not {name}"
                ));
            }
        }
        if plugin == "platform" {
            return directory(entity).ok_or_else(|| {
                format!(
                    "the directory has no {entity:?} (user, group, group_member, role_assignment)"
                )
            });
        }
        if plugin == self.d.id {
            if let Some(cols) = self.views.get(entity) {
                return Ok(cols.clone());
            }
            if self.d.views.iter().any(|v| v.name == entity) {
                return Err(format!("the view {entity:?} does not check"));
            }
            let e = self
                .d
                .entities
                .iter()
                .find(|e| e.name == entity)
                .ok_or_else(|| format!("{} declares no entity or view {entity:?}", self.d.id))?;
            let mut cols = Columns::new();
            for f in &e.fields {
                cols.insert(
                    f.name.clone(),
                    Ty {
                        base: f.ty.clone(),
                        repeated: f.repeated,
                        encrypted: f.encrypted,
                    },
                );
            }
            for c in e.system_columns() {
                cols.insert(
                    c.into(),
                    Ty::of(if c == "version" { "int64" } else { "timestamp" }),
                );
            }
            return Ok(cols);
        }
        let read = self
            .d
            .dependencies
            .iter()
            .filter(|x| x.plugin == plugin)
            .flat_map(|x| x.reads.iter())
            .find(|r| r.entity == entity)
            .ok_or_else(|| {
                format!(
                    "{} does not declare that it reads {plugin}.{entity}",
                    self.d.id
                )
            })?;
        let mut cols: Columns = read
            .fields
            .iter()
            .map(|f| (f.name.clone(), wire(&f.ty)))
            .collect();
        // its key, whatever the read spells it as
        cols.insert("id".into(), Ty::of(&format!("ref:{plugin}.{entity}")));
        Ok(cols)
    }

    fn column(&mut self, scope: &Scope, c: &Column, path: &str) -> Option<Ty> {
        let Some((_, cols)) = scope.iter().rev().find(|(a, _)| *a == c.source) else {
            self.err(
                "E_VIEW_COLUMN",
                path,
                format!("the view has no source {:?}", c.source),
            );
            return None;
        };
        let Some(t) = cols.get(&c.field).cloned() else {
            // a column of the entity its history does not keep: say so
            let unkept = self.histories.get(&c.source).and_then(|entity| {
                let own = crate::lake_model::own(self.d, entity);
                self.d
                    .entities
                    .iter()
                    .find(|e| e.name == own)
                    .filter(|e| e.fields.iter().any(|f| f.name == c.field))
                    .map(|_| own.to_string())
            });
            let message = match unkept {
                Some(entity) => format!(
                    "the history of {entity} does not keep {:?}: mark the column kept (`history` on the field, or \
                     `all` on the entity's history) to read it",
                    c.field
                ),
                None => format!("{} has no column {:?}", c.source, c.field),
            };
            self.err("E_VIEW_COLUMN", path, message);
            return None;
        };
        if t.encrypted {
            self.err(
                "E_VIEW_ENCRYPTED",
                path,
                format!(
                    "{}.{} is encrypted: a view cannot read it",
                    c.source, c.field
                ),
            );
            return None;
        }
        if !c.parse.is_empty() {
            return self.parsed(c, t, path);
        }
        if c.bucket.is_empty() {
            return Some(t);
        }
        if !BUCKETS.contains(&c.bucket.as_str()) {
            self.err(
                "E_VIEW_BUCKET",
                path,
                format!("{:?} is not a bucket ({})", c.bucket, BUCKETS.join(", ")),
            );
            return None;
        }
        if t.repeated || !matches!(t.base.as_str(), "date" | "timestamp") {
            self.err(
                "E_VIEW_BUCKET",
                path,
                format!(
                    "{}.{} is a {}: only a date or a timestamp has a {}",
                    c.source, c.field, t.base, c.bucket
                ),
            );
            return None;
        }
        Some(Ty::of("date"))
    }

    /// A text column read as another type (`parse`): only in a model over lake tables.
    fn parsed(&mut self, c: &Column, t: Ty, path: &str) -> Option<Ty> {
        if !self.parse {
            self.err(
                "E_VIEW_PARSE",
                path,
                format!(
                    "{}.{}: parse reads a text column of a lake table, in a model over lake tables only",
                    c.source, c.field
                ),
            );
            return None;
        }
        if !c.bucket.is_empty() || !PARSES.contains(&c.parse.as_str()) {
            self.err(
                "E_VIEW_PARSE",
                path,
                format!(
                    "{}.{}: {:?} is not a type a text is parsed as ({}), and a parsed column has no bucket",
                    c.source,
                    c.field,
                    c.parse,
                    PARSES.join(", ")
                ),
            );
            return None;
        }
        if t.repeated || t.base != "string" {
            self.err(
                "E_VIEW_PARSE",
                path,
                format!(
                    "{}.{} is a {}: only a string column is parsed",
                    c.source, c.field, t.base
                ),
            );
            return None;
        }
        Some(Ty::of(&c.parse))
    }

    fn term(&mut self, scope: &Scope, t: &Term, path: &str, aggregates: bool) -> Option<Ty> {
        match t {
            Term::Column(c) => self.column(scope, c, path),
            Term::DaysAgo(n) if !(-MAX_DAYS_AGO..=MAX_DAYS_AGO).contains(n) => {
                self.err(
                    "E_VIEW_TERM",
                    path,
                    format!(
                        "days_ago({n}) is out of range: at most {MAX_DAYS_AGO} days back or ahead"
                    ),
                );
                None
            }
            Term::DaysAgo(_) => Some(Ty::of("date")),
            Term::Aggregate(_) if !aggregates => {
                self.err(
                    "E_VIEW_TERM",
                    path,
                    "an aggregate is not a condition on rows: put it in having".into(),
                );
                None
            }
            Term::Aggregate(a) => {
                if !AGGREGATES.contains(&a.func.as_str()) {
                    self.err(
                        "E_VIEW_TERM",
                        path,
                        format!(
                            "{:?} is not an aggregate ({})",
                            a.func,
                            AGGREGATES.join(", ")
                        ),
                    );
                    return None;
                }
                let of = match &a.column {
                    None if a.func == "count" => return Some(Ty::of("int64")),
                    None => {
                        self.err("E_VIEW_TERM", path, format!("{} takes a column", a.func));
                        return None;
                    }
                    Some(c) => self.column(scope, c, path)?,
                };
                if !of.plain() {
                    self.err(
                        "E_VIEW_TERM",
                        path,
                        format!("{}: a list, a message or json is not aggregated", a.func),
                    );
                    return None;
                }
                let number = |b: &str| matches!(b, "int32" | "int64" | "double" | "decimal");
                let out = match a.func.as_str() {
                    "count" | "count_distinct" => "int64".to_string(),
                    "sum" if number(&of.base) => match of.base.as_str() {
                        "int32" | "int64" => "int64".into(),
                        b => b.into(),
                    },
                    "avg" if number(&of.base) => match of.base.as_str() {
                        "double" => "double".into(),
                        _ => "decimal".into(),
                    },
                    "min" | "max"
                        if matches!(
                            of.base.as_str(),
                            "string"
                                | "int32"
                                | "int64"
                                | "double"
                                | "decimal"
                                | "timestamp"
                                | "date"
                        ) =>
                    {
                        of.base.clone()
                    }
                    f => {
                        self.err(
                            "E_VIEW_TERM",
                            path,
                            format!("{f} is not an aggregate of a {} column", of.base),
                        );
                        return None;
                    }
                };
                Some(Ty::of(&out))
            }
        }
    }

    fn expr(&mut self, scope: &mut Scope, e: &Expr, path: &str, aggregates: bool, depth: usize) {
        // as a query's: the top condition is the first of 16 levels
        if depth >= 16 {
            self.err(
                "E_VIEW_FILTER",
                path,
                "the condition nests deeper than 16".into(),
            );
            return;
        }
        match e {
            Expr::And(l) | Expr::Or(l) => {
                for (i, x) in l.iter().enumerate() {
                    self.expr(scope, x, &format!("{path}[{i}]"), aggregates, depth + 1);
                }
            }
            Expr::Not(x) => self.expr(scope, x, path, aggregates, depth + 1),
            Expr::Exists(x) => {
                match self.source(&x.entity) {
                    Ok(cols) => scope.push((x.alias.clone(), cols)),
                    Err(m) => {
                        self.err("E_VIEW_SOURCE", path, m);
                        return;
                    }
                }
                if let Some(f) = &x.filter {
                    self.expr(scope, f, &format!("{path}.exists"), false, depth + 1);
                }
                scope.pop();
            }
            Expr::Compare(c) => {
                if !OPS.contains(&c.op.as_str()) {
                    self.err(
                        "E_VIEW_FILTER",
                        path,
                        format!("{:?} is not an operator ({})", c.op, OPS.join(", ")),
                    );
                    return;
                }
                if c.op == "search" {
                    self.err(
                        "E_VIEW_FILTER",
                        path,
                        "a view cannot search (the analytic engine has no full-text search): filter with contains"
                            .into(),
                    );
                    return;
                }
                let Some(left) = self.term(scope, &c.left, path, aggregates) else {
                    return;
                };
                if matches!(c.left, Term::DaysAgo(_)) {
                    self.err(
                        "E_VIEW_FILTER",
                        path,
                        "days_ago stands on the right of a comparison".into(),
                    );
                    return;
                }
                if c.values
                    .iter()
                    .any(|v| v.as_str().is_some_and(|s| s.contains('\0')))
                {
                    self.err(
                        "E_VIEW_FILTER",
                        path,
                        "a text value holds no NUL character".into(),
                    );
                    return;
                }
                if left.base == "decimal"
                    && let Some(v) = c.values.iter().find(|v| !exact_decimal(v))
                {
                    self.err(
                        "E_VIEW_FILTER",
                        path,
                        format!(
                            "{v} is not an exact decimal: write a decimal value as a string (\"12.50\") or a whole number"
                        ),
                    );
                    return;
                }
                if let Some(r) = &c.right {
                    let Some(right) = self.term(scope, r, path, aggregates) else {
                        return;
                    };
                    let days = matches!(r, Term::DaysAgo(_));
                    let ok = left.plain()
                        && right.plain()
                        && (left.base == right.base
                            || (days && left.base == "timestamp")
                            || (left.is_ref() && right.is_ref() && left.base == right.base));
                    if !ok {
                        self.err(
                            "E_VIEW_TYPE",
                            path,
                            format!("a {} and a {} are not compared", left.base, right.base),
                        );
                    }
                }
            }
        }
    }
}

/// Every problem of the descriptor's views (`E_VIEW_*`).
pub fn validate(d: &Descriptor) -> Vec<Error> {
    checked(d).errors
}

/// The views checked, with the columns of those that hold.
fn checked(d: &Descriptor) -> Check<'_> {
    let mut c = Check {
        d,
        views: BTreeMap::new(),
        errors: vec![],
        model: None,
        parse: false,
        histories: BTreeMap::new(),
    };
    if d.views.len() > MAX_VIEWS {
        c.err(
            "E_VIEW_SIZE",
            "views",
            format!(
                "the plugin declares {} views: at most {MAX_VIEWS}",
                d.views.len()
            ),
        );
        return c;
    }
    let mut seen = HashSet::new();
    for (i, v) in d.views.iter().enumerate() {
        let p = format!(
            "views[{}]",
            if v.name.is_empty() {
                i.to_string()
            } else {
                v.name.clone()
            }
        );
        if !crate::validate::snake(&v.name) || v.name.ends_with("_with_deleted") {
            c.err(
                "E_VIEW_NAME",
                &format!("{p}.name"),
                format!(
                    "{:?} is not a view name (snake_case, not ending in _with_deleted)",
                    v.name
                ),
            );
        } else if d.id.len() + 1 + v.name.len() > 63 {
            c.err(
                "E_VIEW_NAME",
                &format!("{p}.name"),
                format!(
                    "{:?} is too long: {}_{} must stay within 63 characters",
                    v.name, d.id, v.name
                ),
            );
        }
        if !seen.insert(v.name.as_str()) {
            c.err(
                "E_VIEW_DUP",
                &p,
                format!("view {:?} is declared twice", v.name),
            );
        }
        if d.entities.iter().any(|e| e.name == v.name) {
            c.err(
                "E_VIEW_CLASH",
                &p,
                format!("{:?} is the name of an entity of the plugin", v.name),
            );
        }
    }
    let order = match ordered(&d.views) {
        Ok(o) => o,
        Err(on) => {
            c.err(
                "E_VIEW_CYCLE",
                &format!("views[{on}]"),
                format!("the view {on:?} reads itself through other views"),
            );
            return c;
        }
    };
    // how deep each view reads through views
    let names: HashSet<&str> = d.views.iter().map(|v| v.name.as_str()).collect();
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    for v in &order {
        let p = format!("views[{}]", v.name);
        let n = v
            .reads(&names)
            .iter()
            .map(|r| depth.get(r).copied().unwrap_or(0) + 1)
            .max()
            .unwrap_or(0);
        depth.insert(&v.name, n);
        if n >= MAX_VIEW_DEPTH {
            c.err(
                "E_VIEW_DEPTH",
                &p,
                format!(
                    "{:?} reads through {n} views: at most {}",
                    v.name,
                    MAX_VIEW_DEPTH - 1
                ),
            );
            continue;
        }
        if let Some(cols) = check_view(&mut c, v, &p) {
            c.views.insert(v.name.clone(), cols);
        }
    }
    c
}

/// A model of the lake checked as a view whose row is its lake table's (`crate::lake_model`): its sources resolved
/// by `lake` (a lake table's row fields; `None`: not a lake table, then the plugin's own entity or view), `parse`
/// allowed when it reads lake tables. The errors are the view's codes (`E_VIEW_*`), which the caller renames.
pub(crate) fn check_model(
    d: &Descriptor,
    v: &View,
    path: &str,
    lake: &LakeSources<'_>,
    parse: bool,
) -> Vec<Error> {
    let views = checked(d).views;
    let mut c = Check {
        d,
        views,
        errors: vec![],
        model: Some(lake),
        parse,
        histories: BTreeMap::new(),
    };
    check_view(&mut c, v, path);
    c.errors
}

/// One view's checks; its columns when they hold.
fn check_view(c: &mut Check, v: &View, p: &str) -> Option<Columns> {
    let before = c.errors.len();
    if v.sources.is_empty() {
        c.err(
            "E_VIEW_SOURCE",
            &format!("{p}.sources"),
            "a view has a root source".into(),
        );
        return None;
    }
    // its size first: a view past it is not checked further (its checks would cost as much as it does)
    let (sources, nodes) = size(v);
    let bytes = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX);
    if sources > MAX_VIEW_SOURCES {
        c.err(
            "E_VIEW_SOURCE",
            &format!("{p}.sources"),
            format!(
                "the view names {sources} sources (joined and in exists): at most {MAX_VIEW_SOURCES}"
            ),
        );
    }
    if nodes > MAX_VIEW_NODES || bytes > MAX_VIEW_BYTES {
        c.err(
            "E_VIEW_SIZE",
            p,
            format!(
                "the view has {nodes} nodes in {bytes} bytes: at most {MAX_VIEW_NODES} nodes in {MAX_VIEW_BYTES} bytes"
            ),
        );
    }
    if c.errors.len() > before {
        return None;
    }
    let mut scope: Scope = vec![];
    for (i, s) in v.sources.iter().enumerate() {
        let sp = format!("{p}.sources[{i}]");
        let alias_ok = {
            let b = s.alias.as_bytes();
            (1..=64).contains(&b.len())
                && b[0].is_ascii_lowercase()
                && b.iter()
                    .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || *x == b'_')
        };
        if !alias_ok || scope.iter().any(|(a, _)| *a == s.alias) {
            c.err(
                "E_VIEW_SOURCE",
                &sp,
                format!(
                    "{:?} is not an alias of its own ([a-z][a-z0-9_]*, once in the view)",
                    s.alias
                ),
            );
            return None;
        }
        if let Some((entity, _)) = crate::history::parse_source(&s.source_name()) {
            c.histories.insert(s.alias.clone(), entity.to_string());
        }
        let cols = match c.source(&s.source_name()) {
            Ok(cols) => cols,
            // a view that did not check has its own errors
            Err(_) if c.d.views.iter().any(|w| w.name == s.entity) => return None,
            Err(m) => {
                c.err("E_VIEW_SOURCE", &sp, m);
                return None;
            }
        };
        if i == 0 {
            if !s.join.is_empty() || s.left.is_some() || s.right.is_some() {
                c.err("E_VIEW_SOURCE", &sp, "the root is not joined".into());
            }
            scope.push((s.alias.clone(), cols));
            continue;
        }
        if !matches!(s.join.as_str(), "inner" | "left") {
            c.err(
                "E_VIEW_JOIN",
                &sp,
                format!("{:?} is not a join (inner, left)", s.join),
            );
        }
        let (Some(l), Some(r)) = (&s.left, &s.right) else {
            c.err(
                "E_VIEW_JOIN",
                &sp,
                "a join names its left and right columns".into(),
            );
            return None;
        };
        if l.source == s.alias || r.source != s.alias {
            c.err(
                "E_VIEW_JOIN",
                &sp,
                format!(
                    "the join of {}: its left column is of a source before it, its right one of {} itself",
                    s.alias, s.alias
                ),
            );
            return None;
        }
        let left = c.column(&scope, l, &sp);
        scope.push((s.alias.clone(), cols));
        let right = c.column(&scope, r, &sp);
        if let (Some(a), Some(b)) = (left, right) {
            let ok = a.plain() && b.plain() && a.base == b.base;
            if !ok {
                c.err(
                    "E_VIEW_JOIN",
                    &sp,
                    format!(
                        "the join of {}: {}.{} ({}) and {}.{} ({}) are not of one type",
                        s.alias, l.source, l.field, a.base, r.source, r.field, b.base
                    ),
                );
            }
        }
    }
    if let Some(f) = &v.filter {
        c.expr(&mut scope, f, &format!("{p}.filter"), false, 0);
    }
    let mut groups = vec![];
    for (i, g) in v.group_by.iter().enumerate() {
        let gp = format!("{p}.group_by[{i}]");
        if let Some(t) = c.column(&scope, g, &gp) {
            if !t.plain() {
                c.err(
                    "E_VIEW_TERM",
                    &gp,
                    "a list, a message or json is not grouped".into(),
                );
            }
            groups.push(g.clone());
        }
    }
    if let Some(h) = &v.having {
        c.expr(&mut scope, h, &format!("{p}.having"), true, 0);
    }
    // the columns against the row message
    let message =
        c.d.messages
            .iter()
            .find(|m| m.name == v.message && m.kind == MessageKind::Message);
    let Some(message) = message else {
        c.err(
            "E_VIEW_MESSAGE",
            &format!("{p}.message"),
            format!("there is no message {:?}", v.message),
        );
        return None;
    };
    let mut out = Columns::new();
    let grouped = v.grouped();
    let mut named = HashSet::new();
    for (i, col) in v.columns.iter().enumerate() {
        let cp = format!(
            "{p}.columns[{}]",
            if col.name.is_empty() {
                i.to_string()
            } else {
                col.name.clone()
            }
        );
        if !named.insert(col.name.as_str()) {
            c.err(
                "E_VIEW_COLUMN",
                &cp,
                format!("the column {:?} is given twice", col.name),
            );
            continue;
        }
        if matches!(col.value, Term::DaysAgo(_)) {
            c.err(
                "E_VIEW_TERM",
                &cp,
                "a column is a column of a source or an aggregate".into(),
            );
            continue;
        }
        let Some(t) = c.term(&scope, &col.value, &cp, true) else {
            continue;
        };
        if grouped
            && let Term::Column(x) = &col.value
            && !groups.contains(x)
        {
            c.err(
                "E_VIEW_GROUP",
                &cp,
                format!(
                    "{} is a column of a grouped view: group by it, or aggregate it",
                    col.name
                ),
            );
        }
        match message.fields.iter().find(|f| f.name == col.name) {
            None => c.err(
                "E_VIEW_COLUMN",
                &cp,
                format!("{} has no field {:?}", v.message, col.name),
            ),
            Some(f) => {
                let want = match FieldType::parse(&f.ty) {
                    Some(_) => f.ty.clone(),
                    None => String::new(),
                };
                // an enum is spelled by its name in the row message and by its entity's in a source
                let same = want == t.base
                    || matches!((want.strip_prefix("enum:"), t.base.strip_prefix("enum:")),
                        (Some(a), Some(b)) if b.rsplit('.').next() == Some(a) || a == b);
                if f.repeated || !same {
                    c.err(
                        "E_VIEW_TYPE",
                        &cp,
                        format!(
                            "{}.{} is a {}{}, the column a {}",
                            v.message,
                            f.name,
                            if f.repeated { "list of " } else { "" },
                            f.ty,
                            t.base
                        ),
                    );
                }
            }
        }
        out.insert(col.name.clone(), t);
    }
    for f in &message.fields {
        if !v.columns.iter().any(|x| x.name == f.name) {
            c.err(
                "E_VIEW_COLUMN",
                &format!("{p}.columns"),
                format!("{}.{} is no column of the view", v.message, f.name),
            );
        }
    }
    if grouped {
        // a group is told apart by its grouped columns: each is a column of the view
        for (i, g) in groups.iter().enumerate() {
            if !v.columns.iter().any(|x| x.value == Term::Column(g.clone())) {
                c.err(
                    "E_VIEW_GROUP",
                    &format!("{p}.group_by[{i}]"),
                    format!(
                        "{}.{}{} is grouped by but no column of the view: a group is told apart by its columns",
                        g.source,
                        g.field,
                        if g.bucket.is_empty() { String::new() } else { format!(" ({})", g.bucket) }
                    ),
                );
            }
        }
    }
    (c.errors.len() == before).then_some(out)
}
