namespace Celobus.Sdk.Tests;

/// <summary><see cref="Ctx.Test"/>: a context for unit tests of handlers, with only the public API.</summary>
public sealed class TestCtxTests
{
    [Fact]
    public void TheDefaultCallerIsTheSystem()
    {
        var ctx = Ctx.Test("tests", "1.0.0");
        Assert.Equal(new PluginInfo("tests", "1.0.0"), ctx.Plugin);
        Assert.Equal(CallerKind.None, ctx.Caller.Kind);
        Assert.Null(ctx.Caller.UserId);
        Assert.Equal(401, Assert.Throws<PluginError>(() => ctx.Caller.User()).Status);
        Assert.Equal(0ul, ctx.Request.Id);
        Assert.Null(ctx.Request.IdempotencyKey);
        Assert.InRange(ctx.Request.Deadline - DateTimeOffset.UtcNow, TimeSpan.FromSeconds(25), TimeSpan.FromSeconds(30));
        Assert.False(ctx.Job.IsRun);
    }

    [Fact]
    public void WithCallerGivesAPersonWithRolesAndScopesOfThisPlugin()
    {
        var ctx = Ctx.Test("tests", "1.0.0")
            .WithCaller(Caller.TestUser("u1").WithRoles("tests.clerk").WithScopes("tests:read"));
        Assert.Equal((CallerKind.Person, "Test User"), (ctx.Caller.Kind, ctx.Caller.DisplayName));
        Assert.Equal("u1", ctx.Caller.User().ToString());
        Assert.True(ctx.Caller.Has(Roles.Clerk));
        Assert.False(ctx.Caller.Has(Role.WorkspaceAdmin));
        Assert.True(ctx.Caller.HasScope(new Scope("read")));
        Assert.False(ctx.Caller.HasScope(new Scope("write")));

        var plain = Ctx.Test("tests", "1.0.0").WithCaller(Caller.TestUser("u2"));
        Assert.False(plain.Caller.Has(Roles.Clerk));
        Assert.Empty(plain.Caller.Roles);
    }

    [Fact]
    public void WithRunGivesTheHandlerARun()
    {
        var before = DateTimeOffset.UtcNow;
        var ctx = Ctx.Test("tests", "1.0.0").WithCaller(Caller.TestUser("u1")).WithRun("run-1", 1, 4);
        Assert.True(ctx.Job.IsRun);
        Assert.Equal(("", "run-1", 1, 4), (ctx.Job.Job, ctx.Job.RunId, ctx.Job.Attempt, ctx.Job.MaxAttempts));
        Assert.False(ctx.Job.IsLastAttempt);
        Assert.InRange(ctx.Job.ScheduledAt, before, DateTimeOffset.UtcNow);
        Assert.Equal("u1", ctx.Caller.User().ToString()); // the caller is kept
        Assert.True(ctx.WithRun("run-1", 4, 4).Job.IsLastAttempt);
    }

    [Fact]
    public async Task EveryCallToThePlatformAnswersUnavailable()
    {
        var ctx = Ctx.Test("tests", "1.0.0").WithCaller(Caller.TestUser("u1"));
        Func<Task>[] calls =
        [
            async () => await ctx.Db<Rack>().Get(new Id<Rack>("1")),
            () => ctx.Db<Rack>().Create(new Rack(new Id<Rack>("1"), "Top")),
            () => ctx.CallRaw("billing", "GetInvoice", "{}"),
            () => ctx.Members.List(),
            () => ctx.Agents.Run("assistant", "hello"),
            async () => await ctx.Jobs.Enqueue(new RemindUnpaid(7)),
            () => ctx.Sql("select 1"),
            () => ctx.Transaction(tx => tx.Db<Rack>().Create(new Rack(new Id<Rack>("1"), "Top"))),
        ];
        foreach (var call in calls)
        {
            var e = await Assert.ThrowsAsync<PluginError>(call);
            Assert.Equal((503, "unavailable"), (e.Status, e.Code));
        }
        ctx.InvalidateAll(LineTopics.Lines); // nothing to tell: no error
    }
}
