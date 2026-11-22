// The document root and the contributions outside screens: sidebar groups (with live badges), palette commands,
// palette search sources and workspace home widgets, whose queries are document-level.
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>
/// The plugin's UI: <c>new PluginUi().Nav(..).Command(..).Screen(..)</c>, declared with <c>plugin.Ui(..)</c>. The
/// <c>en</c> texts are the catalog texts (<see cref="Ui.Text(string, string)"/>) the document uses.
/// </summary>
public sealed class PluginUi
{
    /// <summary>The catalog revision the document is written for (2: charts and figures, <see cref="Viz"/>, and dashboards, <see cref="BoardNode"/>; 3: the app-like primitives).</summary>
    public const int Catalog = 3;

    private readonly List<object> _nav = [];
    private readonly List<object> _home = [];
    private readonly List<object> _commands = [];
    private readonly List<JsonObject> _queries = [];
    private readonly List<object> _screens = [];
    private readonly List<object> _overlays = [];
    private readonly List<object> _search = [];
    private string? _landing;
    private readonly SortedDictionary<string, SortedDictionary<string, string>> _texts = new(StringComparer.Ordinal);

    /// <summary>A sidebar group; its entries' badge queries become document-level queries.</summary>
    public PluginUi Nav(Nav group)
    {
        foreach (var q in group.Queries)
            if (!_queries.Any(x => x["name"]!.GetValue<string>() == q["name"]!.GetValue<string>()))
                _queries.Add(q);
        _nav.Add(group.Obj);
        return this;
    }

    public PluginUi Screen(Screen screen)
    {
        _screens.Add(screen.Obj);
        return this;
    }

    /// <summary>The screen the plugin opens on; it takes no required param (<c>E_UI_LANDING</c>). Without it, the
    /// client opens the screen of the navigation's first entry that has one.</summary>
    public PluginUi Landing<TParams>(ScreenKey<TParams> screen) where TParams : class
    {
        _landing = screen.Id;
        return this;
    }

    /// <summary>An overlay opened from anywhere (a command, a nav "add" row).</summary>
    public PluginUi Overlay(Overlay overlay)
    {
        _overlays.Add(overlay.Obj);
        return this;
    }

    public PluginUi Command(Command command)
    {
        _commands.Add(command.Obj);
        return this;
    }

    /// <summary>A source of records for the command palette (<c>Search.By(..)</c>).</summary>
    public PluginUi Search<TRow>(Search<TRow> source)
    {
        _search.Add(source.Obj);
        return this;
    }

    /// <summary>A workspace home widget; its function declares queries (document-level, <c>home_&lt;id&gt;_&lt;name&gt;</c>) and returns the node.</summary>
    public PluginUi Home(string id, Txt title, Func<WidgetBuilder, Node> build)
    {
        var node = build(new WidgetBuilder(this, $"home_{id}"));
        _home.Add(new UiObj { ["id"] = id, ["title"] = title, ["node"] = node });
        return this;
    }

    /// <summary>A text in another locale (<c>en</c> comes from <see cref="Ui.Text(string, string)"/>).</summary>
    public PluginUi Text(string locale, string key, string value)
    {
        if (!_texts.TryGetValue(locale, out var catalog)) _texts[locale] = catalog = new(StringComparer.Ordinal);
        catalog[key] = value;
        return this;
    }

    internal void AddQuery(JsonObject query) => _queries.Add(query);

    /// <summary>The document as the descriptor carries it.</summary>
    internal JsonObject Build()
    {
        var root = new UiObj { ["catalog"] = Catalog };
        root.List("nav", true).AddRange(_nav);
        if (_landing is not null) root["landing"] = _landing;
        root.List("home", true).AddRange(_home);
        root.List("commands", true).AddRange(_commands);
        root.List("queries", true).AddRange(_queries);
        root.List("screens", true).AddRange(_screens);
        root.List("overlays", true).AddRange(_overlays);
        root.List("search", true).AddRange(_search);
        var w = new UiWriter();
        var json = (JsonObject)root.ToJson(w);

        var texts = _texts.ToDictionary(t => t.Key, t => new SortedDictionary<string, string>(t.Value, StringComparer.Ordinal));
        if (w.Texts.Count > 0)
        {
            if (!texts.TryGetValue("en", out var en)) texts["en"] = en = new(StringComparer.Ordinal);
            foreach (var (key, text) in w.Texts) en.TryAdd(key, text);
        }
        if (texts.Count > 0)
        {
            var t = new JsonObject();
            foreach (var (locale, catalog) in texts.OrderBy(x => x.Key, StringComparer.Ordinal))
            {
                var c = new JsonObject();
                foreach (var (key, text) in catalog) c[key] = text;
                t[locale] = c;
            }
            json["texts"] = t;
        }
        return json;
    }
}

