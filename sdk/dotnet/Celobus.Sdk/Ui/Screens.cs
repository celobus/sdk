// Screens and overlays (their params, state, queries, actions, header and layout), action chains, and forms.
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>
/// The builder a screen's or overlay's function gets: params from the params message <typeparamref name="TParams"/>,
/// state, queries, actions, overlays, the header; the function returns the layout.
/// </summary>
public sealed class ScreenBuilder<TParams> where TParams : class
{
    internal List<JsonObject> Params { get; } = [];
    internal List<JsonObject> StateList { get; } = [];
    internal List<JsonObject> Queries { get; } = [];
    internal List<object> Actions { get; } = [];
    internal List<object> Overlays { get; } = [];
    internal List<object> HeaderNodes { get; } = [];
    internal Txt? TitleText { get; private set; }
    internal Txt? SubtitleText { get; private set; }
    internal Txt? KickerText { get; private set; }
    internal Size SizeValue { get; private set; } = Celobus.Sdk.Ui.Size.Medium;
    private int _counter;

    internal ScreenBuilder()
    {
        foreach (var f in MessageSchema.For(typeof(TParams)).Fields)
            Params.Add(new JsonObject
            {
                ["name"] = f.Name,
                ["type"] = f.DescriptorType(""),
                ["required"] = !f.Optional && !f.Repeated,
            });
    }

    public ScreenBuilder<TParams> Title(Txt title)
    {
        TitleText = title;
        return this;
    }

    public ScreenBuilder<TParams> Subtitle(Txt subtitle)
    {
        SubtitleText = subtitle;
        return this;
    }

    /// <summary>A small line above an overlay's title (<c>COMMERCE · ORDERS</c>).</summary>
    public ScreenBuilder<TParams> Kicker(Txt kicker)
    {
        KickerText = kicker;
        return this;
    }

    /// <summary>An overlay's size (default medium).</summary>
    public ScreenBuilder<TParams> Size(Size size)
    {
        SizeValue = size;
        return this;
    }

    /// <summary>A param of the params message (a filter, a record id).</summary>
    public ParamRef<T> Param<T>(Expression<Func<TParams, T>> field) => new(FieldPath.Of(field));

    /// <summary>
    /// A param with a default: a screen opened without it behaves as if the link had the default (the queries get it,
    /// a <c>Segmented</c> or a tab bound to it shows it selected), and the link stays without it. The param always has
    /// a value, so the handle is of the plain type: <c>var range = s.ParamOr(p =&gt; p.Range, "30");</c>.
    /// </summary>
    public ParamRef<T> ParamOr<T>(Expression<Func<TParams, T?>> field, T initial) where T : notnull =>
        Defaulted<T>(FieldPath.Of(field), UiJson.Literal(initial));

    /// <summary>The same for an optional field of a value type (an enum, a number): the handle is of the inner type.</summary>
    public ParamRef<T> ParamOr<T>(Expression<Func<TParams, T?>> field, T initial) where T : struct =>
        Defaulted<T>(FieldPath.Of(field), UiJson.Literal(initial));

    private ParamRef<T> Defaulted<T>(string name, JsonNode? initial)
    {
        if (Params.FirstOrDefault(p => (string?)p["name"] == name) is { } param)
        {
            param["required"] = false;
            param["default"] = initial;
        }
        return new ParamRef<T>(name);
    }

    /// <summary>State with its default.</summary>
    public StateRef<T> State<T>(string name, T initial)
    {
        var o = new JsonObject { ["name"] = name, ["type"] = UiJson.TypeName(typeof(T)) };
        if (UiJson.Literal(initial) is { } d) o["default"] = d;
        StateList.Add(o);
        return new StateRef<T>(name);
    }

    /// <summary>
    /// State that returns to its default when one of <paramref name="reset"/> changes (catalog 3; a tab that starts over
    /// in another conversation): <c>s.StateReset("tab", "messages", channel)</c>.
    /// </summary>
    public StateRef<T> StateReset<T>(string name, T initial, params IParamRef[] reset)
    {
        var r = State(name, initial);
        if (reset.Length > 0) StateList[^1]["reset"] = new JsonArray([.. reset.Select(p => (JsonNode)p.Name)]);
        return r;
    }

