//! The descriptor: the conformance plugin (the same declarations as sdk/conformance/hello/rust),
//! endpoint names and the definition errors.

use std::time::Duration;

use celobus_sdk::prelude::*;

pub const CLERK: Role = Role::new("clerk").title("Clerk");
pub const MANAGER: Role = Role::new("manager").title("Manager").strict();

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Happy,
    Sad,
}

#[derive(Message, Clone, Debug)]
pub struct HelloRequest {
    pub name: String,
    pub mood: Option<Mood>,
}

#[derive(Message, Clone, Debug)]
pub struct HelloReply {
    pub text: String,
    pub count: i64,
}

#[derive(Entity, Clone, Debug)]
#[entity(
    key = i64,
    version,
    label = title,
    index(owner_id, created_at desc),
    unique_index(owner_id, title)
)]
pub struct Note {
    pub id: Id<Note>,
    #[col(index, searchable)]
    pub title: String,
    #[col(searchable(lang = "english"))]
    pub body: Option<String>,
    pub owner_id: Id<platform::User>,
    pub secret: Option<String>,
}

#[derive(Entity, Clone, Debug)]
#[entity(key = i64, append_only)]
pub struct Greeting {
    pub id: Id<Greeting>,
    #[col(index)]
    pub name: String,
    #[col(encrypted)]
    pub email: Option<String>,
    #[col(nav = note, inverse = greetings, on_delete = cascade)]
    pub note_id: Option<Id<Note>>,
    #[col(on_delete = keep)]
    pub greeter_id: Option<Id<platform::User>>,
}

#[derive(Message, Clone, Debug)]
pub struct GetNoteRequest {
    pub id: Id<Note>,
}

#[derive(Message, Clone, Debug)]
pub struct SaveNoteRequest {
    pub id: Option<Id<Note>>,
    pub title: String,
    pub body: Option<String>,
    pub owner_id: Id<platform::User>,
}

#[derive(Message, Clone, Debug)]
pub struct DeleteNoteRequest {
    pub id: Id<Note>,
}

#[derive(Message, Clone, Debug)]
pub struct ListNotesRequest {
    pub limit: i32,
}

#[derive(Message, Clone, Debug)]
pub struct ListNotesReply {
    pub titles: Vec<String>,
}

#[derive(Message, Clone, Debug)]
pub struct CountAllNotesRequest {
    pub owner_id: Option<Id<platform::User>>,
}

#[derive(Message, Clone, Debug)]
pub struct NoteCount {
    pub count: i64,
}

#[derive(Message, Clone, Debug)]
pub struct NotesByOwnerRequest {
    pub min_notes: i64,
}

#[derive(Message, Clone, Debug)]
pub struct NotesByOwnerReply {
    pub owners: Vec<NoteTally>,
}

#[derive(Message, Clone, Debug)]
pub struct NoteTally {
    pub owner_id: Id<platform::User>,
    pub notes: i64,
}

async fn notes_by_owner(ctx: Ctx, req: NotesByOwnerRequest) -> Result<NotesByOwnerReply> {
    Ok(NotesByOwnerReply {
        owners: ctx
            .sql("select owner_id, count(*) as notes from hello_note group by owner_id having count(*) >= $1 order by owner_id")
            .bind(req.min_notes)
            .rows()
            .await?,
    })
}

async fn list_notes(_: Ctx, _: ListNotesRequest) -> Result<ListNotesReply> {
    Err(Error::unavailable("describe only"))
}

async fn count_all_notes(_: Ctx, _: CountAllNotesRequest) -> Result<NoteCount> {
    Err(Error::unavailable("describe only"))
}

async fn hello(_: Ctx, req: HelloRequest) -> Result<HelloReply> {
    Ok(HelloReply {
        text: req.name,
        count: 1,
    })
}

async fn get_note(ctx: Ctx, req: GetNoteRequest) -> Result<Row<Note>> {
    ctx.db::<Note>().get(&req.id).await
}

async fn save_note(_: Ctx, _: SaveNoteRequest) -> Result<Row<Note>> {
    Err(Error::unavailable("describe only"))
}

