// Catalog revision 3: the app-like primitives. A `Ui.Stream` of posts
// (`Ui.Post<TRow>()`) with a `process` slot, a `Ui.Strip` of live items, a `Ui.Composer`, a `Ui.Header`, the `pick`
// input (`FormBuilder.Pick`); the options of `split` (`SplitNode.Fill`, `Resize`, `Expand`), `list`, `icon_button` (a
// toggle), `agent_run` (the log) and state (`StateReset`); the steps `Draft`, `ReplyTo`, `ScrollTo`, `Copy`,
// `CopyLink`, `Merge`; the query options `Older`, `DebounceMs`, `Join`; the formats `Elapsed`, `Countdown`, `Span`.
//
//   var who = new Via<Msg, Face>("who");
//   var msgs = s.Query("channel", Query.Call(GetChannel.Ref).Arg(r => r.Id, c.OrEmpty())
//       .Rows(v => v.Messages).Older(v => v.OlderPageToken, r => r.PageToken)
//       .Join(who, v => v.People, f => f.Id, m => m.By));
//   Ui.Stream(msgs).Key(m => m.Id)
//       .Post(p => p.Author(who.F(f => f.Name)).Avatar(who.F(f => f.Avatar)).Tile(who.F(f => f.Icon), who.F(f => f.Engine))
//           .Badge("AGENT", Ui.RowValue(who.F(f => f.Agent)).Eq(true)).Time(m => m.Time).Markdown(m => m.Text)
//           .Reactions(m => m.Reactions, r => r.Emoji, r => r.Count, react).Quick("👍", "🔥")
//           .Hover(Builtin.React(), Builtin.Reply(), Ui.Entry(openThread, "Reply in thread"), Builtin.More()))
//       .Days(m => m.Time).OnSeen(markRead).Id("messages");
//
// Row paths: a field lambda reaches into nested messages (`m => m.Thread!.Count` is `thread.count`), and
// `who.F(f => f.Name)` (a field of a query's join) is a lambda over the row too, so it goes wherever a row field does
// (`r.Get(..)`, `Ui.RowValue(..)`, a list's or a post's fields).
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

// ---------------------------------------------------------------------------------------------- row paths, form values

/// <summary>
/// A lookup of a query's rows (<c>.Join(who, ..)</c>): the rows of <typeparamref name="TRow"/> read the item of
/// <typeparamref name="TJoin"/> as <c>who.&lt;field&gt;</c>.
/// </summary>
public sealed class Via<TRow, TJoin>(string name)
{
    /// <summary>The name the rows read the item by (<c>who</c>).</summary>
    public string Name { get; } = name;

    /// <summary>A field of the looked-up item as a path of the row (<c>who.name</c>): a row field lambda.</summary>
    public Expression<Func<TRow, T>> F<T>(Expression<Func<TJoin, T>> field) =>
        FieldPath.Path<TRow, T>($"{Name}.{FieldPath.Of(field)}");
}

/// <summary>
/// The values a composer's send, a post's actions and a toggle give their chain (<c>{form: …}</c>), and what a
/// <c>pick</c> input tells about its choice.
/// </summary>
public static class Form
{
    private static Val<T> V<T>(string name) => new("form", name);

    /// <summary>The text typed (a composer, a post's inline edit).</summary>
    public static Val<string> Text() => V<string>("text");

    /// <summary>The key of the item Reply was on (a composer with <c>.Reply()</c>).</summary>
    public static Val<string> Reply() => V<string>("reply");

    /// <summary>A dictated recording: a data URL.</summary>
    public static Val<string> Audio() => V<string>("audio");

    /// <summary>The recording's duration.</summary>
    public static Val<string> Duration() => V<string>("duration");

    /// <summary>The recording's language.</summary>
    public static Val<string> Language() => V<string>("language");

    /// <summary>The recording's timed words (JSON).</summary>
    public static Val<string> Words() => V<string>("words");

    /// <summary>A composer's checkbox (<c>.Option(name, ..)</c>).</summary>
    public static Val<bool> Option(string name) => V<bool>(name);

    /// <summary>The emoji of a reaction.</summary>
    public static Val<string> Emoji() => V<string>("emoji");

    /// <summary>The name an identity card's button is for.</summary>
    public static Val<string> To() => V<string>("to");

    /// <summary>A toggle's new state.</summary>
    public static Val<bool> Pressed() => V<bool>("pressed");

    /// <summary>Of a <c>pick</c> input: how many are chosen.</summary>
    public static Val<long> PickedCount<TReq, T>(Expression<Func<TReq, T>> field) => V<long>($"{FieldPath.Of(field)}.count");

    /// <summary>Of a <c>pick</c> input: how many of the chosen are agents.</summary>
    public static Val<long> PickedAgents<TReq, T>(Expression<Func<TReq, T>> field) => V<long>($"{FieldPath.Of(field)}.agents");

    /// <summary>Of a <c>pick</c> input: the first word of the first chosen label.</summary>
    public static Val<string> PickedFirst<TReq, T>(Expression<Func<TReq, T>> field) => V<string>($"{FieldPath.Of(field)}.first");

    /// <summary>Of a <c>pick</c> input: what is typed in its search (its <c>empty</c> text reads it).</summary>
    public static Val<string> PickedQuery<TReq, T>(Expression<Func<TReq, T>> field) => V<string>($"{FieldPath.Of(field)}.query");
}

// ---------------------------------------------------------------------------------------------- header

/// <summary>
/// A head of a screen's column or of a side panel (<see cref="Ui.Header"/>): a leading tile or faces, the title with a
/// glyph, a subtitle line of segments, a faces pill, text pills, icon buttons, a <c>…</c> menu, tabs, and the panel's
/// expand and close. On a phone a column's header is the shell's top bar; a panel's header is a back bar.
/// </summary>
public sealed class HeaderNode : Node
{
    private readonly List<object> _subtitle;
    private readonly List<object> _buttons;
    private readonly List<object> _icons;
    private UiObj? _phone;

    internal HeaderNode(Txt title) : base("header")
    {
        Body["title"] = title;
        _subtitle = Body.List("subtitle", true);
        _buttons = Body.List("buttons", true);
        _icons = Body.List("icons", true);
    }

    private HeaderNode Put(string key, object? value)
    {
        Body[key] = value;
        return this;
    }

    /// <summary>An icon name before the title.</summary>
    public HeaderNode TitleIcon<T>(Val<T> icon) => Put("title_icon", Val<T>.Json(icon));

    /// <summary>The icon the breadcrumb and the phone's top bar put before the screen's title.</summary>
    public HeaderNode CrumbIcon<T>(Val<T> icon) => Put("crumb_icon", Val<T>.Json(icon));

    /// <summary>An agent tile (the engine's mark with the glyph; drawn while the glyph is not empty).</summary>
    public HeaderNode LeadingTile<TA, TB>(Val<TA> icon, Val<TB> engine) =>
        Put("leading", new UiObj { ["tile"] = new UiObj { ["icon"] = Val<TA>.Json(icon), ["engine"] = Val<TB>.Json(engine) } });

