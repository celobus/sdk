//! Catalog revision 3: the app-like primitives. A `stream` of
//! `post`s with a `process` slot, a `strip` of live items, a `composer`, a `header`, the `pick` input; options of
//! `split` (`fill`, `resize`, `expand`), `list`, `icon_button` (a toggle), `agent_run` (the log) and state (`reset`);
//! the steps `draft`, `reply_to`, `scroll_to`, `copy`, `merge`; the query options `older`, `debounce_ms`, `join`.
//!
//! ```ignore
//! let who = Via::<Msg, Face>::new("who");
//! let msgs = s.query("channel", Query::call(get_channel).arg(GetChannelRequest::id, c.or_empty())
//!     .rows(ChannelView::messages).older(ChannelView::older_page_token, GetChannelRequest::page_token)
//!     .join(&who, ChannelView::people, Face::id, Msg::by));
//! stream(&msgs).key(Msg::id)
//!     .post(|p| p.author(who.f(Face::name)).avatar(who.f(Face::avatar)).tile(who.f(Face::icon), who.f(Face::engine))
//!         .badge("AGENT", row_at(who.f(Face::agent)).eq(true)).time(Msg::time).markdown(Msg::text)
//!         .reactions(Msg::reactions, Reaction::emoji, Reaction::count, &react).quick(["👍", "🔥"])
//!         .hover([builtin::react(), builtin::reply(), entry(&open_thread, "Reply in thread"), builtin::more()]))
//!     .days(Msg::time).on_seen(&mark_read).id("messages")
//! ```

use std::marker::PhantomData;

use super::*;
use celobus_descriptor::ui as d;

// ---------------------------------------------------------------------------------------------- row paths

/// A path into a row: a nested field (`Msg::thread.then(Summary::count)` is `thread.count`) or a field of a query's
/// join (`who.f(Face::name)` is `who.name`). Templates take it wherever they take a field.
pub struct PathOf<R, T> {
    path: String,
    _p: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Clone for PathOf<R, T> {
    fn clone(&self) -> Self {
        Self::of(self.path.clone())
    }
}

impl<R, T> PathOf<R, T> {
    fn of(path: String) -> Self {
        Self {
            path,
            _p: PhantomData,
        }
    }

    /// A field of the message this path leads to.
    pub fn then<M, U>(self, f: Field<M, U>) -> PathOf<R, U>
    where
        T: Holds<M>,
    {
        PathOf::of(format!("{}.{}", self.path, f.name))
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

/// What a message field holds: the message itself, or an optional or repeated one.
pub trait Holds<M> {}
impl<M: Message> Holds<M> for M {}
impl<M: Message> Holds<M> for Option<M> {}
impl<M: Message> Holds<M> for Vec<M> {}

impl<R, T> Field<R, T> {
    /// A field of the message this field holds: `Msg::thread.then(Summary::count)`.
    pub fn then<M, U>(self, f: Field<M, U>) -> PathOf<R, U>
    where
        T: Holds<M>,
    {
        PathOf::of(format!("{}.{}", self.name, f.name))
    }
}

/// A field or a path of a row of `R`, with the type `T` it reads.
pub trait RowPath<R> {
    type T;
    fn row_path(&self) -> String;
}

impl<R, T> RowPath<R> for Field<R, T> {
    type T = T;
    fn row_path(&self) -> String {
        self.name.into()
    }
}

impl<R, T> RowPath<R> for PathOf<R, T> {
    type T = T;
    fn row_path(&self) -> String {
        self.path.clone()
    }
}

impl<R, T> RowPath<R> for &PathOf<R, T> {
    type T = T;
    fn row_path(&self) -> String {
        self.path.clone()
    }
}

/// A row value by path (a joined or nested field), for conditions and texts.
pub fn row_at<R, P: RowPath<R>>(p: P) -> Val<P::T> {
    Val::raw(Value::Row(p.row_path()))
}

impl<R> Row<R> {
    /// A row value by path (`r.at(who.f(Face::agent))`).
    pub fn at<P: RowPath<R>>(&self, p: P) -> Val<P::T> {
        row_at(p)
    }
}

/// A lookup of a query's rows (`.join(&who, ..)`): the rows of `R` read the item of `J` as `who.<field>`.
pub struct Via<R, J> {
    name: &'static str,
    _p: PhantomData<fn() -> (R, J)>,
}

impl<R, J> Clone for Via<R, J> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R, J> Copy for Via<R, J> {}

impl<R, J> Via<R, J> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _p: PhantomData,
        }
    }

    /// A field of the looked-up item, as a path of the row.
    pub fn f<T>(&self, f: Field<J, T>) -> PathOf<R, T> {
        PathOf::of(format!("{}.{}", self.name, f.name))
    }
}

// ---------------------------------------------------------------------------------------------- queries, state

impl<Req, Resp, Row> QueryDef<Req, Resp, Row> {
    /// The page before the rows: the same endpoint read with `arg` set to the answer's `token`.
    pub fn older<T, A>(mut self, token: Field<Resp, T>, arg: Field<Req, A>) -> Self {
        self.q.older = Some(d::Older {
            token: token.name.into(),
            arg: arg.name.into(),
        });
        self
    }

    /// Arguments that change wait this long before the read (a preview of what is typed; at most 2000).
    pub fn debounce_ms(mut self, ms: u32) -> Self {
        self.q.debounce_ms = Some(ms);
        self
    }

    /// Each row reads the item of `rows` whose `key` equals its `by` as `<via>.<field>`.
    pub fn join<J, K>(
        mut self,
        via: &Via<Row, J>,
        rows: Field<Resp, Vec<J>>,
        key: Field<J, K>,
        by: Field<Row, K>,
    ) -> Self {
        self.q.join.push(d::Join {
            name: via.name.into(),
            rows: rows.name.into(),
            key: key.name.into(),
            by: by.name.into(),
        });
        self
    }
}

impl<T> ParamRef<T> {
    /// The param's name (for a state's `reset`).
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl<P: Message> S<P> {
    /// State that returns to its default when one of these params changes (a tab that starts over in another
    /// conversation): `s.state_reset("tab", "messages".to_string(), &[c.name()])`.
    pub fn state_reset<T: FieldValue>(
        &mut self,
        name: &str,
        default: T,
        params: &[&str],
    ) -> StateRef<T> {
        let r = self.state(name, default);
        if let Some(s) = self.scope.state.last_mut() {
            s.reset = params.iter().map(|p| p.to_string()).collect();
        }
        r
    }
}

// ---------------------------------------------------------------------------------------------- steps, form values

impl Chain {
    /// Puts the text into the composer of the item's stream (or the primary composer), focused.
    pub fn draft(&mut self, t: impl Into<Txt>) -> &mut Self {
        self.steps.push(Step::Draft { text: txt(t) });
        self
    }

    /// Adds `@<name> ` to the composer and sends `key` as its `reply` while the mention stays.
    pub fn reply_to<K>(&mut self, name: impl IntoVal<String>, key: Val<K>) -> &mut Self {
        self.steps.push(Step::ReplyTo {
            name: name.into_value(),
            key: key.v,
        });
        self
    }

    /// Scrolls the stream holding an item with this key so the item is centred.
    pub fn scroll_to<K>(&mut self, key: Val<K>) -> &mut Self {
        self.steps.push(Step::ScrollTo { key: key.v });
        self
    }

    /// Copies a value to the clipboard.
    pub fn copy<T>(&mut self, v: Val<T>) -> &mut Self {
        self.steps.push(Step::CopyValue(d::CopyValue {
            value: Some(v.v),
            link: None,
        }));
        self
    }

