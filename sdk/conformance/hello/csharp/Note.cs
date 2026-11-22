using Celobus.Sdk;
using Platform = Celobus.Sdk.Platform;

namespace Hello;

[Entity(Key = KeyType.I64, Version = true, Label = nameof(Title))]
[Index(nameof(OwnerId), "CreatedAt desc")]
[Index(nameof(OwnerId), nameof(Title), Unique = true)]
// Secret: read by managers only (others read it as absent)
public sealed partial record Note(Id<Note> Id, [Index, Searchable] string Title, [Searchable("english")] string? Body, Id<Platform.User> OwnerId, string? Secret);

// A greeting: whom it greeted (and the note it was for, if any, and who greeted on it: they read the note). The
// greetings are counted, not a counter kept.
// Email: the greeted one's email, when known: kept encrypted at rest
[Entity(Key = KeyType.I64, AppendOnly = true)]
public sealed partial record Greeting(Id<Greeting> Id, [Index] string Name, [Encrypted] string? Email = null, [Ref(Nav = "note", Inverse = "greetings", OnDelete = OnDelete.Cascade)] Id<Note>? NoteId = null, [Ref(OnDelete = OnDelete.Keep)] Id<Platform.User>? GreeterId = null);