    /// <summary>A named query of this screen or overlay.</summary>
    public Q<TRes, TRow> Query<TReq, TRes, TRow>(string name, QueryDef<TReq, TRes, TRow> query)
    {
        Queries.Add(query.Build(name));
        return new Q<TRes, TRow>(name);
    }

    /// <summary>A chain of steps run by a button, a menu item or a row open.</summary>
    public ActionRef Action(string name, Action<Chain> steps)
    {
        var c = new Chain();
        steps(c);
        return Push(name, c);
    }

    /// <summary>A chain that reads the row it is run on (<c>(a, r) =&gt; a.Navigate(.., p =&gt; p.Arg(x =&gt; x.Id, r.Get(o =&gt; o.Id)))</c>).</summary>
    public ActionRef RowAction<TRes, TRow>(string name, Q<TRes, TRow> rows, Action<Chain, Row<TRow>> steps)
    {
        var c = new Chain();
        steps(c, new Row<TRow>());
        return Push(name, c);
    }

    private ActionRef Push(string name, Chain c)
    {
        Actions.Add(c.ToAction(name));
        return new ActionRef(name);
    }

    /// <summary>Buttons and menus in the header (a screen) or the footer (a dialog).</summary>
    public ScreenBuilder<TParams> Header(params Node[] nodes)
    {
        HeaderNodes.AddRange(nodes);
        return this;
    }

    /// <summary>An overlay this screen opens (<c>a.Open(Edit, ..)</c>).</summary>
    public ScreenBuilder<TParams> Overlay(Overlay overlay)
    {
        Overlays.Add(overlay.Obj);
        return this;
    }

    /// <summary>A form filling <paramref name="endpoint"/>'s request; its submit runs the chain <paramref name="submit"/> builds (<c>a.CallForm(endpoint, ..)</c>).</summary>
    public Node Form<TReq, TRes>(EndpointRef<TReq, TRes> endpoint, Action<FormBuilder<TReq>> inputs, Txt submitLabel,
        Action<Chain, FormVals<TReq>> submit) where TReq : class where TRes : class
    {
        var fb = new FormBuilder<TReq>();
        inputs(fb);
        UiObj? alternate = null;
        if (fb.AlternateButton is { } alt)
        {
            _counter++;
            var altName = $"alternate_{_counter}";
            Push(altName, alt.Chain);
            alternate = new UiObj { ["label"] = alt.Label, ["action"] = altName };
        }
        _counter++;
        var name = $"submit_{_counter}";
        var c = new Chain();
        submit(c, new FormVals<TReq>());
        Push(name, c);
        var node = new Node("form");
        var body = node.Body;
        body["endpoint"] = endpoint.Name;
        body["sections"] = fb.Sections.Where(s => s.Inputs.Count > 0).Select(s => s.ToObj()).ToList<object>();
        body["submit"] = name;
        body["submit_label"] = submitLabel;
        body["phone_submit_label"] = fb.PhoneSubmitLabel;
        body["cancel_label"] = fb.CancelLabel;
        body["summary"] = fb.SummaryObj;
        body["note"] = fb.NoteText;
        body["alternate"] = alternate;
        body.List("notes", true).AddRange(fb.Notes);
        return node;
    }

    internal UiObj Common(string id, Node layout)
    {
        var o = new UiObj
        {
            ["id"] = id,
            ["title"] = TitleText ?? UiJson.LabelOf(id),
        };
        o["subtitle"] = SubtitleText;
        o.List("params", true).AddRange(Params);
        o.List("state", true).AddRange(StateList);
        o.List("queries", true).AddRange(Queries);
        o.List("actions", true).AddRange(Actions);
        o["layout"] = layout;
        return o;
    }
}

/// <summary>A built screen: <c>Screen.Build(Screens.Orders, s =&gt; { ...; return Ui.Stack(..); })</c>.</summary>
public sealed class Screen
{
    internal UiObj Obj { get; }

    private Screen(UiObj obj) { Obj = obj; }

