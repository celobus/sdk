//! The workspace directory, an implicit read-only dependency of every plugin: `platform::User` and
//! `platform::Group` (`ref:platform.user`, `ref:platform.group` in the descriptor), and for typed queries
//! (`ctx.query::<platform::GroupMember>()`) the memberships and role assignments too. A member (base role `member` and
//! above) reads every person, group, membership and assignment (a plugin: of its own roles and the platform's); a
//! guest reads their own user row only, and sees others as labels (`query::label`).

use crate::ctx::CallerKind;
use crate::message::Entity;
use crate::query::{JoinRef, Queryable, canonical};
use crate::types::{Id, Timestamp};
use crate::ui::Field;

/// A member of the workspace (a person or a service account), as `ctx.members()` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Id<User>,
    pub kind: CallerKind,
    pub display_name: String,
    /// Effective roles: `workspace.*` and plugin roles `<plugin>.<role>`.
    pub roles: Vec<String>,
    pub groups: Vec<Id<Group>>,
    /// Empty unless the plugin declares the permission `members.email`.
    pub email: String,
    pub avatar_url: String,
    /// Unix seconds, hour resolution.
    pub last_active: i64,
    pub locale: String,
}

impl Entity for User {
    type Key = String;
    const ENTITY: &'static str = "user";
    const PLUGIN: Option<&'static str> = Some("platform");
}
impl Queryable for User {
    const SOURCE: &'static str = "user";
    const OWNER: Option<&'static str> = Some("platform");
}

/// A person or a service account.
#[derive(crate::Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserKind {
    Person,
    Service,
}

/// The base role in the workspace.
#[derive(crate::Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseRole {
    Owner,
    Admin,
    Member,
    Guest,
}

/// Whether a person is in the workspace: `removed` keeps their name (history reads "Laura Chen (left)"), `erased` (an
/// account deleted) not even that.
#[derive(crate::Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Removed,
    Erased,
}

/// The columns of `platform.user` a query names (`email` is null without the plugin permission `members.email`, and
/// for a guest reader; `base_role` is null for a person who left).
#[allow(non_upper_case_globals)]
impl User {
    pub const id: Field<User, Id<User>> = Field::new("id");
    pub const display_name: Field<User, String> = Field::new("display_name");
    pub const avatar_url: Field<User, String> = Field::new("avatar_url");
    pub const email: Field<User, Option<String>> = Field::new("email");
    pub const kind: Field<User, UserKind> = Field::new("kind");
    pub const base_role: Field<User, Option<BaseRole>> = Field::new("base_role");
    pub const status: Field<User, UserStatus> = Field::new("status");
    pub const locale: Field<User, String> = Field::new("locale");
    pub const joined_at: Field<User, Timestamp> = Field::new("joined_at");
    pub const left_at: Field<User, Option<Timestamp>> = Field::new("left_at");
}

/// A group of the workspace, as `ctx.groups()` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: Id<Group>,
    pub name: String,
    pub members: Vec<Id<User>>,
}

impl Entity for Group {
    type Key = String;
    const ENTITY: &'static str = "group";
    const PLUGIN: Option<&'static str> = Some("platform");
}
impl Queryable for Group {
    const SOURCE: &'static str = "group";
    const OWNER: Option<&'static str> = Some("platform");
}

#[allow(non_upper_case_globals)]
impl Group {
    pub const id: Field<Group, Id<Group>> = Field::new("id");
    pub const name: Field<Group, String> = Field::new("name");
}

/// A person's membership of a group (`platform.group_member`), for queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub group_id: Id<Group>,
    pub user_id: Id<User>,
}

impl Entity for GroupMember {
    type Key = String;
    const ENTITY: &'static str = "group_member";
    const PLUGIN: Option<&'static str> = Some("platform");
}
impl Queryable for GroupMember {
    const SOURCE: &'static str = "group_member";
    const OWNER: Option<&'static str> = Some("platform");
}

#[allow(non_upper_case_globals)]
impl GroupMember {
    pub const group_id: Field<GroupMember, Id<Group>> = Field::new("group_id");
    pub const user_id: Field<GroupMember, Id<User>> = Field::new("user_id");
}

/// A role a person holds (`platform.role_assignment`: `workspace.*` and the plugin's own roles), for queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    pub user_id: Id<User>,
    pub role: String,
}

impl Entity for RoleAssignment {
    type Key = String;
    const ENTITY: &'static str = "role_assignment";
    const PLUGIN: Option<&'static str> = Some("platform");
}
impl Queryable for RoleAssignment {
    const SOURCE: &'static str = "role_assignment";
    const OWNER: Option<&'static str> = Some("platform");
}

#[allow(non_upper_case_globals)]
impl RoleAssignment {
    pub const user_id: Field<RoleAssignment, Id<User>> = Field::new("user_id");
    pub const role: Field<RoleAssignment, String> = Field::new("role");
}

macro_rules! reverse {
    ($e:ty, $t:ty) => {
        impl JoinRef<$e> for Field<$e, Id<$t>> {
            fn join_ref(&self) -> (String, &'static str, String) {
                (canonical::<$e>(), self.name(), canonical::<$t>())
            }
        }
    };
}
reverse!(GroupMember, Group);
reverse!(GroupMember, User);
reverse!(RoleAssignment, User);
