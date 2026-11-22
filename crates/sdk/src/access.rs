//! An entity's access rules (`plugin.access::<Note>(Access::build(|a| ..))`): who reads, creates, changes and
//! deletes its rows, who may change which column to what (guards), which columns only some roles read (masks), and
//! what other plugins may inherit with `via` (export). The platform enforces them on every
//! path; the SDK only describes them.
//!
//! ```
//! use celobus_sdk::prelude::*;
//! use celobus_sdk::access::{Access, Perm, Rel};
//!
//! pub const MANAGER: Role = Role::new("manager");
//! pub const PUBLISH: Perm = Perm::new("publish");
//!
//! #[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
//! pub enum State { Draft, Published }
//!
//! #[derive(Entity, Clone, Debug)]
//! pub struct Note {
//!     pub id: Id<Note>,
//!     pub author_id: Id<platform::User>,
//!     pub state: State,
//!     pub secret: Option<String>,
//! }
//!
//! fn access() -> Access<Note> {
//!     Access::build(|a| {
//!         let author = a.relation("author", Rel::column(Note::author_id));
//!         a.read(author.clone() | MANAGER);
//!         a.create(author.clone());
//!         a.update(author.when(Note::state.eq(State::Draft)) | MANAGER);
//!         a.permit(PUBLISH, MANAGER);
//!         a.guard(Note::state).to(State::Published, Grant::holds(PUBLISH));
//!         a.mask(Note::secret, [MANAGER]);
//!     })
//! }
//! # use celobus_sdk::access::Grant;
//! ```
//!
//! Grants combine with `|` (any of), `&` (all of) and `!` (not). Undeclared means denied: an entity without rules is
//! read and written by no one but a system context (`ctx.as_system(reason)`, within the plugin's `authz:bypass`).

use std::marker::PhantomData;
use std::ops::{BitAnd, BitOr, Not};

use celobus_proto as pb;
use pb::{db_condition::Op, db_filter::Node, db_value::Kind};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db::Filter;
use crate::identity::Role;
use crate::message::{Entity, FieldValue};
use crate::ui::Field;

/// A permission: an operation (`Perm::READ`, `CREATE`, `UPDATE`, `DELETE`) or a custom action of the entity
/// (`Perm::new("publish")`), what `permit`, `holds`, `via` and `export` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perm(&'static str);

impl Perm {
    pub const READ: Perm = Perm("read");
    pub const CREATE: Perm = Perm("create");
    pub const UPDATE: Perm = Perm("update");
    pub const DELETE: Perm = Perm("delete");

    /// A custom action: snake_case.
    pub const fn new(name: &'static str) -> Self {
        Perm(name)
    }

    pub const fn name(&self) -> &'static str {
        self.0
    }
}

/// What a relation of a row names.
pub struct Rel {
    kind: &'static str,
    field: &'static str,
    rows: Option<Value>,
}

impl Rel {
    /// A column that names a person (`Id<platform::User>`), or a row of an entity whose permissions `via` inherits.
    pub fn column<E: Entity, T: FieldValue>(f: Field<E, T>) -> Rel {
        Rel {
            kind: "column",
            field: f.name(),
            rows: None,
        }
    }

    /// A group (`Id<platform::Group>`): its members.
    pub fn group<E: Entity, T: FieldValue>(f: Field<E, T>) -> Rel {
        Rel {
            kind: "group",
            field: f.name(),
            rows: None,
        }
    }

    /// The rows of a junction entity `J` of this plugin that point to the row: each makes the person (an
    /// `Id<platform::User>`) or the group (an `Id<platform::Group>`, its members) its `subject` column names a subject
    /// of the relation. `target` is `J`'s ref to the entity whose access declares the relation. Checked when the
    /// statement runs, so adding, deleting or soft-deleting a junction row grants or revokes at once.
    /// `.when(filter)` counts only the junction rows matching a filter on `J`'s own fields.
    ///
    /// ```ignore
    /// let member = a.relation("member", Rel::rows(ChannelMember::channel_id, ChannelMember::user_id));
    /// let admin = a.relation("admin", Rel::rows(ChannelMember::channel_id, ChannelMember::user_id)
    ///     .when(ChannelMember::role.eq(Role::Admin)));
    /// ```
    pub fn rows<J: Entity, T: FieldValue, S: FieldValue>(
        target: Field<J, T>,
        subject: Field<J, S>,
    ) -> Rows<J> {
        Rows {
            target: target.name(),
            subject: subject.name(),
            filter: vec![],
            _j: PhantomData,
        }
    }
}