async fn delete_note(_: Ctx, _: DeleteNoteRequest) -> Result<Row<Note>> {
    Err(Error::unavailable("describe only"))
}

fn hello_plugin() -> Plugin {
    Plugin::new("hello", "0.1.0")
        .title("Hello")
        .describe("The SDK conformance plugin.")
        .role(CLERK)
        .role(MANAGER)
        .capability(Capability::PUSH_SEND, "Greets people.")
        .capability(Capability::AUTHZ_BYPASS, "Counts every note.")
        .bypass::<Note>([Perm::READ])
        .depends_on("greeter", ["Greet"])
        .entity::<Note>()
        .access::<Note>(Access::build(|a| {
            let owner = a.relation("owner", Rel::column(Note::owner_id));
            let greeted = a.relation(
                "greeted",
                Rel::rows(Greeting::note_id, Greeting::greeter_id)
                    .when(Greeting::name.ne(String::new())),
            );
            a.read(owner.clone() | greeted | MANAGER);
            a.create(owner.clone());
            a.update(owner.clone());
            a.delete(owner | MANAGER);
            a.mask(Note::secret, [MANAGER]);
        }))
        .entity::<Greeting>()
        .access::<Greeting>(Access::build(|a| {
            a.read(Grant::from(CLERK) | MANAGER);
            a.create(Grant::from(CLERK) | MANAGER);
        }))
        .endpoint(
            Endpoint::new(hello)
                .describe("Greets.")
                .roles([CLERK, MANAGER])
                .scopes([Scope::WRITE])
                .timeout(Duration::from_secs(30)),
        )
        .endpoint(
            Endpoint::new(get_note)
                .describe("Reads a note.")
                .read_only(),
        )
        .endpoint(Endpoint::new(save_note).describe("Creates or changes a note."))
        .endpoint(Endpoint::new(delete_note).describe("Deletes a note."))
        .endpoint(
            Endpoint::new(list_notes)
                .describe("Lists the notes the caller may read.")
                .read_only(),
        )
        .endpoint(
            Endpoint::new(count_all_notes)
                .describe("Counts every note, as the system.")
                .roles([MANAGER])
                .read_only(),
        )
        .endpoint(
            Endpoint::new(notes_by_owner)
                .describe("Counts the notes of each owner the caller may read, with SQL.")
                .read_only(),
        )
}

/// A dependency's entity, as `celobus plugin add` generates it.
#[derive(Entity, Clone, Debug)]
#[entity(plugin = "crm", key = uuid)]
pub struct Customer {
    pub id: Id<Customer>,
    pub name: String,
}
impl Exported for Customer {}

#[test]
fn reads_are_described_only_when_declared() {
    let v: serde_json::Value =
        serde_json::from_str(&hello_plugin().describe_json().unwrap()).unwrap();
    assert!(v["dependencies"][0].get("reads").is_none());
    let p = Plugin::new("sales", "1.0.0").reads(
        "crm",
        [Read::new(
            "customer",
            &[("id", "ref:crm.customer"), ("name", "string")],
        )],
    );
    let v: serde_json::Value = serde_json::from_str(&p.describe_json().unwrap()).unwrap();
    assert_eq!(
        v["dependencies"],
        serde_json::json!([{ "plugin": "crm", "calls": [], "reads": [{ "entity": "customer", "fields": [
            { "name": "id", "type": "ref:crm.customer" }, { "name": "name", "type": "string" }] }] }])
    );
    // SQL is sent as text: nothing of it is in the descriptor; the dependency's entity is not this plugin's
    assert!(v.get("statements").is_none());
    assert_eq!(v["entities"], serde_json::json!([]));
}