/// <summary>A sidebar group: <c>Nav.Group("Orders").Icon("shopping-bag").Entry(..)</c>.</summary>
public sealed class Nav
{
    internal UiObj Obj { get; } = new();
    internal List<JsonObject> Queries { get; } = [];
    private readonly List<object> _entries;

    private Nav(Txt title)
    {
        Obj["title"] = title;
        _entries = Obj.List("entries", true);
    }

    public static Nav Group(Txt title) => new(title);

    public Nav Icon(string icon)
    {
        Obj["icon"] = icon;
        return this;
    }

    public Nav Entry(Entry entry)
    {
        _entries.Add(entry.Obj);
        Queries.AddRange(entry.Queries);
        return this;
    }
}

/// <summary>A sidebar entry: a screen with its params, an optional live badge.</summary>
public sealed class Entry
{
    internal UiObj Obj { get; } = new();
    internal List<JsonObject> Queries { get; } = [];

    private Entry() { }

    /// <summary>An entry opening <paramref name="screen"/> with these params.</summary>
    public static Entry To<TParams>(ScreenKey<TParams> screen, Txt title, Action<Args<TParams>>? args = null) where TParams : class
    {
        var e = new Entry();
        e.Obj["title"] = title;
        e.Obj["screen"] = screen.Id;
        e.Obj.List("args", true).AddRange(Args<TParams>.Build(args));
        return e;
    }

    public Entry Icon(string icon)
    {
        Obj["icon"] = icon;
        return this;
    }

    /// <summary>A count from a document-level query <c>badge_&lt;field&gt;</c>, live through the topics it follows (at most every 2 s).</summary>
    public Entry Badge<TReq, TRes, TRow, T>(QueryDef<TReq, TRes, TRow> query, Expression<Func<TRes, T>> count)
    {
        var field = FieldPath.Of(count);
        var name = $"badge_{field}";
        Queries.Add(query.Build(name, 2000));
        Obj["badge"] = UiJson.One("data", new JsonObject { ["query"] = name, ["field"] = field });
        return this;
    }

    /// <summary>The badge asks for attention (a filled pill) instead of a quiet count.</summary>
    public Entry Alert()
    {
        Obj["badge_style"] = "alert";
        return this;
    }

    private UiObj? _rows;

    /// <summary>
    /// Entries from a query's rows (channels, direct messages), each opening <paramref name="screen"/> with args
    /// that read the row (<c>Ui.RowValue</c>); five show before "Show all".
    /// </summary>
    public static Entry Rows<TReq, TRes, TRow, TL, TParams>(Txt title, QueryDef<TReq, TRes, TRow> query,
        Expression<Func<TRow, TL>> label, ScreenKey<TParams> screen, Action<Args<TParams>>? args = null) where TParams : class
    {
        var e = new Entry();
        var field = FieldPath.Of(label);
        var name = $"rows_{field}";
        e.Queries.Add(query.Build(name, 2000));
        e.Obj["title"] = title;
        e._rows = new UiObj { ["query"] = name, ["label"] = field, ["screen"] = screen.Id };
        e._rows.List("args", true).AddRange(Args<TParams>.Build(args));
        e._rows["limit"] = 5;
        e.Obj["rows"] = e._rows;
        return e;
    }

    /// <summary>Rows: each row's icon field (<c>hash</c>, an agent's <c>bot</c>).</summary>
    public Entry RowIcon<TRow, T>(Expression<Func<TRow, T>> icon)
    {
        if (_rows is not null) _rows["icon"] = FieldPath.Of(icon);
        return this;
    }

    /// <summary>Rows: the unread count of each row (its label turns bold) and the viewer's mentions (an amber pill).</summary>
    public Entry Unread<TRow, T, TM>(Expression<Func<TRow, T>> unread, Expression<Func<TRow, TM>> mentions)
    {
        if (_rows is null) return this;
        _rows["badge"] = FieldPath.Of(unread);
        _rows["mentions"] = FieldPath.Of(mentions);
        return this;
    }

    /// <summary>Rows: how many show before "Show all" (1 to 20).</summary>
    public Entry Limit(int n)
    {
        if (_rows is not null) _rows["limit"] = n;
        return this;
    }

    /// <summary>Rows: an add row under them (<c>New channel</c>) that opens a document-level overlay.</summary>
    public Entry Add<TParams>(Txt title, OverlayKey<TParams> overlay) where TParams : class
    {
        if (_rows is not null) _rows["add"] = new UiObj { ["title"] = title, ["overlay"] = overlay.Id };
        return this;
    }
}

