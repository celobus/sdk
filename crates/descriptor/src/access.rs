//! An entity's access rules (`entities[].access`): the relations its rows have to
//! people, groups and other rows, the grants of its operations (`read`, `create`, `update`, `delete`) and of its
//! custom actions (`permits`), the guards on the changes of its columns, the column masks, and the permissions it
//! exports for `via` from other plugins. The platform enforces them on every read and write; this module holds
//! their schema and their definition-time rules.
//!
//! A grant is a tree: `{"rel": "owner"}` (the relation names the caller, or a group of the caller; for a `rows`
//! relation, a row of its junction that points to the row does),
//! `{"via": {"rel": "notebook", "perm": "read"}}` (the caller holds `perm` on the row the relation points to),
//! `{"role": "manager"}` (a role of this plugin, or `workspace.<role>`), `"authenticated"`, `"public"`,
//! `{"holds": "cancel"}` (the grant of a custom action), `{"any": [..]}`, `{"all": [..]}`, `{"not": ..}` and
//! `{"when": {"grant": .., "filter": [..]}}` (the grant, on rows whose own columns match the filter).
//!
//! An entity without `access`, and an operation without a grant, is denied to everyone but a system context.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Descriptor, Entity, Error, FieldType};

/// The deepest chain of `via` an entity's rules may reach through.
pub const MAX_VIA_DEPTH: usize = 3;

/// The operations of an entity that have grants.
pub const OPERATIONS: &[&str] = &["read", "create", "update", "delete"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Access {
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub read: Option<Grant>,
    #[serde(default)]
    pub create: Option<Grant>,
    #[serde(default)]
    pub update: Option<Grant>,
    #[serde(default)]
    pub delete: Option<Grant>,
    /// Custom actions (`cancel`): what `holds` names.
    #[serde(default)]
    pub permits: Vec<Permit>,
    #[serde(default)]
    pub guards: Vec<Guard>,
    #[serde(default)]
    pub masks: Vec<Mask>,
    /// The permissions other plugins may name in `via` (`read`, `update`, `delete`, or an action).
    #[serde(default)]
    pub export: Vec<String>,
    /// `workspace.admin` stands in for every role of the rules that is not strict (default true).
    #[serde(default = "yes")]
    pub admin: bool,
}

fn yes() -> bool {
    true
}

/// A relation of a row: `column` (a ref to a person, or to a row whose permissions `via` inherits), `users` (a
/// list of people the row is shared with), `group` (a ref to a group: its members), `rows` (the rows of a junction
/// entity of the plugin that point to the row: `field` is the junction's column naming a person or a group, `rows`
/// says which entity and which of its columns points to the row).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub name: String,
    pub kind: RelationKind,
    pub field: String,
    /// Only for `rows`: the junction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Rows>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RelationKind {
    Column,
    Users,
    Group,
    Rows,
}