#[test]
fn notification_kinds_are_described_only_when_declared() {
    use celobus_sdk::notify::Kind;
    const MENTION: Kind = Kind::new("mention")
        .title("Mentions")
        .describe("Someone wrote @you")
        .digest();
    const REPLY: Kind = Kind::new("thread_reply")
        .title("Replies in threads you follow")
        .watching();
    let v: serde_json::Value =
        serde_json::from_str(&hello_plugin().describe_json().unwrap()).unwrap();
    assert!(v.get("notifications").is_none());
    let p = Plugin::new("chat", "1.0.0")
        .capability(
            Capability::PUSH_SEND,
            "Tells people when they are mentioned.",
        )
        .notification(MENTION)
        .notification(REPLY);
    let d = p.describe_json().unwrap();
    let v: serde_json::Value = serde_json::from_str(&d).unwrap();
    assert_eq!(
        v["notifications"],
        serde_json::json!([
            { "name": "mention", "title": "Mentions", "description": "Someone wrote @you", "tab": "for_you", "digest": true },
            { "name": "thread_reply", "title": "Replies in threads you follow", "description": "", "tab": "watching", "digest": false }
        ])
    );
    // what the SDK prints, the platform reads
    let parsed = celobus_descriptor::parse(d.as_bytes()).unwrap();
    assert!(celobus_descriptor::validate(&parsed).is_empty());
    // kinds need push:send; a name is snake_case, once
    let errors = Plugin::new("chat", "1.0.0")
        .notification(MENTION)
        .notification(MENTION)
        .notification(Kind::new("Bad").title("Bad"))
        .check();
    let codes: Vec<&str> = errors.iter().map(|e| e.code).collect();
    assert_eq!(
        codes,
        [
            "E_NOTIFY_CAPABILITY",
            "E_NOTIFY_KIND_NAME",
            "E_NOTIFY_KIND_NAME"
        ]
    );
}

#[test]
fn conformance_descriptor() {
    let fixture = include_str!("../../../sdk/conformance/hello/descriptor.json");
    assert_eq!(hello_plugin().describe_json().unwrap(), fixture);
}

#[test]
fn endpoint_names_come_from_the_request() {
    assert_eq!(Endpoint::new(hello).name(), Some("Hello"));
    assert_eq!(Endpoint::new(get_note).name(), Some("GetNote"));

    #[derive(Message, Clone, Debug)]
    pub struct Greeting {
        pub text: String,
    }
    async fn bad(_: Ctx, req: Greeting) -> Result<Greeting> {
        Ok(req)
    }
    let endpoint = Endpoint::new(bad);
    assert_eq!(endpoint.name(), None);
    let errors = Plugin::new("hello", "0.1.0").endpoint(endpoint).check();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].to_string(),
        "error[E_ENDPOINT_NAME] at endpoints[0]: the request type `Greeting` must be named <Endpoint>Request"
    );
}

#[test]
fn definition_errors_are_collected() {
    let errors = Plugin::new("platform", "1.0")
        .role(Role::new("clerk"))
        .role(Role::new("clerk"))
        .role(Role::new("Bad"))
        .capability(Capability::new("net:all"), "")
        .depends_on("platform", ["X"])
        .endpoint(
            Endpoint::new(hello)
                .roles([MANAGER, Role::WORKSPACE_ADMIN])
                .scopes([Scope::new("export")])
                .timeout(Duration::from_secs(601)),
        )
        .endpoint(Endpoint::new(hello))
        .check();
    let codes: Vec<&str> = errors.iter().map(|e| e.code).collect();
    assert_eq!(
        codes,
        vec![
            "E_PLUGIN_ID_RESERVED",
            "E_PLUGIN_VERSION",
            "E_ROLE_DUP",
            "E_ROLE_NAME",
            "E_CAPABILITY_UNKNOWN",
            "E_CAPABILITY_REASON",
            "E_DEP_SELF",
            "E_ENDPOINT_ROLE_UNKNOWN",
            "E_ENDPOINT_SCOPE_UNKNOWN",
            "E_ENDPOINT_TIMEOUT_RANGE",
            "E_ENDPOINT_DUP",
        ]
    );
}

#[test]
fn two_types_with_one_name_are_refused() {
    mod a {
        use celobus_sdk::prelude::*;
        #[derive(Message, Clone, Debug)]
        pub struct Item {
            pub x: i32,
        }
    }
    mod b {
        use celobus_sdk::prelude::*;
        #[derive(Message, Clone, Debug)]
        pub struct Item {
            pub y: String,
        }
    }
    let errors = Plugin::new("hello", "0.1.0")
        .message::<a::Item>()
        .message::<b::Item>()
        .check();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "E_MSG_DUP");
}

