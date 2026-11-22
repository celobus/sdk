//! Access rules in the descriptor: relations, grants, guards, masks, exports and the bypass scope, as the platform
//! reads them.

use celobus_sdk::prelude::*;
use serde_json::json;

pub const MANAGER: Role = Role::new("manager");
pub const AUDITOR: Role = Role::new("auditor").strict();
pub const PUBLISH: Perm = Perm::new("publish");

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Draft,
    Published,
}

#[derive(Entity, Clone, Debug)]
pub struct Book {
    pub id: Id<Book>,
    pub owner_id: Id<platform::User>,
    pub group_id: Option<Id<platform::Group>>,
    pub readers: Vec<Id<platform::User>>,
}

#[derive(Entity, Clone, Debug)]
pub struct Page {
    pub id: Id<Page>,
    pub book_id: Id<Book>,
    pub author_id: Id<platform::User>,
    pub state: State,
    pub secret: Option<String>,
}

fn plugin() -> Plugin {
    Plugin::new("notes", "1.0.0")
        .role(MANAGER)
        .role(AUDITOR)
        .capability(Capability::AUTHZ_BYPASS, "Archives pages.")
        .bypass::<Page>([Perm::READ, Perm::UPDATE])
        .entity::<Book>()
        .entity::<Page>()
        .access::<Book>(Access::build(|a| {
            let owner = a.relation("owner", Rel::column(Book::owner_id));
            let team = a.relation("team", Rel::group(Book::group_id));
            a.read(owner.clone() | team | MANAGER);
            a.create(owner.clone());
            a.update(owner);
            a.export([Perm::READ]);
        }))
        .access::<Page>(Access::build(|a| {
            let author = a.relation("author", Rel::column(Page::author_id));
            let book = a.relation("book", Rel::column(Page::book_id));
            a.read(author.clone() | book.via(Perm::READ));
            a.create(author.clone() & book.via(Perm::READ));
            a.update(author.when(Page::state.eq(State::Draft)) | MANAGER);
            a.delete(Grant::from(author) & !AUDITOR);
            a.permit(PUBLISH, MANAGER);
            a.guard(Page::state)
                .to(State::Published, Grant::holds(PUBLISH))
                .to_any(Role::WORKSPACE_ADMIN);
            a.mask(Page::secret, [MANAGER]);
        }))
}

#[test]
fn rules_are_described_as_the_platform_reads_them() {
    let d: serde_json::Value = serde_json::from_str(&plugin().describe_json().unwrap()).unwrap();
    let page = &d["entities"][1]["access"];
    assert_eq!(
        page["read"],
        json!({ "any": [{ "rel": "author" }, { "via": { "rel": "book", "perm": "read" } }] })
    );
    assert_eq!(
        page["update"],
        json!({ "any": [{ "when": { "grant": { "rel": "author" },
                                    "filter": [{ "field": "state", "op": "eq", "value": "draft" }] } },
                        { "role": "manager" }] })
    );
    assert_eq!(
        page["delete"],
        json!({ "all": [{ "rel": "author" }, { "not": { "role": "auditor" } }] })
    );
    assert_eq!(
        page["guards"],
        json!([{ "fields": ["state"], "rules": [
            { "value": "published", "grant": { "holds": "publish" } },
            { "value": null, "grant": { "role": "workspace.admin" } }] }])
    );
    assert_eq!(
        page["masks"],
        json!([{ "field": "secret", "roles": ["manager"] }])
    );
    assert_eq!(page["export"], json!([]));
    assert_eq!(page["admin"], true);
    assert_eq!(
        d["entities"][0]["access"]["relations"],
        json!([{ "name": "owner", "kind": "column", "field": "owner_id" },
               { "name": "team", "kind": "group", "field": "group_id" }])
    );
    assert_eq!(
        d["capabilities"][0]["on"],
        json!([{ "entity": "page", "ops": ["read", "update"] }])
    );
    // the platform's checks accept it
    let parsed = celobus_descriptor::parse(d.to_string().as_bytes()).unwrap();
    assert_eq!(celobus_descriptor::validate(&parsed), vec![]);
}

#[test]
fn an_entity_without_rules_writes_none_and_misplaced_rules_are_errors() {
    let d: serde_json::Value = serde_json::from_str(
        &Plugin::new("notes", "1.0.0")
            .entity::<Book>()
            .describe_json()
            .unwrap(),
    )
    .unwrap();
    assert!(d["entities"][0].get("access").is_none());
    let errors = Plugin::new("notes", "1.0.0")
        .access::<Book>(Access::build(|a| a.read(Grant::public())))
        .bypass::<Book>([Perm::READ])
        .check();
    let codes: Vec<&str> = errors.iter().map(|e| e.code).collect();
    assert!(codes.contains(&"E_ACCESS_ENTITY"), "{codes:?}");
    assert!(codes.contains(&"E_CAPABILITY_SCOPE"), "{codes:?}");
}

