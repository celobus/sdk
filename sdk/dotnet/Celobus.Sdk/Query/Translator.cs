// A LINQ query over ctx.Query<T>() as the query tree of the session: the operators the C# compiler makes of query
// syntax (Join; GroupJoin with SelectMany over DefaultIfEmpty, a left join; Where; GroupBy; OrderBy and ThenBy; Select;
// Take), their lambdas read against the shape of the rows at that point (an entity under its alias, the anonymous
// records of transparent identifiers and projections, a group's key and its elements), and the projection rebuilt
// from the row's values. What it cannot translate is a NotSupportedException (500 `query_invalid` when read); the
// analyzer CELOBUS060 reports most of it at compile time.

using System.Globalization;
using System.Linq.Expressions;
using System.Reflection;
using System.Text.Json;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>A translated query: its tree, and how a row of the answer becomes the query's element.</summary>
internal sealed class Translated(Celobus.V1.Query tree, Func<JsonElement, ByteString, object?> read)
{
    public Celobus.V1.Query Tree { get; } = tree;
    public Func<JsonElement, ByteString, object?> Read { get; } = read;
}

internal abstract record Shape;

/// <summary>An entity's row under its alias.</summary>
internal sealed record SourceShape(string Alias, Type Entity) : Shape;

/// <summary>A record of shapes: a transparent identifier, an anonymous type, a projection (with how to build it).</summary>
internal sealed record RecordShape(List<(string Name, Shape Shape)> Members, Expression? Build) : Shape;

/// <summary>A group: its key and what its elements are.</summary>
internal sealed record GroupShape(Shape Key, Shape Element) : Shape;

/// <summary>A value of a row: a column, an aggregate or a label, and the type the projection reads it as.</summary>
internal sealed record ValueShape(QueryTerm? Term, QueryColumn? LabelOf, Type Type) : Shape
{
    public QueryColumn? Column => Term?.Column;
    public bool IsAggregate => Term?.Aggregate is not null;
}

/// <summary>The rows a GroupJoin brings, waiting for the SelectMany that makes them a join.</summary>
internal sealed record PendingJoin(QuerySource Source, Type Entity) : Shape;

internal static class Translator
{
    private static readonly Dictionary<Type, string> Directory = new()
    {
        [typeof(Platform.User)] = "platform.user",
        [typeof(Platform.Group)] = "platform.group",
        [typeof(Platform.GroupMember)] = "platform.group_member",
        [typeof(Platform.RoleAssignment)] = "platform.role_assignment",
    };

    public static bool IsDirectory(Type t) => Directory.ContainsKey(t);

    /// <summary>The entity's name in a query: <c>order</c>, <c>crm.customer</c>, <c>platform.user</c>.</summary>
    public static string Canonical(Type t) =>
        Directory.TryGetValue(t, out var d) ? d
        : Schema.ForeignPlugin(t) is { } p ? $"{p}.{Names.Snake(t.Name)}"
        : Names.Snake(t.Name);

    public static string DefaultAlias(Type t) => Canonical(t).Replace('.', '_');

    public static Type ElementOf(Type seq)
    {
        var i = seq.IsGenericType && seq.GetGenericTypeDefinition() == typeof(IEnumerable<>)
            ? seq
            : seq.GetInterfaces().FirstOrDefault(x => x.IsGenericType && x.GetGenericTypeDefinition() == typeof(IEnumerable<>));
        return i?.GetGenericArguments()[0] ?? seq;
    }

    private static NotSupportedException No(string what) => new($"a query cannot say {what}");

    public static Translated Translate(Expression e)
    {
        var t = new Translation();
        t.Operator(e);
        return t.Finish(ElementOf(e.Type));
    }

    private sealed class Translation
    {
        private readonly Celobus.V1.Query _tree = new();
        private Shape _shape = null!;
        private Type _root = null!;

        private bool Taken(string alias) => _tree.Sources.Any(s => s.Alias == alias);