    public static Screen Build<TParams>(ScreenKey<TParams> key, Func<ScreenBuilder<TParams>, Node> build) where TParams : class
    {
        var s = new ScreenBuilder<TParams>();
        var layout = build(s);
        var o = s.Common(key.Id, layout);
        o.List("overlays", true).AddRange(s.Overlays);
        o.List("header", true).AddRange(s.HeaderNodes);
        return new Screen(o);
    }
}

/// <summary>A built overlay: a dialog, a side panel or a sheet.</summary>
public sealed class Overlay
{
    internal UiObj Obj { get; }

    private Overlay(UiObj obj) { Obj = obj; }

    private static Overlay Build<TParams>(OverlayKey<TParams> key, string kind, Func<ScreenBuilder<TParams>, Node> build)
        where TParams : class
    {
        var s = new ScreenBuilder<TParams>();
        var layout = build(s);
        var o = s.Common(key.Id, layout);
        o["kind"] = kind;
        o["size"] = UiJson.Name(s.SizeValue);
        o["kicker"] = s.KickerText;
        o.List("buttons", true).AddRange(s.HeaderNodes);
        return new Overlay(o);
    }

    public static Overlay Dialog<TParams>(OverlayKey<TParams> key, Func<ScreenBuilder<TParams>, Node> build) where TParams : class =>
        Build(key, "dialog", build);

    public static Overlay SidePanel<TParams>(OverlayKey<TParams> key, Func<ScreenBuilder<TParams>, Node> build) where TParams : class =>
        Build(key, "side_panel", build);

    public static Overlay Sheet<TParams>(OverlayKey<TParams> key, Func<ScreenBuilder<TParams>, Node> build) where TParams : class =>
        Build(key, "sheet", build);
}

/// <summary>A confirmation before the next steps: <c>new Confirm("Archive it?")</c> or <c>Confirm.Danger("Cancel this order?")</c>.</summary>
public sealed class Confirm
{
    internal UiObj Obj { get; } = new();

    public Confirm(Txt title) { Obj["title"] = title; }

    public static Confirm Danger(Txt title)
    {
        var c = new Confirm(title);
        c.Obj["danger"] = true;
        return c;
    }

    public Confirm Text(Txt text)
    {
        Obj["text"] = text;
        return this;
    }

    public Confirm ConfirmLabel(Txt label)
    {
        Obj["confirm_label"] = label;
        return this;
    }

    /// <summary>The member types this value to confirm.</summary>
    public Confirm Typed<T>(Val<T> value)
    {
        Obj["typed"] = Val<T>.Json(value);
        return this;
    }

    public Confirm CancelLabel(Txt label)
    {
        Obj["cancel_label"] = label;
        return this;
    }

    /// <summary>What is affected, as a record line with an icon (<c>Order #1048 · Hartwell &amp; Co · $2,480.00</c>).</summary>
    public Confirm Summary(string icon, Txt summary)
    {
        Obj["summary"] = summary;
        Obj["summary_icon"] = icon;
        return this;
    }
}

/// <summary>A chain of steps: <c>a.Confirm(..).Call(..).Toast(..).Close()</c>; <c>OnError</c> for the failure branch.</summary>
public sealed class Chain
{
    private readonly List<object> _steps = [];
    private List<object> _onError = [];
    private UiObj? _lastCall;

    internal Chain() { }

    private Chain Step(string kind, object body)
    {
        _steps.Add(new UiObj { [kind] = body });
        return this;
    }

    internal UiObj ToAction(string name)
    {
        var o = new UiObj { ["name"] = name };
        o.List("steps").AddRange(_steps);
        o.List("on_error", true).AddRange(_onError);
        return o;
    }

    public Chain Confirm(Confirm confirm) => Step("confirm", confirm.Obj);

    private Chain CallStep(string endpoint, List<JsonObject> args, bool form)
    {
        var call = new UiObj { ["endpoint"] = endpoint };
        call.List("args", true).AddRange(args);
        if (form) call["form"] = true;
        _lastCall = call;
        return Step("call", call);
    }