    /// Copies the screen's link with `#<key>`.
    pub fn copy_link<T>(&mut self, key: Val<T>) -> &mut Self {
        self.steps.push(Step::CopyValue(d::CopyValue {
            value: None,
            link: Some(key.v),
        }));
        self
    }

    /// Puts a row into the loaded rows of a query (by the stream's key) until its next answer:
    /// `a.call(post_message, ..).merge(&msgs, result(Posted::message))`.
    pub fn merge<Resp, R, T: Holds<R>>(&mut self, q: &Q<Resp, R>, row: Val<T>) -> &mut Self {
        self.steps.push(Step::Merge {
            query: q.name.clone(),
            row: row.v,
        });
        self
    }
}

/// The values a composer's `send`, a post's actions and a toggle give their chain (`{form: …}`).
pub mod form {
    use super::{Val, Value};

    fn v<T>(name: &str) -> Val<T> {
        Val::raw(Value::Form(name.into()))
    }

    /// The text typed (a composer, a post's inline edit).
    pub fn text() -> Val<String> {
        v("text")
    }
    /// The key of the item Reply was on (a composer with `reply`).
    pub fn reply() -> Val<String> {
        v("reply")
    }
    /// A dictated recording: a data URL, its duration, language and timed words (JSON).
    pub fn audio() -> Val<String> {
        v("audio")
    }
    pub fn duration() -> Val<String> {
        v("duration")
    }
    pub fn language() -> Val<String> {
        v("language")
    }
    pub fn words() -> Val<String> {
        v("words")
    }
    /// A composer's checkbox.
    pub fn option(name: &str) -> Val<bool> {
        v(name)
    }
    /// The emoji of a reaction.
    pub fn emoji() -> Val<String> {
        v("emoji")
    }
    /// The name an identity card's button is for.
    pub fn to() -> Val<String> {
        v("to")
    }
    /// A toggle's new state.
    pub fn pressed() -> Val<bool> {
        v("pressed")
    }
}

/// Of a `pick` input: how many are chosen, how many of them are agents, the first word of the first label.
pub fn picked_count<Req, T>(f: Field<Req, T>) -> Val<i64> {
    Val::raw(Value::Form(format!("{}.count", f.name)))
}

pub fn picked_agents<Req, T>(f: Field<Req, T>) -> Val<i64> {
    Val::raw(Value::Form(format!("{}.agents", f.name)))
}

pub fn picked_first<Req, T>(f: Field<Req, T>) -> Val<String> {
    Val::raw(Value::Form(format!("{}.first", f.name)))
}

/// What is typed in a pick's search (its `empty` text reads it: `Nobody matches “{q}”.`).
pub fn picked_query<Req, T>(f: Field<Req, T>) -> Val<String> {
    Val::raw(Value::Form(format!("{}.query", f.name)))
}

// ---------------------------------------------------------------------------------------------- formats, options

/// Formats of catalog 3 on a text or a metric.
pub trait LiveFormatted: Formatted {
    /// Counts up from a moment in ms (`0:48`, `1:02:05`).
    fn elapsed(mut self) -> Self {
        self.set_format(Format::Elapsed {});
        self
    }
    /// Counts down to a moment in ms (`8 min`).
    fn countdown(mut self) -> Self {
        self.set_format(Format::Countdown {});
        self
    }
    /// A duration in ms (`1m 05s`).
    fn span(mut self) -> Self {
        self.set_format(Format::Span {});
        self
    }
}

impl<F: Formatted> LiveFormatted for F {}

impl N {
    /// A split as an app-like screen: the viewport's height, each side scrolling itself, the detail an in-page
    /// side panel.
    pub fn fill(mut self) -> Self {
        if let Kind::Split(s) = &mut self.0.kind {
            s.fill = true;
        }
        self
    }

    /// The split's panel width is dragged by its border: px from `min` to `max`, `default` at first, the column
    /// keeping at least `keep`.
    pub fn resize(mut self, min: u32, max: u32, default: u32, keep: u32) -> Self {
        if let Kind::Split(s) = &mut self.0.kind {
            s.resize = Some(d::Resize {
                min,
                max,
                default,
                keep: Some(keep),
            });
        }
        self
    }

    /// The bool state the detail's header's expand button toggles.
    pub fn expand(mut self, state: &StateRef<bool>) -> Self {
        if let Kind::Split(s) = &mut self.0.kind {
            s.expand = Some(d::StateName {
                state: state.name.clone(),
            });
        }
        self
    }

    /// An icon button as a toggle: `pressed` says whether it is on; the icon and label given to `icon_button`
    /// are the pressed ones, these the others. Its action reads `form::pressed()`.
    pub fn toggle(mut self, pressed: Val<bool>, icon_off: &str, label_off: impl Into<Txt>) -> Self {
        if let Kind::IconButton(b) = &mut self.0.kind {
            b.pressed = Some(pressed.v);
            b.icon_off = Some(icon_off.into());
            b.label_off = Some(txt(label_off));
        }
        self
    }
}

impl<R> ListB<R> {
    /// A filter box over the rows' titles (no request).
    pub fn search(mut self, placeholder: impl Into<Txt>) -> Self {
        self.l.search = Some(txt(placeholder));
        self
    }

    /// A green dot after the title while this field is true.
    pub fn online(mut self, f: Field<R, bool>) -> Self {
        self.l.online = Some(f.name.into());
        self
    }

    /// On a phone, the first `n` rows, then `Show all`.
    pub fn limit(mut self, n: u32) -> Self {
        self.l.limit = Some(n);
        self
    }

    /// On a phone, a label over the rows in a card (`People · 9`).
    pub fn group_label(mut self, t: impl Into<Txt>) -> Self {
        self.l.group_label = Some(txt(t));
        self
    }
}

impl AgentRunB {
    /// The run's full log in a panel (the head, every step, the note).
    pub fn log(mut self) -> Self {
        self.0.log = true;
        self
    }

    /// The agent's tile in the head: fields of the response with the glyph and the engine.
    pub fn tile<Resp, A, B>(mut self, icon: Field<Resp, A>, engine: Field<Resp, B>) -> Self {
        self.0.icon = Some(icon.name.into());
        self.0.engine = Some(engine.name.into());
        self
    }

    pub fn status_label<Resp, T>(mut self, f: Field<Resp, T>) -> Self {
        self.0.status_label = Some(f.name.into());
        self
    }

    pub fn step_label<Resp, T>(mut self, f: Field<Resp, T>) -> Self {
        self.0.step_label = Some(f.name.into());
        self
    }

    pub fn elapsed<Resp, T>(mut self, f: Field<Resp, T>) -> Self {
        self.0.elapsed = Some(f.name.into());
        self
    }

    /// The line under the steps while the run is not live.
    pub fn note<Resp, T>(mut self, f: Field<Resp, T>) -> Self {
        self.0.note = Some(f.name.into());
        self
    }

    pub fn live_when(mut self, c: Cond) -> Self {
        self.0.live_when = Some(c.0);
        self
    }

    /// Fields of a step: its detail, time, call (as a process row's), evidence and lines.
    pub fn step_detail<S, T>(mut self, f: Field<S, T>) -> Self {
        self.0.step_detail = Some(f.name.into());
        self
    }

    pub fn step_time<S, T>(mut self, f: Field<S, T>) -> Self {
        self.0.step_time = Some(f.name.into());
        self
    }

    pub fn step_tool<S, T>(mut self, f: Field<S, T>) -> Self {
        self.0.step_tool = Some(f.name.into());
        self
    }

    pub fn step_evidence<S, T>(mut self, f: Field<S, T>) -> Self {
        self.0.step_evidence = Some(f.name.into());
        self
    }

