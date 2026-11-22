using System.Collections.Immutable;
using Celobus.Sdk.Analyzers;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Diagnostics;

namespace Celobus.Sdk.Tests;

/// <summary>CELOBUS060 on snippets compiled against the SDK: what a query cannot translate is a compile-time error.</summary>
public class AnalyzerTests
{
    private const string Prelude = """
        using System;
        using System.Linq;
        using System.Collections.Generic;
        using Celobus.Sdk;
        using Celobus.Sdk.Linq;
        using Platform = Celobus.Sdk.Platform;

        [Entity] public sealed partial record Order(Id<Order> Id, string Number, decimal Total, Id<Platform.User> CreatedBy, DateTimeOffset PlacedAt);
        [Entity] public sealed partial record Line(Id<Line> Id, Id<Order> OrderId, decimal Amount);

        public static class Q
        {
            public static string Shout(string s) => s.ToUpperInvariant();

        """;

    private static async Task<ImmutableArray<Diagnostic>> Diagnose(string body)
    {
        var source = Prelude + body + "\n}\n";
        var tpa = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator);
        var refs = tpa.Select(p => MetadataReference.CreateFromFile(p)).ToList();
        refs.Add(MetadataReference.CreateFromFile(typeof(Ctx).Assembly.Location));
        var compilation = CSharpCompilation.Create("snippet", [CSharpSyntaxTree.ParseText(source)], refs,
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, nullableContextOptions: NullableContextOptions.Enable));
        var errors = compilation.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error).ToList();
        Assert.True(errors.Count == 0, string.Join("\n", errors));
        return await compilation.WithAnalyzers([new QueryAnalyzer()]).GetAnalyzerDiagnosticsAsync();
    }

    [Fact]
    public async Task ACallOnARowsValueIsReported()
    {
        var d = await Diagnose("""
            public static object M(Ctx ctx) => ctx.Query<Order>().Where(o => o.Number.ToUpperInvariant() == "X");
            public static object N(Ctx ctx) => from o in ctx.Query<Order>() select new { N = Shout(o.Number) };
            public static object P(Ctx ctx) =>
                from o in ctx.Query<Order>()
                where ctx.Query<Line>().Any(l => l.OrderId == o.Id && l.Amount.ToString() == "1")
                select o.Number;
            """);
        Assert.Equal(["String.ToUpperInvariant", "Q.Shout", "Decimal.ToString"],
            d.OrderBy(x => x.Location.SourceSpan.Start).Select(x => x.GetMessage().Split(' ')[0]).ToArray());
        Assert.All(d, x => Assert.Equal((QueryAnalyzer.Id, DiagnosticSeverity.Error), (x.Id, x.Severity)));
    }

    [Fact]
    public async Task WhatTheQueryTranslatesAndValuesComputedBeforeItAreNot()
    {
        var d = await Diagnose("""
            public static object M(Ctx ctx, List<string> numbers) =>
                from l in ctx.Query<Line>()
                join o in ctx.Query<Order>() on l.OrderId equals o.Id
                where o.PlacedAt >= DateTimeOffset.UtcNow.AddDays(-7) && numbers.Contains(o.Number) && o.Number.Contains("SO")
                where Query.Search(o.Number, "tea") && ctx.Query<Platform.RoleAssignment>().Any(r => r.UserId == o.CreatedBy)
                group l by new { o.CreatedBy } into g
                where g.Count() > 1
                orderby g.Sum(x => x.Amount) descending
                select new { By = Query.Label(g.Key.CreatedBy), Total = g.Sum(x => x.Amount), Max = g.Max(x => x.Amount) };
            // not a query of ctx.Query<T>(): LINQ to objects is not the analyzer's business
            public static object N() => new List<Order>().AsQueryable().Where(o => o.Number.Trim() == "");
            """);
        Assert.Empty(d);
    }
}