    /// <summary>A stack of faces (a list of <c>name</c>, <c>avatar</c>, <c>agent</c>, <c>icon</c>, <c>engine</c>).</summary>
    public HeaderNode LeadingFaces<T>(Val<T> faces) => Put("leading", new UiObj { ["faces"] = Val<T>.Json(faces) });

    /// <summary>The leading's size in px.</summary>
    public HeaderNode LeadingSize(int px)
    {
        if (Body["leading"] is UiObj l) l["size"] = px;
        return this;
    }

    /// <summary>One line of segments joined with <c> · </c>; a segment with an empty value is left out.</summary>
    public HeaderNode Subtitle(params Segment[] segments)
    {
        _subtitle.Clear();
        _subtitle.AddRange(segments.Select(s => s.Obj));
        return this;
    }

    /// <summary>The subtitle line is a button running this action.</summary>
    public HeaderNode SubtitleAction(ActionRef action) => Put("subtitle_action", action.Name);

    /// <summary>A muted text after the title (a panel's <c># returns</c>).</summary>
    public HeaderNode Context(Txt context) => Put("context", context);

    /// <summary>A faces pill: the faces, the label (<c>9 members</c>), the action.</summary>
    public HeaderNode Faces<T>(Val<T> faces, Txt label, ActionRef action) =>
        Put("faces", new UiObj { ["value"] = Val<T>.Json(faces), ["label"] = label, ["action"] = action.Name });

    /// <summary>The faces pill's accessible name (absent: <c>{label}: open</c>). Call it after <c>Faces</c>.</summary>
    public HeaderNode FacesAriaLabel(Txt label)
    {
        if (Body["faces"] is UiObj f) f["aria_label"] = label;
        return this;
    }

    /// <summary>The faces pill is shown while the condition holds.</summary>
    public HeaderNode FacesWhen(Cond when)
    {
        if (Body["faces"] is UiObj f) f["visible"] = when;
        return this;
    }

    /// <summary>A text pill (at most four).</summary>
    public HeaderNode Button(Txt label, ActionRef action)
    {
        _buttons.Add(new UiObj { ["label"] = label, ["action"] = action.Name });
        return this;
    }

    /// <summary>The last text pill is shown while the condition holds.</summary>
    public HeaderNode ButtonWhen(Cond when)
    {
        if (_buttons.Count > 0) ((UiObj)_buttons[^1])["visible"] = when;
        return this;
    }

    /// <summary>Icon buttons (at most four; a toggle bell: <c>Ui.IconButton(..).Toggle(..)</c>).</summary>
    public HeaderNode Icons(params Node[] icons)
    {
        _icons.Clear();
        _icons.AddRange(icons);
        return this;
    }

    /// <summary>The <c>…</c> menu.</summary>
    public HeaderNode Menu(params MenuItem[] items)
    {
        var m = new UiObj();
        m.List("items").AddRange(items);
        return Put("menu", m);
    }

    /// <summary>The <c>…</c> button's and its menu's accessible label (absent: <c>More actions</c>). Call it after
    /// <c>Menu</c>.</summary>
    public HeaderNode MenuLabel(Txt label)
    {
        if (Body["menu"] is UiObj m) m["label"] = label;
        return this;
    }

    /// <summary>Tabs bound to a string state whose default is one of the ids.</summary>
    public HeaderNode Tabs(StateRef<string> state, params HeaderTab[] tabs) => TabsOf(new UiObj { ["state"] = state.Name }, tabs);

    /// <summary>Tabs bound to a string param (the tab in the link).</summary>
    public HeaderNode Tabs(ParamRef<string> param, params HeaderTab[] tabs) => TabsOf(new UiObj { ["param"] = param.Name }, tabs);

    private HeaderNode TabsOf(UiObj bind, HeaderTab[] tabs)
    {
        var t = new UiObj { ["bind"] = bind };
        t.List("items").AddRange(tabs);
        return Put("tabs", t);
    }

    /// <summary>The tab bar's accessible label (absent: the title). Call it after <c>Tabs</c>.</summary>
    public HeaderNode TabsLabel(Txt label)
    {
        if (Body["tabs"] is UiObj t) t["label"] = label;
        return this;
    }

    /// <summary>The enclosing split's expand button (desktop; <c>SplitNode.Expand(state)</c>).</summary>
    public HeaderNode Expand() => Put("expand", true);

    /// <summary>The panel's close button (desktop) and back button (phone).</summary>
    public HeaderNode Close(ActionRef action) => Put("close", action.Name);

    /// <summary>The phone back button's label.</summary>
    public HeaderNode Back(Txt label) => Put("back", label);

    private UiObj Phone()
    {
        if (_phone is null)
        {
            _phone = new UiObj();
            _phone.List("icons", true);
            Body["phone"] = _phone;
        }
        return _phone;
    }

    /// <summary>The line under the phone's top-bar title.</summary>
    public HeaderNode PhoneSub(Txt sub)
    {
        Phone()["sub"] = sub;
        return this;
    }

    /// <summary>Top-bar icon buttons on a phone: the actions of the header's icons or of its faces.</summary>
    public HeaderNode PhoneIcons(params ActionRef[] actions)
    {
        var icons = (List<object>)Phone()["icons"]!;
        icons.Clear();
        icons.AddRange(actions.Select(a => a.Name));
        return this;
    }

    /// <summary>The tabs first in the phone's <c>…</c> menu.</summary>
    public HeaderNode MenuTabs()
    {
        Phone()["menu_tabs"] = true;
        return this;
    }
}

/// <summary>
/// A piece of a header's subtitle: exactly one of a text (<see cref="Text"/>), a dot (<see cref="LiveDot"/>,
/// <see cref="WaitingDot"/>), a value with a format (<see cref="Countdown{T}"/>, <see cref="Elapsed{T}"/>) after an
/// optional <see cref="Prefix"/>, and a tone with no dot drawn (<see cref="LiveTone"/>, <see cref="WaitingTone"/>).
/// </summary>
public sealed class Segment
{
    internal UiObj Obj { get; } = new();

    private Segment() { }

    public static Segment Text(Txt text) => new() { Obj = { ["text"] = text } };

    /// <summary>The amber dot of something at work.</summary>
    public static Segment LiveDot() => new() { Obj = { ["dot"] = "live" } };

    /// <summary>The hollow dot of something waiting.</summary>
    public static Segment WaitingDot() => new() { Obj = { ["dot"] = "waiting" } };

    /// <summary>The line in the tone of something at work (amber), with no dot drawn.</summary>
    public static Segment LiveTone() => new() { Obj = { ["tone"] = "live" } };

    /// <summary>The line in the tone of something waiting, with no dot drawn.</summary>
    public static Segment WaitingTone() => new() { Obj = { ["tone"] = "waiting" } };

    /// <summary>The time left until a moment in ms (left out while empty).</summary>
    public static Segment Countdown<T>(Val<T> moment) =>
        new() { Obj = { ["value"] = Val<T>.Json(moment), ["format"] = Formats.Of("countdown") } };