        private string AliasFor(Type entity, string? name)
        {
            var alias = DefaultAlias(entity);
            if (!Taken(alias)) return alias;
            if (name is not null && !Taken(name)) return name;
            throw No($"{Canonical(entity)} twice without a range variable of its own name");
        }

        private static IQueryRoot? RootOf(Expression e) => e switch
        {
            ConstantExpression { Value: IQueryRoot r } => r,
            MethodCallExpression { Method.Name: "Query", Method.DeclaringType: var d } m when d == typeof(Ctx) && !HasParameter(m)
                => Evaluate(m) as IQueryRoot,
            _ => null,
        };

        public void Operator(Expression e)
        {
            if (RootOf(e) is { } root)
            {
                _root = root.Entity;
                var alias = DefaultAlias(root.Entity);
                _tree.Sources.Add(new QuerySource { Entity = Canonical(root.Entity), Alias = alias });
                _shape = new SourceShape(alias, root.Entity);
                return;
            }
            if (e is not MethodCallExpression m || m.Method.DeclaringType != typeof(Queryable))
                throw No($"{e}: a query starts from ctx.Query<T>() and goes on with LINQ's operators");
            Operator(m.Arguments[0]);
            var name = m.Method.Name;
            switch (name)
            {
                case "Where":
                {
                    var l = Lambda(m.Arguments[1]);
                    var env = Env(l, _shape);
                    if (_shape is GroupShape)
                        _tree.Having = And(_tree.Having, Cond(l.Body, env, true));
                    else
                        _tree.Filter = And(_tree.Filter, Cond(l.Body, env, false));
                    break;
                }
                case "Select":
                {
                    var l = Lambda(m.Arguments[1]);
                    _shape = Resolve(l.Body, Env(l, _shape)) ?? throw No($"select {l.Body}: columns, aggregates, labels and records of them");
                    break;
                }
                case "Join":
                case "GroupJoin":
                {
                    var inner = RootOf(m.Arguments[1]) ?? throw No("a join of anything but ctx.Query<T>()");
                    var outerKey = Lambda(m.Arguments[2]);
                    var innerKey = Lambda(m.Arguments[3]);
                    var result = Lambda(m.Arguments[4]);
                    var alias = AliasFor(inner.Entity, innerKey.Parameters[0].Name);
                    var left = Column(Resolve(outerKey.Body, Env(outerKey, _shape)), "a join's key");
                    var right = Column(Resolve(innerKey.Body, Env(innerKey, new SourceShape(alias, inner.Entity))), "a join's key");
                    if (right.Source != alias) throw No("a join whose key is not a column of the joined entity");
                    var source = new QuerySource
                    {
                        Entity = Canonical(inner.Entity),
                        Alias = alias,
                        Join = QuerySource.Types.Join.Inner,
                        Left = left,
                        Right = right,
                    };
                    Shape joined = name == "Join" ? new SourceShape(alias, inner.Entity) : new PendingJoin(source, inner.Entity);
                    if (name == "Join") _tree.Sources.Add(source);
                    var env = new Dictionary<ParameterExpression, Shape>
                    {
                        [result.Parameters[0]] = _shape,
                        [result.Parameters[1]] = joined,
                    };
                    _shape = Resolve(result.Body, env) ?? throw No($"a join's result {result.Body}");
                    break;
                }
                case "SelectMany":
                {
                    var collection = Lambda(m.Arguments[1]);
                    var body = collection.Body;
                    var outer = false;
                    if (body is MethodCallExpression { Method.Name: "DefaultIfEmpty" } d && d.Arguments.Count == 1)
                    {
                        outer = true;
                        body = d.Arguments[0];
                    }
                    if (Resolve(body, Env(collection, _shape)) is not PendingJoin pending)
                        throw No("from .. in .. other than a join's group (join .. into g from x in g.DefaultIfEmpty())");
                    pending.Source.Join = outer ? QuerySource.Types.Join.Left : QuerySource.Types.Join.Inner;
                    _tree.Sources.Add(pending.Source);
                    var joined = new SourceShape(pending.Source.Alias, pending.Entity);
                    if (m.Arguments.Count == 3)
                    {
                        var result = Lambda(m.Arguments[2]);
                        var env = new Dictionary<ParameterExpression, Shape>
                        {
                            [result.Parameters[0]] = _shape,
                            [result.Parameters[1]] = joined,
                        };
                        _shape = Resolve(result.Body, env) ?? throw No($"a join's result {result.Body}");
                    }
                    else
                    {
                        _shape = joined;
                    }
                    break;
                }
                case "GroupBy":
                {
                    if (_shape is GroupShape) throw No("a group of groups");
                    var keyL = Lambda(m.Arguments[1]);
                    var key = Resolve(keyL.Body, Env(keyL, _shape)) ?? throw No($"group by {keyL.Body}");
                    foreach (var v in Leaves(key))
                    {
                        if (v.IsAggregate) throw No("a group by an aggregate");
                        var c = v.Column ?? v.LabelOf!;
                        if (!_tree.GroupBy.Contains(c)) _tree.GroupBy.Add(c);
                    }
                    var element = _shape;
                    if (m.Arguments.Count >= 3 && Lambda(m.Arguments[2]) is { Parameters.Count: 1 } el)
                        element = Resolve(el.Body, Env(el, _shape)) ?? throw No($"a group's elements {el.Body}");
                    if (m.Arguments.Count > 3 || (m.Arguments.Count == 3 && Lambda(m.Arguments[2]).Parameters.Count == 2))
                        throw No("a group by with a result selector: write `into g select ..`");
                    _shape = new GroupShape(key, element);
                    break;
                }
                case "OrderBy":
                case "OrderByDescending":
                case "ThenBy":
                case "ThenByDescending":
                {
                    var l = Lambda(m.Arguments[1]);
                    var v = Resolve(l.Body, Env(l, _shape)) as ValueShape
                            ?? throw No($"order by {l.Body}: a column or an aggregate");
                    if (v.Term is null) throw No("an order by a label");
                    _tree.Order.Add(new QueryOrder { Term = v.Term, Desc = name.EndsWith("Descending", StringComparison.Ordinal) });
                    break;
                }
                case "Take":
                    _tree.Limit = Convert.ToUInt32(Evaluate(m.Arguments[1]), CultureInfo.InvariantCulture);
                    break;
                default:
                    throw No($"{name}: a query joins, filters (where), groups, orders, selects and takes");
            }
        }