#[test]
fn defaults_and_nested_types() {
    /// A tree of tags.
    ///
    /// Not part of the description.
    #[derive(Message, Clone, Debug)]
    pub struct TagTree {
        /// The tag.
        pub tag: String,
        pub children: Vec<TagTree>,
        pub files: Vec<Vec<u8>>,
        pub group_id: Option<Id<platform::Group>>,
    }
    let d = Plugin::new("tags", "1.0.0")
        .message::<TagTree>()
        .descriptor()
        .unwrap();
    assert_eq!(d.title, "Tags");
    let m = &d.messages[0];
    assert_eq!(m.description, "A tree of tags.");
    let types: Vec<(&str, &str, bool, bool)> = m
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.ty.as_str(), f.repeated, f.optional))
        .collect();
    assert_eq!(
        types,
        vec![
            ("tag", "string", false, false),
            ("children", "message:TagTree", true, false),
            ("files", "bytes", true, false),
            ("group_id", "ref:platform.group", false, true),
        ]
    );
    assert_eq!(m.fields[0].description, "The tag.");
}

#[test]
fn an_agent_says_how_it_runs_and_what_it_leaves_out_is_left_out() {
    let d = Plugin::new("shop", "1.0.0")
        .role(CLERK)
        .agent(Agent::new("sync").roles([CLERK]))
        .agent(
            Agent::new("assistant")
                .title("Assistant")
                .describe("Answers questions about prices.")
                .engine(AgentEngine::Pi)
                .model("claude-opus-5-5")
                .session()
                .timeout(Duration::from_secs(900))
                .skills(["money"]),
        )
        .descriptor()
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&d.to_canonical_json()).unwrap();
    assert_eq!(
        json["agents"],
        serde_json::json!([
            { "name": "sync", "roles": ["clerk"] },
            { "name": "assistant", "roles": [], "title": "Assistant",
              "description": "Answers questions about prices.", "engine": "pi", "model": "claude-opus-5-5",
              "session": true, "timeout_s": 900, "skills": ["money"] },
        ])
    );
}

#[derive(Message, Clone, Debug)]
pub struct AllAssistantRuns {}

pub const ASSISTANT_RUNS: celobus_sdk::ui::Topic<AllAssistantRuns> =
    celobus_sdk::ui::Topic::new("assistant_runs");

#[test]
fn an_agent_names_the_topic_its_runs_invalidate() {
    let d = Plugin::new("shop", "1.0.0")
        .role(CLERK)
        .topic(ASSISTANT_RUNS.roles([CLERK]))
        .agent(Agent::new("assistant").roles([CLERK]).topic(ASSISTANT_RUNS))
        .descriptor()
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&d.to_canonical_json()).unwrap();
    assert_eq!(
        json["agents"],
        serde_json::json!([{ "name": "assistant", "roles": ["clerk"], "topic": "assistant_runs" }])
    );
    assert_eq!(json["topics"][0]["name"], "assistant_runs");
}

#[derive(Entity, Clone, Debug)]
#[entity(soft_delete)]
pub struct Sku {
    pub id: Id<Sku>,
    #[col(unique)]
    pub code: String,
    #[col(index)]
    pub shelf: Option<String>,
    #[col(encrypted)]
    pub supplier_note: Option<String>,
}

/// Column modifiers (unique, index, encrypted) are the entity's: printed on its fields, not on its message's copy; a soft-delete entity has a
/// handle of `deleted_at`, and every entity of its timestamps.
#[test]
fn column_modifiers_are_printed_on_the_entity() {
    let d: serde_json::Value = serde_json::from_str(
        &Plugin::new("shop", "0.1.0")
            .entity::<Sku>()
            .describe_json()
            .unwrap(),
    )
    .unwrap();
    let fields = &d["entities"][0]["fields"];
    assert_eq!(fields[1]["unique"], true);
    assert!(fields[1].get("index").is_none());
    assert_eq!(fields[2]["index"], true);
    assert!(fields[2].get("encrypted").is_none());
    assert_eq!(fields[3]["encrypted"], true);
    assert!(d["messages"][0]["fields"][1].get("unique").is_none());
    assert!(d["messages"][0]["fields"][3].get("encrypted").is_none());
    assert!(d["entities"][0].get("indexes").is_none());
    assert_eq!(Sku::deleted_at.name(), "deleted_at");
    assert_eq!(Sku::created_at.name(), "created_at");
    let errors =
        celobus_descriptor::validate(&celobus_descriptor::parse(d.to_string().as_bytes()).unwrap());
    assert_eq!(errors, vec![]);
}

