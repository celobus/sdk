//! The SDK conformance plugin `hello`: the same declarations in Rust, TypeScript and C#, so that the
//! three `--describe` outputs equal `sdk/conformance/hello/descriptor.json` (`scripts/sdk-conformance.sh`).

use std::time::Duration;

use celobus_sdk::prelude::*;

pub const CLERK: Role = Role::new("clerk").title("Clerk");
pub const MANAGER: Role = Role::new("manager").title("Manager").strict();

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Happy,
    Sad,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct HelloRequest {
    pub name: String,
    pub mood: Option<Mood>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct HelloReply {
    pub text: String,
    pub count: i64,
}

#[derive(Entity, Clone, Debug, PartialEq)]
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
    // read by managers only (others read it as absent)
    pub secret: Option<String>,
}

// A greeting: whom it greeted. The greetings are counted, not a counter kept.
#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(key = i64, append_only)]
pub struct Greeting {
    pub id: Id<Greeting>,
    #[col(index)]
    pub name: String,
    // the greeted one's email, when known: kept encrypted at rest
    #[col(encrypted)]
    pub email: Option<String>,
    // the note it greeted on, if any (`Note::greetings` lists them); its greetings go with it
    #[col(nav = note, inverse = greetings, on_delete = cascade)]
    pub note_id: Option<Id<Note>>,
    // who greeted on it: they read the note (`note_access`); the id stays
    #[col(on_delete = keep)]
    pub greeter_id: Option<Id<platform::User>>,
}

/// Who greets, and reads the greetings: whoever may call `Hello`.
pub fn greeting_access() -> Access<Greeting> {
    Access::build(|a| {
        a.read(Grant::from(CLERK) | MANAGER);
        a.create(Grant::from(CLERK) | MANAGER);
    })
}