        public Translated Finish(Type element)
        {
            if (_shape is SourceShape src && src.Entity == element && src.Alias == _tree.Sources[0].Alias)
            {
                // the root's rows, as the repository reads them
                return new Translated(_tree, (_, raw) =>
                    JsonSerializer.Deserialize(raw.Span, element, CelobusJson.Options)
                    ?? throw PluginError.Unavailable("platform: an empty row"));
            }
            if (!Projectable(_shape)) throw No("rows of entities: select their columns (an anonymous type or a record of them)");
            var leaves = Leaves(_shape).ToList();
            foreach (var v in leaves)
            {
                _tree.Select.Add(v switch
                {
                    { LabelOf: { } c } => new QuerySelect { Label = c },
                    { Term.Column: { } c } => new QuerySelect { Column = c },
                    { Term.Aggregate: { } a } => new QuerySelect { Aggregate = a },
                    _ => throw No("an empty select item"),
                });
            }
            var shape = _shape;
            return new Translated(_tree, (row, _) =>
            {
                var i = 0;
                return Build(shape, row, ref i);
            });
        }

        // --- shapes -----------------------------------------------------------------------------------------------

        private static Dictionary<ParameterExpression, Shape> Env(LambdaExpression l, Shape shape) =>
            new() { [l.Parameters[0]] = shape };

        private static LambdaExpression Lambda(Expression e)
        {
            while (e is UnaryExpression { NodeType: ExpressionType.Quote } q) e = q.Operand;
            return e as LambdaExpression ?? throw No($"{e} where a lambda goes");
        }

        private static IEnumerable<ValueShape> Leaves(Shape s) => s switch
        {
            ValueShape v => [v],
            RecordShape r => r.Members.SelectMany(m => Leaves(m.Shape)),
            _ => [],
        };