    /// <summary>The time since a moment in ms.</summary>
    public static Segment Elapsed<T>(Val<T> moment) =>
        new() { Obj = { ["value"] = Val<T>.Json(moment), ["format"] = Formats.Of("elapsed") } };

    /// <summary>A text before the value, left out with it (<c>ends in </c>).</summary>
    public Segment Prefix(Txt prefix)
    {
        Obj["prefix"] = prefix;
        return this;
    }

    /// <summary>Shown only while the condition holds.</summary>
    public Segment Visible(Cond when)
    {
        Obj["visible"] = when;
        return this;
    }
}

/// <summary>A tab of a header (<see cref="Ui.HeaderTab"/>): selecting it sets the bound value to its id.</summary>
public sealed class HeaderTab : IUiJson
{
    private readonly UiObj _o = new();

    internal HeaderTab(string id, Txt title)
    {
        _o["id"] = id;
        _o["title"] = title;
    }

    /// <summary><c> · N</c> after the title while above 0.</summary>
    public HeaderTab Badge<T>(Val<T> count)
    {
        _o["badge"] = Val<T>.Json(count);
        return this;
    }

    /// <summary>Runs the action instead of selecting the tab.</summary>
    public HeaderTab Action(ActionRef action)
    {
        _o["action"] = action.Name;
        return this;
    }

    JsonNode IUiJson.ToJson(UiWriter w) => _o.ToJson(w);
}

// ---------------------------------------------------------------------------------------------- stream

/// <summary>
/// A live list of a query's rows (<see cref="Ui.Stream{TRes, TRow}"/>), the newest at the bottom by default: older pages
/// by the query's <c>Older</c>, day dividers, follow mode and jump to the latest, a tail query's rows merged after the
/// rows by key, a head row, an action when the newest row is seen, a post template per row.
/// </summary>
public sealed class StreamNode<TRes, TRow> : Node
{
    private readonly UiObj _item = new();

    internal StreamNode(string query) : base("stream")
    {
        Body["query"] = query;
        Body["key"] = "";
        _item["post"] = new PostTemplate<TRow>().Obj;
        Body["item"] = _item;
    }

    private StreamNode<TRes, TRow> Put(string key, object? value)
    {
        Body[key] = value;
        return this;
    }

    /// <summary>The row field that identifies an item (pages, the tail, <c>Merge</c> and <c>ScrollTo</c> go by it).</summary>
    public StreamNode<TRes, TRow> Key<TK>(Expression<Func<TRow, TK>> field) => Put("key", FieldPath.Of(field));

    /// <summary>The template of a row.</summary>
    public StreamNode<TRes, TRow> Post(Func<PostTemplate<TRow>, PostTemplate<TRow>> build)
    {
        _item["post"] = build(new PostTemplate<TRow>()).Obj;
        return this;
    }

    /// <summary>A template built elsewhere (<c>Ui.Post&lt;Msg&gt;()</c>, shared by two streams); it is copied as it is now.</summary>
    public StreamNode<TRes, TRow> Post(PostTemplate<TRow> template)
    {
        _item["post"] = template.Obj.Clone();
        return this;
    }

    /// <summary>Rows whose <paramref name="field"/> equals <paramref name="when"/> are one centred muted line: the author, the text, the time.</summary>
    public StreamNode<TRes, TRow> KindsLine<TK, TA, TX, TT>(Expression<Func<TRow, TK>> field, string when,
        Expression<Func<TRow, TA>> author, Expression<Func<TRow, TX>> text, Expression<Func<TRow, TT>> time) =>
        Put("kinds", new UiObj
        {
            ["field"] = FieldPath.Of(field),
            ["line"] = new UiObj
            {
                ["when"] = when,
                ["author"] = FieldPath.Of(author),
                ["text"] = FieldPath.Of(text),
                ["time"] = FieldPath.Of(time),
            },
        });

    /// <summary>
    /// Another query of the frame whose rows (of the same message) come after the rows (live items); a tail row whose key
    /// is among the rows is dropped, one gone from the next tail answer stays until the main query answers once more.
    /// </summary>
    public StreamNode<TRes, TRow> Tail<TRes2>(Q<TRes2, TRow> tail) => Put("tail", new UiObj { ["query"] = tail.Name, ["hold"] = true });

    /// <summary>A row of the answer before the rows (drawn with the template but its summary), and a rule under it.</summary>
    public StreamNode<TRes, TRow> Head(Expression<Func<TRes, TRow?>> row, Txt rule) =>
        Put("head", new UiObj { ["row"] = FieldPath.Of(row), ["rule"] = rule });

    /// <summary>A list read from the top (no follow mode).</summary>
    public StreamNode<TRes, TRow> AnchorTop() => Put("anchor", "top");

    /// <summary>Older pages by a button only, with this label.</summary>
    public StreamNode<TRes, TRow> OlderButton(Txt label)
    {
        Body["older_load"] = "button";
        return Put("older_label", label);
    }

    /// <summary>The label of the button that loads older pages.</summary>
    public StreamNode<TRes, TRow> OlderLabel(Txt label) => Put("older_label", label);

    /// <summary>A divider before each day (<c>Today</c>, <c>Yesterday</c>, the date) of this row field's moments.</summary>
    public StreamNode<TRes, TRow> Days<T>(Expression<Func<TRow, T>> field) => Put("days", FieldPath.Of(field));

    /// <summary>A <c>New</c> divider after the item with this key.</summary>
    public StreamNode<TRes, TRow> Unread<TK>(Val<TK> key) => Put("unread", Val<TK>.Json(key));

    /// <summary>The jump button's text when nothing new came (<c>Assistant is working</c>).</summary>
    public StreamNode<TRes, TRow> JumpWorking(Txt text) => Put("jump_working", text);

    public StreamNode<TRes, TRow> Empty(Txt text) => Put("empty", text);

    /// <summary>A muted line above the first item on a phone.</summary>
    public StreamNode<TRes, TRow> PhoneIntro(Txt text) => Put("phone_intro", text);

    /// <summary>Runs the action with the main query's newest row once the reader is at the end and that row changed.</summary>
    public StreamNode<TRes, TRow> OnSeen(ActionRef action) => Put("on_seen", new UiObj { ["action"] = action.Name });

    /// <summary>The <c>OnSeen</c> action runs only while this holds (read in the newest row).</summary>
    public StreamNode<TRes, TRow> OnSeenWhen(Cond when)
    {
        if (Body["on_seen"] is UiObj o) o["when"] = when;
        return this;
    }

    /// <summary>The name composers refer to (<c>Ui.Composer(..).For("messages")</c>).</summary>
    public StreamNode<TRes, TRow> Id(string id) => Put("id", id);

    /// <summary>The list's accessible name (<c>Messages</c>).</summary>
    public StreamNode<TRes, TRow> Label(Txt label) => Put("label", label);
}

// ---------------------------------------------------------------------------------------------- post

/// <summary>
/// The template of a stream's row (<see cref="Ui.Post{TRow}"/>, or <c>StreamNode.Post(p =&gt; ..)</c>): who wrote it and
/// when, its text and recording, marks and lines under it, a slot for a run of work, reactions, a summary of what
/// follows from it, and what can be done with it. Every field is a row field or path.
/// </summary>
public sealed class PostTemplate<TRow>
{
    internal UiObj Obj { get; }