    pub fn step_lines<S, T>(mut self, f: Field<S, T>) -> Self {
        self.0.step_lines = Some(f.name.into());
        self
    }
}

// ---------------------------------------------------------------------------------------------- pick, form notes

/// A `pick` input being declared: its directory's fields and texts.
pub struct PickRef<'a, R> {
    i: &'a mut doc::Input,
    _r: PhantomData<fn() -> R>,
}

impl<R> PickRef<'_, R> {
    fn pick(&mut self) -> &mut d::Pick {
        self.i.pick.get_or_insert_with(Default::default)
    }

    pub fn avatar<T>(mut self, f: Field<R, T>) -> Self {
        self.pick().avatar = Some(f.name.into());
        self
    }

    /// A bool field: the row is an agent (drawn with its tile; counted by `picked_agents`).
    pub fn agent(mut self, f: Field<R, bool>) -> Self {
        self.pick().agent = Some(f.name.into());
        self
    }

    pub fn tile<A, B>(mut self, icon: Field<R, A>, engine: Field<R, B>) -> Self {
        let p = self.pick();
        p.icon = Some(icon.name.into());
        p.engine = Some(engine.name.into());
        self
    }

    pub fn detail<T>(mut self, f: Field<R, T>) -> Self {
        self.pick().detail = Some(f.name.into());
        self
    }

    pub fn section<T>(mut self, f: Field<R, T>) -> Self {
        self.pick().section = Some(f.name.into());
        self
    }

    /// A list value of values not offered (the members already in).
    pub fn exclude<T>(mut self, v: Val<T>) -> Self {
        self.pick().exclude = Some(v.v);
        self
    }

    /// The line when nothing matches, and when every row is chosen.
    pub fn empty(mut self, nothing: impl Into<Txt>, all: impl Into<Txt>) -> Self {
        let p = self.pick();
        p.empty = Some(txt(nothing));
        p.empty_done = Some(txt(all));
        self
    }

    pub fn placeholder(self, t: impl Into<Txt>) -> Self {
        self.i.placeholder = Some(txt(t));
        self
    }

    pub fn required(self) -> Self {
        self.i.required = true;
        self
    }
}

impl<Req> FormB<Req> {
    /// Chips over a directory: the rows of `q`, each chosen by `value` and drawn with `title`. The field gets the
    /// chosen values joined with `, `.
    pub fn pick<T, Resp, R, V, L>(
        &mut self,
        f: Field<Req, T>,
        label: impl Into<Txt>,
        q: &Q<Resp, R>,
        value: Field<R, V>,
        title: Field<R, L>,
    ) -> PickRef<'_, R> {
        let i = self.add(doc::Input {
            name: f.name.into(),
            control: Control::Pick,
            label: txt(label),
            help: None,
            placeholder: None,
            required: false,
            initial: None,
            options: vec![],
            options_from: None,
            fields: vec![],
            min_items: None,
            max_items: None,
            icon: None,
            width: None,
            stepper: false,
            aside: None,
            add_label: None,
            pick: Some(d::Pick {
                query: q.name.clone(),
                value: value.name.into(),
                label: title.name.into(),
                ..Default::default()
            }),
        });
        PickRef { i, _r: PhantomData }
    }

    /// A line under the inputs while `c` (over the form's values) holds.
    pub fn note_when(&mut self, t: impl Into<Txt>, c: Cond) -> &mut Self {
        self.notes.push(d::FormNote {
            text: txt(t),
            visible: Some(c.0),
        });
        self
    }
}

// ---------------------------------------------------------------------------------------------- header

/// A head of a screen's column or of a side panel: `header(q.field(View::title)).tabs(&tab, [..]).close(&close)`.
pub struct HeaderB(d::Header);

pub fn header(title: impl Into<Txt>) -> HeaderB {
    HeaderB(d::Header {
        title: txt(title),
        title_icon: None,
        crumb_icon: None,
        leading: None,
        subtitle: vec![],
        subtitle_action: None,
        context: None,
        faces: None,
        buttons: vec![],
        icons: vec![],
        menu: None,
        tabs: None,
        expand: false,
        close: None,
        back: None,
        phone: None,
    })
}

/// A piece of a header's subtitle: `seg_text(..)`, `seg_dot_waiting()`, `seg_countdown(..).prefix("ends in ")`.
pub struct Seg(d::Segment);

pub fn seg_text(t: impl Into<Txt>) -> Seg {
    Seg(d::Segment {
        text: Some(txt(t)),
        ..Default::default()
    })
}

/// The amber dot of something at work.
pub fn seg_dot_live() -> Seg {
    Seg(d::Segment {
        dot: Some(d::Dot::Live),
        ..Default::default()
    })
}

/// The hollow dot of something waiting.
pub fn seg_dot_waiting() -> Seg {
    Seg(d::Segment {
        dot: Some(d::Dot::Waiting),
        ..Default::default()
    })
}

/// The line in the tone of something at work (amber), with no dot drawn.
pub fn seg_tone_live() -> Seg {
    Seg(d::Segment {
        tone: Some(d::Dot::Live),
        ..Default::default()
    })
}

/// The line in the tone of something waiting, with no dot drawn.
pub fn seg_tone_waiting() -> Seg {
    Seg(d::Segment {
        tone: Some(d::Dot::Waiting),
        ..Default::default()
    })
}

/// The time left until a moment in ms (left out while empty).
pub fn seg_countdown<T>(v: Val<T>) -> Seg {
    Seg(d::Segment {
        value: Some(v.v),
        format: Some(Format::Countdown {}),
        ..Default::default()
    })
}

/// The time since a moment in ms.
pub fn seg_elapsed<T>(v: Val<T>) -> Seg {
    Seg(d::Segment {
        value: Some(v.v),
        format: Some(Format::Elapsed {}),
        ..Default::default()
    })
}

impl Seg {
    /// A text before the value, left out with it.
    pub fn prefix(mut self, t: impl Into<Txt>) -> Self {
        self.0.prefix = Some(txt(t));
        self
    }

    pub fn visible(mut self, c: Cond) -> Self {
        self.0.visible = Some(c.0);
        self
    }
}

/// A tab of a header: `header_tab("pinned", "Pinned").badge(q.field(View::pinned_count))`.
pub struct HeaderTabB(d::HeaderTab);

pub fn header_tab(id: &str, title: impl Into<Txt>) -> HeaderTabB {
    HeaderTabB(d::HeaderTab {
        id: id.into(),
        title: txt(title),
        badge: None,
        action: None,
    })
}

impl HeaderTabB {
    /// ` · N` after the title while above 0.
    pub fn badge<T>(mut self, v: Val<T>) -> Self {
        self.0.badge = Some(v.v);
        self
    }

    /// Runs the action instead of selecting the tab.
    pub fn action(mut self, a: &ActionRef) -> Self {
        self.0.action = Some(a.0.clone());
        self
    }
}

impl HeaderB {
    pub fn title_icon<T>(mut self, v: Val<T>) -> Self {
        self.0.title_icon = Some(v.v);
        self
    }

    /// The icon before the screen's title in the breadcrumb and the phone's top bar.
    pub fn crumb_icon<T>(mut self, v: Val<T>) -> Self {
        self.0.crumb_icon = Some(v.v);
        self
    }

    /// An agent tile (40 px): the glyph and the engine.
    pub fn leading_tile<A, B>(mut self, icon: Val<A>, engine: Val<B>) -> Self {
        self.0.leading = Some(d::Leading {
            tile: Some(d::Tile {
                icon: icon.v,
                engine: Some(engine.v),
            }),
            faces: None,
            size: None,
        });
        self
    }

    /// A stack of faces (36 px).
    pub fn leading_faces<T>(mut self, v: Val<T>) -> Self {
        self.0.leading = Some(d::Leading {
            tile: None,
            faces: Some(v.v),
            size: None,
        });
        self
    }