        private static bool Projectable(Shape s) => s switch
        {
            ValueShape => true,
            RecordShape r => r.Build is not null && r.Members.All(m => Projectable(m.Shape)),
            _ => false,
        };

        private static QueryColumn Column(Shape? s, string what) =>
            (s as ValueShape)?.Column ?? throw No($"{what} other than a column");

        private Shape? Resolve(Expression e, Dictionary<ParameterExpression, Shape> env)
        {
            switch (e)
            {
                case ParameterExpression p:
                    return env.TryGetValue(p, out var s) ? s : null;
                case UnaryExpression { NodeType: ExpressionType.Convert or ExpressionType.ConvertChecked } u:
                    return Resolve(u.Operand, env) is ValueShape v ? v with { Type = u.Type } : Resolve(u.Operand, env);
                case MemberExpression { Expression: { } inner } me:
                {
                    var of = Resolve(inner, env);
                    return of switch
                    {
                        SourceShape src => new ValueShape(
                            new QueryTerm { Column = new QueryColumn { Source = src.Alias, Field = Names.Snake(me.Member.Name) } },
                            null, MemberType(me.Member)),
                        RecordShape r => r.Members.FirstOrDefault(x => x.Name == me.Member.Name).Shape
                                         ?? throw No($"{me}: no member {me.Member.Name}"),
                        GroupShape g when me.Member.Name == "Key" => g.Key,
                        _ => null,
                    };
                }
                case NewExpression n:
                {
                    var members = new List<(string, Shape)>();
                    for (var i = 0; i < n.Arguments.Count; i++)
                    {
                        var shape = Resolve(n.Arguments[i], env);
                        if (shape is null) return null;
                        members.Add((n.Members?[i].Name ?? $"Item{i + 1}", shape));
                    }
                    return new RecordShape(members, n);
                }
                case MemberInitExpression mi:
                {
                    if (mi.NewExpression.Arguments.Count > 0) return null;
                    var members = new List<(string, Shape)>();
                    foreach (var b in mi.Bindings)
                    {
                        if (b is not MemberAssignment a || Resolve(a.Expression, env) is not { } shape) return null;
                        members.Add((a.Member.Name, shape));
                    }
                    return new RecordShape(members, mi);
                }
                case MethodCallExpression mc:
                    return Call(mc, env);
                default:
                    return null;
            }
        }

        private static Type MemberType(MemberInfo m) => m switch
        {
            PropertyInfo p => p.PropertyType,
            FieldInfo f => f.FieldType,
            _ => typeof(object),
        };

        /// <summary>A label, or an aggregate of a group.</summary>
        private Shape? Call(MethodCallExpression m, Dictionary<ParameterExpression, Shape> env)
        {
            if (m.Method.DeclaringType == typeof(Linq.Query) && m.Method.Name == nameof(Linq.Query.Label))
            {
                var c = Column(Resolve(m.Arguments[0], env), "a label");
                return new ValueShape(null, c, typeof(Label));
            }
            if (m.Method.DeclaringType != typeof(Enumerable) || m.Arguments.Count == 0) return null;
            // g.Select(x => x.P).Distinct().Count(): the distinct values
            if (m.Method.Name is "Count" or "LongCount" && m.Arguments.Count == 1
                && m.Arguments[0] is MethodCallExpression { Method.Name: "Distinct" } distinct
                && distinct.Arguments[0] is MethodCallExpression { Method.Name: "Select" } sel
                && Resolve(sel.Arguments[0], env) is GroupShape dg)
            {
                var l = Lambda(sel.Arguments[1]);
                var col = Column(Resolve(l.Body, Env(l, dg.Element)), "count of distinct values");
                return Aggregate(QueryAggregate.Types.Fn.CountDistinct, col, m.Type);
            }
            if (Resolve(m.Arguments[0], env) is not GroupShape g) return null;
            QueryColumn? column = null;
            if (m.Arguments.Count == 2)
            {
                var l = Lambda(m.Arguments[1]);
                column = Column(Resolve(l.Body, Env(l, g.Element)), $"{m.Method.Name} of a group");
            }
            var fn = m.Method.Name switch
            {
                "Count" or "LongCount" when column is null => QueryAggregate.Types.Fn.Count,
                "Sum" => QueryAggregate.Types.Fn.Sum,
                "Average" => QueryAggregate.Types.Fn.Avg,
                "Min" => QueryAggregate.Types.Fn.Min,
                "Max" => QueryAggregate.Types.Fn.Max,
                _ => throw No($"{m.Method.Name} of a group: Count, Sum, Average, Min, Max, Select(..).Distinct().Count()"),
            };
            return Aggregate(fn, column, m.Type);
        }