    internal PostTemplate() : this(new UiObj { ["author"] = "", ["time"] = "" }) { }

    private PostTemplate(UiObj obj) { Obj = obj; }

    /// <summary>A copy to change apart from this one.</summary>
    public PostTemplate<TRow> Clone() => new(Obj.Clone());

    private static string P<T>(Expression<Func<TRow, T>> f) => FieldPath.Of(f);

    private PostTemplate<TRow> Put(string key, object? value)
    {
        Obj[key] = value;
        return this;
    }

    private UiObj Part(string key)
    {
        if (Obj[key] is not UiObj o)
        {
            o = new UiObj();
            Obj[key] = o;
        }
        return o;
    }

    public PostTemplate<TRow> Author<T>(Expression<Func<TRow, T>> field) => Put("author", P(field));

    public PostTemplate<TRow> Time<T>(Expression<Func<TRow, T>> field) => Put("time", P(field));

    /// <summary>The picture's URL (else the author's initials).</summary>
    public PostTemplate<TRow> Avatar<T>(Expression<Func<TRow, T>> field)
    {
        Part("avatar")["url"] = P(field);
        return this;
    }

    /// <summary>The agent tile, drawn while the glyph is not empty (or while <see cref="TileWhen"/> holds).</summary>
    public PostTemplate<TRow> Tile<TA, TB>(Expression<Func<TRow, TA>> icon, Expression<Func<TRow, TB>> engine)
    {
        Part("avatar")["tile"] = new UiObj { ["icon"] = P(icon), ["engine"] = P(engine) };
        return this;
    }

    /// <summary>The agent tile is drawn while this holds.</summary>
    public PostTemplate<TRow> TileWhen(Cond when)
    {
        if (Obj["avatar"] is UiObj { } a && a["tile"] is UiObj t) t["when"] = when;
        return this;
    }

    /// <summary>A tag after the name (<c>AGENT</c>) while the condition holds.</summary>
    public PostTemplate<TRow> Badge(Txt text, Cond when) => Put("badge", new UiObj { ["text"] = text, ["when"] = when });

    /// <summary>A quiet line after the name (<c>via Commerce</c>).</summary>
    public PostTemplate<TRow> Source<T>(Expression<Func<TRow, T>> field) => Put("source", P(field));

    /// <summary>The text as Markdown.</summary>
    public PostTemplate<TRow> Markdown<T>(Expression<Func<TRow, T>> field) => Put("text", new UiObj { ["field"] = P(field) });

    /// <summary>The text as plain text with <c>@[Name]</c> and <c>#[x]</c> chips (and the words lit while the recording plays).</summary>
    public PostTemplate<TRow> Plain<T>(Expression<Func<TRow, T>> field) =>
        Put("text", new UiObj { ["field"] = P(field), ["format"] = "plain" });

    /// <summary>A bool field: the text is being written (a caret).</summary>
    public PostTemplate<TRow> Writing<T>(Expression<Func<TRow, T>> field)
    {
        if (Obj["text"] is UiObj t) t["writing"] = P(field);
        return this;
    }

    /// <summary>A recording: its duration (s); <see cref="AudioMore"/> adds its label, URL and timed words.</summary>
    public PostTemplate<TRow> Audio<T>(Expression<Func<TRow, T>> duration) => Put("audio", new UiObj { ["duration"] = P(duration) });

    /// <summary>The recording's label, URL (a data URL) and timed words (<c>word</c>, <c>start</c>, <c>end</c>).</summary>
    public PostTemplate<TRow> AudioMore<TA, TB, TC>(Expression<Func<TRow, TA>> label, Expression<Func<TRow, TB>> url,
        Expression<Func<TRow, TC>> words)
    {
        if (Obj["audio"] is UiObj a)
        {
            a["label"] = P(label);
            a["url"] = P(url);
            a["words"] = P(words);
        }
        return this;
    }

    /// <summary>A bool field: <c>(edited)</c> after the text.</summary>
    public PostTemplate<TRow> Edited(Expression<Func<TRow, bool>> field) => Put("edited", P(field));

    /// <summary>A bool field: pinned; <paramref name="mark"/> true draws <c>Pinned</c> above the head.</summary>
    public PostTemplate<TRow> Pinned(Expression<Func<TRow, bool>> field, Val<bool> mark) =>
        Put("pinned", new UiObj { ["field"] = P(field), ["mark"] = Val<bool>.Json(mark) });

    /// <summary>A bool field: the viewer's own row (no Reply to it).</summary>
    public PostTemplate<TRow> Mine(Expression<Func<TRow, bool>> field) => Put("mine", P(field));

    /// <summary>A line under the text, its tone by a field's value (<c>("done", Tone.Ok)</c>); drawn while the route's <c>to</c> is empty.</summary>
    public PostTemplate<TRow> Receipt<TA, TB>(Expression<Func<TRow, TA>> text, Expression<Func<TRow, TB>> tone,
        params (string Value, Tone Tone)[] tones)
    {
        var t = new UiObj { ["field"] = P(tone) };
        if (tones.Length > 0)
        {
            var m = new UiObj();
            foreach (var (value, to) in tones.OrderBy(x => x.Value, StringComparer.Ordinal)) m[value] = UiJson.Name(to);
            t["tones"] = m;
        }
        return Put("receipt", new UiObj { ["text"] = P(text), ["tone"] = t });
    }

    /// <summary><c>→ &lt;to&gt; · &lt;status&gt;</c> with <paramref name="why"/> as its tooltip; drawn while <c>to</c> is not empty.</summary>
    public PostTemplate<TRow> Route<TA, TB, TC>(Expression<Func<TRow, TA>> to, Expression<Func<TRow, TB>> status,
        Expression<Func<TRow, TC>> why) =>
        Put("route", new UiObj { ["to"] = P(to), ["status"] = P(status), ["why"] = P(why) });

    /// <summary>
    /// A correction the writer may make: inline for 30 s after the row's moment <paramref name="from"/> (a draining
    /// button), then an entry of the menu and the sheet; drawn while <paramref name="label"/> is not empty.
    /// </summary>
    public PostTemplate<TRow> Correction<TA, TB>(Expression<Func<TRow, TA>> label, ActionRef action, Expression<Func<TRow, TB>> from) =>
        Put("correction", new UiObj
        {
            ["label"] = P(label),
            ["action"] = action.Name,
            ["from"] = P(from),
            ["window_ms"] = 30_000,
        });

    /// <summary>The correction's menu label, the words before it inline, its icon (<c>undo</c>, <c>arrow-right</c>).</summary>
    public PostTemplate<TRow> CorrectionWords<TA, TB, TC>(Expression<Func<TRow, TA>> menuLabel, Expression<Func<TRow, TB>> lead,
        Expression<Func<TRow, TC>> icon)
    {
        if (Obj["correction"] is UiObj c)
        {
            c["menu_label"] = P(menuLabel);
            c["lead"] = P(lead);
            c["icon"] = P(icon);
        }
        return this;
    }