    /// The leading's size in px.
    pub fn leading_size(mut self, px: u32) -> Self {
        if let Some(l) = &mut self.0.leading {
            l.size = Some(px);
        }
        self
    }

    pub fn subtitle(mut self, segs: impl IntoIterator<Item = Seg>) -> Self {
        self.0.subtitle = segs.into_iter().map(|s| s.0).collect();
        self
    }

    /// The subtitle line runs this action.
    pub fn subtitle_action(mut self, a: &ActionRef) -> Self {
        self.0.subtitle_action = Some(a.0.clone());
        self
    }

    /// The muted text after the title (`# returns`).
    pub fn context(mut self, t: impl Into<Txt>) -> Self {
        self.0.context = Some(txt(t));
        self
    }

    /// A faces pill: the faces, the label (`9 members`), the action.
    pub fn faces<A>(mut self, faces: Val<A>, label: impl Into<Txt>, a: &ActionRef) -> Self {
        self.0.faces = Some(d::HeaderFaces {
            value: faces.v,
            label: Some(txt(label)),
            aria_label: None,
            action: Some(a.0.clone()),
            visible: None,
        });
        self
    }

    /// The faces pill's accessible name (absent: `{label}: open`). Set it after `faces`.
    pub fn faces_aria_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(f) = &mut self.0.faces {
            f.aria_label = Some(txt(t));
        }
        self
    }

    pub fn faces_when(mut self, c: Cond) -> Self {
        if let Some(f) = &mut self.0.faces {
            f.visible = Some(c.0);
        }
        self
    }

    /// A text pill.
    pub fn button(mut self, label: impl Into<Txt>, a: &ActionRef) -> Self {
        self.0.buttons.push(d::HeaderButton {
            label: txt(label),
            action: a.0.clone(),
            visible: None,
        });
        self
    }

    /// The last pill is shown while `c` holds.
    pub fn button_when(mut self, c: Cond) -> Self {
        if let Some(b) = self.0.buttons.last_mut() {
            b.visible = Some(c.0);
        }
        self
    }

    /// Icon buttons (a toggle bell).
    pub fn icons(mut self, nodes: impl IntoIterator<Item = N>) -> Self {
        self.0.icons = nodes.into_iter().map(|n| n.0).collect();
        self
    }

    /// The `…` menu.
    pub fn menu(mut self, items: impl IntoIterator<Item = doc::MenuItem>) -> Self {
        self.0.menu = Some(doc::Menu {
            label: None,
            icon: None,
            items: items.into_iter().collect(),
        });
        self
    }

    /// The `…` button's and its menu's accessible label (absent: `More actions`). Set it after `menu`.
    pub fn menu_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(m) = self.0.menu.as_mut() {
            m.label = Some(txt(t));
        }
        self
    }

    /// Tabs bound to a string state whose default is one of the ids.
    pub fn tabs(
        mut self,
        state: &StateRef<String>,
        items: impl IntoIterator<Item = HeaderTabB>,
    ) -> Self {
        self.0.tabs = Some(d::HeaderTabs {
            bind: doc::Bind::State(state.name.clone()),
            items: items.into_iter().map(|t| t.0).collect(),
            label: None,
        });
        self
    }

    /// The tab bar's accessible label (absent: the title). Set it after `tabs`.
    pub fn tabs_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(tabs) = self.0.tabs.as_mut() {
            tabs.label = Some(txt(t));
        }
        self
    }

    /// The split's expand button.
    pub fn expand(mut self) -> Self {
        self.0.expand = true;
        self
    }

    /// The close button (desktop) and the back button (phone).
    pub fn close(mut self, a: &ActionRef) -> Self {
        self.0.close = Some(a.0.clone());
        self
    }

    /// The phone back button's label.
    pub fn back(mut self, t: impl Into<Txt>) -> Self {
        self.0.back = Some(txt(t));
        self
    }

    fn phone(&mut self) -> &mut d::HeaderPhone {
        self.0.phone.get_or_insert_with(Default::default)
    }

    /// The line under the phone's top-bar title.
    pub fn phone_sub(mut self, t: impl Into<Txt>) -> Self {
        self.phone().sub = Some(txt(t));
        self
    }

    /// Top-bar icon buttons on a phone: the actions of the header's icons or faces.
    pub fn phone_icons<'a>(mut self, actions: impl IntoIterator<Item = &'a ActionRef>) -> Self {
        self.phone().icons = actions.into_iter().map(|a| a.0.clone()).collect();
        self
    }

    /// The tabs first in the phone's `…` menu.
    pub fn menu_tabs(mut self) -> Self {
        self.phone().menu_tabs = true;
        self
    }

    pub fn visible(self, c: Cond) -> N {
        N::from(self).visible(c)
    }
}

impl From<HeaderB> for N {
    fn from(h: HeaderB) -> N {
        N::of(Kind::Header(h.0))
    }
}

// ---------------------------------------------------------------------------------------------- stream

/// A live list of a query's rows: `stream(&msgs).key(Msg::id).post(|p| ..).days(Msg::time)`.
pub struct StreamB<Resp, R> {
    s: d::Stream,
    _p: PhantomData<fn() -> (Resp, R)>,
}

pub fn stream<Resp, R>(q: &Q<Resp, R>) -> StreamB<Resp, R> {
    StreamB {
        s: d::Stream {
            query: q.name.clone(),
            key: String::new(),
            item: d::StreamItem::Post(Box::default()),
            kinds: None,
            tail: None,
            head: None,
            anchor: d::Anchor::Bottom,
            older_load: d::OlderLoad::Scroll,
            older_label: None,
            days: None,
            unread: None,
            jump_working: None,
            empty: None,
            phone_intro: None,
            on_seen: None,
            id: None,
            label: None,
        },
        _p: PhantomData,
    }
}

impl<Resp, R> StreamB<Resp, R> {
    /// The field that identifies an item.
    pub fn key<K>(mut self, f: Field<R, K>) -> Self {
        self.s.key = f.name.into();
        self
    }

    /// The template of a row.
    pub fn post(mut self, f: impl FnOnce(PostB<R>) -> PostB<R>) -> Self {
        self.s.item = d::StreamItem::Post(Box::new(f(PostB::new()).p));
        self
    }

    /// A template built elsewhere (shared by two streams).
    pub fn post_with(mut self, p: PostB<R>) -> Self {
        self.s.item = d::StreamItem::Post(Box::new(p.p));
        self
    }

    /// Rows whose `field` equals `when` are one centred line: the author, the text, the time.
    pub fn kinds_line<K, A, X, T>(
        mut self,
        field: Field<R, K>,
        when: &str,
        author: impl RowPath<R, T = A>,
        text: impl RowPath<R, T = X>,
        time: impl RowPath<R, T = T>,
    ) -> Self {
        self.s.kinds = Some(d::Kinds {
            field: field.name.into(),
            line: Some(d::KindLine {
                when: when.into(),
                author: Some(author.row_path()),
                text: text.row_path(),
                time: Some(time.row_path()),
            }),
        });
        self
    }

    /// Another query's rows after the rows (live items), by key.
    pub fn tail<Resp2>(mut self, q: &Q<Resp2, R>) -> Self {
        self.s.tail = Some(d::Tail {
            query: q.name.clone(),
            hold: true,
        });
        self
    }

    /// A row of the answer before the rows (drawn with the template, without its summary), and a rule under it.
    pub fn head(mut self, row: Field<Resp, R>, rule: impl Into<Txt>) -> Self {
        self.s.head = Some(d::StreamHead {
            row: row.name.into(),
            rule: Some(txt(rule)),
        });
        self
    }