        private static ValueShape Aggregate(QueryAggregate.Types.Fn fn, QueryColumn? column, Type type)
        {
            var a = new QueryAggregate { Fn = fn };
            if (column is not null) a.Column = column;
            return new ValueShape(new QueryTerm { Aggregate = a }, null, type);
        }

        // --- conditions ---------------------------------------------------------------------------------------------

        private static QueryExpr? And(QueryExpr? prev, QueryExpr next) =>
            prev is null ? next : new QueryExpr { And = new QueryExprs { Items = { prev, next } } };

        private QueryExpr Cond(Expression e, Dictionary<ParameterExpression, Shape> env, bool having)
        {
            switch (e)
            {
                case BinaryExpression { NodeType: ExpressionType.AndAlso or ExpressionType.And } b when b.Type == typeof(bool):
                    return new QueryExpr { And = new QueryExprs { Items = { Cond(b.Left, env, having), Cond(b.Right, env, having) } } };
                case BinaryExpression { NodeType: ExpressionType.OrElse or ExpressionType.Or } b when b.Type == typeof(bool):
                    return new QueryExpr { Or = new QueryExprs { Items = { Cond(b.Left, env, having), Cond(b.Right, env, having) } } };
                case UnaryExpression { NodeType: ExpressionType.Not } u:
                    return new QueryExpr { Not = Cond(u.Operand, env, having) };
                case BinaryExpression b when Ops.TryGetValue(b.NodeType, out var op):
                    return Compare(b, op, env, having);
                case MethodCallExpression m:
                    return CallCond(m, env, having);
                default:
                    if (e.Type == typeof(bool) && Resolve(e, env) is ValueShape v && v.Term is { } t)
                        return Compared(t, DbCondition.Types.Op.Eq, [true], false);
                    throw No($"where {e}");
            }
        }

        private static readonly Dictionary<ExpressionType, DbCondition.Types.Op> Ops = new()
        {
            [ExpressionType.Equal] = DbCondition.Types.Op.Eq,
            [ExpressionType.NotEqual] = DbCondition.Types.Op.Ne,
            [ExpressionType.LessThan] = DbCondition.Types.Op.Lt,
            [ExpressionType.LessThanOrEqual] = DbCondition.Types.Op.Le,
            [ExpressionType.GreaterThan] = DbCondition.Types.Op.Gt,
            [ExpressionType.GreaterThanOrEqual] = DbCondition.Types.Op.Ge,
        };

        private static readonly Dictionary<DbCondition.Types.Op, DbCondition.Types.Op> Mirror = new()
        {
            [DbCondition.Types.Op.Eq] = DbCondition.Types.Op.Eq,
            [DbCondition.Types.Op.Ne] = DbCondition.Types.Op.Ne,
            [DbCondition.Types.Op.Lt] = DbCondition.Types.Op.Gt,
            [DbCondition.Types.Op.Le] = DbCondition.Types.Op.Ge,
            [DbCondition.Types.Op.Gt] = DbCondition.Types.Op.Lt,
            [DbCondition.Types.Op.Ge] = DbCondition.Types.Op.Le,
        };