    /// <summary>How long the correction stays inline (ms, at most 120000).</summary>
    public PostTemplate<TRow> CorrectionWindowMs(int ms)
    {
        if (Obj["correction"] is UiObj c) c["window_ms"] = ms;
        return this;
    }

    /// <summary>
    /// Reaction chips <c>emoji count</c> (pressed when <paramref name="mine"/>): the repeated field, each one's emoji and
    /// count; a click runs <paramref name="action"/> with <c>Form.Emoji()</c>.
    /// </summary>
    public PostTemplate<TRow> Reactions<TX, TE, TC>(Expression<Func<TRow, IEnumerable<TX>>> field, Expression<Func<TX, TE>> emoji,
        Expression<Func<TX, TC>> count, ActionRef action, Expression<Func<TX, bool>>? mine = null)
    {
        var r = new UiObj
        {
            ["field"] = P(field),
            ["emoji"] = FieldPath.Of(emoji),
            ["count"] = FieldPath.Of(count),
            ["mine"] = mine is null ? null : FieldPath.Of(mine),
            ["action"] = action.Name,
        };
        r.List("quick", true);
        return Put("reactions", r);
    }

    /// <summary>The field of a reaction that says it is the viewer's own (pressed): <c>(Reaction r) =&gt; r.Mine</c>.</summary>
    public PostTemplate<TRow> ReactionsMine<TX>(Expression<Func<TX, bool>> field)
    {
        if (Obj["reactions"] is UiObj r) r["mine"] = FieldPath.Of(field);
        return this;
    }

    /// <summary>The emoji the hover's React offers (at most eight).</summary>
    public PostTemplate<TRow> Quick(params string[] emoji)
    {
        if (Obj["reactions"] is UiObj r)
        {
            var quick = r.List("quick", true);
            quick.AddRange(emoji);
        }
        return this;
    }

    /// <summary>
    /// What follows from the item (replies): its count (<c>m =&gt; m.Thread!.Count</c>) and the action that opens it;
    /// <see cref="SummaryLabel"/> and <see cref="SummaryMore"/> add the rest. Drawn while the count is above 0 or someone
    /// works or waits.
    /// </summary>
    public PostTemplate<TRow> Summary<TC>(Expression<Func<TRow, TC>> count, ActionRef action) =>
        Put("summary", new UiObj { ["count"] = P(count), ["action"] = action.Name });

    /// <summary>The summary's label (<c>{count} replies</c>).</summary>
    public PostTemplate<TRow> SummaryLabel(Txt label)
    {
        if (Obj["summary"] is UiObj s) s["label"] = label;
        return this;
    }

    /// <summary>The summary's faces, its last line, who works (since a moment in ms) and who waits.</summary>
    public PostTemplate<TRow> SummaryMore<TA, TB, TC, TD, TE>(Expression<Func<TRow, TA>> faces, Expression<Func<TRow, TB>> last,
        Expression<Func<TRow, TC>> working, Expression<Func<TRow, TD>> since, Expression<Func<TRow, TE>> waiting)
    {
        if (Obj["summary"] is UiObj s)
        {
            s["faces"] = P(faces);
            s["last"] = P(last);
            s["working"] = P(working);
            s["since"] = P(since);
            s["waiting"] = P(waiting);
        }
        return this;
    }

    /// <summary>No summary (a pinned list's posts).</summary>
    public PostTemplate<TRow> NoSummary() => Put("summary", null);

    /// <summary>A run of work above the text: <paramref name="at"/> is the field holding it (<c>m =&gt; m.Work</c>).</summary>
    public PostTemplate<TRow> Process<TW>(Expression<Func<TRow, TW?>> at, Func<ProcessBuilder<TW>, ProcessBuilder<TW>> build)
        where TW : class => Put("process", build(new ProcessBuilder<TW>(P(at))).Obj);

    /// <summary>The rail on hover (desktop), in order.</summary>
    public PostTemplate<TRow> Hover(params PostEntry[] entries) => Entries("hover", entries);

    /// <summary>The <c>…</c> menu, in order.</summary>
    public PostTemplate<TRow> Menu(params PostEntry[] entries) => Entries("menu", entries);

    /// <summary>The phone's long-press sheet, in order.</summary>
    public PostTemplate<TRow> Sheet(params PostEntry[] entries) => Entries("sheet", entries);

    private PostTemplate<TRow> Entries(string key, PostEntry[] entries)
    {
        Obj.List(key, true).AddRange(entries.Select(e => e.Obj.Clone()));
        return this;
    }

    /// <summary>The inline editor of the text: <paramref name="action"/> reads <c>Form.Text()</c>; offered while the condition holds.</summary>
    public PostTemplate<TRow> Edit(ActionRef action, Cond when) => Put("edit", new UiObj { ["action"] = action.Name, ["when"] = when });

    /// <summary>While false: no rail, no menu, no sheet, no inline correction.</summary>
    public PostTemplate<TRow> ActionsWhen(Cond when) => Put("actions_when", when);

    /// <summary>The identity card a mention, the lead or the name opens.</summary>
    public PostTemplate<TRow> Card<TF>(IdentityCard<TF> card) => Put("card", card.Obj.Clone());
}

/// <summary>
/// An entry of a post's hover rail, menu or sheet: an action with its label (<see cref="Ui.Entry"/>) or a built-in
/// gesture (<see cref="Builtin"/>).
/// </summary>
public sealed class PostEntry
{
    internal UiObj Obj { get; } = new();

    internal PostEntry() { }

    public PostEntry Icon(string icon)
    {
        Obj["icon"] = icon;
        return this;
    }

    /// <summary>Drawn as a text button (the hover's <c>Ask agent</c>).</summary>
    public PostEntry TextStyle()
    {
        Obj["style"] = "text";
        return this;
    }

    /// <summary>Drawn as an icon button (the label is its tooltip).</summary>
    public PostEntry IconStyle()
    {
        Obj["style"] = "icon";
        return this;
    }

    public PostEntry Danger()
    {
        Obj["danger"] = true;
        return this;
    }

    /// <summary>Shown only while the condition holds.</summary>
    public PostEntry Visible(Cond when)
    {
        Obj["visible"] = when;
        return this;
    }
}

/// <summary>The built-in entries of a post: client gestures that need no plugin code (or the post's own options).</summary>
public static class Builtin
{
    private static PostEntry Of(string name) => new() { Obj = { ["builtin"] = name } };

    /// <summary>The quick reactions (needs <c>Reactions</c>).</summary>
    public static PostEntry React() => Of("react");

    /// <summary>Reply to the author (needs a composer with <c>.Reply()</c>).</summary>
    public static PostEntry Reply() => Of("reply");

    public static PostEntry CopyText() => Of("copy_text");

    public static PostEntry CopyLink() => Of("copy_link");

    /// <summary>Play or pause the recording (needs <c>Audio</c>).</summary>
    public static PostEntry Play() => Of("play");

    /// <summary>The inline editor (needs <c>Edit</c>).</summary>
    public static PostEntry Edit() => Of("edit");