    /// <summary>Calls a <c>ui</c> endpoint as the member.</summary>
    public Chain Call<TReq, TRes>(EndpointRef<TReq, TRes> endpoint, Action<Args<TReq>>? args = null)
        where TReq : class where TRes : class => CallStep(endpoint.Name, Args<TReq>.Build(args), false);

    /// <summary>Calls with the enclosing form's values (and <paramref name="args"/> over them).</summary>
    public Chain CallForm<TReq, TRes>(EndpointRef<TReq, TRes> endpoint, Action<Args<TReq>>? args = null)
        where TReq : class where TRes : class => CallStep(endpoint.Name, Args<TReq>.Build(args), true);

    /// <summary>The last call shows a pending row in <paramref name="rows"/> until a re-read returns it.</summary>
    public Chain Pending<TRes, TRow>(Q<TRes, TRow> rows)
    {
        if (_lastCall is not null && _steps.Count > 0 && ((UiObj)_steps[^1])["call"] == _lastCall)
            _lastCall["pending"] = rows.Name;
        return this;
    }

    public Chain Toast(Txt text) => Step("toast", new UiObj { ["text"] = text });

    public Chain Toast(Txt text, Tone tone) => Step("toast", new UiObj { ["text"] = text, ["tone"] = UiJson.Name(tone) });

    /// <summary>In <c>OnError</c>: the failing call's own message.</summary>
    public Chain ToastError() =>
        Step("toast", new UiObj { ["tone"] = UiJson.Name(Tone.Danger), ["error"] = true });

    private UiObj? LastToast() => _steps.Count > 0 && ((UiObj)_steps[^1])["toast"] is UiObj t ? t : null;

    /// <summary>The last toast gets a second line.</summary>
    public Chain Described(Txt description)
    {
        if (LastToast() is { } t) t["description"] = description;
        return this;
    }

    /// <summary>The last toast gets a button running <paramref name="action"/> (<c>Undo</c>).</summary>
    public Chain WithButton(Txt label, ActionRef action)
    {
        if (LastToast() is { } t) t["action"] = new UiObj { ["label"] = label, ["action"] = action.Name };
        return this;
    }

    public Chain Open<TParams>(OverlayKey<TParams> overlay, Action<Args<TParams>>? args = null) where TParams : class =>
        Step("open", Target.Of(null, overlay.Id, Args<TParams>.Build(args)));

    public Chain Close() => Step("close", new JsonObject());

    public Chain Navigate<TParams>(ScreenKey<TParams> screen, Action<Args<TParams>>? args = null) where TParams : class =>
        Step("navigate", Target.Of(screen.Id, null, Args<TParams>.Build(args)));

    public Chain Back() => Step("back", new JsonObject());

    public Chain Set<T>(StateRef<T> state, Val<T> value) =>
        Step("set", new JsonObject { ["state"] = state.Name, ["value"] = Val<T>.Json(value) });

    public Chain SetParam<T>(ParamRef<T> param, Val<T> value) =>
        Step("set", new JsonObject { ["param"] = param.Name, ["value"] = Val<T>.Json(value) });

    /// <summary>Re-run these queries (none: every query of the screen).</summary>
    public Chain Refresh(params string[] queries)
    {
        var o = new JsonObject();
        if (queries.Length > 0) o["queries"] = new JsonArray([.. queries.Select(q => (JsonNode)q)]);
        return Step("refresh", o);
    }

    public Chain OpenUrl(Val<string> url) => Step("open_url", new JsonObject { ["value"] = Val<string>.Json(url) });

    /// <summary>
    /// Downloads the file the last call answered in <paramref name="field"/> (a <c>File</c> field of its answer, not a
    /// list), then goes on: <c>a.Call(ExportOrders.Ref, ..).Download((ExportOrdersReply r) =&gt; r.File)</c>. The
    /// person's browser saves it; an answer without the file stops the chain like a failed step (<c>OnError</c> runs).
    /// </summary>
    public Chain Download<TRes>(Expression<Func<TRes, Celobus.Sdk.File?>> field) => Step("download", FieldPath.Of(field));