#[derive(Entity, Clone, Debug)]
pub struct Shelf {
    pub id: Id<Shelf>,
    pub title: String,
}

/// A junction: who is on which shelf, a person or a team, and whether they keep it.
#[derive(Entity, Clone, Debug)]
#[entity(soft_delete)]
pub struct ShelfMember {
    pub id: Id<ShelfMember>,
    pub shelf_id: Id<Shelf>,
    pub user_id: Option<Id<platform::User>>,
    pub team_id: Option<Id<platform::Group>>,
    pub keeper: bool,
}

fn shelves() -> Plugin {
    Plugin::new("notes", "1.0.0")
        .role(MANAGER)
        .entity::<Shelf>()
        .entity::<ShelfMember>()
        .access::<Shelf>(Access::build(|a| {
            let member = a.relation(
                "member",
                Rel::rows(ShelfMember::shelf_id, ShelfMember::user_id),
            );
            let team = a.relation(
                "team",
                Rel::rows(ShelfMember::shelf_id, ShelfMember::team_id),
            );
            let keeper = a.relation(
                "keeper",
                Rel::rows(ShelfMember::shelf_id, ShelfMember::user_id)
                    .when(ShelfMember::keeper.eq(true)),
            );
            a.read(member | team | MANAGER);
            a.update(keeper.clone() | MANAGER);
            a.delete(keeper & !Grant::from(MANAGER));
        }))
        .access::<ShelfMember>(Access::build(|a| {
            let shelf = a.relation("shelf", Rel::column(ShelfMember::shelf_id));
            a.read(shelf.via(Perm::READ));
            a.create(shelf.via(Perm::UPDATE));
            a.delete(shelf.via(Perm::UPDATE));
        }))
}

#[test]
fn a_relation_through_junction_rows_is_described() {
    let d: serde_json::Value = serde_json::from_str(&shelves().describe_json().unwrap()).unwrap();
    let shelf = &d["entities"][0]["access"];
    assert_eq!(
        shelf["relations"],
        json!([
            { "name": "member", "kind": "rows", "field": "user_id",
              "rows": { "entity": "shelf_member", "target": "shelf_id", "filter": [] } },
            { "name": "team", "kind": "rows", "field": "team_id",
              "rows": { "entity": "shelf_member", "target": "shelf_id", "filter": [] } },
            { "name": "keeper", "kind": "rows", "field": "user_id",
              "rows": { "entity": "shelf_member", "target": "shelf_id",
                        "filter": [{ "field": "keeper", "op": "eq", "value": true }] } }
        ])
    );
    assert_eq!(
        shelf["read"],
        json!({ "any": [{ "rel": "member" }, { "rel": "team" }, { "role": "manager" }] })
    );
    // the junction reads through the shelf, the shelf through the junction's rows: no cycle
    let parsed = celobus_descriptor::parse(d.to_string().as_bytes()).unwrap();
    assert_eq!(celobus_descriptor::validate(&parsed), vec![]);
}

#[test]
fn a_relation_through_rows_that_do_not_point_here_is_refused() {
    let json = Plugin::new("notes", "1.0.0")
        .entity::<Book>()
        .entity::<Shelf>()
        .entity::<ShelfMember>()
        .access::<Book>(Access::build(|a| {
            // the junction points to shelves, not books
            let member = a.relation(
                "member",
                Rel::rows(ShelfMember::shelf_id, ShelfMember::user_id),
            );
            let keeper = a.relation(
                "keeper",
                Rel::rows(ShelfMember::shelf_id, ShelfMember::keeper),
            );
            a.read(member | keeper);
        }))
        .describe_json()
        .unwrap();
    // the platform's check (`celobus plugin check`) refuses it
    let parsed = celobus_descriptor::parse(json.as_bytes()).unwrap();
    let errors = celobus_descriptor::validate(&parsed);
    let codes: Vec<&str> = errors.iter().map(|e| e.code).collect();
    assert!(codes.contains(&"E_ACCESS_ROWS_TARGET"), "{codes:?}");
    assert!(codes.contains(&"E_ACCESS_ROWS_SUBJECT"), "{codes:?}");
}