        /// <summary>The term an expression is, through the conversions C# puts around enums and nullables.</summary>
        private ValueShape? Term(Expression e, Dictionary<ParameterExpression, Shape> env)
        {
            while (e is UnaryExpression { NodeType: ExpressionType.Convert or ExpressionType.ConvertChecked } u)
            {
                if (Resolve(u.Operand, env) is ValueShape inner) return inner;
                e = u.Operand;
            }
            return Resolve(e, env) as ValueShape;
        }

        private QueryExpr Compare(BinaryExpression b, DbCondition.Types.Op op, Dictionary<ParameterExpression, Shape> env, bool having)
        {
            var l = Term(b.Left, env);
            var r = Term(b.Right, env);
            if (l is not null && r is not null)
            {
                if (l.Term is null || r.Term is null) throw No("a comparison of labels");
                if (!having && (l.IsAggregate || r.IsAggregate)) throw No("an aggregate in where: compare it after group by");
                return new QueryExpr { Compare = new QueryCompare { Left = l.Term, Op = op, Right = r.Term } };
            }
            var (term, value, mirrored) = l is not null ? (l, b.Right, false) : r is not null ? (r, b.Left, true) : (null, null, false);
            if (term?.Term is null || value is null) throw No($"{b}: a comparison of a column (or an aggregate) with a value or a column");
            if (!having && term.IsAggregate) throw No("an aggregate in where: compare it after group by");
            if (mirrored) op = Mirror[op];
            var v = Evaluate(value);
            if (v is null)
            {
                return op switch
                {
                    DbCondition.Types.Op.Eq => Compared(term.Term, DbCondition.Types.Op.IsNull, [], false),
                    DbCondition.Types.Op.Ne => Compared(term.Term, DbCondition.Types.Op.IsNotNull, [], false),
                    _ => throw No($"{b}: an order comparison with null"),
                };
            }
            // an enum compared as its number (C# converts both sides): its value
            var underlying = Nullable.GetUnderlyingType(term.Type) ?? term.Type;
            if (underlying.IsEnum && !v.GetType().IsEnum) v = Enum.ToObject(underlying, v);
            return Compared(term.Term, op, [v], term.IsAggregate);
        }

        private static QueryExpr Compared(QueryTerm left, DbCondition.Types.Op op, IEnumerable<object?> values, bool aggregate)
        {
            var c = new QueryCompare { Left = left, Op = op };
            foreach (var v in values)
            {
                // an aggregate's value is compared as the text of its number, as the other SDKs send it
                c.Values.Add(aggregate
                    ? new DbValue { StringValue = Convert.ToString(v, CultureInfo.InvariantCulture) ?? "" }
                    : DbValues.Of(v));
            }
            return new QueryExpr { Compare = c };
        }

        private QueryExpr CallCond(MethodCallExpression m, Dictionary<ParameterExpression, Shape> env, bool having)
        {
            // full-text search
            if (m.Method.DeclaringType == typeof(Linq.Query) && m.Method.Name == nameof(Linq.Query.Search))
            {
                var t = Term(m.Arguments[0], env)?.Term ?? throw No("a search of anything but a column");
                return Compared(t, DbCondition.Types.Op.Search, [Evaluate(m.Arguments[1])], false);
            }
            // column.Contains("text"): a substring
            if (m.Method.DeclaringType == typeof(string) && m.Method.Name == nameof(string.Contains) && m.Object is { } obj
                && m.Arguments.Count == 1 && Term(obj, env) is { Term: { } col })
            {
                return Compared(col, DbCondition.Types.Op.Contains, [Evaluate(m.Arguments[0])], false);
            }
            // values.Contains(column): IN
            if (m.Method.Name == "Contains" && !HasParameter(m.Object ?? m.Arguments[0]))
            {
                var (list, item) = m.Object is { } o ? (o, m.Arguments[0]) : (m.Arguments[0], m.Arguments[1]);
                if (Term(item, env) is { Term: { } inCol } inShape)
                {
                    var values = ((System.Collections.IEnumerable)Evaluate(list)!).Cast<object?>().ToList();
                    var underlying = Nullable.GetUnderlyingType(inShape.Type) ?? inShape.Type;
                    return Compared(inCol, DbCondition.Types.Op.In,
                        values.Select(x => underlying.IsEnum && x is not null && !x.GetType().IsEnum ? Enum.ToObject(underlying, x) : x), false);
                }
            }
            // ctx.Query<T>().Any(x => ..): a correlated EXISTS
            if (m.Method.Name == "Any" && (m.Method.DeclaringType == typeof(Queryable) || m.Method.DeclaringType == typeof(Enumerable))
                && RootOf(m.Arguments[0]) is { } root)
            {
                if (having) throw No("exists in a group's condition");
                var alias = AliasFor(root.Entity, m.Arguments.Count == 2 ? Lambda(m.Arguments[1]).Parameters[0].Name : null);
                var exists = new QueryExists { Source = new QuerySource { Entity = Canonical(root.Entity), Alias = alias } };
                if (m.Arguments.Count == 2)
                {
                    var l = Lambda(m.Arguments[1]);
                    var inner = new Dictionary<ParameterExpression, Shape>(env) { [l.Parameters[0]] = new SourceShape(alias, root.Entity) };
                    exists.Filter = Cond(l.Body, inner, false);
                }
                return new QueryExpr { Exists = exists };
            }
            if (m.Type == typeof(bool) && Term(m, env) is { Term: { } b })
                return Compared(b, DbCondition.Types.Op.Eq, [true], false);
            throw No($"{m.Method.DeclaringType?.Name}.{m.Method.Name} in a condition");
        }