/// The junction of a `rows` relation: its rows whose `target` column points to the row (and that match `filter`, on
/// the junction's own columns) make the person or group their `field` names a subject of the relation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rows {
    /// An entity of the same plugin.
    pub entity: String,
    /// Its column that refers to the entity whose access declares the relation.
    pub target: String,
    #[serde(default)]
    pub filter: Vec<Condition>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    Rel(String),
    Via(Via),
    Role(String),
    Authenticated,
    Public,
    Holds(String),
    Any(Vec<Grant>),
    All(Vec<Grant>),
    Not(Box<Grant>),
    When(Box<When>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Via {
    pub rel: String,
    pub perm: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct When {
    pub grant: Grant,
    /// All of these conditions on the row's own columns.
    pub filter: Vec<Condition>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub field: String,
    /// `eq`, `ne`, `in`, `is_null`, `is_not_null`
    pub op: String,
    /// A value (`eq`, `ne`) or a list (`in`), in the protobuf JSON of the field; none for the null tests.
    #[serde(default)]
    pub value: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Permit {
    pub action: String,
    pub grant: Grant,
}

/// Who may change these columns, on update: to a `value` (one column) with a grant, or to anything (`value` absent).
/// Grants are evaluated on the row as it was. A change no rule admits is `guard_denied`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    pub fields: Vec<String>,
    pub rules: Vec<GuardRule>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GuardRule {
    #[serde(default)]
    pub value: Value,
    pub grant: Grant,
}

/// A column only these roles read (others read it as absent; a system context reads it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub field: String,
    pub roles: Vec<String>,
}

impl Access {
    /// The grant of an operation or a custom action.
    pub fn grant(&self, perm: &str) -> Option<&Grant> {
        match perm {
            "read" => self.read.as_ref(),
            "create" => self.create.as_ref(),
            "update" => self.update.as_ref(),
            "delete" => self.delete.as_ref(),
            action => self
                .permits
                .iter()
                .find(|p| p.action == action)
                .map(|p| &p.grant),
        }
    }

    pub fn relation(&self, name: &str) -> Option<&Relation> {
        self.relations.iter().find(|r| r.name == name)
    }
}

impl Grant {
    /// Every grant of the tree, itself first.
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Grant>) {
        out.push(self);
        match self {
            Grant::Any(items) | Grant::All(items) => items.iter().for_each(|g| g.walk(out)),
            Grant::Not(g) => g.walk(out),
            Grant::When(w) => w.grant.walk(out),
            _ => {}
        }
    }
}

/// The ops a capability `authz:bypass` may name for an entity.
pub const BYPASS_OPS: &[&str] = &["read", "create", "update", "delete"];

/// Where an entity's relation points: (plugin, entity) of a `column` relation to a row.
fn target(d: &Descriptor, e: &Entity, r: &Relation) -> Option<(String, String)> {
    if r.kind != RelationKind::Column {
        return None;
    }
    let f = e.fields.iter().find(|f| f.name == r.field)?;
    match FieldType::parse(&f.ty)? {
        FieldType::Ref { plugin, entity } if plugin != "platform" && !f.repeated => {
            let plugin = if plugin == d.id { d.id.clone() } else { plugin };
            Some((plugin, entity))
        }
        _ => None,
    }
}

/// The definition-time rules of every entity's access, and of the `authz:bypass` capability's scope.
pub(crate) fn validate(d: &Descriptor) -> Vec<Error> {
    let mut out = vec![];
    for e in &d.entities {
        if let Some(a) = &e.access {
            out.extend(entity(d, e, a));
        }
    }
    out.extend(cycles(d));
    out.extend(bypass(d));
    out
}

struct Check<'a> {
    d: &'a Descriptor,
    e: &'a Entity,
    a: &'a Access,
    p: String,
    out: Vec<Error>,
}

fn entity(d: &Descriptor, e: &Entity, a: &Access) -> Vec<Error> {
    let mut c = Check {
        d,
        e,
        a,
        p: format!("entities[{}].access", e.name),
        out: vec![],
    };
    c.relations();
    for op in OPERATIONS {
        if let Some(g) = a.grant(op) {
            if e.append_only && matches!(*op, "update" | "delete") {
                c.out.push(
                    Error::new(
                        "E_ENT_APPEND_ONLY_UPDATE",
                        format!("{}.{op}", c.p),
                        format!(
                            "{} is append-only: its rows are not changed or deleted",
                            e.name
                        ),
                    )
                    .help("drop the grant"),
                );
            }
            c.grant(g, &format!("{}.{op}", c.p), op);
        }
    }
    let mut actions = HashSet::new();
    for (i, p) in a.permits.iter().enumerate() {
        let path = format!("{}.permits[{i}]", c.p);
        if !crate::validate::snake(&p.action) || OPERATIONS.contains(&p.action.as_str()) {
            c.out.push(
                Error::new(
                    "E_ACCESS_ACTION_NAME",
                    format!("{path}.action"),
                    format!("{:?} is not an action name", p.action),
                )
                .help("snake_case, not one of read, create, update, delete"),
            );
        }
        if !actions.insert(p.action.as_str()) {
            c.out.push(Error::new(
                "E_ACCESS_ACTION_DUP",
                path.clone(),
                format!("action {:?} is declared twice", p.action),
            ));
        }
        c.grant(&p.grant, &format!("{path}.grant"), &p.action);
    }
    c.guards();
    c.masks();
    for (i, x) in a.export.iter().enumerate() {
        let path = format!("{}.export[{i}]", c.p);
        match a.grant(x) {
            None => c.out.push(
                Error::new(
                    "E_ACCESS_EXPORT",
                    path,
                    format!("{x:?} has no grant to export"),
                )
                .help("export read, update, delete or a declared action"),
            ),
            Some(g) => {
                let mut all = vec![];
                g.walk(&mut all);
                if all.iter().any(|g| matches!(g, Grant::Via(_))) {
                    c.out.push(
                        Error::new(
                            "E_ACCESS_EXPORT_NOT_ROW_LOCAL",
                            path.clone(),
                            format!("the {x} grant of {} uses via: an exported grant reads only the row's own columns, groups and roles", e.name),
                        ),
                    );
                }
                let rows = all.iter().any(|g| {
                    matches!(g, Grant::Rel(name)
                        if a.relation(name).is_some_and(|r| r.kind == RelationKind::Rows))
                });
                if rows {
                    c.out.push(
                        Error::new(
                            "E_ACCESS_EXPORT_NOT_ROW_LOCAL",
                            path,
                            format!("the {x} grant of {} uses a rows relation: an exported grant reads only the row's own columns, groups and roles, not a junction's rows", e.name),
                        ),
                    );
                }
            }
        }
    }
    c.out
}