    /// <summary>
    /// Puts the text into the composer of the item's stream (or the frame's primary composer), focused, the caret at the
    /// end (catalog 3): <c>a.Draft(Ui.Text("ask", "@{who} about this: ").Arg("who", r.Get(who.F(p =&gt; p.Name))))</c>.
    /// </summary>
    public Chain Draft(Txt text) => Step("draft", new UiObj { ["text"] = text });

    /// <summary>
    /// Adds <c>@&lt;name&gt; </c> to that composer (unless there), focuses it, and sends <paramref name="key"/> as its
    /// <c>Form.Reply()</c> while the mention stays in the text (catalog 3).
    /// </summary>
    public Chain ReplyTo<TK>(Val<string> name, Val<TK> key) =>
        Step("reply_to", new JsonObject { ["name"] = Val<string>.Json(name), ["key"] = Val<TK>.Json(key) });

    /// <summary>Scrolls the stream that holds an item with this key so the item is centred (catalog 3).</summary>
    public Chain ScrollTo<TK>(Val<TK> key) => Step("scroll_to", new JsonObject { ["key"] = Val<TK>.Json(key) });

    /// <summary>Copies a value to the clipboard (catalog 3).</summary>
    public Chain Copy<T>(Val<T> value) => Step("copy", new JsonObject { ["value"] = Val<T>.Json(value) });

    /// <summary>Copies the screen's link with <c>#&lt;key&gt;</c> (catalog 3).</summary>
    public Chain CopyLink<TK>(Val<TK> key) => Step("copy", new JsonObject { ["link"] = Val<TK>.Json(key) });

    /// <summary>
    /// Puts a row into the loaded rows of a query (by the stream's key) until the query's next answer (catalog 3):
    /// <c>a.Call(Post.Ref, ..).Merge(messages, Ui.Result((Posted p) =&gt; p.Message))</c>.
    /// </summary>
    public Chain Merge<TRes, TRow, T>(Q<TRes, TRow> query, Val<T> row) =>
        Step("merge", new JsonObject { ["query"] = query.Name, ["row"] = Val<T>.Json(row) });

    /// <summary>The steps run when a step fails.</summary>
    public Chain OnError(Action<Chain> steps)
    {
        var c = new Chain();
        steps(c);
        _onError = c._steps;
        return this;
    }
}

/// <summary>A form or filter input; set its options after adding it (<c>f.Text(r =&gt; r.Name, "Name").Required()</c>).</summary>
public sealed class Input : IUiJson
{
    private readonly UiObj _o = new();
    internal List<object> Options { get; }
    internal List<object> Fields { get; }

    internal Input(string name, string control, Txt label)
    {
        _o["name"] = name;
        _o["control"] = control;
        _o["label"] = label;
        Options = _o.List("options", true);
        Fields = _o.List("fields", true);
    }

    JsonNode IUiJson.ToJson(UiWriter w) => _o.ToJson(w);

    public Input Required()
    {
        _o["required"] = true;
        return this;
    }

    public Input Help(Txt help)
    {
        _o["help"] = help;
        return this;
    }

    public Input Placeholder(Txt placeholder)
    {
        _o["placeholder"] = placeholder;
        return this;
    }

    public Input Initial<T>(Val<T> value)
    {
        _o["initial"] = Val<T>.Json(value);
        return this;
    }

    /// <summary>A literal option of a select.</summary>
    public Input Option<T>(T value, Txt label)
    {
        Options.Add(new UiObj { ["value"] = UiJson.Literal(value), ["label"] = label });
        return this;
    }

    internal Input OptionsFrom(string query, string value, string label)
    {
        _o["options_from"] = new JsonObject { ["query"] = query, ["value"] = value, ["label"] = label };
        return this;
    }

    internal Input Set(string key, object? value)
    {
        _o[key] = value;
        return this;
    }

    internal Input InitialJson(JsonNode value)
    {
        _o["initial"] = value;
        return this;
    }

    internal Input MinItems(int min)
    {
        _o["min_items"] = min;
        return this;
    }

    /// <summary>A filter chip's icon.</summary>
    public Input Icon(string icon)
    {
        _o["icon"] = icon;
        return this;
    }

