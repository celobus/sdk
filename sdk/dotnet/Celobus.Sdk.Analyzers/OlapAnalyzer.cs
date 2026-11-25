// CELOBUS061: ctx.Db<T>() or ctx.Query<T>() of an entity stored in OLAP ([Entity(Storage = EntityStorage.Olap)]). Its
// rows are in the analytic copy only: its load fills it, ctx.Analytics().Query<T>() reads it; the SDK refuses either
// call at run time, and this reports it at compile time.

using System.Collections.Immutable;
using System.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Diagnostics;
using Microsoft.CodeAnalysis.Operations;

namespace Celobus.Sdk.Analyzers;

[DiagnosticAnalyzer(LanguageNames.CSharp)]
public sealed class OlapAnalyzer : DiagnosticAnalyzer
{
    public const string Id = "CELOBUS061";

    public static readonly DiagnosticDescriptor Rule = new(
        Id,
        "An entity stored in OLAP is read with ctx.Analytics()",
        "{0} is stored in OLAP: ctx.{1}<{0}>() is refused; its load fills it and ctx.Analytics().Query<{0}>() reads it",
        "Celobus",
        DiagnosticSeverity.Error,
        isEnabledByDefault: true,
        description: "An entity stored in OLAP keeps its rows in the analytic copy only: it is never written, and only the analytic engine reads it.");

    public override ImmutableArray<DiagnosticDescriptor> SupportedDiagnostics => ImmutableArray.Create(Rule);

    public override void Initialize(AnalysisContext context)
    {
        context.ConfigureGeneratedCodeAnalysis(GeneratedCodeAnalysisFlags.None);
        context.EnableConcurrentExecution();
        context.RegisterOperationAction(Analyze, OperationKind.Invocation);
    }

    private static bool IsCtx(ITypeSymbol? t)
    {
        for (; t is not null; t = t.BaseType)
            if (t.Name == "Ctx" && t.ContainingNamespace?.ToDisplayString() == "Celobus.Sdk") return true;
        return false;
    }

    /// <summary>Whether a type carries [Entity(Storage = EntityStorage.Olap)].</summary>
    private static bool Olap(ITypeSymbol t) =>
        t.GetAttributes().Any(a =>
            a.AttributeClass is { Name: "EntityAttribute" } c && c.ContainingNamespace?.ToDisplayString() == "Celobus.Sdk"
            && a.NamedArguments.Any(n => n.Key == "Storage" && n.Value.Value is int v && v == 1));

    private static void Analyze(OperationAnalysisContext context)
    {
        var inv = (IInvocationOperation)context.Operation;
        var m = inv.TargetMethod;
        if (m.Name is not ("Db" or "Query") || m.TypeArguments.Length != 1 || !IsCtx(m.ContainingType)) return;
        var t = m.TypeArguments[0];
        if (Olap(t))
            context.ReportDiagnostic(Diagnostic.Create(Rule, inv.Syntax.GetLocation(), t.Name, m.Name));
    }
}