impl Check<'_> {
    fn relations(&mut self) {
        let mut names = HashSet::new();
        for (i, r) in self.a.relations.iter().enumerate() {
            let path = format!("{}.relations[{i}]", self.p);
            if !crate::validate::snake(&r.name) {
                self.out.push(Error::new(
                    "E_ACCESS_REL_NAME",
                    format!("{path}.name"),
                    format!("{:?} is not a relation name", r.name),
                ));
            }
            if !names.insert(r.name.as_str()) {
                self.out.push(Error::new(
                    "E_ACCESS_REL_NAME",
                    path.clone(),
                    format!("relation {:?} is declared twice", r.name),
                ));
            }
            let field = self
                .e
                .fields
                .iter()
                .find(|f| f.name == r.field && f.name != "id");
            let ty = field.and_then(|f| FieldType::parse(&f.ty));
            let is = |plugin: &str, entity: &str| matches!(&ty, Some(FieldType::Ref { plugin: p, entity: e }) if p == plugin && e == entity);
            let repeated = field.is_some_and(|f| f.repeated);
            let (ok, code, want) = match r.kind {
                RelationKind::Column => (
                    !repeated
                        && (is("platform", "user")
                            || matches!(&ty, Some(FieldType::Ref { plugin, .. }) if plugin != "platform")),
                    "E_ACCESS_REL_COLUMN",
                    "a ref to a person (platform.user) or to a row of an entity",
                ),
                // a list of people in a column is no relation any more: they are rows of a junction entity
                RelationKind::Users => {
                    self.out.push(Error::new(
                        "E_ACCESS_USERS",
                        path.clone(),
                        format!(
                            "relation {:?}: a list of people in a column is not a relation; keep them as rows of a \
                             junction entity of the plugin (one per person, with a ref to this row and one to the \
                             person) and grant through it with a rows relation (Rel::rows)",
                            r.name
                        ),
                    ));
                    continue;
                }
                RelationKind::Group => (
                    !repeated && is("platform", "group"),
                    "E_ACCESS_GROUP",
                    "a ref to a group (platform.group)",
                ),
                RelationKind::Rows => {
                    self.rows(r, &path);
                    continue;
                }
            };
            if r.rows.is_some() {
                self.out.push(Error::new(
                    "E_ACCESS_ROWS_TARGET",
                    format!("{path}.rows"),
                    format!(
                        "relation {:?} is not a rows relation: it has no junction",
                        r.name
                    ),
                ));
            }
            if !ok {
                self.out.push(
                    Error::new(
                        code,
                        format!("{path}.field"),
                        format!("{:?} is not {want} of {}", r.field, self.e.name),
                    )
                    .help("name a field of the entity with that type"),
                );
            }
        }
    }

    /// A `rows` relation: a junction of this plugin, its column pointing to this entity, its column naming a
    /// person or a group, its filter on its own columns.
    fn rows(&mut self, r: &Relation, path: &str) {
        let Some(rows) = &r.rows else {
            self.out.push(
                Error::new(
                    "E_ACCESS_ROWS_PLUGIN",
                    path,
                    format!("relation {:?} names no junction entity", r.name),
                )
                .help("rows: { entity, target, filter }"),
            );
            return;
        };
        let Some(j) = self.d.entities.iter().find(|x| x.name == rows.entity) else {
            self.out.push(
                Error::new(
                    "E_ACCESS_ROWS_PLUGIN",
                    format!("{path}.rows.entity"),
                    format!("{:?} is not an entity of {}", rows.entity, self.d.id),
                )
                .help("the junction is an entity of the same plugin"),
            );
            return;
        };
        let single = |name: &str| {
            j.fields
                .iter()
                .find(|f| f.name == name && f.name != "id" && !f.repeated)
                .and_then(|f| FieldType::parse(&f.ty))
        };
        let points_here = matches!(single(&rows.target),
            Some(FieldType::Ref { plugin, entity }) if plugin == self.d.id && entity == self.e.name);
        if !points_here {
            self.out.push(
                Error::new(
                    "E_ACCESS_ROWS_TARGET",
                    format!("{path}.rows.target"),
                    format!(
                        "{:?} is not a ref of {} to {}",
                        rows.target, j.name, self.e.name
                    ),
                )
                .help(format!(
                    "a field of {} with the type ref:{}.{}",
                    j.name, self.d.id, self.e.name
                )),
            );
        }
        let subject = matches!(single(&r.field),
            Some(FieldType::Ref { plugin, entity }) if plugin == "platform" && (entity == "user" || entity == "group"));
        if !subject {
            self.out.push(
                Error::new(
                    "E_ACCESS_ROWS_SUBJECT",
                    format!("{path}.field"),
                    format!(
                        "{:?} is not a ref of {} to a person (platform.user) or a group (platform.group)",
                        r.field, j.name
                    ),
                )
                .help("name the junction's column of the member"),
            );
        }
        for (i, c) in rows.filter.iter().enumerate() {
            condition(
                self.d,
                j,
                c,
                &format!("{path}.rows.filter[{i}]"),
                &mut self.out,
            );
        }
    }

    fn role(&mut self, role: &str, path: &str) {
        let known = match role.split_once('.') {
            Some(("workspace", r)) => matches!(r, "owner" | "admin" | "member" | "guest"),
            Some(_) => false,
            None => self.d.roles.iter().any(|r| r.name == role),
        };
        if !known {
            self.out.push(
                Error::new("E_ACCESS_ROLE_UNKNOWN", path, format!("role {role:?} is not declared"))
                    .help(
                        "declare it in the plugin, or use a workspace role: workspace.owner, workspace.admin, workspace.member",
                    ),
            );
        }
    }

    fn grant(&mut self, g: &Grant, path: &str, perm: &str) {
        match g {
            Grant::Rel(name) => match self.a.relation(name) {
                None => self.out.push(Error::new(
                    "E_ACCESS_REL_UNKNOWN",
                    path,
                    format!("relation {name:?} is not declared"),
                )),
                Some(r) if r.kind == RelationKind::Column && target(self.d, self.e, r).is_some() => {
                    self.out.push(
                        Error::new(
                            "E_ACCESS_REL_COLUMN",
                            path,
                            format!("relation {name:?} points to a row, not to a person: name a permission on it"),
                        )
                        .help(format!("{{\"via\": {{\"rel\": \"{name}\", \"perm\": \"read\"}}}}")),
                    )
                }
                Some(_) => {}
            },
            Grant::Via(v) => match self.a.relation(&v.rel) {
                None => self.out.push(Error::new(
                    "E_ACCESS_REL_UNKNOWN",
                    path,
                    format!("relation {:?} is not declared", v.rel),
                )),
                Some(r) => match target(self.d, self.e, r) {
                    None => self.out.push(
                        Error::new(
                            "E_ACCESS_VIA_TARGET",
                            path,
                            format!("relation {:?} does not point to a row of an entity", v.rel),
                        )
                        .help("via follows a column relation whose field is a ref to an entity"),
                    ),
                    Some((plugin, entity)) if plugin == self.d.id => {
                        let t = self.d.entities.iter().find(|x| x.name == entity);
                        let has = t
                            .and_then(|t| t.access.as_ref())
                            .is_some_and(|a| a.grant(&v.perm).is_some());
                        if !has {
                            self.out.push(Error::new(
                                "E_ACCESS_VIA_TARGET",
                                path,
                                format!("{entity} has no grant {:?} to inherit", v.perm),
                            ));
                        }
                    }
                    Some((plugin, _)) => {
                        if !self.d.dependencies.iter().any(|x| x.plugin == plugin) {
                            self.out.push(
                                Error::new(
                                    "E_DEP_UNDECLARED",
                                    path,
                                    format!("via into {plugin}, which is not a dependency"),
                                )
                                .help(format!("celobus plugin add {plugin}")),
                            );
                        }
                    }
                },
            },
            Grant::Role(role) => self.role(role, path),
            Grant::Authenticated | Grant::Public => {}
            Grant::Holds(action) => {
                if !self.a.permits.iter().any(|p| &p.action == action) {
                    self.out.push(Error::new(
                        "E_ACCESS_ACTION_UNKNOWN",
                        path,
                        format!("action {action:?} is not declared"),
                    ));
                } else if action == perm {
                    self.out.push(Error::new(
                        "E_ACCESS_CYCLE",
                        path,
                        format!("the grant of {action} holds itself"),
                    ));
                }
            }
            Grant::Any(items) | Grant::All(items) => {
                for (i, g) in items.iter().enumerate() {
                    self.grant(g, &format!("{path}[{i}]"), perm);
                }
            }
            Grant::Not(g) => self.grant(g, &format!("{path}.not"), perm),
            Grant::When(w) => {
                self.grant(&w.grant, &format!("{path}.when.grant"), perm);
                for (i, c) in w.filter.iter().enumerate() {
                    self.condition(c, &format!("{path}.when.filter[{i}]"));
                }
            }
        }
    }

    fn column(&self, name: &str) -> Option<&crate::Field> {
        self.e.fields.iter().find(|f| f.name == name)
    }

    fn condition(&mut self, c: &Condition, path: &str) {
        condition(self.d, self.e, c, path, &mut self.out);
    }

    fn guards(&mut self) {
        let mut guarded = HashSet::new();
        for (i, g) in self.a.guards.iter().enumerate() {
            let path = format!("{}.guards[{i}]", self.p);
            if self.e.append_only {
                self.out.push(Error::new(
                    "E_ENT_APPEND_ONLY_UPDATE",
                    path.clone(),
                    format!("{} is append-only: it has no updates to guard", self.e.name),
                ));
            }
            if g.fields.is_empty() || g.rules.is_empty() {
                self.out.push(Error::new(
                    "E_ACCESS_GUARD_FIELD",
                    path.clone(),
                    "a guard names its fields and at least one rule",
                ));
            }
            for (j, name) in g.fields.iter().enumerate() {
                if name == "id" || self.column(name).is_none() {
                    self.out.push(Error::new(
                        "E_ACCESS_GUARD_FIELD",
                        format!("{path}.fields[{j}]"),
                        format!(
                            "{name:?} is not a field of {} a guard can watch",
                            self.e.name
                        ),
                    ));
                } else if !guarded.insert(name.as_str()) {
                    self.out.push(Error::new(
                        "E_ACCESS_GUARD_FIELD",
                        format!("{path}.fields[{j}]"),
                        format!("{name:?} has two guards"),
                    ));
                }
            }
            for (j, r) in g.rules.iter().enumerate() {
                let rp = format!("{path}.rules[{j}]");
                if !r.value.is_null() {
                    let fits = g.fields.len() == 1
                        && self
                            .column(&g.fields[0])
                            .is_some_and(|f| !f.repeated && value_fits(self.d, f, &r.value));
                    if !fits {
                        self.out.push(
                            Error::new(
                                "E_ACCESS_GUARD_VALUE",
                                format!("{rp}.value"),
                                format!("{} is not a value of {}", r.value, g.fields.join(", ")),
                            )
                            .help("a value guard watches one field, with a value of its type"),
                        );
                    }
                }
                self.grant(&r.grant, &format!("{rp}.grant"), "update");
            }
        }
    }

    fn masks(&mut self) {
        let mut seen = HashSet::new();
        for (i, m) in self.a.masks.iter().enumerate() {
            let path = format!("{}.masks[{i}]", self.p);
            match self.column(&m.field) {
                _ if m.field == "id" => self.out.push(Error::new(
                    "E_ACCESS_MASK_KEY",
                    format!("{path}.field"),
                    "the key is never masked",
                )),
                None => self.out.push(Error::new(
                    "E_ACCESS_MASK_FIELD",
                    format!("{path}.field"),
                    format!("{:?} is not a field of {}", m.field, self.e.name),
                )),
                Some(f) if !f.optional && !f.repeated => self.out.push(
                    Error::new(
                        "E_ACCESS_MASK_REQUIRED",
                        format!("{path}.field"),
                        format!("{} is required: a reader without the role would read a value that is not there", m.field),
                    )
                    .help("make the field optional (it reads as absent when masked)"),
                ),
                Some(f) if f.unique => self.out.push(Error::new(
                    "E_ACCESS_MASK_KEY",
                    format!("{path}.field"),
                    format!("{} is unique: a key is never masked", m.field),
                )),
                Some(_) => {}
            }
            if !seen.insert(m.field.as_str()) {
                self.out.push(Error::new(
                    "E_ACCESS_MASK_FIELD",
                    path.clone(),
                    format!("{} is masked twice", m.field),
                ));
            }
            if m.roles.is_empty() {
                self.out.push(Error::new(
                    "E_ACCESS_MASK_FIELD",
                    format!("{path}.roles"),
                    "a mask names the roles that read the column",
                ));
            }
            for (j, r) in m.roles.iter().enumerate() {
                self.role(r, &format!("{path}.roles[{j}]"));
            }
        }
    }
}