    /// <summary>Shares its line with the next half input.</summary>
    public Input Half()
    {
        _o["width"] = "half";
        return this;
    }

    /// <summary>A number with − and + buttons.</summary>
    public Input Stepper()
    {
        _o["stepper"] = true;
        return this;
    }

    /// <summary><c>Repeat</c>: a value at the end of each row from the i-th row of <paramref name="rows"/> (a line total from a quote).</summary>
    public Input Aside<TRes, TRow, TV>(Txt title, Q<TRes, TRow> rows, Expression<Func<TRow, TV>> field, string moneyIn)
    {
        _o["aside"] = new UiObj
        {
            ["title"] = title,
            ["query"] = rows.Name,
            ["field"] = FieldPath.Of(field),
            ["format"] = Formats.Money(null, moneyIn),
        };
        return this;
    }

    /// <summary><c>Repeat</c>: the add row's label.</summary>
    public Input AddLabel(Txt label)
    {
        _o["add_label"] = label;
        return this;
    }
}

/// <summary>The inputs of one row of a <see cref="FormBuilder{TReq}.Repeat"/>.</summary>
public sealed class RowForm<TItem>
{
    internal List<object> Inputs { get; } = [];

    internal RowForm() { }

    private Input Push<TV>(Expression<Func<TItem, TV>> field, string control, Txt label)
    {
        var i = new Input(FieldPath.Of(field), control, label);
        Inputs.Add(i);
        return i;
    }

    /// <summary>A select from a query's rows.</summary>
    public RowForm<TItem> SelectFrom<TV, TRes, TRow, TA, TL>(Expression<Func<TItem, TV>> field, Txt label, Q<TRes, TRow> rows,
        Expression<Func<TRow, TA>> value, Expression<Func<TRow, TL>> shown)
    {
        Push(field, "select", label).Required().OptionsFrom(rows.Name, FieldPath.Of(value), FieldPath.Of(shown));
        return this;
    }

    /// <summary>A number with − and + buttons, starting at <paramref name="initial"/>.</summary>
    public RowForm<TItem> Stepper<TV>(Expression<Func<TItem, TV>> field, Txt label, long initial)
    {
        // a number literal, as the Rust SDK writes it (an int64 value would be a string)
        Push(field, "number", label).Stepper().Required().InitialJson(UiJson.One("lit", JsonValue.Create(initial)));
        return this;
    }
}

/// <summary>The inputs of a form over <typeparamref name="TReq"/>, in sections.</summary>
public sealed class FormBuilder<TReq>
{
    internal sealed class FormSection(Txt? title)
    {
        public Txt? Title { get; } = title;
        public List<object> Inputs { get; } = [];

        public UiObj ToObj()
        {
            var o = new UiObj { ["title"] = Title };
            o.List("inputs").AddRange(Inputs);
            return o;
        }
    }

    internal List<FormSection> Sections { get; } = [new(null)];
    internal Txt? CancelLabel { get; private set; }
    internal Txt? PhoneSubmitLabel { get; private set; }
    internal UiObj? SummaryObj { get; private set; }
    internal Txt? NoteText { get; private set; }
    internal (Txt Label, Chain Chain)? AlternateButton { get; private set; }
    internal List<object> Notes { get; } = [];

    internal FormBuilder() { }

    private Input Add<T>(Expression<Func<TReq, T>> field, string control, Txt label)
    {
        var i = new Input(FieldPath.Of(field), control, label);
        Sections[^1].Inputs.Add(i);
        return i;
    }

    /// <summary>Starts a titled section.</summary>
    public FormBuilder<TReq> Section(Txt title)
    {
        Sections.Add(new FormSection(title));
        return this;
    }

    /// <summary>The cancel button's label.</summary>
    public FormBuilder<TReq> Cancel(Txt label)
    {
        CancelLabel = label;
        return this;
    }

    /// <summary>The submit button's label on a phone, where the form is a sheet (absent: the submit label).</summary>
    public FormBuilder<TReq> PhoneSubmit(Txt label)
    {
        PhoneSubmitLabel = label;
        return this;
    }

