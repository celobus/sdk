// CELOBUS060: inside a query of ctx.Query<T>() (its LINQ operators' lambdas), a call the translation into the query
// tree refuses. A query names columns, compares them with values or columns, and calls only what the SDK translates:
// LINQ's aggregates of a group (Count, LongCount, Sum, Average, Min, Max, Select(..).Distinct().Count()), Any of another
// ctx.Query<T>() (exists), Contains (a substring of a column, or a list of values), DefaultIfEmpty (a left join), and
// Celobus.Sdk.Linq.Query's Label and Search. A call that depends on no row (a captured value, DateTime.UtcNow) is a
// value computed before the query is sent, and allowed.

using System.Collections.Immutable;
using System.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Diagnostics;
using Microsoft.CodeAnalysis.Operations;

namespace Celobus.Sdk.Analyzers;

[DiagnosticAnalyzer(LanguageNames.CSharp)]
public sealed class QueryAnalyzer : DiagnosticAnalyzer
{
    public const string Id = "CELOBUS060";

    public static readonly DiagnosticDescriptor Rule = new(
        Id,
        "A query cannot translate this",
        "{0} cannot be part of a query: a query names columns and calls only what it translates (LINQ's aggregates, Any, Contains, DefaultIfEmpty, Query.Label, Query.Search); compute it before the query or after reading the rows",
        "Celobus",
        DiagnosticSeverity.Error,
        isEnabledByDefault: true,
        description: "ctx.Query<T>() is translated into the query tree of the session, never run in the plugin: a call to an arbitrary method on a row's values, or a projection computed in the plugin, cannot be sent.");

    public override ImmutableArray<DiagnosticDescriptor> SupportedDiagnostics => ImmutableArray.Create(Rule);

    private static readonly ImmutableHashSet<string> Aggregates = ImmutableHashSet.Create(
        "Count", "LongCount", "Sum", "Average", "Min", "Max", "Select", "Distinct", "DefaultIfEmpty", "Contains", "Any");

    public override void Initialize(AnalysisContext context)
    {
        context.ConfigureGeneratedCodeAnalysis(GeneratedCodeAnalysisFlags.None);
        context.EnableConcurrentExecution();
        context.RegisterOperationAction(Analyze, OperationKind.Invocation);
    }

    private static bool IsType(ITypeSymbol? t, string ns, string name) =>
        t is not null && t.Name == name && t.ContainingNamespace?.ToDisplayString() == ns;

    private static bool IsQueryable(IMethodSymbol m) => IsType(m.ContainingType, "System.Linq", "Queryable");

    private static bool IsCtxQuery(IMethodSymbol m) => m.Name == "Query" && IsType(m.ContainingType, "Celobus.Sdk", "Ctx");

    private static IOperation Strip(IOperation op)
    {
        while (op is IConversionOperation c) op = c.Operand;
        return op;
    }

    /// <summary>Whether a query's source starts at ctx.Query&lt;T&gt;() (through LINQ's operators), or is one.</summary>
    private static bool FromCtx(IOperation op)
    {
        op = Strip(op);
        if (IsType(op.Type?.OriginalDefinition, "Celobus.Sdk", "CelobusQuery")) return true;
        if (op is IInvocationOperation inv)
        {
            if (IsCtxQuery(inv.TargetMethod)) return true;
            if (IsQueryable(inv.TargetMethod) && inv.Arguments.Length > 0) return FromCtx(inv.Arguments[0].Value);
        }
        return false;
    }

    private static void Analyze(OperationAnalysisContext context)
    {
        var inv = (IInvocationOperation)context.Operation;
        if (!IsQueryable(inv.TargetMethod) || inv.Arguments.Length < 2 || !FromCtx(inv.Arguments[0].Value)) return;
        foreach (var arg in inv.Arguments.Skip(1))
        {
            if (Strip(arg.Value) is IDelegateCreationOperation { Target: IAnonymousFunctionOperation lambda })
                Walk(lambda.Body, context);
            else if (Strip(arg.Value) is IAnonymousFunctionOperation f)
                Walk(f.Body, context);
        }
    }

    /// <summary>Whether a call is one the translation knows.</summary>
    private static bool Translated(IMethodSymbol m) =>
        IsCtxQuery(m)
        || IsType(m.ContainingType, "Celobus.Sdk.Linq", "Query")
        || (IsType(m.ContainingType, "System.Linq", "Enumerable") && Aggregates.Contains(m.Name))
        || (IsType(m.ContainingType, "System.Linq", "Queryable") && m.Name == "Any")
        || (m.ContainingType?.SpecialType == SpecialType.System_String && m.Name == "Contains")
        || m.Name == "Contains";

    /// <summary>Whether an operation reads a row (a parameter of a lambda of the query).</summary>
    private static bool ReadsRow(IOperation op) =>
        op is IParameterReferenceOperation { Parameter.ContainingSymbol: IMethodSymbol { MethodKind: MethodKind.AnonymousFunction } }
        || op.ChildOperations.Any(ReadsRow);

    private static void Walk(IOperation op, OperationAnalysisContext context)
    {
        foreach (var child in op.ChildOperations)
        {
            if (child is IInvocationOperation call)
            {
                if (IsQueryable(call.TargetMethod))
                {
                    // a nested query (exists) has its own analysis
                    continue;
                }
                if (!Translated(call.TargetMethod) && ReadsRow(call))
                {
                    context.ReportDiagnostic(Diagnostic.Create(Rule, call.Syntax.GetLocation(),
                        $"{call.TargetMethod.ContainingType?.Name}.{call.TargetMethod.Name}"));
                    continue;
                }
            }
            Walk(child, context);
        }
    }
}
