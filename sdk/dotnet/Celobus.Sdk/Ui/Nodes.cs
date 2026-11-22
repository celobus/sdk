// The catalog's primitives (revision 1, and of revision 2 the charts and figures, `VizNode`, made by `Viz.*`, and the
// dashboard, `BoardNode`):
// structure, navigation, data, input, actions and feedback. `Ui.*` makes them;
// a primitive with options of its own is a typed node (`ButtonNode.Danger()`, `TableNode<Order>.Money(..)`), and every
// node takes `.Visible(cond)`, `.Enabled(cond)` and `.Fallback(node)`.
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>A primitive with the common options (<see cref="NodeExtensions"/>).</summary>
public class Node : IUiJson
{
    internal UiObj Outer { get; }
    internal UiObj Body { get; }

    internal Node(string kind) { (Outer, Body) = UiObj.Tagged(kind); }

    JsonNode IUiJson.ToJson(UiWriter w) => Outer.ToJson(w);
}

/// <summary>The options every primitive takes.</summary>
public static class NodeExtensions
{
    /// <summary>Shown only when the condition holds (presentation, never access control).</summary>
    public static TNode Visible<TNode>(this TNode node, Cond when) where TNode : Node
    {
        node.Outer["visible"] = when;
        return node;
    }

    public static TNode Enabled<TNode>(this TNode node, Cond when) where TNode : Node
    {
        node.Outer["enabled"] = when;
        return node;
    }

    /// <summary>What an older client draws instead (display primitives only).</summary>
    public static TNode Fallback<TNode>(this TNode node, Node fallback) where TNode : Node
    {
        node.Outer["fallback"] = fallback;
        return node;
    }
}

/// <summary>A field item, a metric, a text, a chart or a card's trailing value: what takes a format (<see cref="Formats"/>).</summary>
public interface IFormatted;

/// <summary>Formats of a field item, a metric, a text, a chart or a card's trailing value (normative semantics in the proposal §4.6).</summary>
public static class Formats
{
    internal static UiObj Of(string kind, UiObj? body = null) => new() { [kind] = body ?? new UiObj() };

    internal static UiObj Money(string? currency, string? code) => Of("money", new UiObj { ["currency"] = currency, ["code"] = code });

    internal static UiObj StatusOf(UiObj tones) => Of("status", new UiObj { ["tones"] = tones });

    private static T Set<T>(T x, UiObj format) where T : IFormatted
    {
        switch (x)
        {
            case FieldItem f: f.Obj["format"] = format; break;
            case CardNode c: c.Body["trailing_format"] = format; break;
            case Node n: n.Body["format"] = format; break;
        }
        return x;
    }

    /// <summary>A decimal amount in this ISO 4217 currency.</summary>
    public static T MoneyIn<T>(this T x, string code) where T : IFormatted => Set(x, Money(null, code));

    public static T Number<T>(this T x) where T : IFormatted => Set(x, Of("number"));

    public static T Percent<T>(this T x) where T : IFormatted => Set(x, Of("percent"));

    public static T Date<T>(this T x) where T : IFormatted => Set(x, Of("date"));

    public static T Datetime<T>(this T x) where T : IFormatted => Set(x, Of("datetime"));

    public static T Relative<T>(this T x) where T : IFormatted => Set(x, Of("relative"));

    public static T Member<T>(this T x) where T : IFormatted => Set(x, Of("member"));

    /// <summary>Counts up, live, from a moment in ms (<c>0:48</c>, <c>12:05</c>, <c>1:02:05</c>; catalog 3).</summary>
    public static T Elapsed<T>(this T x) where T : IFormatted => Set(x, Of("elapsed"));

    /// <summary>Counts down, live, to a moment in ms (<c>8 min</c>, <c>&lt;1 min</c>; nothing once past; catalog 3).</summary>
    public static T Countdown<T>(this T x) where T : IFormatted => Set(x, Of("countdown"));

    /// <summary>A duration in ms (<c>&lt;1s</c>, <c>6s</c>, <c>1m 05s</c>, <c>1h 02m</c>; catalog 3).</summary>
    public static T Span<T>(this T x) where T : IFormatted => Set(x, Of("span"));

    /// <summary>An enum value as a status, with a tone per value name.</summary>
    public static T Status<T>(this T x, params (string Value, Tone Tone)[] tones) where T : IFormatted
    {
        var o = new UiObj();
        foreach (var (value, tone) in tones) o[value] = UiJson.Name(tone);
        return Set(x, StatusOf(o));
    }
}

/// <summary>A stack (vertical) or a row (horizontal).</summary>
public sealed class StackNode : Node
{
    internal StackNode(string direction, Node[] children) : base("stack")
    {
        Body["direction"] = direction;
        Body["gap"] = "normal";
        Body.List("children").AddRange(children);
    }

    public StackNode Gap(Gap gap)
    {
        Body["gap"] = UiJson.Name(gap);
        return this;
    }

    /// <summary>A horizontal stack's children spread to both ends.</summary>
    public StackNode Between()
    {
        Body["justify"] = "between";
        return this;
    }
}

/// <summary>Columns that stack on a phone.</summary>
public sealed class ColumnsNode : Node
{
    internal ColumnsNode(Node[] children) : base("columns")
    {
        Body.List("children").AddRange(children);
    }

    /// <summary>Relative widths (<c>.Ratios(2, 1)</c>); equal when not set.</summary>
    public ColumnsNode Ratios(params int[] ratios)
    {
        Body.List("ratios", true).AddRange(ratios.Cast<object>());
        return this;
    }
}

/// <summary>A titled section with optional description, head buttons and footer.</summary>
public sealed class SectionNode : Node
{
    internal SectionNode(Txt title, Node[] children) : base("section")
    {
        Body["title"] = title;
        Body.List("children").AddRange(children);
    }

    public SectionNode Description(Txt description)
    {
        Body["description"] = description;
        return this;
    }

    /// <summary>Buttons in the section's head.</summary>
    public SectionNode Actions(params Node[] actions)
    {
        Body.List("actions", true).AddRange(actions);
        return this;
    }

    public SectionNode Footer(params Node[] footer)
    {
        Body.List("footer", true).AddRange(footer);
        return this;
    }
}

/// <summary>A card; a titled card can have a value at the end of its title line (its format: <c>.MoneyIn(..)</c> and the like).</summary>
public sealed class CardNode : Node, IFormatted
{
    internal CardNode(Node[] children) : base("card")
    {
        Body.List("children").AddRange(children);
    }

    public CardNode Title(Txt title)
    {
        Body["title"] = title;
        return this;
    }

    /// <summary>A value at the end of the title line (a total).</summary>
    public CardNode Trailing<T>(Val<T> value)
    {
        Body["trailing"] = Val<T>.Json(value);
        return this;
    }
}

/// <summary>One tab of <see cref="Ui.Tabs"/>: its id, title and content.</summary>
public sealed class Tab : IUiJson
{
    private readonly UiObj _o = new();

    internal Tab(string id, Txt title, Node[] children)
    {
        _o["id"] = id;
        _o["title"] = title;
        _o.List("children").AddRange(children);
    }

    /// <summary>A count shown on the tab.</summary>
    public Tab Badge<T>(Val<T> value)
    {
        _o["badge"] = value;
        return this;
    }

    JsonNode IUiJson.ToJson(UiWriter w) => _o.ToJson(w);
}

/// <summary>Tabs (<see cref="Ui.Tabs"/>).</summary>
public sealed class TabsNode : Node
{
    internal TabsNode(Tab[] tabs) : base("tabs")
    {
        Body.List("tabs").AddRange(tabs);
    }

    /// <summary>The selected tab's id is this screen param (so it is in the link, and queries can read it).</summary>
    public TabsNode BindParam<T>(ParamRef<T> param)
    {
        Body["bind"] = new JsonObject { ["param"] = param.Name };
        return this;
    }

    /// <summary>The selected tab's id is this state.</summary>
    public TabsNode BindState<T>(StateRef<T> state)
    {
        Body["bind"] = new JsonObject { ["state"] = state.Name };
        return this;
    }
}

/// <summary>A table over a query's rows; columns and row values are fields of the row message.</summary>
public sealed class TableNode<TRow> : Node
{
    private readonly List<object> _columns;
    private readonly List<object> _rowActions;
    private readonly List<object> _rowTone;

    internal TableNode(string query) : base("table")
    {
        Body["query"] = query;
        _columns = Body.List("columns");
        _rowActions = Body.List("row_actions", true);
        _rowTone = Body.List("row_tone", true);
    }

    private TableNode<TRow> Column<T>(Expression<Func<TRow, T>> field, Txt title, UiObj? format, bool end = false)
    {
        var c = new UiObj { ["field"] = FieldPath.Of(field), ["title"] = title, ["format"] = format };
        if (end) c["align"] = "end";
        _columns.Add(c);
        return this;
    }