/// A condition of a `when` filter (or of a junction's filter) on `e`'s own columns.
fn condition(d: &Descriptor, e: &Entity, c: &Condition, path: &str, out: &mut Vec<Error>) {
    let Some(f) = e.fields.iter().find(|f| f.name == c.field && !f.repeated) else {
        out.push(Error::new(
            "E_ACCESS_WHEN_FIELD",
            path,
            format!(
                "{:?} is not a field of {} a filter can test",
                c.field, e.name
            ),
        ));
        return;
    };
    let ok = match c.op.as_str() {
        "eq" | "ne" => value_fits(d, f, &c.value),
        "in" => c
            .value
            .as_array()
            .is_some_and(|v| !v.is_empty() && v.iter().all(|x| value_fits(d, f, x))),
        "is_null" | "is_not_null" => c.value.is_null(),
        _ => false,
    };
    if !ok {
        out.push(
            Error::new(
                "E_ACCESS_WHEN_FIELD",
                path,
                format!(
                    "{} {} {}: not an operator, or a value of another type",
                    c.field, c.op, c.value
                ),
            )
            .help("eq, ne (a value), in (a list), is_null, is_not_null (no value)"),
        );
    }
}

/// Whether a JSON value is one of the field's type (an enum's value, a string for text and the string forms, a
/// number, a bool).
fn value_fits(d: &Descriptor, f: &crate::Field, v: &Value) -> bool {
    match FieldType::parse(&f.ty) {
        Some(FieldType::Scalar(s)) => match s {
            "bool" => v.is_boolean(),
            "int32" | "double" => v.is_number(),
            "int64" | "decimal" => v.is_number() || v.is_string(),
            "json" | "bytes" | "file" => false,
            _ => v.is_string(),
        },
        Some(FieldType::Enum(name)) => v.as_str().is_some_and(|x| {
            d.enums
                .iter()
                .find(|e| e.name == name)
                .is_some_and(|e| e.values.iter().any(|y| y == x))
        }),
        Some(FieldType::Ref { .. }) => v.is_string() || v.is_number(),
        _ => false,
    }
}

