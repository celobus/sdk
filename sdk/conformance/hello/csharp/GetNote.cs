using Celobus.Sdk;
using Platform = Celobus.Sdk.Platform;

namespace Hello;

[Message] public sealed partial record GetNoteRequest(Id<Note> Id);

// Id absent: a new note
[Message] public sealed partial record SaveNoteRequest(Id<Note>? Id, string Title, string? Body, Id<Platform.User> OwnerId);

[Message] public sealed partial record DeleteNoteRequest(Id<Note> Id);

/// <summary>Reads a note (404 when there is none).</summary>
public sealed class GetNote : Endpoint<GetNoteRequest, Note>
{
    public override void Configure(EndpointOptions e) => e.Describe("Reads a note.").ReadOnly();

    public override async Task<Note> Handle(Ctx ctx, GetNoteRequest req) => await ctx.Db<Note>().Get(req.Id);
}

/// <summary>Creates a note, or changes one, in a transaction: the write, then a read of the owner's note with that title.</summary>
public sealed class SaveNote : Endpoint<SaveNoteRequest, Note>
{
    public override void Configure(EndpointOptions e) => e.Describe("Creates or changes a note.");

    public override Task<Note> Handle(Ctx ctx, SaveNoteRequest req) => ctx.Transaction(async tx =>
    {
        var notes = tx.Db<Note>();
        Stored<Note> saved = req.Id is { } id
            ? await notes.Update(id, Patch<Note>.Set(n => n.Title, req.Title).And(n => n.Body, req.Body))
            : await notes.Create(new Note(default, req.Title, req.Body, req.OwnerId, null));
        var mine = Filter<Note>.Eq(n => n.OwnerId, req.OwnerId) & Filter<Note>.Eq(n => n.Title, req.Title);
        _ = await notes.Find(mine).OrderBy(SystemColumn.CreatedAt, descending: true).First()
            ?? throw new InvalidOperationException("the saved note is not there");
        return saved.Value;
    });
}

/// <summary>Deletes a note and answers it as it was.</summary>
public sealed class DeleteNote : Endpoint<DeleteNoteRequest, Note>
{
    public override void Configure(EndpointOptions e) => e.Describe("Deletes a note.");

    public override async Task<Note> Handle(Ctx ctx, DeleteNoteRequest req)
    {
        var notes = ctx.Db<Note>();
        var row = await notes.Get(req.Id);
        await notes.Delete(req.Id).ExpectVersion(row.Version);
        return row;
    }
}

// Limit: at most this many (0: 100)
[Message] public sealed partial record ListNotesRequest(int Limit);

[Message] public sealed partial record ListNotesReply(IReadOnlyList<string> Titles);

// OwnerId: only this person's notes
[Message] public sealed partial record CountAllNotesRequest(Id<Platform.User>? OwnerId);

[Message] public sealed partial record NoteCount(long Count);

/// <summary>The titles of the notes the caller may read, newest first: their own, or every note for a manager.</summary>
public sealed class ListNotes : Endpoint<ListNotesRequest, ListNotesReply>
{
    public override void Configure(EndpointOptions e) => e.Describe("Lists the notes the caller may read.").ReadOnly();

    public override async Task<ListNotesReply> Handle(Ctx ctx, ListNotesRequest req)
    {
        var page = await ctx.Db<Note>().Find().OrderBy(SystemColumn.CreatedAt, descending: true)
            .Limit(req.Limit > 0 ? (uint)req.Limit : 100).Page();
        return new ListNotesReply([.. page.Rows.Select(n => n.Value.Title)]);
    }
}

/// <summary>Counts every note of the workspace (or one person's), whoever may read them: a system context, audited.</summary>
public sealed class CountAllNotes : Endpoint<CountAllNotesRequest, NoteCount>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Counts every note, as the system.").Roles(Roles.Manager).ReadOnly();

    public override async Task<NoteCount> Handle(Ctx ctx, CountAllNotesRequest req)
    {
        var notes = ctx.AsSystem("count every note").Db<Note>();
        var count = req.OwnerId is { } owner
            ? await notes.Count(Filter<Note>.Eq(n => n.OwnerId, owner))
            : await notes.Count();
        return new NoteCount(count);
    }
}