    public TableNode<TRow> Col<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, null);

    public TableNode<TRow> Mono<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("mono"));

    public TableNode<TRow> Number<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("number"), true);

    /// <summary>A decimal amount in the currency of another field of the row.</summary>
    public TableNode<TRow> Money<T>(Expression<Func<TRow, T>> field, Txt title, Expression<Func<TRow, string>> currency) =>
        Column(field, title, Formats.Money(FieldPath.Of(currency), null), true);

    /// <summary>A decimal amount in this ISO 4217 currency.</summary>
    public TableNode<TRow> MoneyIn<T>(Expression<Func<TRow, T>> field, Txt title, string code) =>
        Column(field, title, Formats.Money(null, code), true);

    public TableNode<TRow> Date<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("date"));

    public TableNode<TRow> Datetime<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("datetime"));

    public TableNode<TRow> Relative<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("relative"));

    public TableNode<TRow> Member<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("member"));

    public TableNode<TRow> Avatar<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("avatar"));

    public TableNode<TRow> Tags<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("tags"));

    public TableNode<TRow> Percent<T>(Expression<Func<TRow, T>> field, Txt title) => Column(field, title, Formats.Of("percent"));

    /// <summary>An enum column drawn as a status with a tone per value.</summary>
    public TableNode<TRow> Status<TEnum>(Expression<Func<TRow, TEnum>> field, Txt title, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => Column(field, title, Formats.StatusOf(UiJson.Tones(tones)));

    /// <inheritdoc cref="Status{TEnum}(Expression{Func{TRow, TEnum}}, Txt, ValueTuple{TEnum, Tone}[])"/>
    public TableNode<TRow> Status<TEnum>(Expression<Func<TRow, TEnum?>> field, Txt title, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => Column(field, title, Formats.StatusOf(UiJson.Tones(tones)));

    /// <summary>The action run when a row is opened.</summary>
    public TableNode<TRow> Open(ActionRef action)
    {
        Body["open"] = action.Name;
        return this;
    }

    /// <summary>A button (or a menu) on every row.</summary>
    public TableNode<TRow> RowButton(Func<Row<TRow>, Node> button)
    {
        _rowActions.Add(button(new Row<TRow>()));
        return this;
    }

    /// <summary>Buttons, search and filters above the table.</summary>
    public TableNode<TRow> Toolbar(params Node[] nodes)
    {
        Body.List("toolbar", true).AddRange(nodes);
        return this;
    }

    public TableNode<TRow> Empty(Txt text)
    {
        Body["empty"] = text;
        return this;
    }

    /// <summary>The selected row's <paramref name="key"/> goes into <paramref name="state"/>.</summary>
    public TableNode<TRow> Select<TKey>(StateRef<TKey> state, Expression<Func<TRow, TKey>> key)
    {
        Body["select"] = new JsonObject { ["state"] = state.Name, ["key"] = FieldPath.Of(key) };
        return this;
    }

    /// <summary>A tone for the rows that match (first match wins).</summary>
    public TableNode<TRow> RowTone(Func<Row<TRow>, Cond> when, Tone tone)
    {
        _rowTone.Add(new UiObj { ["when"] = when(new Row<TRow>()), ["tone"] = UiJson.Name(tone) });
        return this;
    }

    private TableNode<TRow> LastColumn(string key, string value)
    {
        if (_columns.Count > 0) ((UiObj)_columns[^1])[key] = value;
        return this;
    }

    /// <summary>The last column gets a second line from <paramref name="field"/>.</summary>
    public TableNode<TRow> SecondLine<T>(Expression<Func<TRow, T>> field) => LastColumn("subtitle", FieldPath.Of(field));

    /// <summary>
    /// The last column sorts the list by the request's string field <paramref name="by"/>: a click on its head passes
    /// <c>asc</c>, the next <c>desc</c>, the third leaves the field out. One column sorts at a time.
    /// </summary>
    public TableNode<TRow> Sortable<TRequest, T>(Expression<Func<TRequest, T>> by) => LastColumn("sort", FieldPath.Of(by));

    /// <summary>The last column gets an avatar: a picture URL from <paramref name="field"/>, else the value's initials.</summary>
    public TableNode<TRow> WithAvatar<T>(Expression<Func<TRow, T>> field) => LastColumn("avatar", FieldPath.Of(field));

    /// <summary>Rows can be checked; <paramref name="state"/> (a list) holds the checked rows' <paramref name="key"/>s.</summary>
    public TableNode<TRow> Checks<TList, TKey>(StateRef<TList> state, Expression<Func<TRow, TKey>> key) where TList : IEnumerable<TKey>
    {
        Body["checks"] = new JsonObject { ["state"] = state.Name, ["key"] = FieldPath.Of(key) };
        return this;
    }

    /// <summary>How many rows there are in all (<c>Showing 1–10 of 1,248</c>).</summary>
    public TableNode<TRow> Count<T>(Val<T> count)
    {
        Body["count"] = Val<T>.Json(count);
        return this;
    }

    /// <summary>The line over the compact rows (<c>412 of 1,248 orders · newest first</c>); the count alone when absent.</summary>
    public TableNode<TRow> CountLabel(Txt label)
    {
        Body["count_label"] = label;
        return this;
    }

    /// <summary>The row as a list item where a table does not fit (<c>r =&gt; r.View(r.Get(o =&gt; o.Customer)).Subtitle(..)</c>).</summary>
    public TableNode<TRow> Compact(Func<Row<TRow>, RowView<TRow>> view)
    {
        Body["compact"] = view(new Row<TRow>());
        return this;
    }

    /// <summary>The row whose <paramref name="key"/> is in this param is drawn selected (a record panel next to the list).</summary>
    public TableNode<TRow> SelectedBy<TP, TKey>(ParamRef<TP> param, Expression<Func<TRow, TKey>> key)
    {
        Body["select"] = new JsonObject { ["state"] = param.Name, ["key"] = FieldPath.Of(key) };
        return this;
    }
}

/// <summary>A row drawn as a list item (<see cref="Row{TRow}.View"/>): title, subtitle, an amount and a status at the end, an avatar or icon at the start.</summary>
public sealed class RowView<TRow> : IUiJson
{
    private readonly UiObj _o = new();

    internal RowView(Txt title) { _o["title"] = title; }

    public RowView<TRow> Subtitle(Txt subtitle)
    {
        _o["subtitle"] = subtitle;
        return this;
    }

    public RowView<TRow> Meta<T>(Val<T> meta)
    {
        _o["meta"] = Val<T>.Json(meta);
        return this;
    }

    /// <summary>The meta is money in the row's currency field.</summary>
    public RowView<TRow> Money<TC>(Expression<Func<TRow, TC>> currency)
    {
        _o["meta_format"] = Formats.Money(FieldPath.Of(currency), null);
        return this;
    }

    /// <summary>A row field: a picture URL, else initials of the title.</summary>
    public RowView<TRow> Avatar<T>(Expression<Func<TRow, T>> field)
    {
        _o["avatar"] = FieldPath.Of(field);
        return this;
    }

    public RowView<TRow> Icon(string icon)
    {
        _o["icon"] = icon;
        return this;
    }

    public RowView<TRow> Status<TEnum>(Expression<Func<TRow, TEnum>> field, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => SetStatus(FieldPath.Of(field), tones);

    public RowView<TRow> Status<TEnum>(Expression<Func<TRow, TEnum?>> field, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => SetStatus(FieldPath.Of(field), tones);

    private RowView<TRow> SetStatus<TEnum>(string field, (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum
    {
        var o = new UiObj { ["field"] = field };
        if (tones.Length > 0) o["tones"] = UiJson.Tones(tones);
        _o["status"] = o;
        return this;
    }

    JsonNode IUiJson.ToJson(UiWriter w) => _o.ToJson(w);
}

/// <summary>A list over a query's rows: a title per row, optionally a subtitle, meta, avatar, icon, status.</summary>
public sealed class ListNode<TRow> : Node
{
    internal ListNode(string query, string title) : base("list")
    {
        Body["query"] = query;
        Body["title"] = title;
    }

    public ListNode<TRow> Subtitle<T>(Expression<Func<TRow, T>> field)
    {
        Body["subtitle"] = FieldPath.Of(field);
        return this;
    }

    public ListNode<TRow> Meta<T>(Expression<Func<TRow, T>> field)
    {
        Body["meta"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>A time as the row's meta, shown relative (<c>3 min ago</c>).</summary>
    public ListNode<TRow> MetaRelative<T>(Expression<Func<TRow, T>> field)
    {
        Body["meta"] = FieldPath.Of(field);
        Body["meta_format"] = Formats.Of("relative");
        return this;
    }

    public ListNode<TRow> Avatar<T>(Expression<Func<TRow, T>> field)
    {
        Body["avatar"] = FieldPath.Of(field);
        return this;
    }

    public ListNode<TRow> Icon(string icon)
    {
        Body["icon"] = icon;
        return this;
    }

    /// <summary>A row field naming the row's icon.</summary>
    public ListNode<TRow> IconField<T>(Expression<Func<TRow, T>> field)
    {
        Body["icon_field"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>A quiet text before the meta (<c>×2</c>).</summary>
    public ListNode<TRow> Aside(Txt aside)
    {
        Body["aside"] = aside;
        return this;
    }

    /// <summary>The meta is money in this ISO 4217 currency.</summary>
    public ListNode<TRow> MoneyIn(string code)
    {
        Body["meta_format"] = Formats.Money(null, code);
        return this;
    }

    public ListNode<TRow> Status<TEnum>(Expression<Func<TRow, TEnum>> field, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => SetStatus(FieldPath.Of(field), UiJson.Tones(tones));

    public ListNode<TRow> Status<TEnum>(Expression<Func<TRow, TEnum?>> field, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum => SetStatus(FieldPath.Of(field), UiJson.Tones(tones));

    private ListNode<TRow> SetStatus(string field, UiObj tones)
    {
        Body["status"] = new UiObj { ["field"] = field, ["tones"] = tones };
        return this;
    }

    public ListNode<TRow> Open(ActionRef action)
    {
        Body["open"] = action.Name;
        return this;
    }

    public ListNode<TRow> Empty(Txt text)
    {
        Body["empty"] = text;
        return this;
    }

    /// <summary>A filter box over the rows' titles with this placeholder; it filters the loaded rows, no request (catalog 3).</summary>
    public ListNode<TRow> Search(Txt placeholder)
    {
        Body["search"] = placeholder;
        return this;
    }

    /// <summary>A bool row field: a green dot after the title (catalog 3).</summary>
    public ListNode<TRow> Online(Expression<Func<TRow, bool>> field)
    {
        Body["online"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>On a phone, the first <paramref name="rows"/> rows (1 to 100), then <c>Show all N</c> (catalog 3).</summary>
    public ListNode<TRow> Limit(int rows)
    {
        Body["limit"] = rows;
        return this;
    }

    /// <summary>On a phone, a label over the rows in a card (<c>People · 9</c>; catalog 3).</summary>
    public ListNode<TRow> GroupLabel(Txt label)
    {
        Body["group_label"] = label;
        return this;
    }
}

/// <summary>Label/value pairs of one record.</summary>
public sealed class FieldsNode : Node
{
    internal FieldsNode(FieldItem[] items) : base("fields")
    {
        Body.List("items").AddRange(items);
    }

    public FieldsNode Title(Txt title)
    {
        Body["title"] = title;
        return this;
    }
}

/// <summary>One label/value pair of <see cref="Ui.Fields"/> (<c>Ui.Item("Total", order.Field(o =&gt; o.Total)).MoneyIn("USD")</c>).</summary>
public sealed class FieldItem : IUiJson, IFormatted
{
    internal UiObj Obj { get; } = new();

    internal FieldItem(Txt label, JsonNode value)
    {
        Obj["label"] = label;
        Obj["value"] = value;
    }

    /// <summary>The value links to a screen (a chip with an arrow).</summary>
    public FieldItem Link<TParams>(ScreenKey<TParams> screen, Action<Args<TParams>>? args = null) where TParams : class
    {
        Obj["open"] = Target.Of(screen.Id, null, Args<TParams>.Build(args));
        return this;
    }

    /// <summary>The value drawn as a chip with initials.</summary>
    public FieldItem Chip()
    {
        Obj["format"] = Formats.Of("avatar");
        return this;
    }

    /// <summary>The chip's picture URL or initials.</summary>
    public FieldItem ChipAvatar<T>(Val<T> avatar)
    {
        Obj["format"] = Formats.Of("avatar");
        Obj["avatar"] = Val<T>.Json(avatar);
        return this;
    }

    JsonNode IUiJson.ToJson(UiWriter w) => Obj.ToJson(w);
}

public sealed class MetricNode : Node, IFormatted
{
    internal MetricNode(Txt title, JsonNode value) : base("metric")
    {
        Body["title"] = title;
        Body["value"] = value;
    }

    /// <summary>A change shown next to the value, toned up or down.</summary>
    public MetricNode Delta<T>(Val<T> delta)
    {
        Body["delta"] = delta;
        return this;
    }

    public MetricNode Caption(Txt caption)
    {
        Body["caption"] = caption;
        return this;
    }

    /// <summary>The caption's tone and icon (<c>$33.9k overdue</c> in red with an alert icon).</summary>
    public MetricNode CaptionTone(Tone tone, string icon)
    {
        Body["caption_tone"] = UiJson.Name(tone);
        Body["caption_icon"] = icon;
        return this;
    }
}

/// <summary>
/// A chart over a query's rows: the x field and the series. The first charts (catalog 1), kept for compatibility: new
/// screens use <see cref="Viz"/> (<c>Viz.Bar.Column()</c>, <c>Viz.Line.Line()</c>, …), which has every kind of the
/// charts catalogue.
/// </summary>
public sealed class ChartNode<TRow> : Node, IFormatted
{
    private readonly List<object> _series;

    internal ChartNode(string kind, string query, string x) : base("chart")
    {
        Body["kind"] = kind;
        Body["query"] = query;
        Body["x"] = x;
        _series = Body.List("series");
    }

    public ChartNode<TRow> Series<T>(Expression<Func<TRow, T>> field, Txt label)
    {
        _series.Add(new UiObj { ["field"] = FieldPath.Of(field), ["label"] = label });
        return this;
    }

    public ChartNode<TRow> Title(Txt title)
    {
        Body["title"] = title;
        return this;
    }
}

/// <summary>
/// A dashboard (catalog revision 2): cards on one shared 12-column grid:
/// <c>Ui.Board().Quarter(revenue).Quarter(orders).Half(target).Full(Ui.Heading("Sales")).Span(8, line).Third(donut)</c>.
/// The items flow left to right and wrap when a row is full; every row is as tall as its tallest card, and a narrower
/// screen widens the cards. A chart is placed on the board itself (it draws its own card). A board is not put inside
/// another board, and holds at most 60 items. An older client (catalog 1) draws the board's <c>.Fallback(..)</c>.
/// </summary>
public sealed class BoardNode : Node
{
    private readonly List<object> _items;

    internal BoardNode() : base("board") { _items = Body.List("children"); }

    /// <summary>An item <paramref name="columns"/> wide, of the grid's 12.</summary>
    public BoardNode Span(int columns, Node node)
    {
        var item = new UiObj();
        if (columns != 12) item["span"] = columns;
        item["node"] = node;
        _items.Add(item);
        return this;
    }

    /// <summary>An item of the full width: a wide chart, a table, a heading between groups.</summary>
    public BoardNode Full(Node node) => Span(12, node);

    /// <summary>An item of half the width (6 columns).</summary>
    public BoardNode Half(Node node) => Span(6, node);

    /// <summary>An item of a third of the width (4 columns).</summary>
    public BoardNode Third(Node node) => Span(4, node);

    /// <summary>An item of a quarter of the width (3 columns): a row of four metrics.</summary>
    public BoardNode Quarter(Node node) => Span(3, node);
}

/// <summary>
/// A chart or a figure of the charts catalogue (catalog revision 2): a kind of a family (<see cref="Viz"/>), its static
/// options and the data bound into them from the screen's queries:
/// <c>Viz.Line.Line().Title("Revenue").Option("fmt", "money").Dates(sales, d =&gt; d.Day).Values(sales, d =&gt; d.Total)</c>.
/// The data methods add a bind each, in order; <c>To</c>, <c>AsText</c>, <c>AsDate</c> and <c>TextKeys</c> change the
/// bind just added. A chart draws its own card: place it on a <see cref="BoardNode"/>, in columns or in a stack, not
/// next to other children of a card or a section. An older client (catalog 1) draws the node's <c>.Fallback(..)</c>.
/// </summary>
public sealed class VizNode : Node
{
    private readonly List<object> _bind;
    private JsonObject? _options;
    // the bind just added: what `To`, `AsText`, `AsDate`, `TextKeys` and the next `Serie` change
    private UiObj? _last;
    private string? _lastKind;
    private UiObj? _lastSource;
    private string? _lastQuery;
    private List<object>? _lastList;

    internal VizNode(string family, string kind) : base("viz")
    {
        Body["family"] = family;
        Body["kind"] = kind;
        _bind = Body.List("bind", omitEmpty: true);
    }

    public VizNode Title(Txt title)
    {
        Body["title"] = title;
        return this;
    }

    /// <summary>The muted text at the right of the head (<c>Last 30 days</c>).</summary>
    public VizNode Range(Txt range)
    {
        Body["range"] = range;
        return this;
    }

    /// <summary>The height in px (16 to 1200); the kind's own height when not set.</summary>
    public VizNode Height(int px)
    {
        Body["height"] = px;
        return this;
    }

    /// <summary>A static option of the kind: <c>.Option("fmt", "money")</c>, <c>.Option("target", 120)</c>,
    /// <c>.Option("bands", new JsonArray(50, 80))</c>.</summary>
    public VizNode Option(string name, JsonNode? value)
    {
        if (_options is null) Body["options"] = _options = new JsonObject();
        _options[name] = value?.DeepClone();
        return this;
    }

    /// <summary>Several static options at once: <c>.Options(new JsonObject { ["fmt"] = "money", ["curve"] = "smooth" })</c>.</summary>
    public VizNode Options(JsonObject options)
    {
        foreach (var (name, value) in options) Option(name, value);
        return this;
    }

    private VizNode Add(string to, string kind, object source, string? query = null, List<object>? list = null)
    {
        _last = new UiObj { ["to"] = to, ["from"] = new UiObj { [kind] = source } };
        _bind.Add(_last);
        (_lastKind, _lastSource, _lastQuery, _lastList) = (kind, source as UiObj, query, list);
        return this;
    }

    /// <summary>A bind written by hand: <paramref name="from"/> is the source with its one key
    /// (<c>{"column": {"query": "sales", "field": "total"}}</c>); the typed methods below cover the sources.</summary>
    public VizNode Bind(string to, JsonObject from)
    {
        _last = new UiObj { ["to"] = to, ["from"] = from.DeepClone() };
        _bind.Add(_last);
        (_lastKind, _lastSource, _lastQuery, _lastList) = (null, null, null, null);
        return this;
    }

    /// <summary>A row field of every row, as an array in the option <paramref name="to"/>. Numbers and numeric strings
    /// become numbers; <c>.AsText()</c> keeps strings, <c>.AsDate()</c> makes short date labels.</summary>
    public VizNode Column<TRes, TRow, T>(string to, Q<TRes, TRow> rows, Expression<Func<TRow, T>> field) =>
        Add(to, "column", new UiObj { ["query"] = rows.Name, ["field"] = FieldPath.Of(field) });

    /// <summary>The <c>labels</c> (the x positions or the row names) from a row field, kept as text.</summary>
    public VizNode Labels<TRes, TRow, T>(Q<TRes, TRow> rows, Expression<Func<TRow, T>> field) =>
        Column("labels", rows, field).AsText();

    /// <summary>The <c>labels</c> from a date or timestamp field, as short dates (<c>Nov 4</c>).</summary>
    public VizNode Dates<TRes, TRow, T>(Q<TRes, TRow> rows, Expression<Func<TRow, T>> field) =>
        Column("labels", rows, field).AsDate();

    /// <summary>The <c>values</c> (one series) from a row field.</summary>
    public VizNode Values<TRes, TRow, T>(Q<TRes, TRow> rows, Expression<Func<TRow, T>> field) =>
        Column("values", rows, field);

    private VizNode SeriesOf(string query, IEnumerable<(string Field, Txt Label)> fields)
    {
        if (!(_lastKind == "series" && _lastQuery == query && (string?)_last!["to"] == "series"))
        {
            var source = new UiObj { ["query"] = query };
            Add("series", "series", source, query, source.List("fields"));
        }
        _lastList!.AddRange(fields.Select(f => (object)new UiObj { ["field"] = f.Field, ["label"] = f.Label }));
        return this;
    }

    /// <summary>One more series of the option <c>series</c>: the row field with its name in the legend. Calls one
    /// after another over the same query (also after <c>Series</c>) make one bind.</summary>
    public VizNode Serie<TRes, TRow, T>(Q<TRes, TRow> rows, Expression<Func<TRow, T>> field, Txt label) =>
        SeriesOf(rows.Name, [(FieldPath.Of(field), label)]);

    /// <summary>The option <c>series</c>, one series per row field:
    /// <c>.Series(sales, (d =&gt; d.Online, "Online"), (d =&gt; d.Retail, "Retail"))</c>.</summary>
    public VizNode Series<TRes, TRow>(Q<TRes, TRow> rows, params (Expression<Func<TRow, object?>> Field, Txt Label)[] fields)
    {
        // a bind of its own, whatever came before it
        _lastKind = null;
        return SeriesOf(rows.Name, fields.Select(f => (FieldPath.Of(f.Field), f.Label)));
    }

    /// <summary>Long rows to wide: <c>labels</c> gets the distinct values of <paramref name="x"/> in row order, the
    /// option <c>series</c> one series per distinct value of <paramref name="series"/>, with
    /// <paramref name="value"/> in its cells (<c>.AsDate()</c> makes the labels short dates).</summary>
    public VizNode Pivot<TRes, TRow, TX, TS, TV>(Q<TRes, TRow> rows, Expression<Func<TRow, TX>> x,
        Expression<Func<TRow, TS>> series, Expression<Func<TRow, TV>> value) =>
        Add("series", "pivot", new UiObj
        {
            ["query"] = rows.Name,
            ["x"] = FieldPath.Of(x),
            ["series"] = FieldPath.Of(series),
            ["value"] = FieldPath.Of(value),
        });

    /// <summary>The option <c>rows</c>: an object per row with the keys given, each from a row field:
    /// <c>.Rows(top, ("name", p =&gt; p.Title), ("value", p =&gt; p.Revenue))</c>. Numeric strings become numbers except
    /// under the keys named by <c>.TextKeys(..)</c>.</summary>
    public VizNode Rows<TRes, TRow>(Q<TRes, TRow> rows, params (string Key, Expression<Func<TRow, object?>> Field)[] map)
    {
        var keys = new UiObj();
        foreach (var (key, field) in map) keys[key] = FieldPath.Of(field);
        var source = new UiObj { ["query"] = rows.Name, ["map"] = keys };
        return Add("rows", "rows", source, rows.Name, source.List("text", omitEmpty: true));
    }

    /// <summary>The keys of the <c>rows</c> bind just added that stay text (a product named <c>2024</c> is not a number).</summary>
    public VizNode TextKeys(params string[] keys)
    {
        if (_lastKind == "rows") _lastList!.AddRange(keys);
        return this;
    }

    /// <summary>One value in the option <paramref name="to"/> (a name or a dotted path: <c>center.value</c>): a
    /// query's field (<c>summary.Field(r =&gt; r.Revenue)</c>), a state or a param.</summary>
    public VizNode Value<T>(string to, Val<T> value) => Add(to, "value", Val<T>.Json(value));

    /// <summary>A literal in the option <paramref name="to"/>: <c>.Value("target", 120)</c>, <c>.Value("unit", "orders")</c>.</summary>
    public VizNode Value(string to, JsonNode? literal) => Add(to, "value", UiJson.One("lit", literal?.DeepClone()));

    /// <summary>Another option for the bind just added: <c>.Rows(q, ..).To("parts")</c>, <c>.Series(q, ..).To("groups")</c>.</summary>
    public VizNode To(string to)
    {
        if (_last is not null) _last["to"] = to;
        return this;
    }

    private VizNode Cast(string to)
    {
        if (_lastKind is "column" or "pivot") _lastSource!["as"] = to;
        return this;
    }

    /// <summary>The <c>column</c> just added keeps its cells as text (the labels of a <c>pivot</c> too).</summary>
    public VizNode AsText() => Cast("text");

    /// <summary>The <c>column</c> just added (or the labels of the <c>pivot</c>) holds ISO dates or datetimes: they
    /// become short labels (<c>Nov 4</c>, with the year when the rows span more than one).</summary>
    public VizNode AsDate() => Cast("date");

    /// <summary>The state that receives what the member picks in the chart (a bar, a slice, a row): the picked value,
    /// or null when nothing is picked. Queries and conditions of the screen read it like any state.</summary>
    public VizNode SelectState<T>(StateRef<T> state)
    {
        Body["select"] = new JsonObject { ["state"] = state.Name };
        return this;
    }

    /// <summary>The same, into a param of the screen (it is in the URL).</summary>
    public VizNode SelectParam<T>(ParamRef<T> param)
    {
        Body["select"] = new JsonObject { ["param"] = param.Name };
        return this;
    }

    /// <summary>The screen the card's link leads to, with its params like <c>Ui.Link</c> (a literal, a param, a state): <c>.Open(Screens.Orders, a =&gt; a.Arg(p =&gt; p.Status, OrderStatus.Paid))</c>.</summary>
    public VizNode Open<TParams>(ScreenKey<TParams> screen, Action<Args<TParams>>? args = null) where TParams : class
    {
        Body["open"] = Target.Of(screen.Id, null, Args<TParams>.Build(args));
        return this;
    }
}

/// <summary>A timeline over a query's rows: text, time and optionally the actor and a tone.</summary>
public sealed class TimelineNode<TRow> : Node
{
    internal TimelineNode(string query, string text, string time) : base("timeline")
    {
        Body["query"] = query;
        Body["text"] = text;
        Body["time"] = time;
    }

    public TimelineNode<TRow> Actor<T>(Expression<Func<TRow, T>> field)
    {
        Body["actor"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>The actor's picture URL (else initials).</summary>
    public TimelineNode<TRow> Avatar<T>(Expression<Func<TRow, T>> field)
    {
        Body["avatar"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>A row field with an icon name, for actors without a face (an agent, a plugin).</summary>
    public TimelineNode<TRow> Icon<T>(Expression<Func<TRow, T>> field)
    {
        Body["icon"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>Times as <c>2 min</c>, <c>40 min</c>, <c>2 h</c>.</summary>
    public TimelineNode<TRow> ShortTime()
    {
        Body["time_format"] = Formats.Of("relative", new UiObj { ["short"] = true });
        return this;
    }

    public TimelineNode<TRow> Tone<TEnum>(Expression<Func<TRow, TEnum>> field, params (TEnum Value, Tone Tone)[] tones)
        where TEnum : struct, Enum
    {
        Body["tone"] = new UiObj { ["field"] = FieldPath.Of(field), ["tones"] = UiJson.Tones(tones) };
        return this;
    }
}

/// <summary>
/// A conversation (<see cref="Ui.Conversation"/>): its messages oldest first, each with its author, a receipt and a
/// card; a message box under them.
/// </summary>
public sealed class ConversationNode<TRow> : Node
{
    internal ConversationNode(string query, string author, string text, string time) : base("conversation")
    {
        Body["query"] = query;
        Body["author"] = author;
        Body["text"] = text;
        Body["time"] = time;
    }

    private UiObj Card()
    {
        if (Body["card"] is UiObj o) return o;
        var n = new UiObj { ["title"] = "" };
        Body["card"] = n;
        return n;
    }

    /// <summary>The author's picture URL (else their initials).</summary>
    public ConversationNode<TRow> Avatar<T>(Expression<Func<TRow, T>> field) { Body["avatar"] = FieldPath.Of(field); return this; }

    /// <summary>An icon name for authors without a face (an agent: <c>bot</c>).</summary>
    public ConversationNode<TRow> Icon<T>(Expression<Func<TRow, T>> field) { Body["icon"] = FieldPath.Of(field); return this; }

    /// <summary>A small chip after the author (<c>AGENT</c>).</summary>
    public ConversationNode<TRow> Badge<T>(Expression<Func<TRow, T>> field) { Body["badge"] = FieldPath.Of(field); return this; }

    /// <summary>A quiet line after the time (<c>via Commerce</c>).</summary>
    public ConversationNode<TRow> Via<T>(Expression<Func<TRow, T>> field) { Body["via"] = FieldPath.Of(field); return this; }

    /// <summary>A chip before the text (<c>To people</c>).</summary>
    public ConversationNode<TRow> Tag<T>(Expression<Func<TRow, T>> field) { Body["tag"] = FieldPath.Of(field); return this; }

    /// <summary>The receipt under a message (<c>Applied · step 3</c>), toned by an enum field.</summary>
    public ConversationNode<TRow> Receipt<T, TEnum>(Expression<Func<TRow, T>> text, Expression<Func<TRow, TEnum>> tone,
        params (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum
    {
        Body["receipt"] = new UiObj
        {
            ["text"] = FieldPath.Of(text),
            ["tone"] = new UiObj { ["field"] = FieldPath.Of(tone), ["tones"] = UiJson.Tones(tones) },
        };
        return this;
    }

    /// <summary>A card under the message, shown while its title is not empty, with its text.</summary>
    public ConversationNode<TRow> Card<TA, TB>(Expression<Func<TRow, TA>> title, Expression<Func<TRow, TB>> text)
    {
        var c = Card();
        c["title"] = FieldPath.Of(title);
        c["text"] = FieldPath.Of(text);
        return this;
    }

    /// <summary>The card's band and its aside (<c>by 11:30</c>), toned by an enum field.</summary>
    public ConversationNode<TRow> CardBand<TA, TB, TEnum>(Expression<Func<TRow, TA>> band, Expression<Func<TRow, TB>> aside,
        Expression<Func<TRow, TEnum>> tone, params (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum
    {
        var c = Card();
        c["band"] = FieldPath.Of(band);
        c["aside"] = FieldPath.Of(aside);
        c["tone"] = new UiObj { ["field"] = FieldPath.Of(tone), ["tones"] = UiJson.Tones(tones) };
        return this;
    }

    /// <summary>The card's footer line (<c>14 replies · last 10:58</c>).</summary>
    public ConversationNode<TRow> CardFooter<T>(Expression<Func<TRow, T>> field) { Card()["footer"] = FieldPath.Of(field); return this; }

    /// <summary>The card's button: its label is a row field (hidden while empty), the action runs on the row.</summary>
    public ConversationNode<TRow> CardButton<T>(Expression<Func<TRow, T>> label, ActionRef action)
    {
        var c = Card();
        c["button"] = FieldPath.Of(label);
        c["action"] = action.Name;
        return this;
    }

    /// <summary>The message box; its action reads <c>Ui.FormValue</c> of the fields <c>text</c> and, with addressees, <c>to</c>.</summary>
    public ConversationNode<TRow> Composer(Txt placeholder, ActionRef action)
    {
        var c = new UiObj { ["placeholder"] = placeholder, ["action"] = action.Name };
        _to = c.List("to", true);
        Body["composer"] = c;
        return this;
    }

    private List<object>? _to;

    /// <summary>An addressee of the message box (after <see cref="Composer"/>); the first is the default.</summary>
    public ConversationNode<TRow> To(string value, Txt label)
    {
        _to?.Add(new UiObj { ["value"] = value, ["label"] = label });
        return this;
    }

    public ConversationNode<TRow> Empty(Txt text) { Body["empty"] = text; return this; }
}

/// <summary>
/// A messenger screen (<see cref="Ui.Chat"/>): the open thread, a run's log and the members as side panels and the
/// actions on messages. Deprecated in catalog 3: a
/// <c>Ui.Split(..).Fill()</c> of a <see cref="Ui.Header"/>, a <see cref="Ui.Stream{TRes, TRow}"/> of posts, a
/// <see cref="Ui.Strip{TRes, TX, TRow, TK, TT}"/> and a <see cref="Ui.Composer"/> replace it.
/// </summary>
public sealed class ChatNode : Node
{
    private readonly UiObj _actions;

    internal ChatNode(string channel, string send) : base("chat")
    {
        Body["channel"] = channel;
        _actions = new UiObj { ["send"] = send };
        Body["actions"] = _actions;
    }

    /// <summary>The open thread (<c>root</c>, <c>replies</c>) and the reply action (<c>{form: text, to, also}</c>).</summary>
    public ChatNode Thread<TRes, TRow>(Q<TRes, TRow> query, ActionRef reply) { Body["thread"] = query.Name; _actions["reply"] = reply.Name; return this; }

    /// <summary>An agent run's log (<c>steps</c>), with Stop and a note to the agent (<c>{form: text}</c>).</summary>
    public ChatNode Run<TRes, TRow>(Q<TRes, TRow> query, ActionRef stop, ActionRef tell)
    {
        Body["run"] = query.Name;
        _actions["stop"] = stop.Name;
        _actions["tell"] = tell.Name;
        return this;
    }

    /// <summary>The members (<c>people</c>, <c>agents</c>) and the action that opens them.</summary>
    public ChatNode Members<TRes, TRow>(Q<TRes, TRow> query, ActionRef open) { Body["members"] = query.Name; _actions["members"] = open.Name; return this; }

    /// <summary>Opens a message's thread, and closes the side panel.</summary>
    public ChatNode OpenThread(ActionRef open, ActionRef close) { _actions["thread"] = open.Name; _actions["close"] = close.Name; return this; }

    /// <summary>
    /// The open thread's Following bell: the action follows the thread (<c>{form: follow}</c> true) or stops
    /// following it (false); the thread query reads <c>following</c>, whether the reader follows it.
    /// </summary>
    public ChatNode FollowThread(ActionRef action) { _actions["follow"] = action.Name; return this; }

    /// <summary>Opens a message's run log.</summary>
    public ChatNode OpenRun(ActionRef action) { _actions["run"] = action.Name; return this; }

    /// <summary>Approve, Decline and Undo on a message's approval.</summary>
    public ChatNode Approval(ActionRef approve, ActionRef decline, ActionRef undo)
    {
        _actions["approve"] = approve.Name;
        _actions["decline"] = decline.Name;
        _actions["undo"] = undo.Name;
        return this;
    }

    /// <summary>A message card's button.</summary>
    public ChatNode Card(ActionRef action) { _actions["card"] = action.Name; return this; }

    /// <summary>Reactions on a message (<c>{form: emoji}</c>).</summary>
    public ChatNode React(ActionRef action) { _actions["react"] = action.Name; return this; }

    /// <summary>The author's own changes to a message (edit <c>{form: text}</c>, delete), and pin or unpin it.</summary>
    public ChatNode Editing(ActionRef edit, ActionRef delete, ActionRef pin)
    {
        _actions["edit"] = edit.Name;
        _actions["delete"] = delete.Name;
        _actions["pin"] = pin.Name;
        return this;
    }

    /// <summary>Chooses an option of a message's decision (<c>{form: choice}</c>).</summary>
    public ChatNode Choose(ActionRef action) { _actions["choose"] = action.Name; return this; }

    /// <summary>The agent a direct message is with (<c>name</c>) and the action that pauses or resumes it.</summary>
    public ChatNode About<TRes, TRow>(Q<TRes, TRow> query, ActionRef pause) { Body["about"] = query.Name; _actions["pause"] = pause.Name; return this; }

    /// <summary>Whom a new message can go to (<c>people</c>, <c>agents</c>) and the action that starts it (<c>{form: to, text}</c>).</summary>
    public ChatNode Directory<TRes, TRow>(Q<TRes, TRow> query, ActionRef start) { Body["directory"] = query.Name; _actions["start"] = start.Name; return this; }
}

/// <summary>A meter (a value of a maximum) or a progress (a fraction 0..1), with a caption.</summary>
public sealed class GaugeNode : Node
{
    internal GaugeNode(string kind) : base(kind) { }

    public GaugeNode Caption(Txt caption)
    {
        Body["caption"] = caption;
        return this;
    }
}

/// <summary>A text: body, heading, caption, mono or label; it takes a format.</summary>
public sealed class TextNode : Node, IFormatted
{
    internal TextNode(Txt text, string style) : base("text")
    {
        Body["text"] = text;
        Body["style"] = style;
    }
}

/// <summary>An empty state (an icon, a title, a text, a button) or a notice (a tone, a title, a text, a button).</summary>
public sealed class MessageNode : Node
{
    internal MessageNode(string kind) : base(kind) { }

    /// <summary>An empty state's icon.</summary>
    public MessageNode Icon(string icon)
    {
        Body["icon"] = icon;
        return this;
    }

    /// <summary>A notice's title.</summary>
    public MessageNode Title(Txt title)
    {
        Body["title"] = title;
        return this;
    }

    /// <summary>The notice's or the empty state's button.</summary>
    public MessageNode WithAction(Node button)
    {
        Body["action"] = button;
        return this;
    }
}

/// <summary>A button running an action; secondary unless <c>.Primary()</c>, <c>.Ghost()</c> or <c>.Danger()</c>.</summary>
public sealed class ButtonNode : Node
{
    internal ButtonNode(Txt label, ActionRef action) : base("button")
    {
        Body["label"] = label;
        Body["action"] = action.Name;
        Body["emphasis"] = "secondary";
    }

    public ButtonNode Primary() => Emphasis("primary");

    public ButtonNode Ghost() => Emphasis("ghost");

    public ButtonNode Danger() => Emphasis("danger");

    private ButtonNode Emphasis(string e)
    {
        Body["emphasis"] = e;
        return this;
    }

    public ButtonNode Icon(string icon)
    {
        Body["icon"] = icon;
        return this;
    }
}

/// <summary>A menu of items (an overflow button by default).</summary>
public sealed class MenuNode : Node
{
    internal MenuNode(MenuItem[] items) : base("menu")
    {
        Body["icon"] = "more-horizontal";
        Body.List("items").AddRange(items);
    }

    public MenuNode Icon(string icon)
    {
        Body["icon"] = icon;
        return this;
    }

    /// <summary>A labelled menu button instead of the icon.</summary>
    public MenuNode Label(Txt label)
    {
        Body["label"] = label;
        return this;
    }
}

/// <summary>A menu item running an action.</summary>
public sealed class MenuItem : IUiJson
{
    private readonly UiObj _o = new();

    internal MenuItem(Txt label, ActionRef action)
    {
        _o["label"] = label;
        _o["action"] = action.Name;
    }

    public MenuItem Danger()
    {
        _o["danger"] = true;
        return this;
    }

    public MenuItem Icon(string icon)
    {
        _o["icon"] = icon;
        return this;
    }

    /// <summary>Shown only when the condition holds.</summary>
    public MenuItem When(Cond when)
    {
        _o["visible"] = when;
        return this;
    }

    JsonNode IUiJson.ToJson(UiWriter w) => _o.ToJson(w);
}

/// <summary>
/// The head of a record (a panel or a page): <c>Ui.RecordHeader(title).Short(..).Subtitle(..).Tags(..).Actions(..).Menu(..)</c>;
/// on a phone the title moves to the top bar, the actions to the bottom bar and the menu to an action sheet.
/// </summary>
public sealed class RecordHeaderNode : Node
{
    private readonly List<object> _tags;
    private readonly List<object> _actions;

    internal RecordHeaderNode(Txt title) : base("record_header")
    {
        Body["title"] = title;
        _tags = Body.List("tags", true);
        _actions = Body.List("actions", true);
    }

    /// <summary>The top bar's title on a phone (<c>#1048</c>).</summary>
    public RecordHeaderNode Short(Txt text)
    {
        Body["short"] = text;
        return this;
    }

    public RecordHeaderNode Subtitle(Txt subtitle)
    {
        Body["subtitle"] = subtitle;
        return this;
    }

    /// <summary>The heading and the line under it on a phone.</summary>
    public RecordHeaderNode Compact(Txt title, Txt subtitle)
    {
        Body["compact_title"] = title;
        Body["compact_subtitle"] = subtitle;
        return this;
    }

    /// <summary>Status chips next to the title.</summary>
    public RecordHeaderNode Tags(params Node[] tags)
    {
        _tags.Clear();
        _tags.AddRange(tags);
        return this;
    }

    /// <summary>Buttons under the title.</summary>
    public RecordHeaderNode Actions(params Node[] actions)
    {
        _actions.Clear();
        _actions.AddRange(actions);
        return this;
    }

    /// <summary>More actions (<c>…</c>).</summary>
    public RecordHeaderNode Menu(params MenuItem[] items)
    {
        var m = new UiObj { ["label"] = "More actions", ["icon"] = "more-horizontal" };
        m.List("items").AddRange(items);
        Body["menu"] = m;
        return this;
    }

    /// <summary>The action of the panel's expand button.</summary>
    public RecordHeaderNode Expand(ActionRef action)
    {
        Body["expand"] = action.Name;
        return this;
    }

    /// <summary>The action of the panel's close button.</summary>
    public RecordHeaderNode Close(ActionRef action)
    {
        Body["close"] = action.Name;
        return this;
    }
}

/// <summary>An agent's run on a record (<see cref="Ui.AgentRun"/>): who, its state, its steps, Stop and a message box.</summary>
public sealed class AgentRunNode<TRes> : Node
{
    internal AgentRunNode() : base("agent_run") { }

    /// <summary>The line under the agent's name.</summary>
    public AgentRunNode<TRes> Subtitle<T>(Expression<Func<TRes, T>> field)
    {
        Body["subtitle"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>A Stop button, shown while <paramref name="when"/> holds.</summary>
    public AgentRunNode<TRes> Stop(ActionRef action, Cond when)
    {
        Body["stop"] = action.Name;
        Body["stop_when"] = when;
        return this;
    }

    /// <summary>A message box; the action reads the text as <c>Ui.FormValue</c> of the field <c>text</c>.</summary>
    public AgentRunNode<TRes> Message(Txt placeholder, ActionRef action)
    {
        Body["message"] = new UiObj { ["placeholder"] = placeholder, ["action"] = action.Name };
        return this;
    }

    /// <summary>
    /// What the run needs from the viewer, first and in amber: the request (a run field, empty when nothing is needed)
    /// and one button; <paramref name="due"/> is the deadline next to it (<c>by 11:30</c>).
    /// </summary>
    public AgentRunNode<TRes> Needs<T>(Expression<Func<TRes, T>> text, Txt label, ActionRef action,
        Expression<Func<TRes, string>>? due = null)
    {
        var n = new UiObj { ["text"] = FieldPath.Of(text), ["label"] = label, ["action"] = action.Name };
        if (due is not null) n["due"] = FieldPath.Of(due);
        Body["needs"] = n;
        return this;
    }

    /// <summary>Opens the run's session page; the card then shows the current step instead of the step list.</summary>
    public AgentRunNode<TRes> Open(Txt label, ActionRef action)
    {
        Body["open"] = new UiObj { ["label"] = label, ["action"] = action.Name };
        return this;
    }

    /// <summary>The changes the run made to this record: a repeated field and each row's text.</summary>
    public AgentRunNode<TRes> Changes<TC, T>(Txt title, Expression<Func<TRes, IEnumerable<TC>>> field,
        Expression<Func<TC, T>> text)
    {
        Body["changes"] = new UiObj { ["title"] = title, ["field"] = FieldPath.Of(field), ["text"] = FieldPath.Of(text) };
        return this;
    }

    /// <summary>The agent's earlier sessions on this record: a run field with the line, hidden while empty, and its action.</summary>
    public AgentRunNode<TRes> Earlier<T>(Expression<Func<TRes, T>> text, ActionRef action)
    {
        Body["earlier"] = new UiObj { ["text"] = FieldPath.Of(text), ["action"] = action.Name };
        return this;
    }

    private AgentRunNode<TRes> Put(string key, object? value)
    {
        Body[key] = value;
        return this;
    }

    /// <summary>
    /// The run's full log in a panel (catalog 3): a head with the agent's tile, the title, the subtitle, the status with
    /// its label, the step label and the time, Stop; then every step (its state one of <c>done</c>, <c>now</c>,
    /// <c>wait</c>, <c>todo</c>, with its detail, time, call, evidence and lines) and the note while the run is not live.
    /// </summary>
    public AgentRunNode<TRes> Log() => Put("log", true);

    /// <summary>The agent's tile in the log's head: response fields with the glyph and the engine.</summary>
    public AgentRunNode<TRes> Tile<TA, TB>(Expression<Func<TRes, TA>> icon, Expression<Func<TRes, TB>> engine)
    {
        Body["icon"] = FieldPath.Of(icon);
        return Put("engine", FieldPath.Of(engine));
    }

    /// <summary>A response field: the status's label in the log's head.</summary>
    public AgentRunNode<TRes> StatusLabel<T>(Expression<Func<TRes, T>> field) => Put("status_label", FieldPath.Of(field));

    /// <summary>A response field: the current step's label in the log's head.</summary>
    public AgentRunNode<TRes> StepLabel<T>(Expression<Func<TRes, T>> field) => Put("step_label", FieldPath.Of(field));

    /// <summary>A response field: the run's time in the log's head.</summary>
    public AgentRunNode<TRes> Elapsed<T>(Expression<Func<TRes, T>> field) => Put("elapsed", FieldPath.Of(field));

    /// <summary>A response field: the line under the steps while the run is not live.</summary>
    public AgentRunNode<TRes> Note<T>(Expression<Func<TRes, T>> field) => Put("note", FieldPath.Of(field));

    /// <summary>The run is live while this holds (the log's note is shown otherwise).</summary>
    public AgentRunNode<TRes> LiveWhen(Cond when) => Put("live_when", when);

    /// <summary>A field of a step: its detail (<c>(RunStep s) =&gt; s.Detail</c>).</summary>
    public AgentRunNode<TRes> StepDetail<TStep, T>(Expression<Func<TStep, T>> field) => Put("step_detail", FieldPath.Of(field));

    /// <summary>A field of a step: its time.</summary>
    public AgentRunNode<TRes> StepTime<TStep, T>(Expression<Func<TStep, T>> field) => Put("step_time", FieldPath.Of(field));

    /// <summary>A field of a step: its call, as a process row's.</summary>
    public AgentRunNode<TRes> StepTool<TStep, T>(Expression<Func<TStep, T>> field) => Put("step_tool", FieldPath.Of(field));

    /// <summary>A field of a step: its evidence.</summary>
    public AgentRunNode<TRes> StepEvidence<TStep, T>(Expression<Func<TStep, T>> field) => Put("step_evidence", FieldPath.Of(field));

    /// <summary>A field of a step: its lines.</summary>
    public AgentRunNode<TRes> StepLines<TStep, T>(Expression<Func<TStep, T>> field) => Put("step_lines", FieldPath.Of(field));
}

/// <summary>A list and its detail (<see cref="Ui.Split"/>), and of catalog 3 an app-like screen.</summary>
public sealed class SplitNode : Node
{
    internal SplitNode(Node list, Node detail) : base("split")
    {
        Body["list"] = list;
        Body["detail"] = detail;
    }

    /// <summary>
    /// An app-like screen (catalog 3): the screen takes the viewport's height under the shell's header and nothing scrolls
    /// the page; in each side the first stream takes the free height and scrolls itself. The detail is an in-page side
    /// panel shown while its <c>.Visible(..)</c> holds (a full-screen page on a phone).
    /// </summary>
    public SplitNode Fill()
    {
        Body["fill"] = true;
        return this;
    }

    /// <summary>
    /// The panel's width is dragged by its border, kept per viewer (catalog 3): px from <paramref name="min"/> to
    /// <paramref name="max"/> (240 to 1200), <paramref name="initial"/> at first and on reset, the column keeping at least
    /// <paramref name="keep"/>.
    /// </summary>
    public SplitNode Resize(int min, int max, int initial, int? keep = null)
    {
        Body["resize"] = new UiObj { ["min"] = min, ["max"] = max, ["default"] = initial, ["keep"] = keep };
        return this;
    }

    /// <summary>A bool state: true, the detail fills the screen (catalog 3); the detail's header's <c>.Expand()</c> toggles it.</summary>
    public SplitNode Expand(StateRef<bool> state)
    {
        Body["expand"] = new UiObj { ["state"] = state.Name };
        return this;
    }
}

/// <summary>An icon button (<see cref="Ui.IconButton"/>); of catalog 3 a toggle.</summary>
public sealed class IconButtonNode : Node
{
    internal IconButtonNode(string icon, Txt label, ActionRef action) : base("icon_button")
    {
        Body["icon"] = icon;
        Body["label"] = label;
        Body["action"] = action.Name;
    }

    /// <summary>
    /// A toggle (catalog 3): <paramref name="pressed"/> says whether it is on; the icon and label given to
    /// <c>Ui.IconButton</c> are the pressed ones, these the others. A press shows the other state at once and runs the
    /// action, which reads the new state as <c>Form.Pressed()</c>; a failed action reverts it.
    /// </summary>
    public IconButtonNode Toggle(Val<bool> pressed, string iconOff, Txt labelOff)
    {
        Body["pressed"] = Val<bool>.Json(pressed);
        Body["icon_off"] = iconOff;
        Body["label_off"] = labelOff;
        return this;
    }
}

/// <summary>
/// The catalog: primitives (<c>Ui.Stack</c>, <c>Ui.Table</c>, <c>Ui.Button</c>, …), values (<c>Ui.Lit</c>,
/// <c>Ui.CallerId</c>, …) and catalog texts (<c>Ui.Text("nav.all", "All orders")</c>). <c>using static
/// Celobus.Sdk.Ui.Ui;</c> drops the prefix.
/// </summary>
public static partial class Ui
{
    // ------------------------------------------------------------------------------------------ texts and values

    /// <summary>A catalog text: its key and its English (fallback) text, collected into the document's <c>texts.en</c>.</summary>
    public static Txt Text(string key, string en) => Txt.Catalog(key, en);

    /// <summary>A literal value.</summary>
    public static Val<T> Lit<T>(T value) => value;

    /// <summary>A field of the last call's result in the running chain: <c>Ui.Result((Order o) =&gt; o.Id)</c>.</summary>
    public static Val<T> Result<TMsg, T>(Expression<Func<TMsg, T>> field) => new("result", FieldPath.Of(field));

    /// <summary>The member looking at the screen.</summary>
    public static Val<string> CallerId() => new("caller", "user_id");

    public static Val<string> CallerName() => new("caller", "display_name");

    /// <summary>In <c>OnError</c>: the failing call's error code.</summary>
    public static Val<string> ErrorCode() => new("error", "code");

    /// <summary>In <c>OnError</c>: the failing call's message.</summary>
    public static Val<string> ErrorMessage() => new("error", "message");

    public static Cond Not(Cond c) => new(UiJson.One("not", c.Node.DeepClone()));

    /// <summary>No value: clears an optional param or state (<c>a.SetParam(order, Ui.None&lt;Guid?&gt;())</c>).</summary>
    public static Val<T> None<T>() => new("lit", null);

    /// <summary>A field of the current row, where no row handle is at hand (a list's aside text).</summary>
    public static Val<T> RowValue<TRow, T>(Expression<Func<TRow, T>> field) => new("row", FieldPath.Of(field));

    /// <summary>The form's current values, for queries of the same overlay (a quote that reads them as they change).</summary>
    public static Val<T> FormValue<TReq, T>(Expression<Func<TReq, T>> field) => new("form", FieldPath.Of(field));

    // ------------------------------------------------------------------------------------------ structure

    public static StackNode Stack(params Node[] children) => new("vertical", children);

    /// <summary>Children side by side (wrapping on a phone).</summary>
    public static StackNode Row(params Node[] children) => new("horizontal", children);

    public static ColumnsNode Columns(params Node[] children) => new(children);

    public static SectionNode Section(Txt title, params Node[] children) => new(title, children);

    public static CardNode Card(params Node[] children) => new(children);

    /// <summary>A card with a title line (<c>3 items · from catalog</c>); <c>.Trailing(v).MoneyIn(..)</c> adds a value at its end.</summary>
    public static CardNode TitledCard(Txt title, params Node[] children) => new CardNode(children).Title(title);

    public static Tab Tab(string id, Txt title, params Node[] children) => new(id, title, children);

    /// <summary>Tabs; the client keeps the selected one unless <c>.BindParam(..)</c> or <c>.BindState(..)</c> says where it lives.</summary>
    public static TabsNode Tabs(params Tab[] tabs) => new(tabs);

    public static Node Divider() => new("divider");

    /// <summary>
    /// A list and the detail of its selection, side by side (stacked on a phone); <c>.Fill()</c> makes it an app-like
    /// screen (catalog 3).
    /// </summary>
    public static SplitNode Split(Node list, Node detail) => new(list, detail);

    // ------------------------------------------------------------------------------------------ navigation

    /// <summary>A view switch bound to state: <c>Ui.Segmented(view, ("list", "List"), ("board", "Board"))</c>.</summary>
    public static Node Segmented<T>(StateRef<T> state, params (T Value, Txt Label)[] options) =>
        Segmented(new JsonObject { ["state"] = state.Name }, options);

    /// <summary>
    /// A switch bound to a param, so the choice is in the link (the range of a report):
    /// <c>Ui.Segmented(range, ("7", "7 days"), ("30", "30 days"))</c>. The options are values of the param's type; give
    /// the param a default (<c>s.ParamOr(..)</c>) so the switch shows it on a link without the param.
    /// </summary>
    public static Node Segmented<T>(ParamRef<T> param, params (T Value, Txt Label)[] options) =>
        Segmented(new JsonObject { ["param"] = param.Name }, options);

    /// <summary>The same over an optional param of a value type (an enum, a number): the options are of the inner type.</summary>
    public static Node Segmented<T>(ParamRef<T?> param, params (T Value, Txt Label)[] options) where T : struct =>
        Segmented(new JsonObject { ["param"] = param.Name }, options);

    private static Node Segmented<T>(JsonObject bind, (T Value, Txt Label)[] options)
    {
        var n = new Node("segmented");
        n.Body["bind"] = bind;
        n.Body.List("options").AddRange(options.Select(o => new UiObj { ["value"] = UiJson.Literal(o.Value), ["label"] = o.Label }));
        return n;
    }

    public static Node Link<TParams>(Txt text, ScreenKey<TParams> screen, Action<Args<TParams>>? args = null) where TParams : class
    {
        var n = new Node("link");
        n.Body["text"] = text;
        n.Body["target"] = Target.Of(screen.Id, null, Args<TParams>.Build(args));
        return n;
    }

    // ------------------------------------------------------------------------------------------ data

    public static TableNode<TRow> Table<TRes, TRow>(Q<TRes, TRow> rows) => new(rows.Name);

    public static ListNode<TRow> List<TRes, TRow, T>(Q<TRes, TRow> rows, Expression<Func<TRow, T>> title) =>
        new(rows.Name, FieldPath.Of(title));

    public static FieldsNode Fields(params FieldItem[] items) => new(items);

    /// <summary>One label/value pair of <see cref="Fields"/>.</summary>
    public static FieldItem Item<T>(Txt label, Val<T> value) => new(label, Val<T>.Json(value));

    public static MetricNode Metric<T>(Txt title, Val<T> value) => new(title, Val<T>.Json(value));

    public static ChartNode<TRow> BarChart<TRes, TRow, TX>(Q<TRes, TRow> rows, Expression<Func<TRow, TX>> x) =>
        new("bar", rows.Name, FieldPath.Of(x));

    public static ChartNode<TRow> LineChart<TRes, TRow, TX>(Q<TRes, TRow> rows, Expression<Func<TRow, TX>> x) =>
        new("line", rows.Name, FieldPath.Of(x));

    public static ChartNode<TRow> AreaChart<TRes, TRow, TX>(Q<TRes, TRow> rows, Expression<Func<TRow, TX>> x) =>
        new("area", rows.Name, FieldPath.Of(x));

    public static ChartNode<TRow> PieChart<TRes, TRow, TX>(Q<TRes, TRow> rows, Expression<Func<TRow, TX>> x) =>
        new("pie", rows.Name, FieldPath.Of(x));

    /// <summary>A dashboard: cards on a 12-column grid (<c>Ui.Board().Quarter(a).Half(b).Span(8, c).Full(d)</c>).</summary>
    public static BoardNode Board() => new();

    /// <summary>
    /// A chart or figure by the names of its family and kind (<c>Ui.Viz("line", "line")</c>); the class <c>Viz</c> has
    /// a constructor per kind of the catalogue (<c>Viz.Line.Line()</c>), which cannot name a kind that does not exist.
    /// </summary>
    public static VizNode Viz(string family, string kind) => new(family, kind);

    public static TimelineNode<TRow> Timeline<TRes, TRow, TA, TB>(Q<TRes, TRow> rows, Expression<Func<TRow, TA>> text,
        Expression<Func<TRow, TB>> time) => new(rows.Name, FieldPath.Of(text), FieldPath.Of(time));

    /// <summary>An enum value as a status, with a tone per value.</summary>
    public static Node Status<TEnum>(Val<TEnum> value, params (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum =>
        StatusOf(Val<TEnum>.Json(value), tones);

    /// <inheritdoc cref="Status{TEnum}(Val{TEnum}, ValueTuple{TEnum, Tone}[])"/>
    public static Node Status<TEnum>(Val<TEnum?> value, params (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum =>
        StatusOf(Val<TEnum?>.Json(value), tones);

    private static Node StatusOf<TEnum>(JsonNode value, (TEnum Value, Tone Tone)[] tones) where TEnum : struct, Enum
    {
        var n = new Node("status");
        n.Body["value"] = value;
        if (tones.Length > 0) n.Body["tones"] = UiJson.Tones(tones);
        return n;
    }

    public static Node Tag(Txt text)
    {
        var n = new Node("tag");
        n.Body["text"] = text;
        return n;
    }

    /// <summary>A member id (drawn with the member's name and picture) or a name.</summary>
    public static Node Avatar<T>(Val<T> value)
    {
        var n = new Node("avatar");
        n.Body["value"] = Val<T>.Json(value);
        return n;
    }

    /// <summary>A list of member ids (at most five shown).</summary>
    public static Node AvatarStack<T>(Val<T> value)
    {
        var n = new Node("avatar_stack");
        n.Body["value"] = Val<T>.Json(value);
        n.Body["max"] = 5;
        return n;
    }

    public static GaugeNode Meter<TA, TB>(Val<TA> value, Val<TB> max)
    {
        var n = new GaugeNode("meter");
        n.Body["value"] = Val<TA>.Json(value);
        n.Body["max"] = Val<TB>.Json(max);
        return n;
    }

    /// <summary>A fraction 0..1.</summary>
    public static GaugeNode Progress<T>(Val<T> value)
    {
        var n = new GaugeNode("progress");
        n.Body["value"] = Val<T>.Json(value);
        return n;
    }

    /// <summary>Body text (a node; <see cref="Text(string, string)"/> with two arguments is a catalog text).</summary>
    public static TextNode Text(Txt text) => new(text, "body");

    public static TextNode Heading(Txt text) => new(text, "heading");

    public static TextNode Caption(Txt text) => new(text, "caption");

    public static TextNode Mono(Txt text) => new(text, "mono");

    /// <summary>A small uppercase label over a group (<c>ACTIVITY</c>).</summary>
    public static TextNode Label(Txt text) => new(text, "label");

    public static Node CopyField<T>(Txt label, Val<T> value)
    {
        var n = new Node("copy_field");
        n.Body["value"] = Val<T>.Json(value);
        n.Body["label"] = label;
        return n;
    }

    public static MessageNode Empty(Txt title, Txt text)
    {
        var n = new MessageNode("empty");
        n.Body["title"] = title;
        n.Body["text"] = text;
        return n;
    }

    // ------------------------------------------------------------------------------------------ input

    /// <summary>Inputs that set the screen's params (<c>status.Select("Status")</c>).</summary>
    public static Node FilterBar(params Input[] inputs)
    {
        var n = new Node("filter_bar");
        n.Body.List("inputs").AddRange(inputs);
        return n;
    }

    // ------------------------------------------------------------------------------------------ actions

    public static ButtonNode Button(Txt label, ActionRef action) => new(label, action);

    /// <summary>An icon button; <c>.Toggle(..)</c> makes it a toggle (catalog 3).</summary>
    public static IconButtonNode IconButton(string icon, Txt label, ActionRef action) => new(icon, label, action);

    public static MenuNode Menu(params MenuItem[] items) => new(items);

    /// <summary>A menu opened by a labelled button (<c>View</c>).</summary>
    public static MenuNode MenuButton(Txt label, string icon, params MenuItem[] items) => new MenuNode(items).Icon(icon).Label(label);

    public static MenuItem MenuItem(Txt label, ActionRef action) => new(label, action);

    // ------------------------------------------------------------------------------------------ feedback

    public static MessageNode Notice(Tone tone, Txt text)
    {
        var n = new MessageNode("notice");
        n.Body["tone"] = UiJson.Name(tone);
        n.Body["text"] = text;
        return n;
    }

    // ------------------------------------------------------------------------------------------ records

    /// <summary>The head of a record: title, tags, a subtitle, actions, a menu.</summary>
    public static RecordHeaderNode RecordHeader(Txt title) => new(title);

    /// <summary>
    /// An agent's run on a record, from a query of one run: the agent's name, its status with a tone per value, the
    /// repeated steps and each step's text and state (<c>done</c>, <c>running</c>, <c>pending</c>).
    /// </summary>
    public static AgentRunNode<TRes> AgentRun<TRes, TRow, TA, TEnum, TStep, TB, TC>(Q<TRes, TRow> run,
        Expression<Func<TRes, TA>> title, Expression<Func<TRes, TEnum>> status, (TEnum Value, Tone Tone)[] tones,
        Expression<Func<TRes, IEnumerable<TStep>>> steps, Expression<Func<TStep, TB>> stepText,
        Expression<Func<TStep, TC>> stepState) where TEnum : struct, Enum
    {
        var n = new AgentRunNode<TRes>();
        n.Body["query"] = run.Name;
        n.Body["title"] = FieldPath.Of(title);
        var st = new UiObj { ["field"] = FieldPath.Of(status) };
        if (tones.Length > 0) st["tones"] = UiJson.Tones(tones);
        n.Body["status"] = st;
        n.Body["steps"] = FieldPath.Of(steps);
        n.Body["step_text"] = FieldPath.Of(stepText);
        n.Body["step_state"] = FieldPath.Of(stepState);
        return n;
    }

    /// <summary>
    /// A messenger screen over a query of one conversation, posting with <paramref name="send"/>. Deprecated in catalog
    /// 3 (the platform warns <c>W_UI_DEPRECATED</c> and removes it in the next release): build the screen of a
    /// <c>Ui.Split(..).Fill()</c>, <see cref="Header"/>, <see cref="Stream{TRes, TRow}"/>, <see cref="Strip{TRes, TX, TRow, TK, TT}"/>
    /// and <see cref="Composer"/>.
    /// </summary>
    [Obsolete("Deprecated in catalog 3: use Ui.Split(..).Fill() with Ui.Header, Ui.Stream, Ui.Strip and Ui.Composer.")]
    public static ChatNode Chat<TRes, TRow>(Q<TRes, TRow> channel, ActionRef send) => new(channel.Name, send.Name);

    /// <summary>A conversation from a query of messages: the author, the text and the time of each, oldest first.</summary>
    public static ConversationNode<TRow> Conversation<TRes, TRow, TA, TB, TC>(Q<TRes, TRow> rows,
        Expression<Func<TRow, TA>> author, Expression<Func<TRow, TB>> text, Expression<Func<TRow, TC>> time) =>
        new(rows.Name, FieldPath.Of(author), FieldPath.Of(text), FieldPath.Of(time));

    /// <summary>A row as a list item; inside a <c>Compact</c> or <c>Item</c> function <c>r.View(..)</c> infers the row type.</summary>
    public static RowView<TRow> RowView<TRow>(Txt title) => new(title);
}
