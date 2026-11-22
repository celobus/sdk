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
}