/// A relation through the rows of the junction entity `J` (see [`Rel::rows`]).
pub struct Rows<J> {
    target: &'static str,
    subject: &'static str,
    filter: Vec<Value>,
    _j: PhantomData<fn() -> J>,
}

impl<J: Entity> Rows<J> {
    /// Only the junction rows matching `filter` (equality, inequality, `is_in` and null tests on `J`'s own fields,
    /// combined with `&`).
    pub fn when(mut self, filter: Filter<J>) -> Rows<J> {
        if let Some(n) = filter.node() {
            self.filter.extend(conditions(n));
        }
        self
    }
}

impl<J: Entity> From<Rows<J>> for Rel {
    fn from(r: Rows<J>) -> Rel {
        // another plugin's entity is named with its plugin, which the platform's check refuses
        let entity = match J::PLUGIN {
            Some(p) => format!("{p}.{}", J::ENTITY),
            None => J::ENTITY.to_string(),
        };
        Rel {
            kind: "rows",
            field: r.subject,
            rows: Some(json!({ "entity": entity, "target": r.target, "filter": r.filter })),
        }
    }
}

/// A grant: who passes. From a [`Relation`], a [`Role`], [`Grant::public`], [`Grant::authenticated`],
/// [`Grant::holds`]; combined with `|`, `&`, `!`.
#[derive(Debug, Clone, PartialEq)]
pub struct Grant(Value);

impl Grant {
    /// Everyone, members or not.
    pub fn public() -> Grant {
        Grant(json!("public"))
    }

    /// Every member (a person or a service account).
    pub fn authenticated() -> Grant {
        Grant(json!("authenticated"))
    }

    /// Whoever the grant of a custom action admits.
    pub fn holds(action: Perm) -> Grant {
        Grant(json!({ "holds": action.name() }))
    }

    /// This grant, on the rows matching `filter` (equality, inequality, `is_in` and null tests on the row's own
    /// fields, combined with `&`).
    pub fn when<E>(self, filter: Filter<E>) -> Grant {
        let filter = match filter.node() {
            Some(n) => conditions(n),
            None => vec![],
        };
        Grant(json!({ "when": { "grant": self.0, "filter": filter } }))
    }

    fn any(self, other: Grant) -> Grant {
        let mut items = match self.0 {
            Value::Object(mut o) if o.contains_key("any") => match o.remove("any") {
                Some(Value::Array(a)) => a,
                _ => vec![],
            },
            v => vec![v],
        };
        items.push(other.0);
        Grant(json!({ "any": items }))
    }

    fn all(self, other: Grant) -> Grant {
        let mut items = match self.0 {
            Value::Object(mut o) if o.contains_key("all") => match o.remove("all") {
                Some(Value::Array(a)) => a,
                _ => vec![],
            },
            v => vec![v],
        };
        items.push(other.0);
        Grant(json!({ "all": items }))
    }
}

/// A filter as access conditions (only and-of comparisons; anything else becomes a condition the platform's check
/// refuses with `E_ACCESS_WHEN_FIELD`).
fn conditions(n: &pb::DbFilter) -> Vec<Value> {
    let value = |v: &pb::DbValue| match &v.kind {
        Some(Kind::StringValue(s)) => json!(s),
        Some(Kind::IntValue(i)) => json!(i),
        Some(Kind::DoubleValue(d)) => json!(d),
        Some(Kind::BoolValue(b)) => json!(b),
        _ => Value::Null,
    };
    match &n.node {
        Some(Node::And(list)) => list.items.iter().flat_map(conditions).collect(),
        Some(Node::Condition(c)) => {
            let op = Op::try_from(c.op).unwrap_or(Op::Unspecified);
            let (op, v) = match op {
                Op::Eq => ("eq", c.values.first().map(value).unwrap_or_default()),
                Op::Ne => ("ne", c.values.first().map(value).unwrap_or_default()),
                Op::In => ("in", Value::Array(c.values.iter().map(value).collect())),
                Op::IsNull => ("is_null", Value::Null),
                Op::IsNotNull => ("is_not_null", Value::Null),
                _ => ("unsupported", Value::Null),
            };
            vec![json!({ "field": c.field, "op": op, "value": v })]
        }
        _ => vec![json!({ "field": "", "op": "unsupported", "value": null })],
    }
}