/// <summary>A command-palette entry.</summary>
public sealed class Command
{
    internal UiObj Obj { get; } = new();

    private Command(string id, Txt title, JsonObject target)
    {
        Obj["id"] = id;
        Obj["title"] = title;
        Obj["target"] = target;
    }

    /// <summary>
    /// Navigates to a screen, with params like a link: <c>Command.Navigate("week", "Sales: last 7 days", Screens.Report,
    /// a =&gt; a.Arg(p =&gt; p.Range, "7"))</c>. A command runs from anywhere, so its args are literals.
    /// </summary>
    public static Command Navigate<TParams>(string id, Txt title, ScreenKey<TParams> screen, Action<Args<TParams>>? args = null)
        where TParams : class => new(id, title, Target.Of(screen.Id, null, Args<TParams>.Build(args)));

    /// <summary>Opens a document-level overlay (<see cref="PluginUi.Overlay"/>), with params (literals).</summary>
    public static Command Open<TParams>(string id, Txt title, OverlayKey<TParams> overlay, Action<Args<TParams>>? args = null)
        where TParams : class => new(id, title, Target.Of(null, overlay.Id, Args<TParams>.Build(args)));

    public Command Icon(string icon)
    {
        Obj["icon"] = icon;
        return this;
    }
}

/// <summary>Starts a palette search source: <c>Search.By("Orders", FindOrders.Ref, r =&gt; r.Text, r =&gt; r.Orders)</c>.</summary>
public static class Search
{
    /// <summary>Records the command palette finds: a read <c>ui</c> endpoint called with the typed text in <paramref name="text"/>, its results in <paramref name="rows"/>.</summary>
    public static Search<TRow> By<TReq, TRes, TRow>(Txt title, EndpointRef<TReq, TRes> endpoint,
        Expression<Func<TReq, string>> text, Expression<Func<TRes, IEnumerable<TRow>>> rows)
        where TReq : class where TRes : class => new(title, endpoint.Name, FieldPath.Of(text), FieldPath.Of(rows));
}

/// <summary>A palette search source: how a result is drawn (<c>.Item(..)</c>) and where it leads (<c>.Open(..)</c>).</summary>
public sealed class Search<TRow>
{
    internal UiObj Obj { get; } = new();
    private readonly List<object> _actions;

    internal Search(Txt title, string endpoint, string text, string rows)
    {
        Obj["title"] = title;
        Obj["endpoint"] = endpoint;
        Obj["text"] = text;
        Obj["rows"] = rows;
        Obj["item"] = new RowView<TRow>("");
        Obj["open"] = new JsonObject();
        _actions = Obj.List("actions", true);
    }

    /// <summary>A result as a list item (<c>r =&gt; r.View(r.Get(o =&gt; o.Number)).Subtitle(..)</c>).</summary>
    public Search<TRow> Item(Func<Row<TRow>, RowView<TRow>> view)
    {
        Obj["item"] = view(new Row<TRow>());
        return this;
    }

    /// <summary>Where a result leads (<c>(a, r) =&gt; a.Arg(x =&gt; x.Id, r.Get(o =&gt; o.Id))</c>).</summary>
    public Search<TRow> Open<TParams>(ScreenKey<TParams> screen, Action<Args<TParams>, Row<TRow>>? args = null) where TParams : class
    {
        var a = new Args<TParams>();
        args?.Invoke(a, new Row<TRow>());
        Obj["open"] = Target.Of(screen.Id, null, a.List);
        return this;
    }

    /// <summary>An action the palette offers on the top result: a document-level overlay with args from the result (<c>New order for {name}</c>).</summary>
    public Search<TRow> Action<TParams>(Txt title, string icon, OverlayKey<TParams> overlay, Action<Args<TParams>, Row<TRow>>? args = null)
        where TParams : class
    {
        var a = new Args<TParams>();
        args?.Invoke(a, new Row<TRow>());
        _actions.Add(new UiObj { ["title"] = title, ["icon"] = icon, ["target"] = Target.Of(null, overlay.Id, a.List) });
        return this;
    }
}

/// <summary>The builder a home widget's function gets: its queries are document-level.</summary>
public sealed class WidgetBuilder
{
    private readonly PluginUi _ui;
    private readonly string _prefix;

    internal WidgetBuilder(PluginUi ui, string prefix)
    {
        _ui = ui;
        _prefix = prefix;
    }

    public Q<TRes, TRow> Query<TReq, TRes, TRow>(string name, QueryDef<TReq, TRes, TRow> query)
    {
        var full = $"{_prefix}_{name}";
        _ui.AddQuery(query.Build(full, 2000));
        return new Q<TRes, TRow>(full);
    }
}