    /// A list read from the top (no follow mode).
    pub fn anchor_top(mut self) -> Self {
        self.s.anchor = d::Anchor::Top;
        self
    }

    /// Older pages by a button only, with this label.
    pub fn older_button(mut self, label: impl Into<Txt>) -> Self {
        self.s.older_load = d::OlderLoad::Button;
        self.s.older_label = Some(txt(label));
        self
    }

    pub fn older_label(mut self, label: impl Into<Txt>) -> Self {
        self.s.older_label = Some(txt(label));
        self
    }

    /// A divider before each day of this field's moments.
    pub fn days<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.s.days = Some(f.row_path());
        self
    }

    /// A `New` divider after the item with this key.
    pub fn unread<T>(mut self, key: Val<T>) -> Self {
        self.s.unread = Some(key.v);
        self
    }

    /// The jump button's text when nothing new came.
    pub fn jump_working(mut self, t: impl Into<Txt>) -> Self {
        self.s.jump_working = Some(txt(t));
        self
    }

    pub fn empty(mut self, t: impl Into<Txt>) -> Self {
        self.s.empty = Some(txt(t));
        self
    }

    /// A muted line above the first item on a phone.
    pub fn phone_intro(mut self, t: impl Into<Txt>) -> Self {
        self.s.phone_intro = Some(txt(t));
        self
    }

    /// Runs the action with the newest row once the reader is at the end.
    pub fn on_seen(mut self, a: &ActionRef) -> Self {
        self.s.on_seen = Some(d::OnSeen {
            action: a.0.clone(),
            when: None,
        });
        self
    }

    pub fn on_seen_when(mut self, c: Cond) -> Self {
        if let Some(o) = &mut self.s.on_seen {
            o.when = Some(c.0);
        }
        self
    }

    /// The name composers refer to (`composer(..).for_stream("messages")`).
    pub fn id(mut self, id: &str) -> Self {
        self.s.id = Some(id.into());
        self
    }

    /// The list's accessible name.
    pub fn label(mut self, t: impl Into<Txt>) -> Self {
        self.s.label = Some(txt(t));
        self
    }

    pub fn visible(self, c: Cond) -> N {
        N::from(self).visible(c)
    }
}

impl<Resp, R> From<StreamB<Resp, R>> for N {
    fn from(s: StreamB<Resp, R>) -> N {
        N::of(Kind::Stream(s.s))
    }
}

// ---------------------------------------------------------------------------------------------- post

/// The template of a stream's row; every field is a row field or path.
pub struct PostB<R> {
    p: d::Post,
    _r: PhantomData<fn() -> R>,
}

impl<R> Clone for PostB<R> {
    fn clone(&self) -> Self {
        Self {
            p: self.p.clone(),
            _r: PhantomData,
        }
    }
}

/// A post template to share: `post::<Msg>().author(..)`.
pub fn post<R>() -> PostB<R> {
    PostB::new()
}

impl<R> PostB<R> {
    fn new() -> Self {
        Self {
            p: d::Post::default(),
            _r: PhantomData,
        }
    }