/// `via` across the plugin's own entities: no cycle, at most [`MAX_VIA_DEPTH`] deep.
fn cycles(d: &Descriptor) -> Vec<Error> {
    // entity -> entities its grants inherit from
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &d.entities {
        let Some(a) = &e.access else { continue };
        let mut all = vec![];
        for op in OPERATIONS {
            if let Some(g) = a.grant(op) {
                g.walk(&mut all);
            }
        }
        for p in &a.permits {
            p.grant.walk(&mut all);
        }
        for g in &a.guards {
            for r in &g.rules {
                r.grant.walk(&mut all);
            }
        }
        for g in all {
            if let Grant::Via(v) = g
                && let Some(r) = a.relation(&v.rel)
                && let Some((plugin, entity)) = target(d, e, r)
                && plugin == d.id
                && let Some(t) = d.entities.iter().find(|x| x.name == entity)
            {
                edges
                    .entry(e.name.as_str())
                    .or_default()
                    .push(t.name.as_str());
            }
        }
    }
    let mut out = vec![];
    for e in &d.entities {
        // the longest chain from e, or a cycle back to a visited entity
        fn depth<'a>(
            at: &'a str,
            edges: &HashMap<&'a str, Vec<&'a str>>,
            path: &mut Vec<&'a str>,
        ) -> Result<usize, Vec<String>> {
            if path.contains(&at) {
                let mut cycle: Vec<String> = path.iter().map(|s| s.to_string()).collect();
                cycle.push(at.to_string());
                return Err(cycle);
            }
            path.push(at);
            let mut best = 0;
            for next in edges.get(at).into_iter().flatten() {
                best = best.max(1 + depth(next, edges, path)?);
            }
            path.pop();
            Ok(best)
        }
        match depth(&e.name, &edges, &mut vec![]) {
            Err(cycle) => out.push(
                Error::new(
                    "E_ACCESS_CYCLE",
                    format!("entities[{}].access", e.name),
                    format!("via goes round: {}", cycle.join(" -> ")),
                )
                .help("one of these entities must grant without via"),
            ),
            Ok(n) if n > MAX_VIA_DEPTH => out.push(Error::new(
                "E_ACCESS_VIA_DEPTH",
                format!("entities[{}].access", e.name),
                format!(
                    "via chains are at most {MAX_VIA_DEPTH} deep; {} reaches {n}",
                    e.name
                ),
            )),
            Ok(_) => {}
        }
    }
    out
}