    /// <summary>The correction after its inline window (needs <c>Correction</c>).</summary>
    public static PostEntry Correction() => Of("correction");

    /// <summary>The <c>…</c> button that opens the menu (the hover rail only).</summary>
    public static PostEntry More() => Of("more");

    public static PostEntry Separator() => Of("separator");
}

/// <summary>
/// An identity card (<see cref="Ui.CardOf{TRes, TF, T}"/>): a name looked up (case-insensitive) in a query's rows, with
/// its picture or agent tile and a line; its button runs an action with <c>Form.To()</c>.
/// </summary>
public sealed class IdentityCard<TF>
{
    internal UiObj Obj { get; } = new();

    internal IdentityCard(string query, string name)
    {
        Obj["query"] = query;
        Obj["name"] = name;
    }

    private IdentityCard<TF> Put(string key, object? value)
    {
        Obj[key] = value;
        return this;
    }

    public IdentityCard<TF> Avatar<T>(Expression<Func<TF, T>> field) => Put("avatar", FieldPath.Of(field));

    /// <summary>A bool field: the row is an agent.</summary>
    public IdentityCard<TF> Agent(Expression<Func<TF, bool>> field) => Put("agent", FieldPath.Of(field));

    public IdentityCard<TF> Tile<TA, TB>(Expression<Func<TF, TA>> icon, Expression<Func<TF, TB>> engine)
    {
        Obj["icon"] = FieldPath.Of(icon);
        return Put("engine", FieldPath.Of(engine));
    }

    public IdentityCard<TF> Detail<T>(Expression<Func<TF, T>> field) => Put("detail", FieldPath.Of(field));

    /// <summary>The card's button: <paramref name="action"/> reads <c>Form.To()</c>.</summary>
    public IdentityCard<TF> Action(ActionRef action) => Put("action", action.Name);

    /// <summary>What the card says of a name not in the rows.</summary>
    public IdentityCard<TF> Unknown(Txt text) => Put("unknown", text);
}

// ---------------------------------------------------------------------------------------------- process

/// <summary>
/// A run of work: as a post's slot (<c>p.Process(m =&gt; m.Work, w =&gt; w.State(x =&gt; x.State)..)</c>) or on its own
/// (<see cref="Ui.Process{TRes, TRow}"/>). Live (<c>starting</c>, <c>working</c>), done, or ended (<c>stopped</c>,
/// <c>failed</c>, <c>timed_out</c>, <c>gone</c>); no block while the state is empty. Its rows and calls have fixed field
/// names.
/// </summary>
public sealed class ProcessBuilder<TW>
{
    private readonly string? _at;

    internal UiObj Obj { get; } = new();

    internal ProcessBuilder(string? at)
    {
        _at = at;
        Obj["state"] = "";
    }

    private string P<T>(Expression<Func<TW, T>> f) => _at is null ? FieldPath.Of(f) : $"{_at}.{FieldPath.Of(f)}";

    private ProcessBuilder<TW> Put(string key, object? value)
    {
        Obj[key] = value;
        return this;
    }

    /// <summary><c>starting</c>, <c>working</c>, <c>done</c>, <c>stopped</c>, <c>failed</c>, <c>timed_out</c>, <c>gone</c>; no block while empty.</summary>
    public ProcessBuilder<TW> State<T>(Expression<Func<TW, T>> field) => Put("state", P(field));

    public ProcessBuilder<TW> Label<T>(Expression<Func<TW, T>> field) => Put("label", P(field));

    public ProcessBuilder<TW> Meta<T>(Expression<Func<TW, T>> field) => Put("meta", P(field));

    /// <summary>The moment (ms) the live block counts from.</summary>
    public ProcessBuilder<TW> Started<T>(Expression<Func<TW, T>> field) => Put("started", P(field));

    /// <summary>The rows: <c>kind</c>, <c>title</c>, <c>detail</c>, <c>state</c>, <c>time</c>, <c>at</c>, <c>calls</c>, <c>tools</c>.</summary>
    public ProcessBuilder<TW> Rows<T>(Expression<Func<TW, T>> field) => Put("rows", P(field));

    /// <summary>The number of steps (<c>View N steps</c>).</summary>
    public ProcessBuilder<TW> Steps<T>(Expression<Func<TW, T>> field) => Put("steps", P(field));

    /// <summary>The text being written (with a caret).</summary>
    public ProcessBuilder<TW> Saying<T>(Expression<Func<TW, T>> field) => Put("saying", P(field));

    /// <summary>Retry on an ended block: its action, a field with its label, while the condition holds.</summary>
    public ProcessBuilder<TW> Retry<T>(ActionRef action, Expression<Func<TW, T>> label, Cond when) =>
        Put("retry", new UiObj { ["action"] = action.Name, ["label"] = P(label), ["when"] = when });

    /// <summary><c>Open full log</c>.</summary>
    public ProcessBuilder<TW> Log(ActionRef action) => Put("log", new UiObj { ["action"] = action.Name });

    /// <summary>
    /// How a truncated call is read whole: <paramref name="query"/> read once more with <paramref name="args"/> and the
    /// request field <paramref name="call"/> set to the call's id (<c>(GetRunRequest r) =&gt; r.Call</c>); the answer's
    /// <paramref name="field"/> replaces the call.
    /// </summary>
    public ProcessBuilder<TW> FetchCall<TReq, TQRes, TQRow, TC, TF>(Q<TQRes, TQRow> query, Expression<Func<TReq, TC>> call,
        Expression<Func<TQRes, TF>> field, Action<Args<TReq>>? args = null)
    {
        var f = new UiObj { ["query"] = query.Name };
        f.List("args", true).AddRange(Args<TReq>.Build(args));
        f["call"] = FieldPath.Of(call);
        f["field"] = FieldPath.Of(field);
        return Put("fetch_call", f);
    }
}

// ---------------------------------------------------------------------------------------------- strip

/// <summary>
/// A pinned list of live items (<see cref="Ui.Strip{TRes, TX, TRow, TK, TT}"/>): a tile, the title, a dot (live,
/// waiting), <c>Starting</c> or the time since a moment, a sub line (and the time left of a waiting item), a jump to the
/// item's place in a stream and buttons. Hidden while there are no rows.
/// </summary>
public sealed class StripNode<TRow> : Node
{
    private readonly List<object> _buttons;

    internal StripNode(string query, string? rows, string key, string title) : base("strip")
    {
        Body["query"] = query;
        Body["rows"] = rows;
        Body["key"] = key;
        Body["title"] = title;
        _buttons = Body.List("buttons", true);
    }

    private StripNode<TRow> Put(string key, object? value)
    {
        Body[key] = value;
        return this;
    }

    /// <summary>Above the items (<c>2 live runs</c>); none while empty.</summary>
    public StripNode<TRow> Label(Txt label) => Put("label", label);

    public StripNode<TRow> Tile<TA, TB>(Expression<Func<TRow, TA>> icon, Expression<Func<TRow, TB>> engine) =>
        Put("tile", new UiObj { ["icon"] = FieldPath.Of(icon), ["engine"] = FieldPath.Of(engine) });