    pub fn author<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.author = f.row_path();
        self
    }

    pub fn time<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.time = f.row_path();
        self
    }

    /// The picture's URL (else the author's initials).
    pub fn avatar<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.avatar.get_or_insert_with(Default::default).url = Some(f.row_path());
        self
    }

    /// The agent tile (default: drawn while the glyph is not empty).
    pub fn tile<A, B>(
        mut self,
        icon: impl RowPath<R, T = A>,
        engine: impl RowPath<R, T = B>,
    ) -> Self {
        self.p.avatar.get_or_insert_with(Default::default).tile = Some(d::PostTile {
            icon: icon.row_path(),
            engine: Some(engine.row_path()),
            when: None,
        });
        self
    }

    pub fn tile_when(mut self, c: Cond) -> Self {
        if let Some(t) = self.p.avatar.as_mut().and_then(|a| a.tile.as_mut()) {
            t.when = Some(c.0);
        }
        self
    }

    /// A tag after the name (`AGENT`) while `c` holds.
    pub fn badge(mut self, t: impl Into<Txt>, c: Cond) -> Self {
        self.p.badge = Some(d::PostBadge {
            text: txt(t),
            when: Some(c.0),
        });
        self
    }

    pub fn source<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.source = Some(f.row_path());
        self
    }

    /// The text as Markdown.
    pub fn markdown<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.text = Some(d::PostText {
            field: f.row_path(),
            format: d::TextFormat::Markdown,
            writing: None,
        });
        self
    }

    /// The text as plain text with `@[Name]` and `#[x]` chips.
    pub fn plain<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        self.p.text = Some(d::PostText {
            field: f.row_path(),
            format: d::TextFormat::Plain,
            writing: None,
        });
        self
    }

    /// A bool field: the text is being written (a caret).
    pub fn writing<T>(mut self, f: impl RowPath<R, T = T>) -> Self {
        if let Some(t) = &mut self.p.text {
            t.writing = Some(f.row_path());
        }
        self
    }

    /// A recording: its duration (s); `.audio_more(label, url, words)`.
    pub fn audio<T>(mut self, duration: impl RowPath<R, T = T>) -> Self {
        self.p.audio = Some(d::PostAudio {
            duration: duration.row_path(),
            ..Default::default()
        });
        self
    }

    /// The recording's label, URL and timed words.
    pub fn audio_more<A, B, C>(
        mut self,
        label: impl RowPath<R, T = A>,
        url: impl RowPath<R, T = B>,
        words: impl RowPath<R, T = C>,
    ) -> Self {
        if let Some(a) = &mut self.p.audio {
            a.label = Some(label.row_path());
            a.url = Some(url.row_path());
            a.words = Some(words.row_path());
        }
        self
    }

    pub fn edited(mut self, f: impl RowPath<R, T = bool>) -> Self {
        self.p.edited = Some(f.row_path());
        self
    }

    /// A bool field: pinned; `mark` draws `Pinned` above the head.
    pub fn pinned(mut self, f: impl RowPath<R, T = bool>, mark: Val<bool>) -> Self {
        self.p.pinned = Some(d::PostPinned {
            field: f.row_path(),
            mark: Some(mark.v),
        });
        self
    }

    /// A bool field: the viewer's own row.
    pub fn mine(mut self, f: impl RowPath<R, T = bool>) -> Self {
        self.p.mine = Some(f.row_path());
        self
    }

    /// A line under the text; its tone by a field's value (`done` → ok).
    pub fn receipt<A, B>(
        mut self,
        text: impl RowPath<R, T = A>,
        tone: impl RowPath<R, T = B>,
        tones: impl IntoIterator<Item = (&'static str, Tone)>,
    ) -> Self {
        self.p.receipt = Some(d::PostReceipt {
            text: text.row_path(),
            tone: Some(doc::StatusField {
                field: tone.row_path(),
                tones: tones.into_iter().map(|(k, t)| (k.into(), t)).collect(),
            }),
        });
        self
    }

    /// `→ <to> · <status>` with `why` as its tooltip.
    pub fn route<A, B, C>(
        mut self,
        to: impl RowPath<R, T = A>,
        status: impl RowPath<R, T = B>,
        why: impl RowPath<R, T = C>,
    ) -> Self {
        self.p.route = Some(d::PostRoute {
            to: to.row_path(),
            status: Some(status.row_path()),
            why: Some(why.row_path()),
        });
        self
    }

    /// A correction inline for 30 s after the moment `from`, then in the menu: its label, the action.
    pub fn correction<A, B>(
        mut self,
        label: impl RowPath<R, T = A>,
        a: &ActionRef,
        from: impl RowPath<R, T = B>,
    ) -> Self {
        self.p.correction = Some(d::PostCorrection {
            label: label.row_path(),
            menu_label: None,
            lead: None,
            icon: None,
            action: a.0.clone(),
            from: from.row_path(),
            window_ms: 30_000,
        });
        self
    }

    /// The correction's menu label, the words before it inline, its icon (`undo`, `arrow-right`).
    pub fn correction_words<A, B, C>(
        mut self,
        menu_label: impl RowPath<R, T = A>,
        lead: impl RowPath<R, T = B>,
        icon: impl RowPath<R, T = C>,
    ) -> Self {
        if let Some(c) = &mut self.p.correction {
            c.menu_label = Some(menu_label.row_path());
            c.lead = Some(lead.row_path());
            c.icon = Some(icon.row_path());
        }
        self
    }

    pub fn correction_window_ms(mut self, ms: u32) -> Self {
        if let Some(c) = &mut self.p.correction {
            c.window_ms = ms;
        }
        self
    }

    /// Reaction chips: the repeated field, each one's emoji and count; a click runs `a` with `form::emoji()`.
    pub fn reactions<X, E, C>(
        mut self,
        field: impl RowPath<R, T = Vec<X>>,
        emoji: Field<X, E>,
        count: Field<X, C>,
        a: &ActionRef,
    ) -> Self {
        self.p.reactions = Some(d::PostReactions {
            field: field.row_path(),
            emoji: emoji.name.into(),
            count: count.name.into(),
            mine: None,
            action: Some(a.0.clone()),
            quick: vec![],
        });
        self
    }

    /// The field of a reaction: the viewer's own (pressed).
    pub fn reactions_mine<X>(mut self, f: Field<X, bool>) -> Self {
        if let Some(r) = &mut self.p.reactions {
            r.mine = Some(f.name.into());
        }
        self
    }

    /// The emoji the hover's React offers.
    pub fn quick<const K: usize>(mut self, emoji: [&str; K]) -> Self {
        if let Some(r) = &mut self.p.reactions {
            r.quick = emoji.iter().map(|e| e.to_string()).collect();
        }
        self
    }

    /// What follows from the item: its count and the action that opens it; `label`, faces and the rest by
    /// `summary_more`.
    pub fn summary<C>(mut self, count: impl RowPath<R, T = C>, a: &ActionRef) -> Self {
        self.p.summary = Some(d::PostSummary {
            count: count.row_path(),
            action: a.0.clone(),
            label: None,
            faces: None,
            last: None,
            working: None,
            since: None,
            waiting: None,
        });
        self
    }

    /// The summary's label (`{count} replies`).
    pub fn summary_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(s) = &mut self.p.summary {
            s.label = Some(txt(t));
        }
        self
    }

    /// The summary's faces, last line, who works (since a moment in ms) and who waits.
    pub fn summary_more<A, B, C, D, E>(
        mut self,
        faces: impl RowPath<R, T = A>,
        last: impl RowPath<R, T = B>,
        working: impl RowPath<R, T = C>,
        since: impl RowPath<R, T = D>,
        waiting: impl RowPath<R, T = E>,
    ) -> Self {
        if let Some(s) = &mut self.p.summary {
            s.faces = Some(faces.row_path());
            s.last = Some(last.row_path());
            s.working = Some(working.row_path());
            s.since = Some(since.row_path());
            s.waiting = Some(waiting.row_path());
        }
        self
    }

    /// No summary (a pinned list's posts).
    pub fn no_summary(mut self) -> Self {
        self.p.summary = None;
        self
    }

    /// A run of work above the text: `at` is the field holding it.
    pub fn process<W, F: RowPath<R>>(
        mut self,
        at: F,
        f: impl FnOnce(ProcessB<W>) -> ProcessB<W>,
    ) -> Self
    where
        F::T: Holds<W>,
    {
        self.p.process = Some(f(ProcessB::under(Some(at.row_path()))).p);
        self
    }

    /// The hover rail (desktop), in order.
    pub fn hover(mut self, e: impl IntoIterator<Item = EntryB>) -> Self {
        self.p.hover = e.into_iter().map(|e| e.0).collect();
        self
    }

    /// The `…` menu, in order.
    pub fn menu(mut self, e: impl IntoIterator<Item = EntryB>) -> Self {
        self.p.menu = e.into_iter().map(|e| e.0).collect();
        self
    }

    /// The phone's long-press sheet, in order.
    pub fn sheet(mut self, e: impl IntoIterator<Item = EntryB>) -> Self {
        self.p.sheet = e.into_iter().map(|e| e.0).collect();
        self
    }

    /// The inline editor: `a` reads `form::text()`; offered while `c` holds.
    pub fn edit(mut self, a: &ActionRef, c: Cond) -> Self {
        self.p.edit = Some(d::PostEdit {
            action: a.0.clone(),
            when: Some(c.0),
        });
        self
    }

    /// While false: no rail, menu, sheet or inline correction.
    pub fn actions_when(mut self, c: Cond) -> Self {
        self.p.actions_when = Some(c.0);
        self
    }

    /// The identity card of a mention, the lead and the name.
    pub fn card<F>(mut self, c: CardB<F>) -> Self {
        self.p.card = Some(c.0);
        self
    }
}

/// An entry of a post's hover rail, menu or sheet.
pub struct EntryB(d::Entry);

/// An entry running an action on the row: `entry(&open_thread, "Reply in thread").icon("message-square-text")`.
pub fn entry(a: &ActionRef, label: impl Into<Txt>) -> EntryB {
    EntryB(d::Entry {
        action: Some(a.0.clone()),
        label: Some(txt(label)),
        ..Default::default()
    })
}

impl EntryB {
    pub fn icon(mut self, icon: &str) -> Self {
        self.0.icon = Some(icon.into());
        self
    }

    /// Drawn as a text button (the hover's `Ask agent`).
    pub fn text_style(mut self) -> Self {
        self.0.style = Some(d::EntryStyle::Text);
        self
    }

    /// Drawn as an icon button.
    pub fn icon_style(mut self) -> Self {
        self.0.style = Some(d::EntryStyle::Icon);
        self
    }

    pub fn danger(mut self) -> Self {
        self.0.danger = true;
        self
    }

    pub fn visible(mut self, c: Cond) -> Self {
        self.0.visible = Some(c.0);
        self
    }
}

/// The built-in entries of a post: client gestures that need no plugin (or the post's own options).
pub mod builtin {
    use super::{EntryB, d};

    fn b(name: &str) -> EntryB {
        EntryB(d::Entry {
            builtin: Some(name.into()),
            ..Default::default()
        })
    }

    /// The quick reactions (needs `reactions`).
    pub fn react() -> EntryB {
        b("react")
    }
    /// Reply to the author (needs a composer with `reply`).
    pub fn reply() -> EntryB {
        b("reply")
    }
    pub fn copy_text() -> EntryB {
        b("copy_text")
    }
    pub fn copy_link() -> EntryB {
        b("copy_link")
    }
    /// Play or pause the recording (needs `audio`).
    pub fn play() -> EntryB {
        b("play")
    }
    /// The inline editor (needs `edit`).
    pub fn edit() -> EntryB {
        b("edit")
    }
    /// The correction after its inline window (needs `correction`).
    pub fn correction() -> EntryB {
        b("correction")
    }
    /// The `…` button that opens the menu (the hover rail only).
    pub fn more() -> EntryB {
        b("more")
    }
    pub fn separator() -> EntryB {
        b("separator")
    }
}

/// An identity card: a name looked up in a query's rows.
pub struct CardB<F>(d::PostCard, PhantomData<fn() -> F>);