/// Who reads and writes notes: the owner their own, whoever greeted on one by name that one, a manager every note
/// (and the secret).
pub fn note_access() -> Access<Note> {
    Access::build(|a| {
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
    })
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct GetNoteRequest {
    pub id: Id<Note>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct SaveNoteRequest {
    // None: a new note
    pub id: Option<Id<Note>>,
    pub title: String,
    pub body: Option<String>,
    pub owner_id: Id<platform::User>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct DeleteNoteRequest {
    pub id: Id<Note>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct ListNotesRequest {
    // at most this many (0: 100)
    pub limit: i32,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct ListNotesReply {
    pub titles: Vec<String>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct CountAllNotesRequest {
    // only this person's notes
    pub owner_id: Option<Id<platform::User>>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct NoteCount {
    pub count: i64,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct NotesByOwnerRequest {
    // owners with at least this many notes
    pub min_notes: i64,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct NotesByOwnerReply {
    pub owners: Vec<NoteTally>,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct NoteTally {
    pub owner_id: Id<platform::User>,
    pub notes: i64,
}

/// Greets, and counts the greetings of that name: a row each, in a transaction with its count.
pub async fn hello(ctx: Ctx, req: HelloRequest) -> Result<HelloReply> {
    let count = ctx
        .transaction(async |tx| {
            let greetings = tx.db::<Greeting>();
            greetings
                .create(Greeting {
                    id: Id::none(),
                    name: req.name.clone(),
                    email: None,
                    note_id: None,
                    greeter_id: None,
                })
                .await?;
            greetings
                .count_where(Greeting::name.eq(req.name.clone()))
                .await
        })
        .await? as i64;
    let text = match req.mood {
        Some(Mood::Sad) => format!("Cheer up, {}!", req.name),
        _ => format!("Hello, {}!", req.name),
    };
    Ok(HelloReply { text, count })
}

/// Reads a note (404 when there is none).
pub async fn get_note(ctx: Ctx, req: GetNoteRequest) -> Result<Row<Note>> {
    ctx.db::<Note>().get(&req.id).await
}

/// Creates a note, or changes one, in a transaction: the write, then a read of the owner's note with that title.
pub async fn save_note(ctx: Ctx, req: SaveNoteRequest) -> Result<Row<Note>> {
    ctx.transaction(async |tx| {
        let notes = tx.db::<Note>();
        let saved = match &req.id {
            None => {
                let note = Note {
                    id: Id::none(),
                    title: req.title.clone(),
                    body: req.body.clone(),
                    owner_id: req.owner_id.clone(),
                    secret: None,
                };
                notes.create(note).await?
            }
            Some(id) => {
                let patch = Patch::new().set(Note::title, req.title.clone());
                let patch = match &req.body {
                    Some(body) => patch.set(Note::body, Some(body.clone())),
                    None => patch.clear(Note::body),
                };
                notes.update(id, patch).await?
            }
        };
        let mine = Note::owner_id.eq(req.owner_id.clone()) & Note::title.eq(req.title.clone());
        notes
            .find(mine)
            .order_by(Note::created_at.desc())
            .first()
            .await?
            .ok_or_else(|| Error::internal("the saved note is not there"))?;
        Ok(saved)
    })
    .await
}

/// Deletes a note and answers it as it was.
pub async fn delete_note(ctx: Ctx, req: DeleteNoteRequest) -> Result<Row<Note>> {
    let notes = ctx.db::<Note>();
    let note = notes.get(&req.id).await?;
    notes.delete(&req.id).expect_version(note.version()).await?;
    Ok(note)
}

/// The titles of the notes the caller may read, newest first: their own, or every note for a manager. (`GetNote`
/// shows the mask: a note's secret is absent for everyone but a manager.)
pub async fn list_notes(ctx: Ctx, req: ListNotesRequest) -> Result<ListNotesReply> {
    let limit = if req.limit > 0 { req.limit as u32 } else { 100 };
    let page = ctx
        .db::<Note>()
        .find(Filter::all())
        .order_by(Note::created_at.desc())
        .limit(limit)
        .page()
        .await?;
    Ok(ListNotesReply {
        titles: page
            .rows
            .into_iter()
            .map(|r| r.into_inner().title)
            .collect(),
    })
}

/// Counts every note of the workspace (or one person's), whoever may read them: a system context, audited.
pub async fn count_all_notes(ctx: Ctx, req: CountAllNotesRequest) -> Result<NoteCount> {
    let notes = ctx.as_system("count every note").db::<Note>();
    let count = match req.owner_id {
        Some(owner) => notes.count_where(Note::owner_id.eq(owner)).await?,
        None => notes.count().await?,
    };
    Ok(NoteCount {
        count: count as i64,
    })
}

/// Counts the notes of each owner the caller may read: SQL over the notes' secure view (`hello_note`), run as the
/// caller through the notes' rules.
pub async fn notes_by_owner(ctx: Ctx, req: NotesByOwnerRequest) -> Result<NotesByOwnerReply> {
    Ok(NotesByOwnerReply {
        owners: ctx
            .sql("select owner_id, count(*) as notes from hello_note group by owner_id having count(*) >= $1 order by owner_id")
            .bind(req.min_notes)
            .rows()
            .await?,
    })
}

pub fn plugin() -> Plugin {
    Plugin::new("hello", env!("CARGO_PKG_VERSION"))
        .title("Hello")
        .describe("The SDK conformance plugin.")
        .role(CLERK)
        .role(MANAGER)
        .capability(Capability::PUSH_SEND, "Greets people.")
        .capability(Capability::AUTHZ_BYPASS, "Counts every note.")
        .bypass::<Note>([Perm::READ])
        .depends_on("greeter", ["Greet"])
        .entity::<Note>()
        .access::<Note>(note_access())
        .entity::<Greeting>()
        .access::<Greeting>(greeting_access())
        .endpoint(
            Endpoint::new(hello)
                .describe("Greets.")
                .roles([CLERK, MANAGER])
                .scopes([Scope::WRITE])
                .auth(Auth::User)
                .idempotency(Idempotency::Off)
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