    /// <summary>Which values of the field are live, waiting and starting.</summary>
    public StripNode<TRow> State<T>(Expression<Func<TRow, T>> field, string[] live, string[] waiting, string[] starting)
    {
        var s = new UiObj { ["field"] = FieldPath.Of(field) };
        s.List("live", true).AddRange(live);
        s.List("waiting", true).AddRange(waiting);
        s.List("starting", true).AddRange(starting);
        return Put("state", s);
    }

    /// <summary>The moment (ms) an item counts from.</summary>
    public StripNode<TRow> Since<T>(Expression<Func<TRow, T>> field) => Put("since", FieldPath.Of(field));

    /// <summary>The moment (ms) a waiting item counts down to.</summary>
    public StripNode<TRow> Until<T>(Expression<Func<TRow, T>> field) => Put("until", FieldPath.Of(field));

    public StripNode<TRow> SubLine<T>(Expression<Func<TRow, T>> field) => Put("sub", FieldPath.Of(field));

    /// <summary>After a waiting item's sub line, with <c>{left}</c> the countdown (<c>ends in {left}</c>).</summary>
    public StripNode<TRow> CountdownText(Txt text) => Put("countdown_text", text);

    /// <summary>The tile and the title run it (with the row).</summary>
    public StripNode<TRow> Open(ActionRef action) => Put("open", action.Name);

    /// <summary>The jump to the item's place: <c>ScrollTo</c> the key in <paramref name="field"/>; its labels by state.</summary>
    public StripNode<TRow> Jump<T>(Expression<Func<TRow, T>> field, Txt label, Txt labelWaiting) =>
        Put("jump", new UiObj { ["field"] = FieldPath.Of(field), ["label"] = label, ["label_waiting"] = labelWaiting });

    public StripNode<TRow> Button(StripButton button)
    {
        _buttons.Add(button.Obj.Clone());
        return this;
    }

    public StripNode<TRow> AriaLabel(Txt label) => Put("aria_label", label);
}

/// <summary>A button of a strip's item (<see cref="Ui.StripButton"/>).</summary>
public sealed class StripButton
{
    internal UiObj Obj { get; } = new();

    internal StripButton(Txt label, ActionRef action)
    {
        Obj["label"] = label;
        Obj["action"] = action.Name;
    }

    public StripButton Icon(string icon)
    {
        Obj["icon"] = icon;
        return this;
    }

    /// <summary>The red-outlined Stop with the filled square.</summary>
    public StripButton StopStyle()
    {
        Obj["style"] = "stop";
        return this;
    }

    /// <summary>A muted text button.</summary>
    public StripButton TextStyle()
    {
        Obj["style"] = "text";
        return this;
    }

    public StripButton Visible(Cond when)
    {
        Obj["visible"] = when;
        return this;
    }
}

// ---------------------------------------------------------------------------------------------- composer

/// <summary>A tool of a composer's box.</summary>
public enum ComposerTool
{
    Add,
    Emoji,
    Attach,
}

/// <summary>
/// A message box (<see cref="Ui.Composer"/>): its send runs with <c>Form.Text()</c>, <c>Form.Reply()</c>, the recording
/// (<c>Form.Audio()</c>, …) and each option (<c>Form.Option(name)</c>); mentions over a directory, tags, dictation, a
/// reply reference, caption lines, Send turned Stop, a read-only reason.
/// </summary>
public sealed class ComposerNode : Node
{
    private readonly List<object> _tools;
    private readonly List<object> _options;
    private readonly List<object> _caption;

    internal ComposerNode(ActionRef send) : base("composer")
    {
        Body["send"] = send.Name;
        _tools = Body.List("tools", true);
        _options = Body.List("options", true);
        _caption = Body.List("caption", true);
    }

    private ComposerNode Put(string key, object? value)
    {
        Body[key] = value;
        return this;
    }

    /// <summary>The <c>Id</c> of the stream it belongs to (jumps to its end on send; <c>Draft</c> and <c>ReplyTo</c> from its items).</summary>
    public ComposerNode For(string streamId) => Put("for", streamId);

    public ComposerNode Placeholder(Txt placeholder) => Put("placeholder", placeholder);

    /// <summary>Owns ⌥ Space and the steps of rows outside a stream; one per frame.</summary>
    public ComposerNode Primary() => Put("primary", true);

    /// <summary>The panel's box.</summary>
    public ComposerNode Compact() => Put("compact", true);

    public ComposerNode Tools(params ComposerTool[] tools)
    {
        _tools.Clear();
        _tools.AddRange(tools.Select(t => UiJson.Name(t)));
        return this;
    }

    /// <summary>The mention picker's directory.</summary>
    public ComposerNode Mentions<TF>(MentionsDirectory<TF> mentions) => Put("mentions", mentions.Obj.Clone());

    /// <summary><c>#word</c> becomes <c>#[word]</c> on send.</summary>
    public ComposerNode Tags() => Put("tags", true);

    /// <summary>The Dictate button, shown when the workspace has a speech connection.</summary>
    public ComposerNode Dictation()
    {
        if (Body["dictation"] is not UiObj) Body["dictation"] = new UiObj();
        return this;
    }

    /// <summary>Words for the recogniser besides the mentionable names.</summary>
    public ComposerNode Vocabulary(Txt words)
    {
        Dictation();
        ((UiObj)Body["dictation"]!)["vocabulary"] = words;
        return this;
    }

    /// <summary>Takes <c>ReplyTo</c>: <c>Form.Reply()</c> is the key.</summary>
    public ComposerNode Reply() => Put("reply", true);

    /// <summary>A checkbox under the box (<c>Form.Option(name)</c>).</summary>
    public ComposerNode Option(string name, Txt label)
    {
        _options.Add(new UiObj { ["name"] = name, ["label"] = label });
        return this;
    }

    /// <summary>The last option is shown while the condition holds.</summary>
    public ComposerNode OptionWhen(Cond when)
    {
        if (_options.Count > 0) ((UiObj)_options[^1])["visible"] = when;
        return this;
    }

    /// <summary>A caption line naming who will answer (<c>↳ …</c>); the first caption with a value is drawn.</summary>
    public ComposerNode CaptionRoute(Txt value)
    {
        _caption.Add(new UiObj { ["value"] = value, ["icon"] = "corner-down-right", ["style"] = "route" });
        return this;
    }

    /// <summary>A plain caption line.</summary>
    public ComposerNode CaptionDot(Txt value)
    {
        _caption.Add(new UiObj { ["value"] = value, ["style"] = "dot" });
        return this;
    }

    /// <summary>While the condition holds and the box is empty, Send is a labelled Stop running the action (and Esc runs it).</summary>
    public ComposerNode Stop(ActionRef action, Cond when) => Put("stop", new UiObj { ["action"] = action.Name, ["when"] = when });

    public ComposerNode StopLabel(Txt label)
    {
        if (Body["stop"] is UiObj s) s["label"] = label;
        return this;
    }

    /// <summary>The Stop button's accessible name (absent: its label). Call it after <c>Stop</c>.</summary>
    public ComposerNode StopAriaLabel(Txt label)
    {
        if (Body["stop"] is UiObj s) s["aria_label"] = label;
        return this;
    }