pub fn card_of<Resp, F, T>(q: &Q<Resp, F>, name: Field<F, T>) -> CardB<F> {
    CardB(
        d::PostCard {
            query: q.name.clone(),
            name: name.name.into(),
            ..Default::default()
        },
        PhantomData,
    )
}

impl<F> CardB<F> {
    pub fn avatar<T>(mut self, f: Field<F, T>) -> Self {
        self.0.avatar = Some(f.name.into());
        self
    }

    pub fn agent(mut self, f: Field<F, bool>) -> Self {
        self.0.agent = Some(f.name.into());
        self
    }

    pub fn tile<A, B>(mut self, icon: Field<F, A>, engine: Field<F, B>) -> Self {
        self.0.icon = Some(icon.name.into());
        self.0.engine = Some(engine.name.into());
        self
    }

    pub fn detail<T>(mut self, f: Field<F, T>) -> Self {
        self.0.detail = Some(f.name.into());
        self
    }

    /// The card's button: `a` reads `form::to()`.
    pub fn action(mut self, a: &ActionRef) -> Self {
        self.0.action = Some(a.0.clone());
        self
    }

    /// What the card says of a name not in the rows.
    pub fn unknown(mut self, t: impl Into<Txt>) -> Self {
        self.0.unknown = Some(txt(t));
        self
    }
}

// ---------------------------------------------------------------------------------------------- process

/// A run of work: as a post's slot (`p.process(Msg::work, |w| w.state(Work::state)..)`) or on its own
/// (`process(&run_q, |w| ..)`). Its rows and calls have fixed field names.
pub struct ProcessB<W> {
    p: d::Process,
    at: Option<String>,
    _w: PhantomData<fn() -> W>,
}

/// A run of work on its own, over a query's answer.
pub fn process<Resp, R>(q: &Q<Resp, R>, f: impl FnOnce(ProcessB<Resp>) -> ProcessB<Resp>) -> N {
    let mut p = f(ProcessB::under(None)).p;
    p.query = Some(q.name.clone());
    N::of(Kind::Process(p))
}

impl<W> ProcessB<W> {
    fn under(at: Option<String>) -> Self {
        Self {
            p: d::Process::default(),
            at,
            _w: PhantomData,
        }
    }

    fn path<T>(&self, f: Field<W, T>) -> String {
        match &self.at {
            Some(a) => format!("{a}.{}", f.name),
            None => f.name.into(),
        }
    }

    /// `starting`, `working`, `done`, `stopped`, `failed`, `timed_out`, `gone`; no block while empty.
    pub fn state<T>(mut self, f: Field<W, T>) -> Self {
        self.p.state = self.path(f);
        self
    }

    pub fn label<T>(mut self, f: Field<W, T>) -> Self {
        self.p.label = Some(self.path(f));
        self
    }

    pub fn meta<T>(mut self, f: Field<W, T>) -> Self {
        self.p.meta = Some(self.path(f));
        self
    }

    /// The moment (ms) the live block counts from.
    pub fn started<T>(mut self, f: Field<W, T>) -> Self {
        self.p.started = Some(self.path(f));
        self
    }

    /// The rows: `kind`, `title`, `detail`, `state`, `time`, `at`, `calls`, `tools`.
    pub fn rows<T>(mut self, f: Field<W, T>) -> Self {
        self.p.rows = Some(self.path(f));
        self
    }

    pub fn steps<T>(mut self, f: Field<W, T>) -> Self {
        self.p.steps = Some(self.path(f));
        self
    }

    /// The text being written.
    pub fn saying<T>(mut self, f: Field<W, T>) -> Self {
        self.p.saying = Some(self.path(f));
        self
    }

    /// Retry on an ended block, with its label from a field, while `c` holds.
    pub fn retry<T>(mut self, a: &ActionRef, label: Field<W, T>, c: Cond) -> Self {
        self.p.retry = Some(d::ProcessAction {
            action: a.0.clone(),
            label: Some(self.path(label)),
            when: Some(c.0),
        });
        self
    }

    /// `Open full log`.
    pub fn log(mut self, a: &ActionRef) -> Self {
        self.p.log = Some(d::ProcessAction {
            action: a.0.clone(),
            label: None,
            when: None,
        });
        self
    }

    /// How a truncated call is read whole: `q` read with `args` and `call` set to the call's id; the answer's
    /// `field` is the call.
    pub fn fetch_call<Req, Resp, R, C, F>(
        mut self,
        q: &Q<Resp, R>,
        call: Field<Req, C>,
        field: Field<Resp, F>,
        args: impl FnOnce(&mut Args<Req>) -> &mut Args<Req>,
    ) -> Self {
        let mut a = Args::new();
        args(&mut a);
        self.p.fetch_call = Some(d::FetchCall {
            query: q.name.clone(),
            args: a.args,
            call: call.name.into(),
            field: field.name.into(),
        });
        self
    }
}

// ---------------------------------------------------------------------------------------------- strip

/// A pinned list of live items: `strip(&live, LiveView::runs, Run::run, Run::agent).state(..).button(..)`.
pub struct StripB<R> {
    s: d::Strip,
    _r: PhantomData<fn() -> R>,
}

/// A strip over a repeated field of a query's answer (`rows`), each item keyed by `key` and titled by `title`.
pub fn strip<Resp, X, R, K, T>(
    q: &Q<Resp, X>,
    rows: Field<Resp, Vec<R>>,
    key: Field<R, K>,
    title: Field<R, T>,
) -> StripB<R> {
    StripB {
        s: d::Strip {
            query: q.name.clone(),
            rows: Some(rows.name.into()),
            key: key.name.into(),
            title: title.name.into(),
            ..Default::default()
        },
        _r: PhantomData,
    }
}

/// A button of a strip's row: `strip_button("Stop", &stop).icon("square").stop_style()`.
pub struct StripButtonB(d::StripButton);

pub fn strip_button(label: impl Into<Txt>, a: &ActionRef) -> StripButtonB {
    StripButtonB(d::StripButton {
        label: txt(label),
        action: a.0.clone(),
        icon: None,
        style: None,
        visible: None,
    })
}

impl StripButtonB {
    pub fn icon(mut self, icon: &str) -> Self {
        self.0.icon = Some(icon.into());
        self
    }

    /// The red-outlined Stop.
    pub fn stop_style(mut self) -> Self {
        self.0.style = Some(d::StripStyle::Stop);
        self
    }

    /// A muted text button.
    pub fn text_style(mut self) -> Self {
        self.0.style = Some(d::StripStyle::Text);
        self
    }

    pub fn visible(mut self, c: Cond) -> Self {
        self.0.visible = Some(c.0);
        self
    }
}

impl<R> StripB<R> {
    /// Above the items (`2 live runs`).
    pub fn label(mut self, t: impl Into<Txt>) -> Self {
        self.s.label = Some(txt(t));
        self
    }

    pub fn tile<A, B>(mut self, icon: Field<R, A>, engine: Field<R, B>) -> Self {
        self.s.tile = Some(d::PostTile {
            icon: icon.name.into(),
            engine: Some(engine.name.into()),
            when: None,
        });
        self
    }

    /// Which values of the field are live, waiting and starting.
    pub fn state<T>(
        mut self,
        f: Field<R, T>,
        live: &[&str],
        waiting: &[&str],
        starting: &[&str],
    ) -> Self {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect();
        self.s.state = Some(d::StripState {
            field: f.name.into(),
            live: v(live),
            waiting: v(waiting),
            starting: v(starting),
        });
        self
    }

    /// The moments (ms) an item counts from and, while waiting, down to.
    pub fn since<T>(mut self, f: Field<R, T>) -> Self {
        self.s.since = Some(f.name.into());
        self
    }

