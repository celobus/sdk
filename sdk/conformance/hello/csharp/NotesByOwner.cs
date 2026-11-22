using Celobus.Sdk;
using Platform = Celobus.Sdk.Platform;

namespace Hello;

// MinNotes: only owners with at least this many notes
[Message] public sealed partial record NotesByOwnerRequest(long MinNotes);

[Message] public sealed partial record NotesByOwnerReply(IReadOnlyList<NoteTally> Owners);

[Message] public sealed partial record NoteTally(Id<Platform.User> OwnerId, long Notes);

/// <summary>
/// Counts the notes of each owner the caller may read: SQL over the notes' secure view (<c>hello_note</c>), run as the
/// caller through the notes' read rules (the caller's own, or every owner's for a manager).
/// </summary>
public sealed class NotesByOwner : Endpoint<NotesByOwnerRequest, NotesByOwnerReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Counts the notes of each owner the caller may read, with SQL.").ReadOnly();

    public override async Task<NotesByOwnerReply> Handle(Ctx ctx, NotesByOwnerRequest req) =>
        new(await ctx.Sql<NoteTally>(
            "select owner_id, count(*) as notes from hello_note group by owner_id having count(*) >= $1 order by owner_id",
            req.MinNotes));
}
