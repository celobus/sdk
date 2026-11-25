namespace Celobus.Sdk.Tests;

public class SyscallErrorTests
{
    [Fact]
    public void AdmissionRefusalsAre403WithTheirCode()
    {
        var e = Ctx.SyscallFailed("call_undeclared: the calling plugin does not declare /c.v1.C/D among its calls");
        Assert.Equal((403, "call_undeclared"), (e.Status, e.Code));
        var other = Ctx.SyscallFailed("membership_stale: the member table is not current");
        Assert.Equal(503, other.Status);
    }

    [Theory]
    [InlineData("analytic_time_limit: the query ran past 10 s", 422)]
    [InlineData("analytic_memory_limit: the query needs more memory than the copy has", 422)]
    [InlineData("analytic_busy: four queries of the workspace are running", 503)]
    [InlineData("unavailable: the analytic copy is being made", 503)]
    [InlineData("lake_rows_limit: a write holds at most 1000000 rows", 422)]
    [InlineData("lake_time_limit: the query took longer than 10 s", 422)]
    [InlineData("lake_memory_limit: the query needs more memory than the lake's engine has", 422)]
    [InlineData("lake_busy: other writes of this table ran for more than 60 s", 503)]
    public void TheAnalyticEnginesRefusalsComeBackTyped(string error, int status)
    {
        var e = Ctx.SyscallFailed(error);
        Assert.Equal((status, error[..error.IndexOf(':')]), (e.Status, e.Code));
    }
}