#[derive(Entity, Clone, Debug)]
pub struct Report {
    pub id: Id<Report>,
    pub title: String,
    #[field(max_size = "1 GiB", accept = ["image/*", "application/pdf"])]
    pub scans: Vec<File>,
    pub cover: Option<File>,
    pub signed: File,
}

#[derive(Message, Clone, Debug)]
pub struct ImportCsvRequest {
    #[field(accept = ["text/csv"], max_size = 1048576)]
    pub file: File,
}

#[derive(Message, Clone, Debug)]
pub struct Imported {
    pub rows: i64,
}

async fn import_csv(_: Ctx, _: ImportCsvRequest) -> Result<Imported> {
    Ok(Imported { rows: 0 })
}

#[test]
fn file_fields_and_their_options_are_printed() {
    let json = Plugin::new("reports", "0.1.0")
        .entity::<Report>()
        .endpoint(Endpoint::new(import_csv))
        .describe_json()
        .unwrap();
    let d: serde_json::Value = serde_json::from_str(&json).unwrap();
    let fields = &d["entities"][0]["fields"];
    assert_eq!(
        fields[2],
        serde_json::json!({ "name": "scans", "number": 3, "type": "file", "repeated": true, "optional": false,
                            "description": "", "max_size": 1u64 << 30, "accept": ["image/*", "application/pdf"] })
    );
    assert_eq!(
        (&fields[3]["type"], &fields[3]["optional"]),
        (&serde_json::json!("file"), &serde_json::json!(true))
    );
    assert!(fields[3].get("max_size").is_none() && fields[3].get("accept").is_none());
    assert_eq!(
        (
            &fields[4]["type"],
            &fields[4]["optional"],
            &fields[4]["repeated"]
        ),
        (
            &serde_json::json!("file"),
            &serde_json::json!(false),
            &serde_json::json!(false)
        )
    );
    // the entity's message keeps the options too
    let report = d["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Report")
        .unwrap();
    assert_eq!(report["fields"][2]["max_size"], 1u64 << 30);
    let import = d["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "ImportCsvRequest")
        .unwrap();
    assert_eq!(
        import["fields"][0]["accept"],
        serde_json::json!(["text/csv"])
    );
    assert_eq!(import["fields"][0]["max_size"], 1_048_576);
    let errors = celobus_descriptor::validate(&celobus_descriptor::parse(json.as_bytes()).unwrap());
    assert_eq!(errors, vec![]);
}

#[test]
fn a_file_value_reads_and_writes_its_json() {
    let f = File {
        id: "0193".into(),
        name: "ads.csv".into(),
        content_type: "text/csv".into(),
        size: 2_100_000,
    };
    let json = serde_json::json!({ "id": "0193", "name": "ads.csv", "type": "text/csv", "size": "2100000" });
    assert_eq!(serde_json::to_value(&f).unwrap(), json);
    let req = ImportCsvRequest { file: f.clone() };
    assert_eq!(req.to_json(), serde_json::json!({ "file": json }));
    // a size as a number reads the same
    let back = ImportCsvRequest::from_json(
        &serde_json::json!({ "file": { "id": "0193", "name": "ads.csv", "type": "text/csv", "size": 2_100_000 } }),
        "",
    )
    .unwrap();
    assert_eq!(back.file, f);
    let serde_back: File =
        serde_json::from_value(serde_json::json!({ "id": "0193", "size": 7 })).unwrap();
    assert_eq!(serde_back.size, 7);
    assert_eq!(serde_json::from_value::<File>(json).unwrap(), f);
    // a file without an id is refused
    assert!(
        ImportCsvRequest::from_json(&serde_json::json!({ "file": { "name": "x" } }), "").is_err()
    );
    // and a missing single file is required
    assert!(ImportCsvRequest::from_json(&serde_json::json!({}), "").is_err());
}