impl From<Role> for Grant {
    fn from(r: Role) -> Grant {
        Grant(json!({ "role": r.name() }))
    }
}

impl From<Relation> for Grant {
    fn from(r: Relation) -> Grant {
        Grant(json!({ "rel": r.name }))
    }
}

impl From<&Relation> for Grant {
    fn from(r: &Relation) -> Grant {
        Grant(json!({ "rel": r.name }))
    }
}

impl<R: Into<Grant>> BitOr<R> for Grant {
    type Output = Grant;
    fn bitor(self, rhs: R) -> Grant {
        self.any(rhs.into())
    }
}

impl<R: Into<Grant>> BitAnd<R> for Grant {
    type Output = Grant;
    fn bitand(self, rhs: R) -> Grant {
        self.all(rhs.into())
    }
}

impl Not for Grant {
    type Output = Grant;
    fn not(self) -> Grant {
        Grant(json!({ "not": self.0 }))
    }
}

impl<R: Into<Grant>> BitOr<R> for Relation {
    type Output = Grant;
    fn bitor(self, rhs: R) -> Grant {
        Grant::from(self) | rhs
    }
}

impl<R: Into<Grant>> BitAnd<R> for Relation {
    type Output = Grant;
    fn bitand(self, rhs: R) -> Grant {
        Grant::from(self) & rhs
    }
}

impl<R: Into<Grant>> BitOr<R> for Role {
    type Output = Grant;
    fn bitor(self, rhs: R) -> Grant {
        Grant::from(self) | rhs
    }
}

impl Not for Role {
    type Output = Grant;
    fn not(self) -> Grant {
        !Grant::from(self)
    }
}

/// A relation of the entity's rows, from [`Access::relation`]: a grant by itself, or `.via(perm)` for a relation
/// to a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    name: String,
}

impl Relation {
    /// The caller holds `perm` on the row this relation points to (another plugin's row: a permission it
    /// exports).
    pub fn via(&self, perm: Perm) -> Grant {
        Grant(json!({ "via": { "rel": self.name, "perm": perm.name() } }))
    }