/// The scope of `authz:bypass`: entities of the plugin and their operations.
fn bypass(d: &Descriptor) -> Vec<Error> {
    let mut out = vec![];
    for (i, c) in d.capabilities.iter().enumerate() {
        let path = format!("capabilities[{i}]");
        if c.name != "authz:bypass" {
            if !c.on.is_empty() {
                out.push(Error::new(
                    "E_CAPABILITY_SCOPE",
                    format!("{path}.on"),
                    format!("{} has no scope", c.name),
                ));
            }
            continue;
        }
        if c.on.is_empty() {
            out.push(
                Error::new(
                    "E_CAPABILITY_SCOPE",
                    path.clone(),
                    "authz:bypass names the entities and the operations it bypasses",
                )
                .help("on: [{ entity: note, ops: [read] }]"),
            );
        }
        for (j, s) in c.on.iter().enumerate() {
            if !d.entities.iter().any(|e| e.name == s.entity) {
                out.push(Error::new(
                    "E_CAPABILITY_SCOPE",
                    format!("{path}.on[{j}].entity"),
                    format!("{:?} is not an entity of {}", s.entity, d.id),
                ));
            }
            if s.ops.is_empty() || s.ops.iter().any(|o| !BYPASS_OPS.contains(&o.as_str())) {
                out.push(
                    Error::new(
                        "E_CAPABILITY_SCOPE",
                        format!("{path}.on[{j}].ops"),
                        format!("{:?} are not operations", s.ops),
                    )
                    .help("read, create, update, delete"),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