        // --- values -------------------------------------------------------------------------------------------------

        private sealed class ParameterFinder : ExpressionVisitor
        {
            public bool Found;
            protected override Expression VisitParameter(ParameterExpression node)
            {
                Found = true;
                return node;
            }
            protected override Expression VisitLambda<T>(Expression<T> node) => node;
        }

        private static bool HasParameter(Expression e)
        {
            var f = new ParameterFinder();
            f.Visit(e);
            return f.Found;
        }

        /// <summary>A value the query carries (a constant, a captured variable), computed now.</summary>
        private static object? Evaluate(Expression e)
        {
            if (HasParameter(e)) throw No($"{e}: a value that depends on the rows is a column or a condition");
            if (e is ConstantExpression c) return c.Value;
            return Expression.Lambda(Expression.Convert(e, typeof(object))).Compile(preferInterpretation: true).DynamicInvoke();
        }

        // --- the projection -----------------------------------------------------------------------------------------

        private static object? Build(Shape shape, JsonElement row, ref int i)
        {
            switch (shape)
            {
                case ValueShape v:
                    return ReadValue(v, row[i++]);
                case RecordShape { Build: NewExpression n } r:
                {
                    var args = new object?[r.Members.Count];
                    for (var k = 0; k < args.Length; k++) args[k] = Build(r.Members[k].Shape, row, ref i);
                    return n.Constructor is null ? null : n.Constructor.Invoke(args);
                }
                case RecordShape { Build: MemberInitExpression mi } r:
                {
                    var o = Activator.CreateInstance(mi.Type)!;
                    foreach (var (name, s) in r.Members)
                    {
                        var value = Build(s, row, ref i);
                        var member = mi.Type.GetMember(name).First();
                        if (member is PropertyInfo p) p.SetValue(o, value);
                        else if (member is FieldInfo f) f.SetValue(o, value);
                    }
                    return o;
                }
                default:
                    throw No("rows of this shape");
            }
        }

        private static object? ReadValue(ValueShape v, JsonElement e)
        {
            if (v.LabelOf is not null)
            {
                if (e.ValueKind != JsonValueKind.Object) return null;
                string? S(string k) => e.TryGetProperty(k, out var x) && x.ValueKind == JsonValueKind.String ? x.GetString() : null;
                return new Label(S("id") ?? "", S("text") ?? "", S("avatarUrl"));
            }
            if (e.ValueKind == JsonValueKind.Null)
                return v.Type.IsValueType && Nullable.GetUnderlyingType(v.Type) is null ? Activator.CreateInstance(v.Type) : null;
            return e.Deserialize(v.Type, CelobusJson.Options);
        }
    }
}