    /// The relation, on the rows matching `filter`.
    pub fn when<E>(&self, filter: Filter<E>) -> Grant {
        Grant::from(self).when(filter)
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub(crate) struct AccessDecl {
    relations: Vec<Value>,
    read: Option<Value>,
    create: Option<Value>,
    update: Option<Value>,
    delete: Option<Value>,
    permits: Vec<Value>,
    guards: Vec<Value>,
    masks: Vec<Value>,
    export: Vec<String>,
    admin: bool,
}

/// The rules of the entity `E`.
pub struct Access<E> {
    pub(crate) decl: AccessDecl,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> Access<E> {
    /// The rules `f` declares.
    pub fn build(f: impl FnOnce(&mut Access<E>)) -> Access<E> {
        let mut a = Access {
            decl: AccessDecl {
                relations: vec![],
                read: None,
                create: None,
                update: None,
                delete: None,
                permits: vec![],
                guards: vec![],
                masks: vec![],
                export: vec![],
                admin: true,
            },
            _e: PhantomData,
        };
        f(&mut a);
        a
    }

    /// Declares a relation; the handle is its grant.
    pub fn relation(&mut self, name: &str, rel: impl Into<Rel>) -> Relation {
        let rel = rel.into();
        let mut decl = json!({ "name": name, "kind": rel.kind, "field": rel.field });
        if let Some(rows) = rel.rows {
            decl["rows"] = rows;
        }
        self.decl.relations.push(decl);
        Relation {
            name: name.to_string(),
        }
    }

    pub fn read(&mut self, g: impl Into<Grant>) {
        self.decl.read = Some(g.into().0);
    }

    /// Checks the new row (an owner relation must name the caller).
    pub fn create(&mut self, g: impl Into<Grant>) {
        self.decl.create = Some(g.into().0);
    }

    /// Checks the row as it was; the row as it becomes must stay readable to the writer.
    pub fn update(&mut self, g: impl Into<Grant>) {
        self.decl.update = Some(g.into().0);
    }

    pub fn delete(&mut self, g: impl Into<Grant>) {
        self.decl.delete = Some(g.into().0);
    }

    /// A custom action and its grant (what [`Grant::holds`] names).
    pub fn permit(&mut self, action: Perm, g: impl Into<Grant>) {
        self.decl
            .permits
            .push(json!({ "action": action.name(), "grant": g.into().0 }));
    }

    /// Who may change a column, on update: `.to(value, grant)`, `.to_any(grant)`.
    pub fn guard<T: FieldValue>(&mut self, f: Field<E, T>) -> Guard<'_, E, T> {
        self.decl
            .guards
            .push(json!({ "fields": [f.name()], "rules": [] }));
        Guard {
            access: self,
            _t: PhantomData,
        }
    }

    /// Who may change any of several columns (`.to_any(grant)` only).
    pub fn guard_fields(
        &mut self,
        fields: impl IntoIterator<Item = &'static str>,
    ) -> Guard<'_, E, ()> {
        let fields: Vec<&str> = fields.into_iter().collect();
        self.decl
            .guards
            .push(json!({ "fields": fields, "rules": [] }));
        Guard {
            access: self,
            _t: PhantomData,
        }
    }

    /// Only these roles read the column (an optional field: others read it as absent).
    pub fn mask<T: FieldValue>(
        &mut self,
        f: Field<E, Option<T>>,
        roles: impl IntoIterator<Item = Role>,
    ) {
        let roles: Vec<&str> = roles.into_iter().map(|r| r.name()).collect();
        self.decl
            .masks
            .push(json!({ "field": f.name(), "roles": roles }));
    }

    /// Permissions other plugins may inherit with `via` (their grants read only the row's own columns).
    pub fn export(&mut self, perms: impl IntoIterator<Item = Perm>) {
        self.decl
            .export
            .extend(perms.into_iter().map(|p| p.name().to_string()));
    }

    /// `workspace.admin` stands in for the roles that are not strict (the default); `false` for rows admins must
    /// not see.
    pub fn admin_bypass(&mut self, yes: bool) {
        self.decl.admin = yes;
    }
}

/// The rules of a guard (see [`Access::guard`]).
pub struct Guard<'a, E, T> {
    access: &'a mut Access<E>,
    _t: PhantomData<fn() -> T>,
}

impl<E, T> Guard<'_, E, T> {
    fn rule(self, value: Value, g: Grant) -> Self {
        if let Some(Value::Array(rules)) = self
            .access
            .decl
            .guards
            .last_mut()
            .and_then(|x| x.get_mut("rules"))
        {
            rules.push(json!({ "value": value, "grant": g.0 }));
        }
        self
    }

    /// To any value, by whoever `g` admits.
    pub fn to_any(self, g: impl Into<Grant>) -> Self {
        self.rule(Value::Null, g.into())
    }
}

impl<E, T: FieldValue> Guard<'_, E, T> {
    /// To `value`, by whoever `g` admits (evaluated on the row as it was).
    pub fn to(self, value: T, g: impl Into<Grant>) -> Self {
        let v = value.encode();
        self.rule(v, g.into())
    }
}

/// The operations a system context may bypass on an entity (`plugin.bypass::<Note>([Perm::READ])`).
pub(crate) fn bypass_decl(entity: &str, ops: &[Perm]) -> Value {
    json!({ "entity": entity, "ops": ops.iter().map(|p| p.name()).collect::<Vec<_>>() })
}