    public Input Text<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "text", label);

    public Input Textarea<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "textarea", label);

    public Input Number<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "number", label);

    public Input Money<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "money", label);

    public Input Date<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "date", label);

    public Input Checkbox(Expression<Func<TReq, bool>> field, Txt label) => Add(field, "checkbox", label);

    public Input Switch(Expression<Func<TReq, bool>> field, Txt label) => Add(field, "switch", label);

    public Input Member<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "member", label);

    /// <summary>
    /// A write-only text drawn as a password field: a settings <see cref="Sdk.Secret"/>, or a string an endpoint takes (a
    /// value already saved is never shown).
    /// </summary>
    public Input Secret<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "secret", label);

    /// <summary>A select that takes several of an enum's values (a list of them).</summary>
    public Input MultiSelect<T>(Expression<Func<TReq, IEnumerable<T>>> field, Txt label)
    {
        var i = Add(field, "multi_select", label);
        i.Options.AddRange(UiJson.EnumOptions(typeof(T)));
        return i;
    }

    /// <summary>
    /// A drop zone and a picker for a <c>File</c> or <c>List&lt;File&gt;</c> field, within its <c>[MaxSize]</c> and
    /// <c>[Accept]</c>: files upload as they are picked and their ids go with the form; an edit form shows the
    /// record's files and sends the kept ones back.
    /// </summary>
    public Input Upload<T>(Expression<Func<TReq, T>> field, Txt label) => Add(field, "upload", label);

    /// <summary>A select over an enum's values (or <c>.Option(..)</c>).</summary>
    public Input Select<T>(Expression<Func<TReq, T>> field, Txt label)
    {
        var i = Add(field, "select", label);
        i.Options.AddRange(UiJson.EnumOptions(typeof(T)));
        return i;
    }

    /// <summary>Radio cards over an enum's values.</summary>
    public Input Radio<T>(Expression<Func<TReq, T>> field, Txt label)
    {
        var i = Add(field, "radio_cards", label);
        i.Options.AddRange(UiJson.EnumOptions(typeof(T)));
        return i;
    }

    /// <summary>A select whose options come from a query's rows: <paramref name="value"/> is the option's value, <paramref name="shown"/> its label.</summary>
    public Input SelectFrom<T, TRes, TRow, TV, TL>(Expression<Func<TReq, T>> field, Txt label, Q<TRes, TRow> rows,
        Expression<Func<TRow, TV>> value, Expression<Func<TRow, TL>> shown) =>
        Add(field, "select", label).OptionsFrom(rows.Name, FieldPath.Of(value), FieldPath.Of(shown));

    /// <summary>Rows of a repeated message field (<c>Items</c>: product, quantity per row).</summary>
    public Input Repeat<TItem>(Expression<Func<TReq, IEnumerable<TItem>>> field, Txt label, Action<RowForm<TItem>> row)
    {
        var rf = new RowForm<TItem>();
        row(rf);
        var i = Add(field, "repeat", label);
        i.Fields.AddRange(rf.Inputs);
        return i.MinItems(1);
    }

    /// <summary>A total under the inputs, in a currency.</summary>
    public FormBuilder<TReq> Summary<T>(Txt title, Txt? caption, Val<T> value, string moneyIn)
    {
        SummaryObj = new UiObj
        {
            ["title"] = title,
            ["caption"] = caption,
            ["value"] = Val<T>.Json(value),
            ["format"] = Formats.Money(null, moneyIn),
        };
        return this;
    }

    /// <summary>
    /// Chips over a directory (catalog 3): the rows of <paramref name="rows"/>, each chosen by <paramref name="value"/>
    /// and drawn with <paramref name="title"/>; the field gets the chosen values joined with <c>, </c>. Conditions and
    /// texts read <c>Form.PickedCount(..)</c>, <c>Form.PickedAgents(..)</c> and <c>Form.PickedFirst(..)</c>.
    /// </summary>
    public PickInput<TRow> Pick<T, TRes, TRow, TV, TL>(Expression<Func<TReq, T>> field, Txt label, Q<TRes, TRow> rows,
        Expression<Func<TRow, TV>> value, Expression<Func<TRow, TL>> title) =>
        new(Add(field, "pick", label), rows.Name, null, FieldPath.Of(value), FieldPath.Of(title));

    /// <summary>The same over a repeated field of the query's answer other than its rows.</summary>
    public PickInput<TRow> Pick<T, TRes, TQRow, TRow, TV, TL>(Expression<Func<TReq, T>> field, Txt label,
        Q<TRes, TQRow> query, Expression<Func<TRes, IEnumerable<TRow>>> rows, Expression<Func<TRow, TV>> value,
        Expression<Func<TRow, TL>> title) =>
        new(Add(field, "pick", label), query.Name, FieldPath.Of(rows), FieldPath.Of(value), FieldPath.Of(title));

    /// <summary>A line under the inputs, shown while <paramref name="when"/> (over the form's values) holds (catalog 3).</summary>
    public FormBuilder<TReq> NoteWhen(Txt text, Cond when)
    {
        Notes.Add(new UiObj { ["text"] = text, ["visible"] = when });
        return this;
    }

    /// <summary>A line next to the buttons.</summary>
    public FormBuilder<TReq> Note(Txt note)
    {
        NoteText = note;
        return this;
    }

    /// <summary>A second button that sends the values another way (<c>Save draft</c>): its chain, like the submit's.</summary>
    public FormBuilder<TReq> Alternate(Txt label, Action<Chain> steps)
    {
        var c = new Chain();
        steps(c);
        AlternateButton = (label, c);
        return this;
    }
}