    pub fn until<T>(mut self, f: Field<R, T>) -> Self {
        self.s.until = Some(f.name.into());
        self
    }

    pub fn sub_line<T>(mut self, f: Field<R, T>) -> Self {
        self.s.sub = Some(f.name.into());
        self
    }

    /// After a waiting item's sub line, `{left}` the countdown (`ends in {left}`).
    pub fn countdown_text(mut self, t: impl Into<Txt>) -> Self {
        self.s.countdown_text = Some(txt(t));
        self
    }

    /// The tile and the title run it.
    pub fn open(mut self, a: &ActionRef) -> Self {
        self.s.open = Some(a.0.clone());
        self
    }

    /// The jump to the item's place: the key in `field`, the labels by state.
    pub fn jump<T>(
        mut self,
        f: Field<R, T>,
        label: impl Into<Txt>,
        label_waiting: impl Into<Txt>,
    ) -> Self {
        self.s.jump = Some(d::StripJump {
            field: f.name.into(),
            label: Some(txt(label)),
            label_waiting: Some(txt(label_waiting)),
        });
        self
    }

    pub fn button(mut self, b: StripButtonB) -> Self {
        self.s.buttons.push(b.0);
        self
    }

    pub fn aria_label(mut self, t: impl Into<Txt>) -> Self {
        self.s.aria_label = Some(txt(t));
        self
    }

    pub fn visible(self, c: Cond) -> N {
        N::from(self).visible(c)
    }
}

impl<R> From<StripB<R>> for N {
    fn from(s: StripB<R>) -> N {
        N::of(Kind::Strip(s.s))
    }
}

// ---------------------------------------------------------------------------------------------- composer

pub use celobus_descriptor::ui::ComposerTool;

/// A message box: `composer(&send).for_stream("messages").primary().mentions(..).reply()`. `send` reads
/// `form::text()`, `form::reply()`, the recording and each option (`form::option("also")`).
pub struct ComposerB(d::ComposerNode);

pub fn composer(send: &ActionRef) -> ComposerB {
    ComposerB(d::ComposerNode {
        send: send.0.clone(),
        ..Default::default()
    })
}

/// The mention picker's directory: `mentions(&people, Face::name).tile(Face::icon, Face::engine)`.
pub struct MentionsB<F>(d::Mentions, PhantomData<fn() -> F>);

pub fn mentions<Resp, F, T>(q: &Q<Resp, F>, name: Field<F, T>) -> MentionsB<F> {
    MentionsB(
        d::Mentions {
            query: q.name.clone(),
            name: name.name.into(),
            ..Default::default()
        },
        PhantomData,
    )
}

impl<F> MentionsB<F> {
    pub fn avatar<T>(mut self, f: Field<F, T>) -> Self {
        self.0.avatar = Some(f.name.into());
        self
    }

    pub fn agent(mut self, f: Field<F, bool>) -> Self {
        self.0.agent = Some(f.name.into());
        self
    }

    pub fn tile<A, B>(mut self, icon: Field<F, A>, engine: Field<F, B>) -> Self {
        self.0.icon = Some(icon.name.into());
        self.0.engine = Some(engine.name.into());
        self
    }

    pub fn detail<T>(mut self, f: Field<F, T>) -> Self {
        self.0.detail = Some(f.name.into());
        self
    }

    /// The rows are grouped by this field on a desktop.
    pub fn section<T>(mut self, f: Field<F, T>) -> Self {
        self.0.section = Some(f.name.into());
        self
    }

    /// The picker's footer note.
    pub fn note(mut self, t: impl Into<Txt>) -> Self {
        self.0.note = Some(txt(t));
        self
    }
}

impl ComposerB {
    /// The `id` of the stream it belongs to.
    pub fn for_stream(mut self, id: &str) -> Self {
        self.0.stream = Some(id.into());
        self
    }

    pub fn placeholder(mut self, t: impl Into<Txt>) -> Self {
        self.0.placeholder = Some(txt(t));
        self
    }

    /// Owns ⌥ Space; one per frame.
    pub fn primary(mut self) -> Self {
        self.0.primary = true;
        self
    }

    /// The panel's box.
    pub fn compact(mut self) -> Self {
        self.0.compact = true;
        self
    }

    pub fn tools(mut self, tools: impl IntoIterator<Item = ComposerTool>) -> Self {
        self.0.tools = tools.into_iter().collect();
        self
    }

    pub fn mentions<F>(mut self, m: MentionsB<F>) -> Self {
        self.0.mentions = Some(m.0);
        self
    }

    /// `#word` becomes `#[word]`.
    pub fn tags(mut self) -> Self {
        self.0.tags = true;
        self
    }

    /// The Dictate button (with the workspace's speech connection).
    pub fn dictation(mut self) -> Self {
        self.0.dictation = Some(d::Dictation::default());
        self
    }

    /// Words for the recogniser besides the mentionable names.
    pub fn vocabulary(mut self, t: impl Into<Txt>) -> Self {
        self.0
            .dictation
            .get_or_insert_with(Default::default)
            .vocabulary = Some(txt(t));
        self
    }

    /// Takes `reply_to`: `form::reply()` is the key.
    pub fn reply(mut self) -> Self {
        self.0.reply = true;
        self
    }

    /// A checkbox under the box (`form::option(name)`).
    pub fn option(mut self, name: &str, label: impl Into<Txt>) -> Self {
        self.0.options.push(d::ComposerOption {
            name: name.into(),
            label: txt(label),
            visible: None,
        });
        self
    }

    /// The last option is shown while `c` holds.
    pub fn option_when(mut self, c: Cond) -> Self {
        if let Some(o) = self.0.options.last_mut() {
            o.visible = Some(c.0);
        }
        self
    }

    /// A caption line naming who will answer (`↳ …`).
    pub fn caption_route(mut self, t: impl Into<Txt>) -> Self {
        self.0.caption.push(d::Caption {
            value: txt(t),
            icon: Some("corner-down-right".into()),
            style: Some(d::CaptionStyle::Route),
        });
        self
    }

    /// A plain caption line.
    pub fn caption_dot(mut self, t: impl Into<Txt>) -> Self {
        self.0.caption.push(d::Caption {
            value: txt(t),
            icon: None,
            style: Some(d::CaptionStyle::Dot),
        });
        self
    }

    /// While `c` holds and the box is empty, Send is a labelled Stop.
    pub fn stop(mut self, a: &ActionRef, c: Cond) -> Self {
        self.0.stop = Some(d::ComposerStop {
            action: a.0.clone(),
            when: Some(c.0),
            label: None,
            aria_label: None,
        });
        self
    }

    pub fn stop_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(s) = &mut self.0.stop {
            s.label = Some(txt(t));
        }
        self
    }

    /// The Stop button's accessible name (absent: its label). Set it after `stop`.
    pub fn stop_aria_label(mut self, t: impl Into<Txt>) -> Self {
        if let Some(s) = &mut self.0.stop {
            s.aria_label = Some(txt(t));
        }
        self
    }

    /// While `c` holds, the reason stands where the box would be.
    pub fn readonly(mut self, c: Cond, reason: impl Into<Txt>) -> Self {
        self.0.readonly = Some(d::Readonly {
            when: c.0,
            reason: txt(reason),
        });
        self
    }

    /// The text is mirrored into this state, for a query that reads what is typed.
    pub fn draft_state(mut self, s: &StateRef<String>) -> Self {
        self.0.draft_state = Some(s.name.clone());
        self
    }

    pub fn visible(self, c: Cond) -> N {
        N::from(self).visible(c)
    }
}

impl From<ComposerB> for N {
    fn from(c: ComposerB) -> N {
        N::of(Kind::Composer(c.0))
    }
}