    /// <summary>While the condition holds, the reason stands where the box would be.</summary>
    public ComposerNode Readonly(Cond when, Txt reason) => Put("readonly", new UiObj { ["when"] = when, ["reason"] = reason });

    /// <summary>The text is mirrored into this state, for a query that reads what is typed (with <c>DebounceMs</c>).</summary>
    public ComposerNode DraftState(StateRef<string> state) => Put("draft_state", state.Name);
}

/// <summary>
/// The mention picker's directory (<see cref="Ui.Mentions{TRes, TF, T}"/>): rows of a query in the order given, grouped by
/// a section on a desktop, with a note in the footer.
/// </summary>
public sealed class MentionsDirectory<TF>
{
    internal UiObj Obj { get; } = new();

    internal MentionsDirectory(string query, string? rows, string name)
    {
        Obj["query"] = query;
        Obj["rows"] = rows;
        Obj["name"] = name;
    }

    private MentionsDirectory<TF> Put(string key, object? value)
    {
        Obj[key] = value;
        return this;
    }

    public MentionsDirectory<TF> Avatar<T>(Expression<Func<TF, T>> field) => Put("avatar", FieldPath.Of(field));

    /// <summary>A bool field: the row is an agent.</summary>
    public MentionsDirectory<TF> Agent(Expression<Func<TF, bool>> field) => Put("agent", FieldPath.Of(field));

    public MentionsDirectory<TF> Tile<TA, TB>(Expression<Func<TF, TA>> icon, Expression<Func<TF, TB>> engine)
    {
        Obj["icon"] = FieldPath.Of(icon);
        return Put("engine", FieldPath.Of(engine));
    }

    public MentionsDirectory<TF> Detail<T>(Expression<Func<TF, T>> field) => Put("detail", FieldPath.Of(field));

    /// <summary>The rows are grouped by this field on a desktop.</summary>
    public MentionsDirectory<TF> Section<T>(Expression<Func<TF, T>> field) => Put("section", FieldPath.Of(field));

    /// <summary>The picker's footer note.</summary>
    public MentionsDirectory<TF> Note(Txt note) => Put("note", note);
}

// ---------------------------------------------------------------------------------------------- the catalog's constructors

public static partial class Ui
{
    /// <summary>
    /// A head of a screen's column or of a side panel (catalog 3):
    /// <c>Ui.Header(q.Field(v =&gt; v.Title)).Tabs(tab, Ui.HeaderTab("pinned", "Pinned")).Close(close)</c>.
    /// </summary>
    public static HeaderNode Header(Txt title) => new(title);

    /// <summary>A tab of a header: <c>Ui.HeaderTab("pinned", "Pinned").Badge(q.Field(v =&gt; v.PinnedCount))</c>.</summary>
    public static HeaderTab HeaderTab(string id, Txt title) => new(id, title);

    /// <summary>A live list of a query's rows (catalog 3): <c>Ui.Stream(msgs).Key(m =&gt; m.Id).Post(p =&gt; ..).Days(m =&gt; m.Time)</c>.</summary>
    public static StreamNode<TRes, TRow> Stream<TRes, TRow>(Q<TRes, TRow> rows) => new(rows.Name);

    /// <summary>A post template to share between streams: <c>Ui.Post&lt;Msg&gt;().Author(..)</c>.</summary>
    public static PostTemplate<TRow> Post<TRow>() => new();

    /// <summary>An entry of a post's rail, menu or sheet running an action on the row: <c>Ui.Entry(openThread, "Reply in thread")</c>.</summary>
    public static PostEntry Entry(ActionRef action, Txt label) => new() { Obj = { ["action"] = action.Name, ["label"] = label } };

    /// <summary>An identity card: a name (<paramref name="name"/>) looked up in a query's rows.</summary>
    public static IdentityCard<TF> CardOf<TRes, TF, T>(Q<TRes, TF> rows, Expression<Func<TF, T>> name) =>
        new(rows.Name, FieldPath.Of(name));

    /// <summary>A run of work on its own, over a query's answer (catalog 3): <c>Ui.Process(run, w =&gt; w.State(r =&gt; r.State)..)</c>.</summary>
    public static Node Process<TRes, TRow>(Q<TRes, TRow> query, Func<ProcessBuilder<TRes>, ProcessBuilder<TRes>> build)
    {
        var p = build(new ProcessBuilder<TRes>(null)).Obj;
        var n = new Node("process");
        n.Body["query"] = query.Name;
        foreach (var key in new[] { "state", "label", "meta", "started", "rows", "steps", "saying", "retry", "log", "fetch_call" })
            n.Body[key] = p[key];
        return n;
    }

    /// <summary>
    /// A pinned list of live items over a repeated field of a query's answer (catalog 3), each keyed by
    /// <paramref name="key"/> and titled by <paramref name="title"/>:
    /// <c>Ui.Strip(live, v =&gt; v.Runs, r =&gt; r.Run, r =&gt; r.Agent).State(..).Button(..)</c>.
    /// </summary>
    public static StripNode<TRow> Strip<TRes, TX, TRow, TK, TT>(Q<TRes, TX> query, Expression<Func<TRes, IEnumerable<TRow>>> rows,
        Expression<Func<TRow, TK>> key, Expression<Func<TRow, TT>> title) =>
        new(query.Name, FieldPath.Of(rows), FieldPath.Of(key), FieldPath.Of(title));

    /// <summary>A strip over the query's own rows.</summary>
    public static StripNode<TRow> Strip<TRes, TRow, TK, TT>(Q<TRes, TRow> query, Expression<Func<TRow, TK>> key,
        Expression<Func<TRow, TT>> title) => new(query.Name, null, FieldPath.Of(key), FieldPath.Of(title));

    /// <summary>A button of a strip's item: <c>Ui.StripButton("Stop", stop).Icon("square").StopStyle()</c>.</summary>
    public static StripButton StripButton(Txt label, ActionRef action) => new(label, action);

    /// <summary>
    /// A message box (catalog 3): <c>Ui.Composer(send).For("messages").Primary().Mentions(..).Reply()</c>; the action
    /// reads <c>Form.Text()</c>, <c>Form.Reply()</c>, the recording and each option.
    /// </summary>
    public static ComposerNode Composer(ActionRef send) => new(send);

    /// <summary>The mention picker's directory: <c>Ui.Mentions(people, f =&gt; f.Name).Tile(f =&gt; f.Icon, f =&gt; f.Engine)</c>.</summary>
    public static MentionsDirectory<TF> Mentions<TRes, TF, T>(Q<TRes, TF> rows, Expression<Func<TF, T>> name) =>
        new(rows.Name, null, FieldPath.Of(name));

    /// <summary>The same over a repeated field of the query's answer other than its rows.</summary>
    public static MentionsDirectory<TF> Mentions<TRes, TX, TF, T>(Q<TRes, TX> query, Expression<Func<TRes, IEnumerable<TF>>> rows,
        Expression<Func<TF, T>> name) => new(query.Name, FieldPath.Of(rows), FieldPath.Of(name));
}