/// <summary>A <c>pick</c> input (<see cref="FormBuilder{TReq}.Pick{T, TRes, TRow, TV, TL}"/>): its directory's fields and texts.</summary>
public sealed class PickInput<TRow>
{
    private readonly UiObj _pick = new();

    internal PickInput(Input input, string query, string? rows, string value, string label)
    {
        Input = input;
        _pick["query"] = query;
        _pick["rows"] = rows;
        _pick["value"] = value;
        _pick["label"] = label;
        input.Set("pick", _pick);
    }

    /// <summary>The input itself.</summary>
    public Input Input { get; }

    private PickInput<TRow> Put(string key, object? value)
    {
        _pick[key] = value;
        return this;
    }

    /// <summary>A row field: the picture.</summary>
    public PickInput<TRow> Avatar<T>(Expression<Func<TRow, T>> field) => Put("avatar", FieldPath.Of(field));

    /// <summary>A bool row field: the row is an agent (drawn with its tile; counted by <c>Form.PickedAgents</c>).</summary>
    public PickInput<TRow> Agent(Expression<Func<TRow, bool>> field) => Put("agent", FieldPath.Of(field));

    /// <summary>The agent tile: the glyph and the engine fields.</summary>
    public PickInput<TRow> Tile<TA, TB>(Expression<Func<TRow, TA>> icon, Expression<Func<TRow, TB>> engine)
    {
        _pick["icon"] = FieldPath.Of(icon);
        return Put("engine", FieldPath.Of(engine));
    }

    public PickInput<TRow> Detail<T>(Expression<Func<TRow, T>> field) => Put("detail", FieldPath.Of(field));

    /// <summary>The rows are grouped by this field.</summary>
    public PickInput<TRow> Section<T>(Expression<Func<TRow, T>> field) => Put("section", FieldPath.Of(field));

    /// <summary>A list value of values not offered (the members already in).</summary>
    public PickInput<TRow> Exclude<T>(Val<T> values) => Put("exclude", Val<T>.Json(values));

    /// <summary>The line when nothing matches the typed text, and when every row is chosen.</summary>
    public PickInput<TRow> Empty(Txt nothing, Txt all)
    {
        _pick["empty"] = nothing;
        return Put("empty_done", all);
    }

    public PickInput<TRow> Placeholder(Txt placeholder)
    {
        Input.Placeholder(placeholder);
        return this;
    }

    public PickInput<TRow> Help(Txt help)
    {
        Input.Help(help);
        return this;
    }

    public PickInput<TRow> Required()
    {
        Input.Required();
        return this;
    }
}
