// The plugin: identity and subjects. Only builds it, so the SDK's tests can load it without serving.
using Celobus.Sdk;

namespace Hello;

public static class Roles
{
    public static readonly Role Clerk = new("clerk") { Title = "Clerk" };
    public static readonly Role Manager = new("manager") { Title = "Manager", Strict = true };
}

public static class HelloPlugin
{
    public static Plugin Define() => Plugin.Create("hello")
        .Title("Hello")
        .Describe("The SDK conformance plugin.")
        .Role(Roles.Clerk)
        .Role(Roles.Manager)
        .Capability(Capability.PushSend, "Greets people.")
        .Capability(Capability.AuthzBypass, "Counts every note.")
        .Bypass<Note>(Perm.Read)
        .DependsOn("greeter", "Greet")
        .Entity<Note>()
        // who reads and writes notes: the owner their own, whoever greeted on one by name that one, a manager every
        // note (and the secret)
        .Access<Note>(a =>
        {
            var owner = a.Relation("owner", a.Column(n => n.OwnerId));
            var greeted = a.Relation("greeted",
                a.Rows<Greeting>(g => g.NoteId, g => g.GreeterId).When(Filter<Greeting>.Ne(g => g.Name, "")));
            a.Read(owner | greeted | Roles.Manager);
            a.Create(owner);
            a.Update(owner);
            a.Delete(owner | Roles.Manager);
            a.Mask(n => n.Secret, Roles.Manager);
        })
        .Entity<Greeting>()
        // who greets, and reads the greetings: whoever may call `Hello`
        .Access<Greeting>(a =>
        {
            a.Read((Grant)Roles.Clerk | Roles.Manager);
            a.Create((Grant)Roles.Clerk | Roles.Manager);
        })
        .Endpoint<Hello>()
        .Endpoint<GetNote>()
        .Endpoint<SaveNote>()
        .Endpoint<DeleteNote>()
        .Endpoint<ListNotes>()
        .Endpoint<CountAllNotes>()
        .Endpoint<NotesByOwner>();
}
