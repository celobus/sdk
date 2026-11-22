//! The definition-time rules of the UI document (`E_UI_*`), its warnings (`W_UI_*`), the catalog revision each
//! screen needs (for the client's `compat` verdict) and the change classes of a new version's UI.
//!
//! Every reference is checked: endpoints (declared, marked `ui`), topics, queries in scope, actions, screens,
//! overlays, params and state; request fields of arguments and forms; response fields of `rows`, `data`, row
//! fields of tables, lists, charts, the binds of a `viz` and timelines; text keys; value contexts (`row` inside a
//! row, `result` after a call, `error` inside `on_error`); limits. Types are checked where both sides are known.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ui::*;
use crate::{Change, Descriptor, Endpoint, Error, Field, FieldType};

/// Limits per document (`E_UI_LIMIT`).
pub const MAX_SCREENS: usize = 200;
pub const MAX_QUERIES: usize = 50;
pub const MAX_FOLLOWED: usize = 8;
pub const MAX_NODES: usize = 2_000;
pub const MAX_DOCUMENT: usize = 256 << 10;
pub const MAX_TEXTS: usize = 64 << 10;
/// The shortest poll.
pub const MIN_POLL: u32 = 15;

/// What a screen, overlay or contribution needs of a client: the highest catalog revision of its interactive parts
/// (forms, inputs, buttons, menus, steps) and of its display parts. An interactive part inside a newer layout (a
/// button in a `board`) needs the layout's revision: a client that cannot draw the layout cannot show the button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Requires {
    pub interactive: u32,
    pub display: u32,
}

impl Requires {
    /// `full`, `degraded` (a display part is newer: its fallback or a placeholder is drawn) or `unsupported`.
    pub fn compat(&self, client: u32) -> &'static str {
        if self.interactive > client {
            "unsupported"
        } else if self.display > client {
            "degraded"
        } else {
            "full"
        }
    }
}

/// Per screen id, document-level overlay id (`overlay:<id>`) and `nav`/`home`/`commands`, what it requires.
pub fn requirements(ui: &Ui) -> BTreeMap<String, Requires> {
    /// `within`: the highest revision of the layouts the node is in.
    fn walk(n: &Node, r: &mut Requires, within: u32) {
        let rev = n.kind.revision().max(within);
        if n.kind.interactive() {
            r.interactive = r.interactive.max(rev);
        } else {
            r.display = r.display.max(rev);
        }
        for c in children(&n.kind) {
            walk(c, r, rev);
        }
        if let Kind::Form(f) = &n.kind {
            for s in &f.sections {
                for i in &s.inputs {
                    r.interactive = r.interactive.max(1);
                    let _ = i;
                }
            }
        }
    }
    fn node(n: &Node, r: &mut Requires) {
        walk(n, r, 0);
    }
    fn actions(a: &[Action], r: &mut Requires) {
        for s in a.iter().flat_map(|a| a.steps.iter().chain(&a.on_error)) {
            r.interactive = r.interactive.max(s.revision());
        }
    }
    let mut out = BTreeMap::new();
    for s in &ui.screens {
        let mut r = Requires::default();
        node(&s.layout, &mut r);
        s.header.iter().for_each(|n| node(n, &mut r));
        actions(&s.actions, &mut r);
        for o in &s.overlays {
            node(&o.layout, &mut r);
            o.buttons.iter().for_each(|n| node(n, &mut r));
            actions(&o.actions, &mut r);
        }
        out.insert(s.id.clone(), r);
    }
    for o in &ui.overlays {
        let mut r = Requires::default();
        node(&o.layout, &mut r);
        o.buttons.iter().for_each(|n| node(n, &mut r));
        actions(&o.actions, &mut r);
        out.insert(format!("overlay:{}", o.id), r);
    }
    let mut home = Requires::default();
    for w in &ui.home {
        node(&w.node, &mut home);
    }
    if !ui.home.is_empty() {
        out.insert("home".into(), home);
    }
    out
}

/// The children of a primitive (layout containers, row actions, toolbars, actions of notices and empties).
pub fn children(k: &Kind) -> Vec<&Node> {
    match k {
        Kind::Stack(s) => s.children.iter().collect(),
        Kind::Columns(c) => c.children.iter().collect(),
        Kind::Section(s) => s
            .actions
            .iter()
            .chain(&s.children)
            .chain(&s.footer)
            .collect(),
        Kind::Card(c) => c.children.iter().collect(),
        Kind::Tabs(t) => t.tabs.iter().flat_map(|t| t.children.iter()).collect(),
        Kind::Split(s) => vec![s.list.as_ref(), s.detail.as_ref()],
        Kind::Board(b) => b.children.iter().map(|c| &c.node).collect(),
        Kind::Table(t) => t.row_actions.iter().chain(&t.toolbar).collect(),
        Kind::List(l) => l.row_actions.iter().collect(),
        Kind::Empty(e) => e.action.iter().map(|b| b.as_ref()).collect(),
        Kind::Notice(n) => n.action.iter().map(|b| b.as_ref()).collect(),
        Kind::RecordHeader(h) => h.tags.iter().chain(&h.actions).collect(),
        Kind::Header(h) => h.icons.iter().collect(),
        _ => vec![],
    }
}

/// Every problem of the descriptor's `topics` and `ui` (empty when valid or absent).
pub fn validate_ui(d: &Descriptor) -> Vec<Error> {
    let mut c = Check::new(d);
    c.topics();
    if let Some(ui) = &d.ui {
        c.document(ui);
    }
    c.errors
}

/// Every node of the document: screens (header, layout, overlays), document overlays, home widgets.
pub fn all_nodes(ui: &Ui) -> Vec<(String, &Node)> {
    fn walk<'n>(n: &'n Node, p: String, out: &mut Vec<(String, &'n Node)>) {
        for (i, c) in children(&n.kind).into_iter().enumerate() {
            walk(c, format!("{p}.{}[{i}]", n.kind.name()), out);
        }
        if let Some(f) = &n.fallback {
            walk(f, format!("{p}.fallback"), out);
        }
        out.push((p, n));
    }
    fn overlay<'n>(o: &'n Overlay, p: String, out: &mut Vec<(String, &'n Node)>) {
        walk(&o.layout, format!("{p}.layout"), out);
        for (i, b) in o.buttons.iter().enumerate() {
            walk(b, format!("{p}.buttons[{i}]"), out);
        }
    }
    let mut out = vec![];
    for s in &ui.screens {
        let p = format!("ui.screens[{}]", s.id);
        walk(&s.layout, format!("{p}.layout"), &mut out);
        for (i, h) in s.header.iter().enumerate() {
            walk(h, format!("{p}.header[{i}]"), &mut out);
        }
        for o in &s.overlays {
            overlay(o, format!("{p}.overlays[{}]", o.id), &mut out);
        }
    }
    for o in &ui.overlays {
        overlay(o, format!("ui.overlays[{}]", o.id), &mut out);
    }
    for w in &ui.home {
        walk(&w.node, format!("ui.home[{}].node", w.id), &mut out);
    }
    out
}

/// Where the document uses the deprecated `chat` node.
fn chat_nodes(ui: &Ui) -> Vec<String> {
    all_nodes(ui)
        .into_iter()
        .filter(|(_, n)| matches!(n.kind, Kind::Chat(_)))
        .map(|(p, _)| p)
        .collect()
}

/// Warnings: a mutating `ui` endpoint called by an action without idempotency; a followed topic no endpoint
/// and no agent's runs invalidate; the deprecated `chat` node.
pub fn ui_warnings(d: &Descriptor) -> Vec<Error> {
    let mut out = vec![];
    let Some(ui) = &d.ui else { return out };
    for p in chat_nodes(ui) {
        out.push(
            Error::new(
                "W_UI_DEPRECATED",
                format!("{p}.chat"),
                "the chat node is deprecated: stream, post, composer, header and strip replace it in catalog 3",
            )
            .help("build the screen from a fill split with a header, a stream of posts and a composer"),
        );
    }
    let mut called = HashSet::new();
    let all_actions = ui
        .screens
        .iter()
        .flat_map(|s| {
            s.actions
                .iter()
                .chain(s.overlays.iter().flat_map(|o| o.actions.iter()))
        })
        .chain(ui.overlays.iter().flat_map(|o| o.actions.iter()))
        .chain(ui.menus.iter().flat_map(|m| m.actions.iter()));
    for a in all_actions {
        for s in a.steps.iter().chain(&a.on_error) {
            if let Step::Call(c) = s {
                called.insert(c.endpoint.as_str());
            }
        }
    }
    for e in &d.endpoints {
        if called.contains(e.name.as_str())
            && !read_only(e)
            && e.idempotency == crate::Idempotency::Off
        {
            out.push(
                Error::new(
                    "W_UI_CALL_NOT_IDEMPOTENT",
                    format!("endpoints[{}]", e.name),
                    format!("{} changes data, is called from the UI and has no idempotency", e.name),
                )
                .help("an unknown outcome (a lost answer) can then not be retried safely; set idempotency optional"),
            );
        }
    }
    let invalidated: HashSet<&str> = d
        .endpoints
        .iter()
        .flat_map(|e| e.invalidates.iter().map(|i| i.topic.as_str()))
        // the runs of an agent invalidate the topic it names
        .chain(d.agents.iter().map(|a| a.topic.as_str()))
        .collect();
    let mut followed = BTreeMap::new();
    let queries = ui.queries.iter().chain(ui.screens.iter().flat_map(|s| {
        s.queries
            .iter()
            .chain(s.overlays.iter().flat_map(|o| o.queries.iter()))
    }));
    for q in queries {
        for f in &q.follow {
            followed.entry(f.topic.as_str()).or_insert(q.name.as_str());
        }
    }
    for (topic, q) in followed {
        if !invalidated.contains(topic) && !d.topics.iter().any(|t| t.name == topic && t.manual) {
            out.push(
                Error::new(
                    "W_UI_TOPIC_NEVER_INVALIDATED",
                    format!("topics[{topic}]"),
                    format!("query {q} follows {topic}, but no endpoint declares that it invalidates it"),
                )
                .help("declare .invalidates(TOPIC) on the endpoints that change it, or mark the topic manual if code calls ctx.invalidate"),
            );
        }
    }
    out
}

fn read_only(e: &Endpoint) -> bool {
    e.read_only
}

/// The UI change classes of a new version: removed screens, document overlays, commands and record menus, and new
/// required params, are breaking; new screens and contributions are additions (the rest is a patch).
pub fn ui_changes(old: &Descriptor, new: &Descriptor) -> Vec<Change> {
    let empty = Ui::default();
    let (o, n) = (
        old.ui.as_ref().unwrap_or(&empty),
        new.ui.as_ref().unwrap_or(&empty),
    );
    let mut out = vec![];
    for s in &o.screens {
        match n.screens.iter().find(|x| x.id == s.id) {
            None => out.push(Change::new(
                "E_UI_SCREEN_REMOVED",
                format!("ui.screens[{}]", s.id),
                format!(
                    "screen {} was removed: links and bookmarks to it break",
                    s.id
                ),
                true,
            )),
            Some(x) => {
                // a param with a default is never missing: a link without it gets the default
                for p in x
                    .params
                    .iter()
                    .filter(|p| p.required && p.default.is_none())
                {
                    if !s
                        .params
                        .iter()
                        .any(|q| q.name == p.name && q.required && q.default.is_none())
                    {
                        out.push(Change::new(
                            "E_UI_PARAM_REQUIRED",
                            format!("ui.screens[{}].params[{}]", s.id, p.name),
                            format!(
                                "screen {} now requires param {}: older links lack it",
                                s.id, p.name
                            ),
                            true,
                        ));
                    }
                }
            }
        }
    }
    for x in &o.overlays {
        if !n.overlays.iter().any(|y| y.id == x.id) {
            out.push(Change::new(
                "E_UI_OVERLAY_REMOVED",
                format!("ui.overlays[{}]", x.id),
                format!("overlay {} was removed", x.id),
                true,
            ));
        }
    }
    for c in &o.commands {
        if !n.commands.iter().any(|y| y.id == c.id) {
            out.push(Change::new(
                "E_UI_COMMAND_REMOVED",
                format!("ui.commands[{}]", c.id),
                format!("command {} was removed", c.id),
                true,
            ));
        }
    }
    for m in &o.menus {
        if !n.menus.iter().any(|y| y.message == m.message) {
            out.push(Change::new(
                "E_UI_MENU_REMOVED",
                format!("ui.menus[{}]", m.message),
                format!("the record menu of {} was removed", m.message),
                true,
            ));
        }
    }
    // the deprecated chat node stays valid where it was, but no version may start using it
    if chat_nodes(o).is_empty() {
        for p in chat_nodes(n) {
            out.push(Change::new(
                "E_UI_DEPRECATED_ADDED",
                format!("{p}.chat"),
                "the chat node is deprecated: a new version may keep it, not add it (use stream, post, composer)",
                true,
            ));
        }
    }
    for s in &n.screens {
        if !o.screens.iter().any(|x| x.id == s.id) {
            out.push(Change::new(
                "UI_SCREEN_ADDED",
                format!("ui.screens[{}]", s.id),
                format!("screen {} was added", s.id),
                false,
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------- the checker

/// What names a scope (a screen, an overlay, the document level) offers.
#[derive(Default, Clone)]
struct Scope<'a> {
    params: HashMap<&'a str, &'a str>,
    /// The defaults of the params that have one.
    defaults: HashMap<&'a str, &'a serde_json::Value>,
    state: HashMap<&'a str, &'a str>,
    /// The defaults of the state values.
    state_defaults: HashMap<&'a str, &'a serde_json::Value>,
    queries: HashMap<&'a str, &'a Query>,
    actions: HashMap<&'a str, &'a Action>,
    overlays: HashSet<&'a str>,
}

/// Where a value is read: inside a row of a message, a form of a request, after a call, inside `on_error`.
#[derive(Default, Clone)]
struct Ctx {
    row: Option<String>,
    /// The lookups of the row's query (`row.<as>.<field>`): name, message.
    joins: Vec<(String, String)>,
    form: Option<String>,
    /// The form's `pick` inputs (`form.<name>.count`, `.agents`, `.first`, `.query`).
    picks: Vec<String>,
    result: Option<String>,
    error: bool,
}

/// The form context of an agent run's message box (its only value is `{form: text}`).
const MESSAGE_BOX: &str = "$message";
/// The form context of a `composer` (catalog 3): `$composer:<option>,<option>`; `{form: text, audio, duration,
/// language, words, reply}` and each option (a bool).
const COMPOSER: &str = "$composer";
/// The form context of a post's edit, reactions and card (catalog 3): `{form: text, emoji, to}`.
const POST_FORM: &str = "$post";
/// The form context of a toggle `icon_button` (catalog 3): `{form: pressed}`.
const TOGGLE: &str = "$toggle";
/// The form context of a conversation's message box: `{form: text}` and `{form: to}`.
const THREAD_COMPOSER: &str = "$thread_composer";
/// The form context of a chat screen's composers: `{form: text}`, `to`, `also` and `emoji`; the thread's bell:
/// `{form: follow}`.
const CHAT_FORM: &str = "$chat";

struct Check<'a> {
    d: &'a Descriptor,
    errors: Vec<Error>,
    nodes: usize,
    en: HashSet<&'a str>,
    /// actions referenced, with the contexts they are referenced from (for `row` and `form` values)
    used: HashMap<(String, &'a str), Vec<Ctx>>,
    /// the request of the form in the frame whose queries are checked (a quote query reads the form's values)
    frame_form: Option<String>,
    /// the node being checked is inside a board (a board is not nested in another)
    in_board: bool,
    /// the frame being checked (catalog 3): its streams' ids, the composers' `for`, its primary composers, the
    /// bottom-anchored streams of each column (a split's side starts a column)
    frame: Frame,
}

#[derive(Default)]
struct Frame {
    streams: HashSet<String>,
    fors: Vec<(String, String)>,
    primaries: usize,
    columns: Vec<usize>,
}

impl<'a> Check<'a> {
    fn new(d: &'a Descriptor) -> Self {
        let en =
            d.ui.as_ref()
                .and_then(|u| u.texts.get("en"))
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
        Self {
            d,
            errors: vec![],
            nodes: 0,
            en,
            used: HashMap::new(),
            frame_form: None,
            in_board: false,
            frame: Frame::default(),
        }
    }

    fn err(&mut self, code: &'static str, path: impl Into<String>, msg: impl Into<String>) {
        self.errors.push(Error::new(code, path, msg));
    }

    fn topics(&mut self) {
        let mut seen = HashSet::new();
        for (i, t) in self.d.topics.iter().enumerate() {
            let p = format!(
                "topics[{}]",
                if t.name.is_empty() {
                    i.to_string()
                } else {
                    t.name.clone()
                }
            );
            if !snake(&t.name) {
                self.err(
                    "E_TOPIC_NAME",
                    format!("{p}.name"),
                    format!("{:?} is not a topic name (snake_case)", t.name),
                );
            }
            if !seen.insert(t.name.as_str()) {
                self.err(
                    "E_TOPIC_DUP",
                    &p,
                    format!("topic {} is declared twice", t.name),
                );
            }
            if !self.d.messages.iter().any(|m| m.name == t.key) {
                self.err(
                    "E_TOPIC_KEY",
                    format!("{p}.key"),
                    format!("the key message {} is not declared", t.key),
                );
            }
            for r in &t.roles {
                if !self.d.roles.iter().any(|x| &x.name == r) && !r.starts_with("workspace.") {
                    self.err(
                        "E_TOPIC_ROLE",
                        format!("{p}.roles"),
                        format!("role {r} is not declared"),
                    );
                }
            }
        }
        for t in &self.d.topics {
            // a keyed topic (its key message has fields) needs a read endpoint that admits keys
            let keyed = self
                .d
                .messages
                .iter()
                .find(|m| m.name == t.key)
                .is_some_and(|m| !m.fields.is_empty());
            if keyed && t.authorize.is_none() {
                self.err(
                    "E_TOPIC_AUTHORIZE",
                    format!("topics[{}].authorize", t.name),
                    format!(
                        "topic {} is keyed by {}: name the read endpoint that admits a key (.authorize(..))",
                        t.name, t.key
                    ),
                );
            }
            if let Some(a) = &t.authorize {
                match self.d.endpoints.iter().find(|e| &e.name == a) {
                    None => self.err("E_UI_ENDPOINT_UNKNOWN", format!("topics[{}].authorize", t.name), format!("endpoint {a} is not declared")),
                    Some(e) if !self.covers(&e.input, &t.key) => self.err(
                        "E_TOPIC_AUTHORIZE",
                        format!("topics[{}].authorize", t.name),
                        format!("{a} takes {}, which lacks a field of the topic's key {} (same name and type)", e.input, t.key),
                    ),
                    Some(e) if !read_only(e) => self.err("E_UI_QUERY_MUTATES", format!("topics[{}].authorize", t.name), format!("{a} must be read-only")),
                    _ => {}
                }
            }
        }
        for e in &self.d.endpoints {
            for (i, inv) in e.invalidates.iter().enumerate() {
                let p = format!("endpoints[{}].invalidates[{i}]", e.name);
                match self.d.topics.iter().find(|x| x.name == inv.topic) {
                    None => self.err(
                        "E_UI_TOPIC_UNKNOWN",
                        &p,
                        format!("topic {} is not declared", inv.topic),
                    ),
                    Some(t) => {
                        for k in &inv.key {
                            if self.path(&t.key, &k.field).is_none() {
                                self.err(
                                    "E_UI_FIELD_UNKNOWN",
                                    format!("{p}.key"),
                                    format!("{} has no field {}", t.key, k.field),
                                );
                            }
                            if self.path(&e.output, &k.from).is_none() {
                                self.err(
                                    "E_UI_FIELD_UNKNOWN",
                                    format!("{p}.key"),
                                    format!("{} has no field {}", e.output, k.from),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// The landing names a screen of the plugin that opens without arguments (`E_UI_LANDING`).
    fn landing(&mut self, ui: &Ui, screens: &HashMap<&str, &Screen>) {
        let Some(id) = &ui.landing else { return };
        match screens.get(id.as_str()) {
            None => self.err(
                "E_UI_LANDING",
                "ui.landing",
                format!("the landing {id:?} is not a screen of this plugin"),
            ),
            Some(s) => {
                let required: Vec<&str> = s
                    .params
                    .iter()
                    .filter(|p| p.required && p.default.is_none())
                    .map(|p| p.name.as_str())
                    .collect();
                if !required.is_empty() {
                    self.err(
                        "E_UI_LANDING",
                        "ui.landing",
                        format!(
                            "the landing screen {id} requires param {}: a plugin opens on it without arguments \
                             (give the param a default or make it optional)",
                            required.join(", ")
                        ),
                    );
                }
            }
        }
    }

    fn document(&mut self, ui: &'a Ui) {
        if ui.catalog == 0 || ui.catalog > CATALOG {
            self.err(
                "E_UI_CATALOG",
                "ui.catalog",
                format!(
                    "catalog revision {} is not known (this platform knows 1..={CATALOG})",
                    ui.catalog
                ),
            );
        }
        if let Ok(s) = serde_json::to_string(ui)
            && s.len() > MAX_DOCUMENT
        {
            self.err(
                "E_UI_LIMIT",
                "ui",
                format!("the document is {} bytes, at most {MAX_DOCUMENT}", s.len()),
            );
        }
        for (loc, m) in &ui.texts {
            let size: usize = m.iter().map(|(k, v)| k.len() + v.len()).sum();
            if size > MAX_TEXTS {
                self.err(
                    "E_UI_LIMIT",
                    format!("ui.texts[{loc}]"),
                    format!("the {loc} catalog is {size} bytes, at most {MAX_TEXTS}"),
                );
            }
        }
        if !ui.texts.is_empty() && !ui.texts.contains_key("en") {
            self.err(
                "E_UI_TEXT_LOCALE",
                "ui.texts",
                "a text catalog needs an en locale (the fallback)",
            );
        }
        if ui.screens.len() + ui.overlays.len() > MAX_SCREENS {
            self.err(
                "E_UI_LIMIT",
                "ui.screens",
                format!("at most {MAX_SCREENS} screens and overlays"),
            );
        }
        // document level: its queries and overlays are visible everywhere
        let mut doc = Scope::default();
        self.queries("ui", &ui.queries, &mut doc, &Scope::default());
        let mut ids = HashSet::new();
        for o in &ui.overlays {
            if !ids.insert(o.id.as_str()) {
                self.err(
                    "E_UI_DUP",
                    format!("ui.overlays[{}]", o.id),
                    format!("overlay {} is declared twice", o.id),
                );
            }
            doc.overlays.insert(&o.id);
        }
        let screens: HashMap<&str, &Screen> =
            ui.screens.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut seen = HashSet::new();
        for s in &ui.screens {
            if !snake(&s.id) {
                self.err(
                    "E_UI_NAME",
                    format!("ui.screens[{}].id", s.id),
                    format!("{:?} is not a screen id (snake_case)", s.id),
                );
            }
            if !seen.insert(s.id.as_str()) {
                self.err(
                    "E_UI_DUP",
                    format!("ui.screens[{}]", s.id),
                    format!("screen {} is declared twice", s.id),
                );
            }
        }
        for s in &ui.screens {
            let p = format!("ui.screens[{}]", s.id);
            self.text(&s.title, &format!("{p}.title"));
            let mut sc = doc.clone();
            self.params(&p, &s.params, &s.state, &mut sc);
            self.frame_form = self.form_request(&s.layout);
            self.queries(&p, &s.queries, &mut sc, &doc);
            self.frame_form = None;
            for o in &s.overlays {
                if !sc.overlays.insert(&o.id) {
                    self.err(
                        "E_UI_DUP",
                        format!("{p}.overlays[{}]", o.id),
                        format!("overlay {} is declared twice", o.id),
                    );
                }
            }
            self.action_names(&p, &s.actions, &mut sc);
            self.nodes = 0;
            self.begin_frame();
            for (i, n) in s.header.iter().enumerate() {
                self.node(
                    n,
                    &format!("{p}.header[{i}]"),
                    &sc,
                    &Ctx::default(),
                    &screens,
                );
            }
            self.node(
                &s.layout,
                &format!("{p}.layout"),
                &sc,
                &Ctx::default(),
                &screens,
            );
            self.end_frame(&p);
            for o in &s.overlays {
                self.overlay(&format!("{p}.overlays[{}]", o.id), o, &sc, &screens);
            }
            self.nodes_limit(&p);
            self.actions(&p, &s.actions, &sc, &screens);
        }
        for o in &ui.overlays {
            self.nodes = 0;
            self.overlay(&format!("ui.overlays[{}]", o.id), o, &doc, &screens);
            self.nodes_limit(&format!("ui.overlays[{}]", o.id));
        }
        // contributions
        self.landing(ui, &screens);
        for (i, g) in ui.nav.iter().enumerate() {
            let p = format!("ui.nav[{i}]");
            self.text(&g.title, &format!("{p}.title"));
            for (j, e) in g.entries.iter().enumerate() {
                self.nav_entry(e, &format!("{p}.entries[{j}]"), &doc, &screens);
            }
        }
        for (i, w) in ui.home.iter().enumerate() {
            let p = format!(
                "ui.home[{}]",
                if w.id.is_empty() {
                    i.to_string()
                } else {
                    w.id.clone()
                }
            );
            self.text(&w.title, &format!("{p}.title"));
            self.node(
                &w.node,
                &format!("{p}.node"),
                &doc,
                &Ctx::default(),
                &screens,
            );
            if let Some(t) = &w.open {
                self.target(
                    t,
                    &format!("{p}.open"),
                    &doc,
                    &Ctx::default(),
                    &screens,
                    false,
                );
            }
        }
        let mut seen = HashSet::new();
        for c in &ui.commands {
            let p = format!("ui.commands[{}]", c.id);
            if !seen.insert(c.id.as_str()) {
                self.err(
                    "E_UI_DUP",
                    &p,
                    format!("command {} is declared twice", c.id),
                );
            }
            self.text(&c.title, &format!("{p}.title"));
            self.target(
                &c.target,
                &format!("{p}.target"),
                &doc,
                &Ctx::default(),
                &screens,
                true,
            );
        }
        for (i, src) in ui.search.iter().enumerate() {
            let p = format!("ui.search[{i}]");
            self.text(&src.title, &format!("{p}.title"));
            let Some(e) = self.endpoint(&src.endpoint, &format!("{p}.endpoint")) else {
                continue;
            };
            if !read_only(e) {
                self.err(
                    "E_UI_QUERY_MUTATES",
                    format!("{p}.endpoint"),
                    format!("{} may change data; a search reads", e.name),
                );
            }
            let (input, output) = (e.input.clone(), e.output.clone());
            if self
                .path(&input, &src.text)
                .is_none_or(|f| f.ty != "string")
            {
                self.err(
                    "E_UI_FIELD_UNKNOWN",
                    format!("{p}.text"),
                    format!("{input} has no string field {}", src.text),
                );
            }
            let msg = match self
                .path(&output, &src.rows)
                .map(|f| (f.repeated, FieldType::parse(&f.ty)))
            {
                Some((true, Some(FieldType::Message(m)))) => m,
                _ => {
                    self.err(
                        "E_UI_ROWS",
                        format!("{p}.rows"),
                        format!("{output}.{} is not a repeated message field", src.rows),
                    );
                    continue;
                }
            };
            let rc = Ctx {
                row: Some(msg.clone()),
                ..Default::default()
            };
            self.row_view(&src.item, &msg, &format!("{p}.item"), &doc, &rc);
            self.target(&src.open, &format!("{p}.open"), &doc, &rc, &screens, true);
            for (j, a) in src.actions.iter().enumerate() {
                let ap = format!("{p}.actions[{j}]");
                self.text(&a.title, &format!("{ap}.title"));
                self.target(
                    &a.target,
                    &format!("{ap}.target"),
                    &doc,
                    &rc,
                    &screens,
                    true,
                );
            }
        }
        for m in &ui.menus {
            let p = format!("ui.menus[{}]", m.message);
            if self.message(&m.message).is_none() {
                self.err(
                    "E_UI_FIELD_UNKNOWN",
                    format!("{p}.message"),
                    format!("message {} is not declared", m.message),
                );
            }
            let mut sc = doc.clone();
            self.action_names(&p, &m.actions, &mut sc);
            let ctx = Ctx {
                row: Some(m.message.clone()),
                ..Default::default()
            };
            for (i, it) in m.items.iter().enumerate() {
                self.menu_item(it, &format!("{p}.items[{i}]"), &sc, &ctx);
            }
            self.actions(&p, &m.actions, &sc, &screens);
        }
        // text keys
        let keys = collect_keys(ui);
        for (path, key) in keys {
            if !self.en.contains(key.as_str()) {
                self.err(
                    "E_UI_TEXT_UNKNOWN",
                    path,
                    format!("text key {key} is not in the en catalog"),
                );
            }
        }
    }

    fn nodes_limit(&mut self, p: &str) {
        if self.nodes > MAX_NODES {
            self.err(
                "E_UI_LIMIT",
                p,
                format!("{} nodes, at most {MAX_NODES}", self.nodes),
            );
        }
    }

    fn overlay(
        &mut self,
        p: &str,
        o: &'a Overlay,
        outer: &Scope<'a>,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        if !snake(&o.id) {
            self.err(
                "E_UI_NAME",
                format!("{p}.id"),
                format!("{:?} is not an overlay id (snake_case)", o.id),
            );
        }
        self.text(&o.title, &format!("{p}.title"));
        let mut sc = outer.clone();
        sc.params.clear();
        sc.defaults.clear();
        sc.state.clear();
        sc.actions.clear();
        self.params(p, &o.params, &o.state, &mut sc);
        let base = outer.clone();
        self.frame_form = self.form_request(&o.layout);
        self.queries(p, &o.queries, &mut sc, &base);
        self.frame_form = None;
        self.action_names(p, &o.actions, &mut sc);
        self.begin_frame();
        for (i, n) in o.buttons.iter().enumerate() {
            self.node(
                n,
                &format!("{p}.buttons[{i}]"),
                &sc,
                &Ctx::default(),
                screens,
            );
        }
        self.node(
            &o.layout,
            &format!("{p}.layout"),
            &sc,
            &Ctx::default(),
            screens,
        );
        self.end_frame(p);
        self.actions(p, &o.actions, &sc, screens);
    }

    fn begin_frame(&mut self) {
        self.frame = Frame {
            columns: vec![0],
            ..Frame::default()
        };
    }

    /// The frame's composers belong to its streams; one primary composer.
    fn end_frame(&mut self, p: &str) {
        let f = std::mem::take(&mut self.frame);
        for (path, id) in &f.fors {
            if !f.streams.contains(id) {
                self.err(
                    "E_UI_STREAM_UNKNOWN",
                    format!("{path}.for"),
                    format!("no stream of this frame has the id {id}"),
                );
            }
        }
        if f.primaries > 1 {
            self.err(
                "E_UI_DUP",
                p,
                format!("{} primary composers; a frame has one", f.primaries),
            );
        }
    }

    fn params(&mut self, p: &str, params: &'a [Param], state: &'a [State], sc: &mut Scope<'a>) {
        for x in params {
            if !snake(&x.name) {
                self.err(
                    "E_UI_NAME",
                    format!("{p}.params[{}]", x.name),
                    format!("{:?} is not a param name", x.name),
                );
            }
            if FieldType::parse(&x.ty).is_none() {
                self.err(
                    "E_UI_STATE_TYPE",
                    format!("{p}.params[{}].type", x.name),
                    format!("{:?} is not a type", x.ty),
                );
            }
            if sc.params.insert(&x.name, &x.ty).is_some() {
                self.err(
                    "E_UI_DUP",
                    format!("{p}.params[{}]", x.name),
                    format!("param {} is declared twice", x.name),
                );
            }
            if let Some(d) = &x.default {
                sc.defaults.insert(&x.name, d);
                if let Some(why) = self.param_default(&x.ty, d) {
                    self.err(
                        "E_UI_PARAM_DEFAULT",
                        format!("{p}.params[{}].default", x.name),
                        format!("the default of param {} {why}", x.name),
                    );
                }
            }
        }
        for x in state {
            if !snake(&x.name) {
                self.err(
                    "E_UI_NAME",
                    format!("{p}.state[{}]", x.name),
                    format!("{:?} is not a state name", x.name),
                );
            }
            if FieldType::parse(&x.ty).is_none() {
                self.err(
                    "E_UI_STATE_TYPE",
                    format!("{p}.state[{}].type", x.name),
                    format!("{:?} is not a type", x.ty),
                );
            }
            if sc.state.insert(&x.name, &x.ty).is_some() {
                self.err(
                    "E_UI_DUP",
                    format!("{p}.state[{}]", x.name),
                    format!("state {} is declared twice", x.name),
                );
            }
            if let Some(d) = &x.default {
                sc.state_defaults.insert(&x.name, d);
            }
            for r in &x.reset {
                if !sc.params.contains_key(r.as_str()) {
                    self.err(
                        "E_UI_PARAM_UNKNOWN",
                        format!("{p}.state[{}].reset", x.name),
                        format!("param {r} is not declared"),
                    );
                }
            }
        }
    }

    /// What is wrong with a param's default, if anything. A default is what the param is when the link has none, so
    /// it is a value a link can carry: a string, a number or a boolean of the param's type, a value of its enum.
    fn param_default(&self, ty: &str, d: &serde_json::Value) -> Option<String> {
        use serde_json::Value as J;
        if !matches!(d, J::String(_) | J::Number(_) | J::Bool(_)) {
            return Some("is a string, a number or a boolean (a value a link can carry)".into());
        }
        if !lit_fits(ty, false, d) {
            return Some(format!("is a {ty}, not {d}"));
        }
        if let (Some(FieldType::Enum(e)), J::String(v)) = (FieldType::parse(ty), d)
            && let Some(known) = self.d.enums.iter().find(|x| x.name == e)
            && !known.values.contains(v)
        {
            return Some(format!("is a value of {e}, not {v:?}"));
        }
        None
    }

    fn queries(&mut self, p: &str, qs: &'a [Query], sc: &mut Scope<'a>, outer: &Scope<'a>) {
        if qs.len() > MAX_QUERIES {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.queries"),
                format!("at most {MAX_QUERIES} queries"),
            );
        }
        let mut followed = HashSet::new();
        for q in qs {
            let qp = format!("{p}.queries[{}]", q.name);
            if !snake(&q.name) {
                self.err(
                    "E_UI_NAME",
                    format!("{qp}.name"),
                    format!("{:?} is not a query name", q.name),
                );
            }
            if sc.queries.insert(&q.name, q).is_some()
                && !outer.queries.contains_key(q.name.as_str())
            {
                self.err(
                    "E_UI_DUP",
                    &qp,
                    format!("query {} is declared twice", q.name),
                );
            }
            let Some(e) = self.endpoint(&q.endpoint, &format!("{qp}.endpoint")) else {
                continue;
            };
            if !read_only(e) {
                self.errors.push(
                    Error::new("E_UI_QUERY_MUTATES", format!("{qp}.endpoint"), format!("{} may change data; a query runs by itself", e.name))
                        .help("queries call read-only endpoints (.read_only()); changes go through actions"),
                );
            }
            let ctx = Ctx {
                form: self.frame_form.clone(),
                ..Default::default()
            };
            for a in &q.args {
                self.arg(a, &e.input, &format!("{qp}.args[{}]", a.field), sc, &ctx);
            }
            if let Some(c) = &q.when {
                self.cond(c, &format!("{qp}.when"), sc, &ctx);
            }
            if let Some(r) = &q.rows {
                match self.path(&e.output, r) {
                    Some(f)
                        if f.repeated
                            && matches!(FieldType::parse(&f.ty), Some(FieldType::Message(_))) => {}
                    Some(_) => self.err(
                        "E_UI_ROWS",
                        format!("{qp}.rows"),
                        format!("{}.{r} is not a repeated message field", e.output),
                    ),
                    None => self.err(
                        "E_UI_FIELD_UNKNOWN",
                        format!("{qp}.rows"),
                        format!("{} has no field {r}", e.output),
                    ),
                }
            }
            if q.paged
                && (self.path(&e.input, "page_token").is_none()
                    || self.path(&e.output, "next_page_token").is_none())
            {
                self.err(
                    "E_UI_PAGED",
                    format!("{qp}.paged"),
                    format!(
                        "a paged query needs page_token in {} and next_page_token in {}",
                        e.input, e.output
                    ),
                );
            }
            if let Some(f) = &q.feed {
                if self.path(&e.input, &f.after).is_none() {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        format!("{qp}.feed.after"),
                        format!("{} has no field {}", e.input, f.after),
                    );
                }
                if self.path(&e.output, &f.cursor).is_none() {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        format!("{qp}.feed.cursor"),
                        format!("{} has no field {}", e.output, f.cursor),
                    );
                }
                if q.rows.is_none() {
                    self.err("E_UI_ROWS", format!("{qp}.feed"), "a feed needs rows");
                }
            }
            for (i, f) in q.follow.iter().enumerate() {
                followed.insert(f.topic.as_str());
                match self.d.topics.iter().find(|t| t.name == f.topic) {
                    None => self.err(
                        "E_UI_TOPIC_UNKNOWN",
                        format!("{qp}.follow[{i}]"),
                        format!("topic {} is not declared", f.topic),
                    ),
                    Some(t) => {
                        for a in &f.key {
                            self.arg(
                                a,
                                &t.key,
                                &format!("{qp}.follow[{i}].key[{}]", a.field),
                                sc,
                                &ctx,
                            );
                        }
                    }
                }
            }
            self.query_options(q, e, &qp);
            if let Some(s) = q.poll
                && s < MIN_POLL
            {
                self.err(
                    "E_UI_POLL",
                    format!("{qp}.poll"),
                    format!("poll every {s} s; at least {MIN_POLL}"),
                );
            }
        }
        if followed.len() > MAX_FOLLOWED {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.queries"),
                format!("{} topics followed, at most {MAX_FOLLOWED}", followed.len()),
            );
        }
    }

    fn action_names(&mut self, p: &str, actions: &'a [Action], sc: &mut Scope<'a>) {
        for a in actions {
            if !snake(&a.name) {
                self.err(
                    "E_UI_NAME",
                    format!("{p}.actions[{}]", a.name),
                    format!("{:?} is not an action name", a.name),
                );
            }
            if sc.actions.insert(&a.name, a).is_some() {
                self.err(
                    "E_UI_DUP",
                    format!("{p}.actions[{}]", a.name),
                    format!("action {} is declared twice", a.name),
                );
            }
        }
    }

    /// Validates the chains, each in the contexts it is used from (`row`, `form`).
    fn actions(
        &mut self,
        p: &str,
        actions: &'a [Action],
        sc: &Scope<'a>,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        for a in actions {
            let ap = format!("{p}.actions[{}]", a.name);
            let ctxs = self
                .used
                .get(&(p.to_string(), a.name.as_str()))
                .cloned()
                .unwrap_or_else(|| vec![Ctx::default()]);
            let ctx = merge(&ctxs);
            let mut c = ctx.clone();
            for (i, s) in a.steps.iter().enumerate() {
                self.step(s, &format!("{ap}.steps[{i}]"), sc, &mut c, screens);
            }
            let mut c = Ctx { error: true, ..ctx };
            for (i, s) in a.on_error.iter().enumerate() {
                self.step(s, &format!("{ap}.on_error[{i}]"), sc, &mut c, screens);
            }
        }
    }

    fn step(
        &mut self,
        s: &Step,
        p: &str,
        sc: &Scope<'a>,
        ctx: &mut Ctx,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        match s {
            Step::Call(c) => {
                if let Some(q) = &c.pending
                    && !sc.queries.contains_key(q.as_str())
                {
                    self.err(
                        "E_UI_QUERY_UNKNOWN",
                        format!("{p}.pending"),
                        format!("query {q} is not declared here"),
                    );
                }
                if let Some(e) = self.endpoint(&c.endpoint, &format!("{p}.endpoint")) {
                    for a in &c.args {
                        self.arg(a, &e.input, &format!("{p}.args[{}]", a.field), sc, ctx);
                    }
                    if c.form {
                        match &ctx.form {
                            None => self.err(
                                "E_UI_REF_CONTEXT",
                                format!("{p}.form"),
                                "form values are only available to a form's submit action",
                            ),
                            // the call takes the form's message, or one with each of its fields (a draft)
                            Some(f) if *f != e.input && !self.covers(&e.input, f) => self.err(
                                "E_UI_ARG_TYPE",
                                format!("{p}.form"),
                                format!("the form fills {f}, the call takes {}", e.input),
                            ),
                            _ => {}
                        }
                    }
                    ctx.result = Some(e.output.clone());
                }
            }
            Step::Confirm(c) => {
                self.text(&c.title, &format!("{p}.title"));
                if let Some(t) = &c.text {
                    self.text(t, &format!("{p}.text"));
                }
                if let Some(v) = &c.typed {
                    self.value(v, &format!("{p}.typed"), sc, ctx);
                }
                for (k, t) in [("cancel_label", &c.cancel_label), ("summary", &c.summary)] {
                    if let Some(t) = t {
                        self.text(t, &format!("{p}.{k}"));
                    }
                }
            }
            Step::Open(t) => {
                if t.overlay.is_none() || t.screen.is_some() {
                    self.err("E_UI_TARGET", p, "open names an overlay");
                }
                self.target(t, p, sc, ctx, screens, true);
            }
            Step::Navigate(t) => {
                if t.screen.is_none() || t.overlay.is_some() {
                    self.err("E_UI_TARGET", p, "navigate names a screen");
                }
                self.target(t, p, sc, ctx, screens, false);
            }
            Step::Set(x) => {
                match (&x.state, &x.param) {
                    (Some(s), None) if !sc.state.contains_key(s.as_str()) => self.err(
                        "E_UI_STATE_UNKNOWN",
                        format!("{p}.state"),
                        format!("state {s} is not declared"),
                    ),
                    (None, Some(s)) if !sc.params.contains_key(s.as_str()) => self.err(
                        "E_UI_PARAM_UNKNOWN",
                        format!("{p}.param"),
                        format!("param {s} is not declared"),
                    ),
                    (Some(_), None) | (None, Some(_)) => {}
                    _ => self.err("E_UI_TARGET", p, "set names one state or one param"),
                }
                self.value(&x.value, &format!("{p}.value"), sc, ctx);
            }
            Step::Refresh { queries } => {
                for q in queries {
                    if !sc.queries.contains_key(q.as_str()) {
                        self.err(
                            "E_UI_QUERY_UNKNOWN",
                            format!("{p}.queries"),
                            format!("query {q} is not declared here"),
                        );
                    }
                }
            }
            Step::Toast(t) => {
                if let Some(x) = &t.text {
                    self.text(x, &format!("{p}.text"));
                }
                if let Some(x) = &t.description {
                    self.text(x, &format!("{p}.description"));
                }
                if let Some(a) = &t.action {
                    self.text(&a.label, &format!("{p}.action.label"));
                    if !sc.actions.contains_key(a.action.as_str()) {
                        self.err(
                            "E_UI_ACTION_UNKNOWN",
                            format!("{p}.action"),
                            format!("action {} is not declared here", a.action),
                        );
                    }
                }
                if t.error && !ctx.error {
                    self.err(
                        "E_UI_REF_CONTEXT",
                        format!("{p}.error"),
                        "an error toast belongs in on_error",
                    );
                }
            }
            Step::OpenUrl { value } => {
                if let Value::Lit(v) = value
                    && !v.as_str().is_some_and(|u| u.starts_with("https://"))
                {
                    self.err(
                        "E_UI_URL",
                        format!("{p}.value"),
                        "a URL is an https literal or a response field",
                    );
                }
                self.value(value, &format!("{p}.value"), sc, ctx);
            }
            Step::Draft { text } => self.text_val(text, &format!("{p}.text"), sc, ctx),
            Step::ReplyTo { name, key } => {
                self.value(name, &format!("{p}.name"), sc, ctx);
                self.value(key, &format!("{p}.key"), sc, ctx);
            }
            Step::ScrollTo { key } => {
                self.value(key, &format!("{p}.key"), sc, ctx);
            }
            Step::CopyValue(c) => match (&c.value, &c.link) {
                (Some(v), None) | (None, Some(v)) => {
                    self.value(v, p, sc, ctx);
                }
                _ => self.err("E_UI_SHAPE", p, "copy takes one of value and link"),
            },
            Step::Merge { query, row } => {
                let msg = self.rows_message(query, sc, p);
                let got = self.value(row, &format!("{p}.row"), sc, ctx);
                if let (Some(m), Some(g)) = (msg, got)
                    && let Some(FieldType::Message(x)) = FieldType::parse(&g)
                    && x != m
                {
                    self.err(
                        "E_UI_ARG_TYPE",
                        format!("{p}.row"),
                        format!("the rows of {query} are {m}, the value is {x}"),
                    );
                }
            }
            Step::Close {} | Step::Back {} => {}
        }
    }

    fn target(
        &mut self,
        t: &Target,
        p: &str,
        sc: &Scope<'a>,
        ctx: &Ctx,
        screens: &HashMap<&str, &'a Screen>,
        overlay_ok: bool,
    ) {
        if let Some(s) = &t.screen {
            match screens.get(s.as_str()) {
                None => self.err(
                    "E_UI_SCREEN_UNKNOWN",
                    format!("{p}.screen"),
                    format!("screen {s} is not declared"),
                ),
                Some(scr) => {
                    for a in &t.args {
                        if !scr.params.iter().any(|x| x.name == a.field) {
                            self.err(
                                "E_UI_PARAM_UNKNOWN",
                                format!("{p}.args[{}]", a.field),
                                format!("screen {s} has no param {}", a.field),
                            );
                        }
                        self.value(&a.value, &format!("{p}.args[{}]", a.field), sc, ctx);
                    }
                    for x in scr
                        .params
                        .iter()
                        .filter(|x| x.required && x.default.is_none())
                    {
                        if !t.args.iter().any(|a| a.field == x.name) {
                            self.err(
                                "E_UI_PARAM_MISSING",
                                format!("{p}.args"),
                                format!("screen {s} requires param {}", x.name),
                            );
                        }
                    }
                }
            }
        }
        if let Some(o) = &t.overlay {
            if !overlay_ok {
                self.err("E_UI_TARGET", p, "this target is a screen");
            }
            if !sc.overlays.contains(o.as_str()) {
                self.err(
                    "E_UI_OVERLAY_UNKNOWN",
                    format!("{p}.overlay"),
                    format!("overlay {o} is not declared here"),
                );
            }
            for a in &t.args {
                self.value(&a.value, &format!("{p}.args[{}]", a.field), sc, ctx);
            }
        }
        if t.screen.is_none() && t.overlay.is_none() {
            self.err("E_UI_TARGET", p, "a target names a screen or an overlay");
        }
    }

    fn nav_entry(
        &mut self,
        e: &'a NavEntry,
        p: &str,
        doc: &Scope<'a>,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        self.text(&e.title, &format!("{p}.title"));
        if let Some(s) = &e.screen {
            let t = Target {
                screen: Some(s.clone()),
                overlay: None,
                args: e.args.clone(),
            };
            self.target(&t, p, doc, &Ctx::default(), screens, false);
        } else if e.children.is_empty() && e.rows.is_none() {
            self.err("E_UI_TARGET", p, "an entry opens a screen or has children");
        }
        if let Some(b) = &e.badge {
            self.value(b, &format!("{p}.badge"), doc, &Ctx::default());
        }
        for (i, c) in e.children.iter().enumerate() {
            if !c.children.is_empty() {
                self.err(
                    "E_UI_NAV_DEPTH",
                    format!("{p}.children[{i}]"),
                    "navigation has one level of children",
                );
            }
            self.nav_entry(c, &format!("{p}.children[{i}]"), doc, screens);
        }
        if let Some(r) = &e.rows {
            let rp = format!("{p}.rows");
            if let Some(msg) = self.rows_message(&r.query, doc, &rp) {
                for f in [Some(&r.label), r.badge.as_ref(), r.mentions.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    if self.path(&msg, f).is_none() {
                        self.err("E_UI_FIELD_UNKNOWN", &rp, format!("{msg} has no field {f}"));
                    }
                }
                let ctx = Ctx {
                    row: Some(msg),
                    ..Default::default()
                };
                let t = Target {
                    screen: Some(r.screen.clone()),
                    overlay: None,
                    args: r.args.clone(),
                };
                self.target(&t, &rp, doc, &ctx, screens, false);
            }
            if !(1..=20).contains(&r.limit) {
                self.err(
                    "E_UI_LIMIT",
                    format!("{rp}.limit"),
                    "nav rows show 1 to 20 rows",
                );
            }
            if let Some(a) = &r.add
                && !doc.overlays.contains(a.overlay.as_str())
            {
                self.err(
                    "E_UI_OVERLAY_UNKNOWN",
                    format!("{rp}.add"),
                    format!("overlay {} is not a document overlay", a.overlay),
                );
            }
        }
    }

    fn menu_item(&mut self, it: &'a MenuItem, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        self.text(&it.label, &format!("{p}.label"));
        self.use_action(&it.action, p, sc, ctx);
        for c in [&it.visible, &it.enabled].into_iter().flatten() {
            self.cond(c, p, sc, ctx);
        }
    }

    fn use_action(&mut self, name: &str, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        match sc.actions.get_key_value(name) {
            None => self.err(
                "E_UI_ACTION_UNKNOWN",
                format!("{p}.action"),
                format!("action {name} is not declared here"),
            ),
            Some((k, _)) => {
                let scope = scope_path(p);
                self.used.entry((scope, k)).or_default().push(ctx.clone());
            }
        }
    }

    fn node(
        &mut self,
        n: &'a Node,
        p: &str,
        sc: &Scope<'a>,
        ctx: &Ctx,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        self.nodes += 1;
        for c in [&n.visible, &n.enabled].into_iter().flatten() {
            self.cond(c, p, sc, ctx);
        }
        if let Some(f) = &n.fallback {
            if n.kind.interactive() {
                self.err(
                    "E_UI_FALLBACK",
                    format!("{p}.fallback"),
                    "only display primitives have a fallback",
                );
            }
            self.node(f, &format!("{p}.fallback"), sc, ctx, screens);
        }
        let k = n.kind.name();
        let p = format!("{p}.{k}");
        match &n.kind {
            Kind::Table(t) => {
                if let Some(msg) = self.rows_message(&t.query, sc, &p) {
                    for c in &t.columns {
                        self.text(&c.title, &format!("{p}.columns[{}].title", c.field));
                        self.row_field(&msg, &c.field, &format!("{p}.columns[{}]", c.field));
                        if let Some(by) = &c.sort {
                            self.sort_field(
                                &t.query,
                                by,
                                &format!("{p}.columns[{}].sort", c.field),
                                sc,
                            );
                        }
                        if let Some(Format::Money {
                            currency: Some(cur),
                            ..
                        }) = &c.format
                        {
                            self.row_field(&msg, cur, &format!("{p}.columns[{}].format", c.field));
                        }
                    }
                    let rc = Ctx {
                        row: Some(msg.clone()),
                        ..ctx.clone()
                    };
                    if let Some(a) = &t.open {
                        self.use_action(a, &format!("{p}.open"), sc, &rc);
                    }
                    for (i, b) in t.row_actions.iter().enumerate() {
                        self.node(b, &format!("{p}.row_actions[{i}]"), sc, &rc, screens);
                    }
                    for tw in &t.row_tone {
                        self.cond(&tw.when, &format!("{p}.row_tone"), sc, &rc);
                    }
                    for c in &t.columns {
                        for f in [c.subtitle.as_ref(), c.avatar.as_ref()]
                            .into_iter()
                            .flatten()
                        {
                            self.row_field(&msg, f, &format!("{p}.columns[{}]", c.field));
                        }
                    }
                    if let Some(v) = &t.compact {
                        self.row_view(v, &msg, &format!("{p}.compact"), sc, &rc);
                    }
                    if let Some(s) = &t.checks {
                        if !sc.state.contains_key(s.state.as_str()) {
                            self.err(
                                "E_UI_STATE_UNKNOWN",
                                format!("{p}.checks"),
                                format!("state {} is not declared", s.state),
                            );
                        }
                        self.row_field(&msg, &s.key, &format!("{p}.checks.key"));
                    }
                    if let Some(s) = &t.select {
                        if !sc.state.contains_key(s.state.as_str())
                            && !sc.params.contains_key(s.state.as_str())
                        {
                            self.err(
                                "E_UI_STATE_UNKNOWN",
                                format!("{p}.select"),
                                format!("state {} is not declared", s.state),
                            );
                        }
                        self.row_field(&msg, &s.key, &format!("{p}.select.key"));
                    }
                }
                for (i, b) in t.toolbar.iter().enumerate() {
                    self.node(b, &format!("{p}.toolbar[{i}]"), sc, ctx, screens);
                }
                if let Some(v) = &t.count {
                    self.value(v, &format!("{p}.count"), sc, ctx);
                }
                if let Some(l) = &t.count_label {
                    self.text(l, &format!("{p}.count_label"));
                }
            }
            Kind::List(l) => {
                if let Some(t) = &l.search {
                    self.text(t, &format!("{p}.search"));
                }
                if let Some(t) = &l.group_label {
                    self.text_val(t, &format!("{p}.group_label"), sc, ctx);
                }
                if l.limit.is_some_and(|n| !(1..=100).contains(&n)) {
                    self.err(
                        "E_UI_LIMIT",
                        format!("{p}.limit"),
                        "a list's phone limit is 1 to 100 rows",
                    );
                }
                if let Some(msg) = self.rows_message(&l.query, sc, &p) {
                    for f in [
                        Some(&l.title),
                        l.subtitle.as_ref(),
                        l.meta.as_ref(),
                        l.avatar.as_ref(),
                        l.icon_field.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.row_field(&msg, f, &p);
                    }
                    if let Some(s) = &l.status {
                        self.row_field(&msg, &s.field, &format!("{p}.status"));
                    }
                    if let Some(f) = &l.online {
                        self.row_field(&msg, f, &format!("{p}.online"));
                    }
                    let rc = Ctx {
                        row: Some(msg),
                        ..ctx.clone()
                    };
                    if let Some(a) = &l.open {
                        self.use_action(a, &format!("{p}.open"), sc, &rc);
                    }
                    for (i, b) in l.row_actions.iter().enumerate() {
                        self.node(b, &format!("{p}.row_actions[{i}]"), sc, &rc, screens);
                    }
                }
            }
            Kind::Split(x) => self.split(x, &p, sc),
            Kind::Header(h) => self.header(h, &p, sc, ctx),
            Kind::Stream(x) => self.stream(x, &p, sc, ctx),
            Kind::Process(x) => match &x.query {
                Some(q) => {
                    let out = sc
                        .queries
                        .get(q.as_str())
                        .and_then(|d| self.d.endpoints.iter().find(|e| e.name == d.endpoint))
                        .map(|e| e.output.clone());
                    match out {
                        Some(out) => self.process(x, &out, &[], &p, sc, ctx),
                        None => {
                            if !sc.queries.contains_key(q.as_str()) {
                                self.err(
                                    "E_UI_QUERY_UNKNOWN",
                                    format!("{p}.query"),
                                    format!("query {q} is not declared here"),
                                );
                            }
                        }
                    }
                }
                None => self.err(
                    "E_UI_SHAPE",
                    format!("{p}.query"),
                    "a process on its own reads a query",
                ),
            },
            Kind::Strip(x) => self.strip(x, &p, sc, ctx),
            Kind::Composer(x) => self.composer(x, &p, sc, ctx),
            Kind::Chart(c) => {
                if let Some(msg) = self.rows_message(&c.query, sc, &p) {
                    self.row_field(&msg, &c.x, &format!("{p}.x"));
                    for s in &c.series {
                        self.row_field(&msg, &s.field, &format!("{p}.series[{}]", s.field));
                    }
                }
            }
            Kind::Viz(v) => self.viz(v, &p, sc, ctx, screens),
            Kind::Board(b) => {
                if self.in_board {
                    self.errors.push(
                        Error::new(
                            "E_UI_BOARD_NESTED",
                            &p,
                            "a board is not nested in another board",
                        )
                        .help("put the cards on the one board: its grid is shared by all of them"),
                    );
                }
                if b.children.len() > BOARD_MAX_ITEMS {
                    self.err(
                        "E_UI_LIMIT",
                        format!("{p}.children"),
                        format!("{} items, at most {BOARD_MAX_ITEMS}", b.children.len()),
                    );
                }
                for (i, c) in b.children.iter().enumerate() {
                    if !(1..=BOARD_COLUMNS).contains(&c.span) {
                        self.err(
                            "E_UI_LIMIT",
                            format!("{p}.children[{i}].span"),
                            format!("a span is 1 to {BOARD_COLUMNS} columns, not {}", c.span),
                        );
                    }
                }
            }
            Kind::Chat(c) => {
                // the shape the client draws: the conversation's messages, the thread's replies, the run's steps
                let mut message: Option<String> = None;
                for (q, field, need) in [
                    (
                        Some(&c.channel),
                        "messages",
                        &["id", "author", "text", "time"][..],
                    ),
                    (
                        c.thread.as_ref(),
                        "replies",
                        &["id", "author", "text", "time"][..],
                    ),
                    (c.run.as_ref(), "steps", &["title", "state"][..]),
                    (c.members.as_ref(), "people", &["name"][..]),
                    (c.directory.as_ref(), "people", &["name"][..]),
                ] {
                    let Some(q) = q else { continue };
                    let Some(def) = sc.queries.get(q.as_str()) else {
                        self.err(
                            "E_UI_QUERY_UNKNOWN",
                            format!("{p}.query"),
                            format!("query {q} is not declared here"),
                        );
                        continue;
                    };
                    let Some(e) = self.d.endpoints.iter().find(|e| e.name == def.endpoint) else {
                        continue;
                    };
                    let rows =
                        self.path(&e.output, field)
                            .and_then(|f| match FieldType::parse(&f.ty) {
                                Some(FieldType::Message(m)) => Some(m),
                                _ => None,
                            });
                    match rows {
                        Some(m) => {
                            for n in need {
                                if self.path(&m, n).is_none() {
                                    self.err(
                                        "E_UI_FIELD_UNKNOWN",
                                        format!("{p}.{q}"),
                                        format!("{m} has no field {n}"),
                                    );
                                }
                            }
                            if field == "messages" {
                                message = Some(m);
                            }
                        }
                        None => self.err(
                            "E_UI_ROWS",
                            format!("{p}.{q}"),
                            format!("{} has no repeated message field {field}", e.output),
                        ),
                    }
                }
                if let Some(q) = &c.about {
                    match sc.queries.get(q.as_str()) {
                        None => self.err(
                            "E_UI_QUERY_UNKNOWN",
                            format!("{p}.about"),
                            format!("query {q} is not declared here"),
                        ),
                        Some(def) => {
                            if let Some(e) =
                                self.d.endpoints.iter().find(|e| e.name == def.endpoint)
                                && self.path(&e.output, "name").is_none()
                            {
                                self.err(
                                    "E_UI_FIELD_UNKNOWN",
                                    format!("{p}.about"),
                                    format!("{} has no field name", e.output),
                                );
                            }
                        }
                    }
                }
                // the bell's state: the thread view says whether the reader follows the thread
                if c.actions.follow.is_some() {
                    let thread = c.thread.as_ref().and_then(|q| sc.queries.get(q.as_str()));
                    match thread {
                        None => self.err(
                            "E_UI_QUERY_UNKNOWN",
                            format!("{p}.actions.follow"),
                            "a thread's follow needs the chat's thread query".to_string(),
                        ),
                        Some(def) => {
                            if let Some(e) =
                                self.d.endpoints.iter().find(|e| e.name == def.endpoint)
                                && self.path(&e.output, "following").is_none()
                            {
                                self.err(
                                    "E_UI_FIELD_UNKNOWN",
                                    format!("{p}.thread"),
                                    format!("{} has no field following", e.output),
                                );
                            }
                        }
                    }
                }
                let a = &c.actions;
                let row = Ctx {
                    row: message,
                    ..ctx.clone()
                };
                let form = Ctx {
                    form: Some(CHAT_FORM.into()),
                    ..ctx.clone()
                };
                let row_form = Ctx {
                    form: Some(CHAT_FORM.into()),
                    ..row.clone()
                };
                self.use_action(&a.send, &format!("{p}.actions.send"), sc, &form);
                for (name, act, cx) in [
                    ("reply", &a.reply, &form),
                    ("tell", &a.tell, &form),
                    ("members", &a.members, ctx),
                    ("close", &a.close, ctx),
                    ("stop", &a.stop, ctx),
                    ("pause", &a.pause, ctx),
                    ("start", &a.start, &form),
                    ("follow", &a.follow, &form),
                    ("edit", &a.edit, &row_form),
                    ("delete", &a.delete, &row),
                    ("pin", &a.pin, &row),
                    ("choose", &a.choose, &row_form),
                    ("thread", &a.thread, &row),
                    ("run", &a.run, &row),
                    ("approve", &a.approve, &row),
                    ("decline", &a.decline, &row),
                    ("undo", &a.undo, &row),
                    ("card", &a.card, &row),
                    ("react", &a.react, &row_form),
                ] {
                    if let Some(x) = act {
                        self.use_action(x, &format!("{p}.actions.{name}"), sc, cx);
                    }
                }
            }
            Kind::Conversation(c) => {
                if let Some(msg) = self.rows_message(&c.query, sc, &p) {
                    let card = c.card.as_ref();
                    for f in [
                        Some(&c.author),
                        Some(&c.text),
                        Some(&c.time),
                        c.avatar.as_ref(),
                        c.icon.as_ref(),
                        c.badge.as_ref(),
                        c.via.as_ref(),
                        c.tag.as_ref(),
                        c.receipt.as_ref().map(|r| &r.text),
                        c.receipt
                            .as_ref()
                            .and_then(|r| r.tone.as_ref())
                            .map(|t| &t.field),
                        card.map(|k| &k.title),
                        card.and_then(|k| k.text.as_ref()),
                        card.and_then(|k| k.band.as_ref()),
                        card.and_then(|k| k.aside.as_ref()),
                        card.and_then(|k| k.tone.as_ref()).map(|t| &t.field),
                        card.and_then(|k| k.footer.as_ref()),
                        card.and_then(|k| k.button.as_ref()),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.row_field(&msg, f, &p);
                    }
                    if let Some(a) = card.and_then(|k| k.action.as_ref()) {
                        let rc = Ctx {
                            row: Some(msg),
                            ..ctx.clone()
                        };
                        self.use_action(a, &format!("{p}.card.action"), sc, &rc);
                    }
                }
                if let Some(e) = &c.empty {
                    self.text(e, &format!("{p}.empty"));
                }
                if let Some(m) = &c.composer {
                    self.text(&m.placeholder, &format!("{p}.composer.placeholder"));
                    for (i, o) in m.to.iter().enumerate() {
                        self.text(&o.label, &format!("{p}.composer.to[{i}]"));
                    }
                    let mc = Ctx {
                        form: Some(THREAD_COMPOSER.into()),
                        ..ctx.clone()
                    };
                    self.use_action(&m.action, &format!("{p}.composer"), sc, &mc);
                }
            }
            Kind::Timeline(t) => {
                if let Some(msg) = self.rows_message(&t.query, sc, &p) {
                    for f in [
                        Some(&t.text),
                        Some(&t.time),
                        t.actor.as_ref(),
                        t.avatar.as_ref(),
                        t.icon.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.row_field(&msg, f, &p);
                    }
                }
            }
            Kind::Fields(f) => {
                for (i, it) in f.items.iter().enumerate() {
                    self.text(&it.label, &format!("{p}.items[{i}].label"));
                    self.value(&it.value, &format!("{p}.items[{i}].value"), sc, ctx);
                    if let Some(t) = &it.open {
                        self.target(t, &format!("{p}.items[{i}].open"), sc, ctx, screens, false);
                    }
                    if let Some(a) = &it.avatar {
                        self.value(a, &format!("{p}.items[{i}].avatar"), sc, ctx);
                    }
                }
            }
            Kind::Metric(m) => {
                self.text(&m.title, &format!("{p}.title"));
                self.value(&m.value, &format!("{p}.value"), sc, ctx);
                if let Some(d) = &m.delta {
                    self.value(d, &format!("{p}.delta"), sc, ctx);
                }
            }
            Kind::Status(s) => self.val(&s.value, &format!("{p}.value"), sc, ctx),
            Kind::Avatar(a) => self.val(&a.value, &format!("{p}.value"), sc, ctx),
            Kind::AvatarStack(a) => self.val(&a.value, &format!("{p}.value"), sc, ctx),
            Kind::Meter(m) => {
                self.value(&m.value, &format!("{p}.value"), sc, ctx);
                self.value(&m.max, &format!("{p}.max"), sc, ctx);
            }
            Kind::Progress(m) => self.val(&m.value, &format!("{p}.value"), sc, ctx),
            Kind::CopyField(c) => self.val(&c.value, &format!("{p}.value"), sc, ctx),
            Kind::Text(t) => self.text(&t.text, &format!("{p}.text")),
            Kind::Tag(t) => self.text(&t.text, &format!("{p}.text")),
            Kind::Form(f) => {
                if let Some(e) = self.endpoint(&f.endpoint, &format!("{p}.endpoint")) {
                    for s in &f.sections {
                        for i in &s.inputs {
                            self.input(
                                i,
                                &e.input,
                                &format!("{p}.inputs[{}]", i.name),
                                sc,
                                ctx,
                                false,
                            );
                        }
                    }
                    let fc = Ctx {
                        form: Some(e.input.clone()),
                        picks: f
                            .sections
                            .iter()
                            .flat_map(|s| &s.inputs)
                            .filter(|i| i.control == Control::Pick)
                            .map(|i| i.name.clone())
                            .collect(),
                        ..ctx.clone()
                    };
                    for (i, n) in f.notes.iter().enumerate() {
                        self.text_val(&n.text, &format!("{p}.notes[{i}].text"), sc, &fc);
                        if let Some(c) = &n.visible {
                            self.cond(c, &format!("{p}.notes[{i}].visible"), sc, &fc);
                        }
                    }
                    self.use_action(&f.submit, &format!("{p}.submit"), sc, &fc);
                    if let Some(b) = &f.alternate {
                        self.text(&b.label, &format!("{p}.alternate.label"));
                        self.use_action(&b.action, &format!("{p}.alternate"), sc, &fc);
                    }
                }
                self.text(&f.submit_label, &format!("{p}.submit_label"));
                if let Some(t) = &f.phone_submit_label {
                    self.text(t, &format!("{p}.phone_submit_label"));
                }
                if let Some(n) = &f.note {
                    self.text(n, &format!("{p}.note"));
                }
                if let Some(sm) = &f.summary {
                    self.text(&sm.title, &format!("{p}.summary.title"));
                    self.value(&sm.value, &format!("{p}.summary.value"), sc, ctx);
                }
            }
            Kind::FilterBar(f) => {
                for i in &f.inputs {
                    if !sc.params.contains_key(i.name.as_str()) {
                        self.err(
                            "E_UI_PARAM_UNKNOWN",
                            format!("{p}.inputs[{}]", i.name),
                            format!("param {} is not declared", i.name),
                        );
                    }
                    // several values live in one string param, joined with commas
                    if i.control == Control::MultiSelect
                        && sc
                            .params
                            .get(i.name.as_str())
                            .is_some_and(|ty| *ty != "string")
                    {
                        self.errors.push(
                            Error::new(
                                "E_UI_ARG_TYPE",
                                format!("{p}.inputs[{}]", i.name),
                                format!("a multi-select filter keeps its values in a string param; {} is not", i.name),
                            )
                            .help("declare the param as a string: it holds the chosen option values joined with commas"),
                        );
                    }
                    self.input(i, "", &format!("{p}.inputs[{}]", i.name), sc, ctx, true);
                }
            }
            Kind::Segmented(s) => {
                self.bind(&s.bind, &p, sc);
                // a param's default is what the switch shows on a link without the param: it is one of the options
                if let Bind::Param(name) = &s.bind
                    && let Some(d) = sc.defaults.get(name.as_str())
                    && !s
                        .options
                        .iter()
                        .any(|o| scalar_text(&o.value) == scalar_text(d))
                {
                    self.err(
                        "E_UI_PARAM_DEFAULT",
                        format!("{p}.options"),
                        format!("the default of param {name} ({d}) is not one of the options"),
                    );
                }
            }
            Kind::Tabs(t) => {
                if let Some(b) = &t.bind {
                    self.bind(b, &p, sc);
                }
            }
            Kind::Link(l) => {
                self.text(&l.text, &format!("{p}.text"));
                self.target(&l.target, &format!("{p}.target"), sc, ctx, screens, false);
            }
            Kind::Breadcrumbs(b) => {
                for (i, c) in b.items.iter().enumerate() {
                    if let Some(t) = &c.target {
                        self.target(t, &format!("{p}.items[{i}]"), sc, ctx, screens, false);
                    }
                }
            }
            Kind::Button(b) => {
                self.text(&b.label, &format!("{p}.label"));
                self.use_action(&b.action, &p, sc, ctx);
            }
            Kind::IconButton(b) => {
                self.text(&b.label, &format!("{p}.label"));
                if let Some(t) = &b.label_off {
                    self.text(t, &format!("{p}.label_off"));
                }
                match &b.pressed {
                    Some(v) => {
                        self.value(v, &format!("{p}.pressed"), sc, ctx);
                        let tc = Ctx {
                            form: Some(TOGGLE.into()),
                            picks: vec![],
                            ..ctx.clone()
                        };
                        self.use_action(&b.action, &p, sc, &tc);
                    }
                    None => self.use_action(&b.action, &p, sc, ctx),
                }
            }
            Kind::Menu(m) => {
                for (i, it) in m.items.iter().enumerate() {
                    self.menu_item(it, &format!("{p}.items[{i}]"), sc, ctx);
                }
            }
            Kind::Notice(x) => self.text(&x.text, &format!("{p}.text")),
            Kind::Empty(x) => self.text(&x.title, &format!("{p}.title")),
            Kind::Section(s) => {
                if let Some(t) = &s.title {
                    self.text(t, &format!("{p}.title"));
                }
            }
            Kind::Card(c) => {
                if let Some(v) = &c.trailing {
                    self.value(v, &format!("{p}.trailing"), sc, ctx);
                }
            }
            Kind::RecordHeader(h) => {
                self.text(&h.title, &format!("{p}.title"));
                for a in [&h.expand, &h.close].into_iter().flatten() {
                    self.use_action(a, &p, sc, ctx);
                }
                if let Some(m) = &h.menu {
                    for (i, it) in m.items.iter().enumerate() {
                        self.menu_item(it, &format!("{p}.menu.items[{i}]"), sc, ctx);
                    }
                }
            }
            Kind::AgentRun(r) => {
                let data = |f: &str| Value::Data {
                    query: r.query.clone(),
                    field: f.to_string(),
                };
                for f in [Some(&r.title), r.subtitle.as_ref(), Some(&r.status.field)]
                    .into_iter()
                    .flatten()
                {
                    self.value(&data(f), &p, sc, ctx);
                }
                if let Some(ty) = self.value(&data(&r.steps), &format!("{p}.steps"), sc, ctx) {
                    match FieldType::parse(&ty) {
                        Some(FieldType::Message(m)) => {
                            self.row_field(&m, &r.step_text, &format!("{p}.step_text"));
                            self.row_field(&m, &r.step_state, &format!("{p}.step_state"));
                        }
                        _ => self.err(
                            "E_UI_ROWS",
                            format!("{p}.steps"),
                            format!("{} is not a repeated message field", r.steps),
                        ),
                    }
                }
                if let Some(a) = &r.stop {
                    self.use_action(a, &format!("{p}.stop"), sc, ctx);
                }
                if let Some(c) = &r.stop_when {
                    self.cond(c, &format!("{p}.stop_when"), sc, ctx);
                }
                if let Some(m) = &r.message {
                    self.text(&m.placeholder, &format!("{p}.message.placeholder"));
                    // the action reads the typed text as `{form: text}`
                    let mc = Ctx {
                        form: Some(MESSAGE_BOX.into()),
                        ..ctx.clone()
                    };
                    self.use_action(&m.action, &format!("{p}.message"), sc, &mc);
                }
                if let Some(n) = &r.needs {
                    for f in [Some(&n.text), n.due.as_ref()].into_iter().flatten() {
                        self.value(&data(f), &format!("{p}.needs"), sc, ctx);
                    }
                    self.text(&n.label, &format!("{p}.needs.label"));
                    self.use_action(&n.action, &format!("{p}.needs"), sc, ctx);
                }
                if let Some(o) = &r.open {
                    self.text(&o.label, &format!("{p}.open.label"));
                    self.use_action(&o.action, &format!("{p}.open"), sc, ctx);
                }
                if let Some(c) = &r.changes {
                    self.text(&c.title, &format!("{p}.changes.title"));
                    if let Some(ty) = self.value(&data(&c.field), &format!("{p}.changes"), sc, ctx)
                    {
                        match FieldType::parse(&ty) {
                            Some(FieldType::Message(m)) => {
                                self.row_field(&m, &c.text, &format!("{p}.changes.text"));
                            }
                            _ => self.err(
                                "E_UI_ROWS",
                                format!("{p}.changes"),
                                format!("{} is not a repeated message field", c.field),
                            ),
                        }
                    }
                }
                if let Some(e) = &r.earlier {
                    self.value(&data(&e.text), &format!("{p}.earlier"), sc, ctx);
                    self.use_action(&e.action, &format!("{p}.earlier"), sc, ctx);
                }
                for (k, f) in [
                    ("icon", &r.icon),
                    ("engine", &r.engine),
                    ("status_label", &r.status_label),
                    ("step_label", &r.step_label),
                    ("elapsed", &r.elapsed),
                    ("note", &r.note),
                ] {
                    if let Some(f) = f {
                        self.value(&data(f), &format!("{p}.{k}"), sc, ctx);
                    }
                }
                if let Some(c) = &r.live_when {
                    self.cond(c, &format!("{p}.live_when"), sc, ctx);
                }
                let step_fields = [
                    ("step_detail", &r.step_detail),
                    ("step_time", &r.step_time),
                    ("step_tool", &r.step_tool),
                    ("step_evidence", &r.step_evidence),
                    ("step_lines", &r.step_lines),
                ];
                if step_fields.iter().any(|(_, f)| f.is_some())
                    && let Some(def) = sc.queries.get(r.query.as_str())
                    && let Some(e) = self.d.endpoints.iter().find(|e| e.name == def.endpoint)
                    && let Some(steps) = self.path(&e.output, &r.steps)
                    && let Some(FieldType::Message(m)) = FieldType::parse(&steps.ty)
                {
                    for (k, f) in step_fields {
                        if let Some(f) = f {
                            self.row_field(&m, f, &format!("{p}.{k}"));
                        }
                    }
                }
            }
            _ => {}
        }
        // a viz draws its own card frame: as one of several children of a card or a section it is a frame in a frame
        let body = match &n.kind {
            Kind::Card(c) => Some((0, &c.children)),
            // a section's actions come before its children in the paths
            Kind::Section(s) => Some((s.actions.len(), &s.children)),
            _ => None,
        };
        if let Some((first, body)) = body.filter(|(_, b)| b.len() > 1) {
            for (i, c) in body.iter().enumerate() {
                if matches!(c.kind, Kind::Viz(_)) {
                    self.errors.push(
                        Error::new(
                            "E_UI_VIZ_FRAMED",
                            format!("{p}[{}]", first + i),
                            format!("a viz draws its own card: next to other children of a {k} it is a frame in a frame"),
                        )
                        .help("place it directly in a board, columns or a stack (or make it the only child)"),
                    );
                }
            }
        }
        // containers (and the row actions handled above are not walked twice)
        match &n.kind {
            Kind::Table(_) | Kind::List(_) => {}
            k => {
                let outer = self.in_board;
                self.in_board = outer || matches!(k, Kind::Board(_));
                // each side of a split is a column of its own (one bottom-anchored stream each)
                let split = matches!(k, Kind::Split(_));
                for (i, c) in children(k).into_iter().enumerate() {
                    if split {
                        self.frame.columns.push(0);
                    }
                    self.node(c, &format!("{p}[{i}]"), sc, ctx, screens);
                    if split {
                        self.frame.columns.pop();
                    }
                }
                self.in_board = outer;
            }
        }
    }

    /// A chart or figure: a known kind, options within the limit, binds over queries in scope and fields of their
    /// rows, a selection target of this frame, a link to a screen.
    fn viz(
        &mut self,
        v: &Viz,
        p: &str,
        sc: &Scope<'a>,
        ctx: &Ctx,
        screens: &HashMap<&str, &'a Screen>,
    ) {
        use crate::viz_kinds;
        if !viz_kinds::known(&v.family, &v.kind) {
            let help = match viz_kinds::kinds(&v.family) {
                Some(kinds) => format!("the kinds of {}: {}", v.family, kinds.join(", ")),
                None => format!(
                    "the families: {}",
                    viz_kinds::FAMILIES
                        .iter()
                        .map(|(f, _)| *f)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            self.errors.push(
                Error::new(
                    "E_UI_VIZ_KIND",
                    format!("{p}.kind"),
                    format!(
                        "{}.{} is not a kind of the charts catalogue",
                        v.family, v.kind
                    ),
                )
                .help(help),
            );
        }
        for (name, t) in [("title", &v.title), ("range", &v.range)] {
            if let Some(t) = t {
                self.text(t, &format!("{p}.{name}"));
            }
        }
        let size = serde_json::to_string(&v.options).map_or(0, |s| s.len());
        if size > VIZ_MAX_OPTIONS {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.options"),
                format!("the options are {size} bytes, at most {VIZ_MAX_OPTIONS}"),
            );
        }
        // the kind is the one named (and checked) above: neither an option nor a bind replaces it
        if v.options.contains_key("kind") {
            self.err(
                "E_UI_VIZ_KIND",
                format!("{p}.options.kind"),
                "the kind is named by viz.kind, not by an option",
            );
        }
        if v.bind.len() > VIZ_MAX_BINDS {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.bind"),
                format!("{} binds, at most {VIZ_MAX_BINDS}", v.bind.len()),
            );
        }
        if let Some(h) = v.height
            && !(VIZ_MIN_HEIGHT..=VIZ_MAX_HEIGHT).contains(&h)
        {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.height"),
                format!("the height is {h} px, {VIZ_MIN_HEIGHT} to {VIZ_MAX_HEIGHT}"),
            );
        }
        for (i, b) in v.bind.iter().enumerate() {
            let bp = format!("{p}.bind[{i}]");
            if !b.to.split('.').all(option_name) {
                self.err(
                    "E_UI_VIZ_BIND",
                    format!("{bp}.to"),
                    format!("{:?} is not an option name or a dotted path of names", b.to),
                );
            } else if b.to.split('.').next() == Some("kind") {
                self.err(
                    "E_UI_VIZ_KIND",
                    format!("{bp}.to"),
                    "the kind is named by viz.kind, not by a bind",
                );
            }
            match &b.from {
                VizSource::Column { query, field, .. } => {
                    let fp = format!("{bp}.from.column");
                    if let Some(msg) = self.rows_message(query, sc, &fp) {
                        self.row_field(&msg, field, &format!("{fp}.field"));
                    }
                }
                VizSource::Series { query, fields } => {
                    let fp = format!("{bp}.from.series");
                    if fields.is_empty() {
                        self.err(
                            "E_UI_VIZ_BIND",
                            format!("{fp}.fields"),
                            "a series source names at least one field",
                        );
                    }
                    let msg = self.rows_message(query, sc, &fp);
                    for f in fields {
                        self.text(&f.label, &format!("{fp}.fields[{}].label", f.field));
                        if let Some(msg) = &msg {
                            self.row_field(msg, &f.field, &format!("{fp}.fields[{}]", f.field));
                        }
                    }
                }
                VizSource::Pivot {
                    query,
                    x,
                    series,
                    value,
                    ..
                } => {
                    let fp = format!("{bp}.from.pivot");
                    if let Some(msg) = self.rows_message(query, sc, &fp) {
                        for (name, f) in [("x", x), ("series", series), ("value", value)] {
                            self.row_field(&msg, f, &format!("{fp}.{name}"));
                        }
                    }
                }
                VizSource::Rows { query, map, text } => {
                    let fp = format!("{bp}.from.rows");
                    if map.is_empty() {
                        self.err(
                            "E_UI_VIZ_BIND",
                            format!("{fp}.map"),
                            "a rows source maps at least one key",
                        );
                    }
                    let msg = self.rows_message(query, sc, &fp);
                    for (key, f) in map {
                        if !option_name(key) {
                            self.err(
                                "E_UI_VIZ_BIND",
                                format!("{fp}.map[{key}]"),
                                format!("{key:?} is not a key name"),
                            );
                        }
                        if let Some(msg) = &msg {
                            self.row_field(msg, f, &format!("{fp}.map[{key}]"));
                        }
                    }
                    for key in text {
                        if !map.contains_key(key) {
                            self.err(
                                "E_UI_VIZ_BIND",
                                format!("{fp}.text"),
                                format!("{key} is not a key of the map"),
                            );
                        }
                    }
                }
                VizSource::Value(x) => self.val(x, &format!("{bp}.from.value"), sc, ctx),
            }
        }
        match &v.select {
            Some(Bind::State(s)) if !sc.state.contains_key(s.as_str()) => self.err(
                "E_UI_STATE_UNKNOWN",
                format!("{p}.select"),
                format!("state {s} is not declared"),
            ),
            Some(Bind::Param(s)) if !sc.params.contains_key(s.as_str()) => self.err(
                "E_UI_PARAM_UNKNOWN",
                format!("{p}.select"),
                format!("param {s} is not declared"),
            ),
            _ => {}
        }
        if let Some(t) = &v.open {
            self.target(t, &format!("{p}.open"), sc, ctx, screens, false);
        }
    }

    // ------------------------------------------------------------------------------------------ catalog 3

    /// `older`, `debounce_ms`, `join` of a query.
    fn query_options(&mut self, q: &Query, e: &Endpoint, qp: &str) {
        if let Some(o) = &q.older {
            if self.path(&e.output, &o.token).is_none() {
                self.err(
                    "E_UI_FIELD_UNKNOWN",
                    format!("{qp}.older.token"),
                    format!("{} has no field {}", e.output, o.token),
                );
            }
            if self.path(&e.input, &o.arg).is_none() {
                self.err(
                    "E_UI_FIELD_UNKNOWN",
                    format!("{qp}.older.arg"),
                    format!("{} has no field {}", e.input, o.arg),
                );
            }
            if q.rows.is_none() {
                self.err("E_UI_ROWS", format!("{qp}.older"), "older pages need rows");
            }
        }
        if let Some(ms) = q.debounce_ms
            && ms > MAX_DEBOUNCE_MS
        {
            self.err(
                "E_UI_LIMIT",
                format!("{qp}.debounce_ms"),
                format!("a query waits at most {MAX_DEBOUNCE_MS} ms, not {ms}"),
            );
        }
        let rows = q
            .rows
            .as_ref()
            .and_then(|r| self.path(&e.output, r))
            .and_then(|f| match FieldType::parse(&f.ty) {
                Some(FieldType::Message(m)) => Some(m),
                _ => None,
            });
        let mut names = HashSet::new();
        for j in &q.join {
            let jp = format!("{qp}.join[{}]", j.name);
            if !snake(&j.name) || !names.insert(j.name.as_str()) {
                self.err(
                    "E_UI_NAME",
                    &jp,
                    format!("{:?} is not a join name, or it is used twice", j.name),
                );
            }
            match self.repeated_message(&e.output, &j.rows) {
                Some(m) => {
                    self.row_field(&m, &j.key, &format!("{jp}.key"));
                }
                None => self.err(
                    "E_UI_ROWS",
                    format!("{jp}.rows"),
                    format!("{}.{} is not a repeated message field", e.output, j.rows),
                ),
            }
            match &rows {
                None => self.err("E_UI_ROWS", &jp, "a join needs the query's rows"),
                Some(r) => {
                    self.row_field(r, &j.by, &format!("{jp}.by"));
                    if self.path(r, &j.name).is_some() {
                        self.err(
                            "E_UI_NAME",
                            &jp,
                            format!("the join {} hides the row field {}", j.name, j.name),
                        );
                    }
                }
            }
        }
    }

    /// The message of a repeated message field of `msg`.
    fn repeated_message(&self, msg: &str, path: &str) -> Option<String> {
        let f = self.path(msg, path)?;
        match FieldType::parse(&f.ty) {
            Some(FieldType::Message(m)) if f.repeated => Some(m),
            _ => None,
        }
    }

    /// The row context of a query's rows: the message and the query's joins.
    fn row_ctx(&self, query: &str, msg: String, sc: &Scope<'a>, outer: &Ctx) -> Ctx {
        let joins = sc
            .queries
            .get(query)
            .and_then(|q| {
                let e = self.d.endpoints.iter().find(|e| e.name == q.endpoint)?;
                Some(
                    q.join
                        .iter()
                        .filter_map(|j| {
                            self.repeated_message(&e.output, &j.rows)
                                .map(|m| (j.name.clone(), m))
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        Ctx {
            row: Some(msg),
            joins,
            ..outer.clone()
        }
    }

    /// A row field by path, through the query's joins (`who.name`).
    fn row_path(
        &mut self,
        msg: &str,
        joins: &[(String, String)],
        f: &str,
        p: &str,
    ) -> Option<&'a Field> {
        if let Some((head, rest)) = f.split_once('.')
            && let Some((_, m)) = joins.iter().find(|(n, _)| n == head)
        {
            let m = m.clone();
            return self.row_field(&m, rest, p);
        }
        self.row_field(msg, f, p)
    }

    /// Row fields of a template in its row context.
    fn fields_in(&mut self, rc: &Ctx, fields: &[(&str, Option<&String>)], p: &str) {
        let Some(msg) = rc.row.clone() else { return };
        for (k, f) in fields {
            if let Some(f) = f {
                self.row_path(&msg, &rc.joins, f, &format!("{p}.{k}"));
            }
        }
    }

    /// A text, and the values it reads.
    fn text_val(&mut self, t: &Text, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        match t {
            Text::Plain(_) => self.text(t, p),
            Text::Key { args, .. } => {
                for (k, v) in args {
                    self.value(v, &format!("{p}.args.{k}"), sc, ctx);
                }
            }
            Text::Value(v) => {
                self.value(v, p, sc, ctx);
            }
        }
    }

    /// The rows of a directory: a query's rows, or another repeated field of its answer.
    fn directory(
        &mut self,
        query: &str,
        rows: Option<&str>,
        sc: &Scope<'a>,
        p: &str,
    ) -> Option<String> {
        let Some(r) = rows else {
            return self.rows_message(query, sc, p);
        };
        let Some(q) = sc.queries.get(query) else {
            self.err(
                "E_UI_QUERY_UNKNOWN",
                format!("{p}.query"),
                format!("query {query} is not declared here"),
            );
            return None;
        };
        let out = self
            .d
            .endpoints
            .iter()
            .find(|e| e.name == q.endpoint)?
            .output
            .clone();
        let m = self.repeated_message(&out, r);
        if m.is_none() {
            self.err(
                "E_UI_ROWS",
                format!("{p}.rows"),
                format!("{out}.{r} is not a repeated message field"),
            );
        }
        m
    }

    /// A state of the frame of this type (`E_UI_STATE_UNKNOWN`, `E_UI_TYPE`).
    fn state_of(&mut self, name: &str, ty: &str, p: &str, sc: &Scope<'a>) {
        match sc.state.get(name) {
            None => self.err(
                "E_UI_STATE_UNKNOWN",
                p,
                format!("state {name} is not declared"),
            ),
            Some(t) if *t != ty => self.err(
                "E_UI_TYPE",
                p,
                format!("state {name} is {t}; this needs a {ty} state"),
            ),
            Some(_) => {}
        }
    }

    fn split(&mut self, x: &Split, p: &str, sc: &Scope<'a>) {
        if let Some(r) = &x.resize {
            let range = RESIZE_MIN..=RESIZE_MAX;
            let keep_ok = r.keep.is_none_or(|k| k <= RESIZE_MAX);
            if !range.contains(&r.min)
                || !range.contains(&r.max)
                || !range.contains(&r.default)
                || !(r.min <= r.default && r.default <= r.max)
                || !keep_ok
            {
                self.err(
                    "E_UI_LIMIT",
                    format!("{p}.resize"),
                    format!(
                        "a panel's width is {RESIZE_MIN} to {RESIZE_MAX} px with min ≤ default ≤ max (and keep ≤ \
                         {RESIZE_MAX})"
                    ),
                );
            }
        }
        if let Some(e) = &x.expand {
            self.state_of(&e.state, "bool", &format!("{p}.expand"), sc);
        }
    }

    fn header(&mut self, h: &'a Header, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        self.text_val(&h.title, &format!("{p}.title"), sc, ctx);
        for (k, v) in [("title_icon", &h.title_icon), ("crumb_icon", &h.crumb_icon)] {
            if let Some(v) = v {
                self.value(v, &format!("{p}.{k}"), sc, ctx);
            }
        }
        if let Some(l) = &h.leading {
            let lp = format!("{p}.leading");
            match (&l.tile, &l.faces) {
                (Some(t), None) => {
                    self.value(&t.icon, &format!("{lp}.tile.icon"), sc, ctx);
                    if let Some(e) = &t.engine {
                        self.value(e, &format!("{lp}.tile.engine"), sc, ctx);
                    }
                }
                (None, Some(f)) => {
                    self.value(f, &format!("{lp}.faces"), sc, ctx);
                }
                _ => self.err("E_UI_SHAPE", &lp, "a leading is a tile or faces"),
            }
            if l.size.is_some_and(|s| !(16..=64).contains(&s)) {
                self.err(
                    "E_UI_LIMIT",
                    format!("{lp}.size"),
                    "a leading is 16 to 64 px",
                );
            }
        }
        for (i, g) in h.subtitle.iter().enumerate() {
            self.segment(g, &format!("{p}.subtitle[{i}]"), sc, ctx);
        }
        if let Some(a) = &h.subtitle_action {
            self.use_action(a, &format!("{p}.subtitle_action"), sc, ctx);
        }
        if let Some(t) = &h.context {
            self.text_val(t, &format!("{p}.context"), sc, ctx);
        }
        if let Some(f) = &h.faces {
            let fp = format!("{p}.faces");
            self.value(&f.value, &format!("{fp}.value"), sc, ctx);
            if let Some(t) = &f.label {
                self.text_val(t, &format!("{fp}.label"), sc, ctx);
            }
            if let Some(t) = &f.aria_label {
                self.text_val(t, &format!("{fp}.aria_label"), sc, ctx);
            }
            if let Some(a) = &f.action {
                self.use_action(a, &fp, sc, ctx);
            }
            if let Some(c) = &f.visible {
                self.cond(c, &fp, sc, ctx);
            }
        }
        if h.buttons.len() > HEADER_MAX_BUTTONS {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.buttons"),
                format!("at most {HEADER_MAX_BUTTONS} buttons"),
            );
        }
        for (i, b) in h.buttons.iter().enumerate() {
            let bp = format!("{p}.buttons[{i}]");
            self.text_val(&b.label, &format!("{bp}.label"), sc, ctx);
            self.use_action(&b.action, &bp, sc, ctx);
            if let Some(c) = &b.visible {
                self.cond(c, &bp, sc, ctx);
            }
        }
        if h.icons.len() > HEADER_MAX_ICONS {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.icons"),
                format!("at most {HEADER_MAX_ICONS} icons"),
            );
        }
        for (i, n) in h.icons.iter().enumerate() {
            if !matches!(n.kind, Kind::IconButton(_)) {
                self.err(
                    "E_UI_SHAPE",
                    format!("{p}.icons[{i}]"),
                    "a header's icons are icon buttons",
                );
            }
        }
        if let Some(m) = &h.menu {
            for (i, it) in m.items.iter().enumerate() {
                self.menu_item(it, &format!("{p}.menu.items[{i}]"), sc, ctx);
            }
        }
        if let Some(t) = &h.tabs {
            let tp = format!("{p}.tabs");
            if let Some(l) = &t.label {
                self.text_val(l, &format!("{tp}.label"), sc, ctx);
            }
            if t.items.len() > HEADER_MAX_TABS {
                self.err(
                    "E_UI_LIMIT",
                    format!("{tp}.items"),
                    format!("at most {HEADER_MAX_TABS} tabs"),
                );
            }
            let mut ids = HashSet::new();
            for (i, it) in t.items.iter().enumerate() {
                let ip = format!("{tp}.items[{i}]");
                if !ids.insert(it.id.as_str()) {
                    self.err("E_UI_DUP", &ip, format!("tab {} is declared twice", it.id));
                }
                self.text_val(&it.title, &format!("{ip}.title"), sc, ctx);
                if let Some(b) = &it.badge {
                    self.value(b, &format!("{ip}.badge"), sc, ctx);
                }
                if let Some(a) = &it.action {
                    self.use_action(a, &ip, sc, ctx);
                }
            }
            self.bind(&t.bind, &tp, sc);
            let (ty, default) = match &t.bind {
                Bind::State(n) => (
                    sc.state.get(n.as_str()).copied(),
                    sc.state_defaults.get(n.as_str()).copied(),
                ),
                Bind::Param(n) => (
                    sc.params.get(n.as_str()).copied(),
                    sc.defaults.get(n.as_str()).copied(),
                ),
            };
            if ty.is_some_and(|t| t != "string") {
                self.err(
                    "E_UI_TYPE",
                    format!("{tp}.bind"),
                    "tabs are bound to a string",
                );
            }
            if let Some(d) = default
                && !t.items.iter().any(|it| scalar_text(d) == it.id)
            {
                self.err(
                    "E_UI_STATE_DEFAULT",
                    format!("{tp}.bind"),
                    format!("the default {d} is not one of the tabs' ids"),
                );
            }
        }
        if let Some(a) = &h.close {
            self.use_action(a, &format!("{p}.close"), sc, ctx);
        }
        if let Some(t) = &h.back {
            self.text_val(t, &format!("{p}.back"), sc, ctx);
        }
        if let Some(ph) = &h.phone {
            if let Some(t) = &ph.sub {
                self.text_val(t, &format!("{p}.phone.sub"), sc, ctx);
            }
            for a in &ph.icons {
                self.use_action(a, &format!("{p}.phone.icons"), sc, ctx);
            }
        }
    }

    fn segment(&mut self, g: &Segment, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        let n = [
            g.text.is_some(),
            g.dot.is_some(),
            g.value.is_some(),
            g.tone.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if n != 1 {
            self.err(
                "E_UI_SHAPE",
                p,
                "a segment is one of text, dot, value and tone",
            );
        }
        if g.format.is_some() && g.value.is_none() || g.prefix.is_some() && g.value.is_none() {
            self.err(
                "E_UI_SHAPE",
                p,
                "format and prefix belong to a value segment",
            );
        }
        if let Some(t) = &g.text {
            self.text_val(t, &format!("{p}.text"), sc, ctx);
        }
        if let Some(v) = &g.value {
            self.value(v, &format!("{p}.value"), sc, ctx);
        }
        if let Some(t) = &g.prefix {
            self.text_val(t, &format!("{p}.prefix"), sc, ctx);
        }
        if let Some(c) = &g.visible {
            self.cond(c, p, sc, ctx);
        }
    }

    fn stream(&mut self, x: &'a Stream, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        if x.anchor == Anchor::Bottom
            && let Some(n) = self.frame.columns.last_mut()
        {
            *n += 1;
            if *n > 1 {
                self.err(
                    "E_UI_STREAM_TWICE",
                    p,
                    "a column has one bottom-anchored stream (others are anchor: top)",
                );
            }
        }
        if let Some(id) = &x.id
            && (!snake(id) || !self.frame.streams.insert(id.clone()))
        {
            self.err(
                "E_UI_DUP",
                format!("{p}.id"),
                format!("{id:?} is not a stream id, or another stream of the frame has it"),
            );
        }
        for (k, t) in [
            ("older_label", &x.older_label),
            ("jump_working", &x.jump_working),
            ("empty", &x.empty),
            ("phone_intro", &x.phone_intro),
            ("label", &x.label),
        ] {
            if let Some(t) = t {
                self.text_val(t, &format!("{p}.{k}"), sc, ctx);
            }
        }
        if let Some(v) = &x.unread {
            self.value(v, &format!("{p}.unread"), sc, ctx);
        }
        let Some(msg) = self.rows_message(&x.query, sc, p) else {
            return;
        };
        let rc = self.row_ctx(&x.query, msg.clone(), sc, ctx);
        self.row_path(&msg, &rc.joins, &x.key, &format!("{p}.key"));
        if let Some(d) = &x.days {
            self.row_path(&msg, &rc.joins, d, &format!("{p}.days"));
        }
        if let Some(k) = &x.kinds {
            self.row_path(&msg, &rc.joins, &k.field, &format!("{p}.kinds.field"));
            if let Some(l) = &k.line {
                self.fields_in(
                    &rc,
                    &[
                        ("author", l.author.as_ref()),
                        ("text", Some(&l.text)),
                        ("time", l.time.as_ref()),
                    ],
                    &format!("{p}.kinds.line"),
                );
            }
        }
        if let Some(t) = &x.tail {
            let tp = format!("{p}.tail");
            if let Some(m) = self.rows_message(&t.query, sc, &tp)
                && m != msg
            {
                self.err(
                    "E_UI_ROWS",
                    format!("{tp}.query"),
                    format!("the tail's rows are {m}, the stream's {msg}"),
                );
            }
        }
        if let Some(h) = &x.head {
            let hp = format!("{p}.head");
            let out = sc
                .queries
                .get(x.query.as_str())
                .and_then(|q| self.d.endpoints.iter().find(|e| e.name == q.endpoint))
                .map(|e| e.output.clone());
            if let Some(out) = out {
                match self
                    .path(&out, &h.row)
                    .map(|f| (f.repeated, FieldType::parse(&f.ty)))
                {
                    Some((false, Some(FieldType::Message(m)))) if m == msg => {}
                    Some(_) => self.err(
                        "E_UI_ROWS",
                        format!("{hp}.row"),
                        format!("{out}.{} is not one {msg}", h.row),
                    ),
                    None => self.err(
                        "E_UI_FIELD_UNKNOWN",
                        format!("{hp}.row"),
                        format!("{out} has no field {}", h.row),
                    ),
                }
            }
            if let Some(t) = &h.rule {
                self.text_val(t, &format!("{hp}.rule"), sc, ctx);
            }
        }
        if let Some(o) = &x.on_seen {
            self.use_action(&o.action, &format!("{p}.on_seen"), sc, &rc);
            if let Some(c) = &o.when {
                self.cond(c, &format!("{p}.on_seen.when"), sc, &rc);
            }
        }
        match &x.item {
            StreamItem::Post(post) => self.post(post, &format!("{p}.item.post"), sc, &rc),
        }
    }

    fn post(&mut self, x: &'a Post, p: &str, sc: &Scope<'a>, rc: &Ctx) {
        let pf = Ctx {
            form: Some(POST_FORM.into()),
            picks: vec![],
            ..rc.clone()
        };
        let a = x.avatar.as_ref();
        let tile = a.and_then(|a| a.tile.as_ref());
        let audio = x.audio.as_ref();
        let route = x.route.as_ref();
        let corr = x.correction.as_ref();
        let sum = x.summary.as_ref();
        self.fields_in(
            rc,
            &[
                ("author", Some(&x.author)),
                ("time", Some(&x.time)),
                ("avatar.url", a.and_then(|a| a.url.as_ref())),
                ("avatar.tile.icon", tile.map(|t| &t.icon)),
                ("avatar.tile.engine", tile.and_then(|t| t.engine.as_ref())),
                ("source", x.source.as_ref()),
                ("text.field", x.text.as_ref().map(|t| &t.field)),
                (
                    "text.writing",
                    x.text.as_ref().and_then(|t| t.writing.as_ref()),
                ),
                ("audio.duration", audio.map(|a| &a.duration)),
                ("audio.label", audio.and_then(|a| a.label.as_ref())),
                ("audio.url", audio.and_then(|a| a.url.as_ref())),
                ("audio.words", audio.and_then(|a| a.words.as_ref())),
                ("edited", x.edited.as_ref()),
                ("pinned.field", x.pinned.as_ref().map(|t| &t.field)),
                ("mine", x.mine.as_ref()),
                ("receipt.text", x.receipt.as_ref().map(|r| &r.text)),
                (
                    "receipt.tone",
                    x.receipt
                        .as_ref()
                        .and_then(|r| r.tone.as_ref())
                        .map(|t| &t.field),
                ),
                ("route.to", route.map(|r| &r.to)),
                ("route.status", route.and_then(|r| r.status.as_ref())),
                ("route.why", route.and_then(|r| r.why.as_ref())),
                ("correction.label", corr.map(|c| &c.label)),
                (
                    "correction.menu_label",
                    corr.and_then(|c| c.menu_label.as_ref()),
                ),
                ("correction.lead", corr.and_then(|c| c.lead.as_ref())),
                ("correction.icon", corr.and_then(|c| c.icon.as_ref())),
                ("correction.from", corr.map(|c| &c.from)),
                ("summary.count", sum.map(|s| &s.count)),
                ("summary.faces", sum.and_then(|s| s.faces.as_ref())),
                ("summary.last", sum.and_then(|s| s.last.as_ref())),
                ("summary.working", sum.and_then(|s| s.working.as_ref())),
                ("summary.since", sum.and_then(|s| s.since.as_ref())),
                ("summary.waiting", sum.and_then(|s| s.waiting.as_ref())),
            ],
            p,
        );
        if let Some(c) = tile.and_then(|t| t.when.as_ref()) {
            self.cond(c, &format!("{p}.avatar.tile.when"), sc, rc);
        }
        if let Some(b) = &x.badge {
            self.text_val(&b.text, &format!("{p}.badge.text"), sc, rc);
            if let Some(c) = &b.when {
                self.cond(c, &format!("{p}.badge.when"), sc, rc);
            }
        }
        if let Some(v) = x.pinned.as_ref().and_then(|t| t.mark.as_ref()) {
            self.value(v, &format!("{p}.pinned.mark"), sc, rc);
        }
        if let Some(c) = corr {
            self.use_action(&c.action, &format!("{p}.correction"), sc, rc);
            if c.window_ms > CORRECTION_MAX_WINDOW_MS {
                self.err(
                    "E_UI_LIMIT",
                    format!("{p}.correction.window_ms"),
                    format!("a correction stays inline at most {CORRECTION_MAX_WINDOW_MS} ms"),
                );
            }
        }
        if let Some(r) = &x.reactions {
            let rp = format!("{p}.reactions");
            if let Some(msg) = rc.row.clone() {
                let joins = rc.joins.clone();
                let item = self
                    .row_path(&msg, &joins, &r.field, &format!("{rp}.field"))
                    .and_then(|f| match FieldType::parse(&f.ty) {
                        Some(FieldType::Message(m)) if f.repeated => Some(m),
                        _ => None,
                    });
                match item {
                    Some(m) => {
                        for (k, f) in [
                            ("emoji", Some(&r.emoji)),
                            ("count", Some(&r.count)),
                            ("mine", r.mine.as_ref()),
                        ] {
                            if let Some(f) = f {
                                self.row_field(&m, f, &format!("{rp}.{k}"));
                            }
                        }
                    }
                    None => {
                        if self.path(&msg, &r.field).is_some() {
                            self.err(
                                "E_UI_ROWS",
                                format!("{rp}.field"),
                                format!("{msg}.{} is not a repeated message field", r.field),
                            );
                        }
                    }
                }
            }
            if let Some(a) = &r.action {
                self.use_action(a, &rp, sc, &pf);
            }
            if r.quick.len() > POST_MAX_QUICK {
                self.err(
                    "E_UI_LIMIT",
                    format!("{rp}.quick"),
                    format!("at most {POST_MAX_QUICK} quick reactions"),
                );
            }
        }
        if let Some(s) = sum {
            self.use_action(&s.action, &format!("{p}.summary"), sc, rc);
            if let Some(t) = &s.label {
                self.text_val(t, &format!("{p}.summary.label"), sc, rc);
            }
        }
        if let Some(w) = &x.process
            && let Some(msg) = rc.row.clone()
        {
            let joins = rc.joins.clone();
            self.process(w, &msg, &joins, &format!("{p}.process"), sc, rc);
        }
        if let Some(e) = &x.edit {
            self.use_action(&e.action, &format!("{p}.edit"), sc, &pf);
            if let Some(c) = &e.when {
                self.cond(c, &format!("{p}.edit.when"), sc, rc);
            }
        }
        if let Some(c) = &x.actions_when {
            self.cond(c, &format!("{p}.actions_when"), sc, rc);
        }
        if let Some(k) = &x.card {
            let kp = format!("{p}.card");
            if let Some(msg) = self.rows_message(&k.query, sc, &kp) {
                for f in [
                    Some(&k.name),
                    k.avatar.as_ref(),
                    k.agent.as_ref(),
                    k.icon.as_ref(),
                    k.engine.as_ref(),
                    k.detail.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    self.row_field(&msg, f, &kp);
                }
            }
            if let Some(a) = &k.action {
                self.use_action(a, &kp, sc, &pf);
            }
            if let Some(t) = &k.unknown {
                self.text(t, &format!("{kp}.unknown"));
            }
        }
        for (list, entries) in [("hover", &x.hover), ("menu", &x.menu), ("sheet", &x.sheet)] {
            if entries.len() > POST_MAX_ENTRIES {
                self.err(
                    "E_UI_LIMIT",
                    format!("{p}.{list}"),
                    format!("at most {POST_MAX_ENTRIES} entries"),
                );
            }
            for (i, e) in entries.iter().enumerate() {
                self.entry(e, x, list, &format!("{p}.{list}[{i}]"), sc, rc);
            }
        }
    }

    fn entry(&mut self, e: &Entry, post: &Post, list: &str, p: &str, sc: &Scope<'a>, rc: &Ctx) {
        match (&e.builtin, &e.action) {
            (Some(b), None) => {
                let need = match b.as_str() {
                    "react" => post
                        .reactions
                        .as_ref()
                        .and_then(|r| r.action.as_ref())
                        .is_none()
                        .then_some("reactions with an action"),
                    "edit" => post.edit.is_none().then_some("edit"),
                    "correction" => post.correction.is_none().then_some("a correction"),
                    "play" => post.audio.is_none().then_some("audio"),
                    "more" => (list != "hover").then_some("the hover rail (more opens the menu)"),
                    x if POST_BUILTINS.contains(&x) => None,
                    _ => {
                        self.errors.push(
                            Error::new("E_UI_BUILTIN", p, format!("{b} is not a built-in entry"))
                                .help(format!("the built-ins: {}", POST_BUILTINS.join(", "))),
                        );
                        None
                    }
                };
                if let Some(need) = need {
                    self.err("E_UI_BUILTIN", p, format!("the built-in {b} needs {need}"));
                }
            }
            (None, Some(a)) => {
                match &e.label {
                    Some(t) => self.text_val(t, &format!("{p}.label"), sc, rc),
                    None => self.err("E_UI_TEXT", p, "an entry with an action has a label"),
                }
                self.use_action(a, p, sc, rc);
            }
            _ => self.err("E_UI_SHAPE", p, "an entry is a builtin or an action"),
        }
        if let Some(c) = &e.visible {
            self.cond(c, p, sc, rc);
        }
    }

    /// A process: paths in `msg` (a row with its joins, or a query's answer).
    fn process(
        &mut self,
        x: &Process,
        msg: &str,
        joins: &[(String, String)],
        p: &str,
        sc: &Scope<'a>,
        ctx: &Ctx,
    ) {
        for (k, f) in [
            ("state", Some(&x.state)),
            ("label", x.label.as_ref()),
            ("meta", x.meta.as_ref()),
            ("started", x.started.as_ref()),
            ("steps", x.steps.as_ref()),
            ("saying", x.saying.as_ref()),
            (
                "retry.label",
                x.retry.as_ref().and_then(|r| r.label.as_ref()),
            ),
            ("log.label", x.log.as_ref().and_then(|r| r.label.as_ref())),
        ] {
            if let Some(f) = f {
                self.row_path(msg, joins, f, &format!("{p}.{k}"));
            }
        }
        if let Some(r) = &x.rows {
            let rows =
                self.row_path(msg, joins, r, &format!("{p}.rows"))
                    .map(|f| match FieldType::parse(&f.ty) {
                        Some(FieldType::Message(m)) if f.repeated => Some(m),
                        _ => None,
                    });
            match rows {
                Some(Some(m)) => {
                    for f in ["kind", "title"] {
                        self.row_field(&m, f, &format!("{p}.rows"));
                    }
                }
                Some(None) => self.err(
                    "E_UI_ROWS",
                    format!("{p}.rows"),
                    format!("{r} is not a repeated message field"),
                ),
                None => {}
            }
        }
        for (k, a) in [("retry", &x.retry), ("log", &x.log)] {
            if let Some(a) = a {
                self.use_action(&a.action, &format!("{p}.{k}"), sc, ctx);
                if let Some(c) = &a.when {
                    self.cond(c, &format!("{p}.{k}.when"), sc, ctx);
                }
            }
        }
        if let Some(f) = &x.fetch_call {
            let fp = format!("{p}.fetch_call");
            let e = sc
                .queries
                .get(f.query.as_str())
                .and_then(|q| self.d.endpoints.iter().find(|e| e.name == q.endpoint));
            match e {
                None => {
                    if !sc.queries.contains_key(f.query.as_str()) {
                        self.err(
                            "E_UI_QUERY_UNKNOWN",
                            format!("{fp}.query"),
                            format!("query {} is not declared here", f.query),
                        );
                    }
                }
                Some(e) => {
                    let (input, output) = (e.input.clone(), e.output.clone());
                    for a in &f.args {
                        self.arg(a, &input, &format!("{fp}.args[{}]", a.field), sc, ctx);
                    }
                    if self.path(&input, &f.call).is_none() {
                        self.err(
                            "E_UI_FIELD_UNKNOWN",
                            format!("{fp}.call"),
                            format!("{input} has no field {}", f.call),
                        );
                    }
                    if self.path(&output, &f.field).is_none() {
                        self.err(
                            "E_UI_FIELD_UNKNOWN",
                            format!("{fp}.field"),
                            format!("{output} has no field {}", f.field),
                        );
                    }
                }
            }
        }
    }

    fn strip(&mut self, x: &'a Strip, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        for (k, t) in [
            ("label", &x.label),
            ("countdown_text", &x.countdown_text),
            ("aria_label", &x.aria_label),
        ] {
            if let Some(t) = t {
                self.text_val(t, &format!("{p}.{k}"), sc, ctx);
            }
        }
        let Some(msg) = self.directory(&x.query, x.rows.as_deref(), sc, p) else {
            return;
        };
        let rc = Ctx {
            row: Some(msg),
            joins: vec![],
            ..ctx.clone()
        };
        let tile = x.tile.as_ref();
        self.fields_in(
            &rc,
            &[
                ("key", Some(&x.key)),
                ("title", Some(&x.title)),
                ("tile.icon", tile.map(|t| &t.icon)),
                ("tile.engine", tile.and_then(|t| t.engine.as_ref())),
                ("state.field", x.state.as_ref().map(|s| &s.field)),
                ("since", x.since.as_ref()),
                ("until", x.until.as_ref()),
                ("sub", x.sub.as_ref()),
                ("jump.field", x.jump.as_ref().map(|j| &j.field)),
            ],
            p,
        );
        if let Some(c) = tile.and_then(|t| t.when.as_ref()) {
            self.cond(c, &format!("{p}.tile.when"), sc, &rc);
        }
        if let Some(j) = &x.jump {
            for (k, t) in [("label", &j.label), ("label_waiting", &j.label_waiting)] {
                if let Some(t) = t {
                    self.text_val(t, &format!("{p}.jump.{k}"), sc, &rc);
                }
            }
        }
        if let Some(a) = &x.open {
            self.use_action(a, &format!("{p}.open"), sc, &rc);
        }
        if x.buttons.len() > HEADER_MAX_BUTTONS {
            self.err(
                "E_UI_LIMIT",
                format!("{p}.buttons"),
                format!("at most {HEADER_MAX_BUTTONS} buttons"),
            );
        }
        for (i, b) in x.buttons.iter().enumerate() {
            let bp = format!("{p}.buttons[{i}]");
            self.text_val(&b.label, &format!("{bp}.label"), sc, &rc);
            self.use_action(&b.action, &bp, sc, &rc);
            if let Some(c) = &b.visible {
                self.cond(c, &bp, sc, &rc);
            }
        }
    }

    fn composer(&mut self, x: &'a ComposerNode, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        let mut names = HashSet::new();
        for (i, o) in x.options.iter().enumerate() {
            let op = format!("{p}.options[{i}]");
            if !snake(&o.name)
                || matches!(
                    o.name.as_str(),
                    "text" | "audio" | "duration" | "language" | "words" | "reply"
                )
                || !names.insert(o.name.as_str())
            {
                self.err(
                    "E_UI_NAME",
                    &op,
                    format!("{:?} is not an option name, is a form value of the composer, or is used twice", o.name),
                );
            }
            self.text_val(&o.label, &format!("{op}.label"), sc, ctx);
            if let Some(c) = &o.visible {
                self.cond(c, &op, sc, ctx);
            }
        }
        let options: Vec<&str> = x.options.iter().map(|o| o.name.as_str()).collect();
        let fc = Ctx {
            form: Some(format!("{COMPOSER}:{}", options.join(","))),
            picks: vec![],
            ..ctx.clone()
        };
        self.use_action(&x.send, &format!("{p}.send"), sc, &fc);
        if let Some(s) = &x.stop {
            self.use_action(&s.action, &format!("{p}.stop"), sc, ctx);
            if let Some(c) = &s.when {
                self.cond(c, &format!("{p}.stop.when"), sc, ctx);
            }
            if let Some(t) = &s.label {
                self.text_val(t, &format!("{p}.stop.label"), sc, ctx);
            }
            if let Some(t) = &s.aria_label {
                self.text_val(t, &format!("{p}.stop.aria_label"), sc, ctx);
            }
        }
        if let Some(r) = &x.readonly {
            self.cond(&r.when, &format!("{p}.readonly.when"), sc, ctx);
            self.text_val(&r.reason, &format!("{p}.readonly.reason"), sc, ctx);
        }
        if let Some(t) = &x.placeholder {
            self.text_val(t, &format!("{p}.placeholder"), sc, ctx);
        }
        for (i, c) in x.caption.iter().enumerate() {
            self.text_val(&c.value, &format!("{p}.caption[{i}]"), sc, ctx);
        }
        if let Some(v) = x.dictation.as_ref().and_then(|d| d.vocabulary.as_ref()) {
            self.text_val(v, &format!("{p}.dictation.vocabulary"), sc, ctx);
        }
        if let Some(m) = &x.mentions {
            let mp = format!("{p}.mentions");
            if let Some(msg) = self.directory(&m.query, m.rows.as_deref(), sc, &mp) {
                for f in [
                    Some(&m.name),
                    m.avatar.as_ref(),
                    m.agent.as_ref(),
                    m.icon.as_ref(),
                    m.engine.as_ref(),
                    m.detail.as_ref(),
                    m.section.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    self.row_field(&msg, f, &mp);
                }
            }
            if let Some(t) = &m.note {
                self.text_val(t, &format!("{mp}.note"), sc, ctx);
            }
        }
        if let Some(d) = &x.draft_state {
            self.state_of(d, "string", &format!("{p}.draft_state"), sc);
        }
        if let Some(s) = &x.stream {
            self.frame.fors.push((p.to_string(), s.clone()));
        }
        if x.primary {
            self.frame.primaries += 1;
        }
    }

    /// The request message of the first form in a frame's layout.
    fn form_request(&self, n: &Node) -> Option<String> {
        if let Kind::Form(f) = &n.kind {
            return self
                .d
                .endpoints
                .iter()
                .find(|e| e.name == f.endpoint)
                .map(|e| e.input.clone());
        }
        children(&n.kind)
            .into_iter()
            .find_map(|c| self.form_request(c))
    }

    fn row_view(&mut self, v: &RowView, msg: &str, p: &str, sc: &Scope<'a>, rc: &Ctx) {
        self.text(&v.title, &format!("{p}.title"));
        if let Some(t) = &v.subtitle {
            self.text(t, &format!("{p}.subtitle"));
        }
        if let Some(m) = &v.meta {
            self.value(m, &format!("{p}.meta"), sc, rc);
        }
        if let Some(a) = &v.avatar {
            self.row_field(msg, a, &format!("{p}.avatar"));
        }
        if let Some(s) = &v.status {
            self.row_field(msg, &s.field, &format!("{p}.status"));
        }
    }

    fn input(&mut self, i: &'a Input, req: &str, p: &str, sc: &Scope<'a>, ctx: &Ctx, param: bool) {
        self.text(&i.label, &format!("{p}.label"));
        if !param && !req.is_empty() {
            match self.path(req, &i.name) {
                None => self.err(
                    "E_UI_FIELD_UNKNOWN",
                    p,
                    format!("{req} has no field {}", i.name),
                ),
                Some(f) => {
                    if i.control == Control::Repeat {
                        let Some(FieldType::Message(m)) =
                            FieldType::parse(&f.ty).filter(|_| f.repeated)
                        else {
                            self.err(
                                "E_UI_ARG_TYPE",
                                p,
                                format!("repeat fills a repeated message field; {} is not", i.name),
                            );
                            return;
                        };
                        for x in &i.fields {
                            self.input(x, &m, &format!("{p}.fields[{}]", x.name), sc, ctx, false);
                        }
                    }
                }
            }
        }
        if let Some(v) = &i.initial {
            self.value(v, &format!("{p}.initial"), sc, ctx);
        }
        if let Some(o) = &i.options_from
            && let Some(msg) = self.rows_message(&o.query, sc, &format!("{p}.options_from"))
        {
            self.row_field(&msg, &o.value, &format!("{p}.options_from.value"));
            self.row_field(&msg, &o.label, &format!("{p}.options_from.label"));
        }
        if let Some(a) = &i.aside
            && let Some(msg) = self.rows_message(&a.query, sc, &format!("{p}.aside"))
        {
            self.row_field(&msg, &a.field, &format!("{p}.aside.field"));
        }
        match (i.control == Control::Pick, &i.pick) {
            (true, Some(k)) => {
                if !param
                    && !req.is_empty()
                    && self
                        .path(req, &i.name)
                        .is_some_and(|f| f.ty != "string" || f.repeated)
                {
                    self.err(
                        "E_UI_ARG_TYPE",
                        p,
                        format!("a pick fills a string field (the values joined with \", \"); {} is not", i.name),
                    );
                }
                let pp = format!("{p}.pick");
                if let Some(msg) = self.directory(&k.query, k.rows.as_deref(), sc, &pp) {
                    for f in [
                        Some(&k.value),
                        Some(&k.label),
                        k.avatar.as_ref(),
                        k.icon.as_ref(),
                        k.engine.as_ref(),
                        k.agent.as_ref(),
                        k.detail.as_ref(),
                        k.section.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.row_field(&msg, f, &pp);
                    }
                }
                if let Some(v) = &k.exclude {
                    self.value(v, &format!("{pp}.exclude"), sc, ctx);
                }
                for (n, t) in [("empty", &k.empty), ("empty_done", &k.empty_done)] {
                    if let Some(t) = t {
                        self.text(t, &format!("{pp}.{n}"));
                    }
                }
            }
            (true, None) => self.err("E_UI_SHAPE", p, "a pick input names its directory (pick)"),
            (false, Some(_)) => self.err("E_UI_SHAPE", p, "pick belongs to a pick input"),
            (false, None) => {}
        }
    }

    fn bind(&mut self, b: &Bind, p: &str, sc: &Scope<'a>) {
        match b {
            Bind::State(s) if !sc.state.contains_key(s.as_str()) => self.err(
                "E_UI_STATE_UNKNOWN",
                format!("{p}.bind"),
                format!("state {s} is not declared"),
            ),
            Bind::Param(s) if !sc.params.contains_key(s.as_str()) => self.err(
                "E_UI_PARAM_UNKNOWN",
                format!("{p}.bind"),
                format!("param {s} is not declared"),
            ),
            _ => {}
        }
    }

    fn cond(&mut self, c: &Cond, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        match c {
            Cond::Eq(a, b) | Cond::Ne(a, b) | Cond::Gt(a, b) | Cond::Lt(a, b) => {
                self.value(a, p, sc, ctx);
                self.value(b, p, sc, ctx);
            }
            Cond::In { value, .. } | Cond::Empty(value) => self.val(value, p, sc, ctx),
            Cond::And(cs) | Cond::Or(cs) => cs.iter().for_each(|c| self.cond(c, p, sc, ctx)),
            Cond::Not(c) => self.cond(c, p, sc, ctx),
        }
    }

    fn val(&mut self, v: &Value, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        self.value(v, p, sc, ctx);
    }

    /// Checks a value's references; answers its descriptor type when it is known.
    fn value(&mut self, v: &Value, p: &str, sc: &Scope<'a>, ctx: &Ctx) -> Option<String> {
        match v {
            Value::Lit(_) => None,
            Value::Param(n) => match sc.params.get(n.as_str()) {
                Some(t) => Some(t.to_string()),
                None => {
                    self.err(
                        "E_UI_PARAM_UNKNOWN",
                        p,
                        format!("param {n} is not declared"),
                    );
                    None
                }
            },
            Value::State(n) => match sc.state.get(n.as_str()) {
                Some(t) => Some(t.to_string()),
                None => {
                    self.err(
                        "E_UI_STATE_UNKNOWN",
                        p,
                        format!("state {n} is not declared"),
                    );
                    None
                }
            },
            Value::Data { query, field } => {
                let q = sc.queries.get(query.as_str()).copied();
                match q {
                    None => {
                        self.err(
                            "E_UI_QUERY_UNKNOWN",
                            p,
                            format!("query {query} is not declared here"),
                        );
                        None
                    }
                    Some(q) => {
                        let out = self
                            .d
                            .endpoints
                            .iter()
                            .find(|e| e.name == q.endpoint)?
                            .output
                            .clone();
                        match self.path(&out, field) {
                            Some(f) => Some(f.ty.clone()),
                            None => {
                                self.err(
                                    "E_UI_FIELD_UNKNOWN",
                                    p,
                                    format!("{out} has no field {field}"),
                                );
                                None
                            }
                        }
                    }
                }
            }
            Value::Row(f) => match &ctx.row {
                None => {
                    self.err(
                        "E_UI_REF_CONTEXT",
                        p,
                        format!("row.{f} is only available inside a row"),
                    );
                    None
                }
                Some(m) => {
                    let (m, joins) = (m.clone(), ctx.joins.clone());
                    self.row_path(&m, &joins, f, p).map(|x| x.ty.clone())
                }
            },
            Value::Form(f) if ctx.form.as_deref().is_some_and(|x| x.starts_with(COMPOSER)) => {
                let options = ctx.form.as_deref().unwrap_or("")[COMPOSER.len()..]
                    .trim_start_matches(':')
                    .to_string();
                match f.as_str() {
                    "text" | "audio" | "duration" | "language" | "words" | "reply" => {
                        Some("string".into())
                    }
                    o if options.split(',').any(|x| x == o) => Some("bool".into()),
                    _ => {
                        self.err(
                            "E_UI_FIELD_UNKNOWN",
                            p,
                            format!(
                                "a composer sends form.text, audio, duration, language, words, reply and its \
                                 options, not form.{f}"
                            ),
                        );
                        None
                    }
                }
            }
            Value::Form(f) if ctx.form.as_deref() == Some(POST_FORM) => match f.as_str() {
                "text" | "emoji" | "to" => Some("string".into()),
                _ => {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        p,
                        format!("a post's actions read form.text, emoji and to, not form.{f}"),
                    );
                    None
                }
            },
            Value::Form(f) if ctx.form.as_deref() == Some(TOGGLE) => {
                if f == "pressed" {
                    Some("bool".into())
                } else {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        p,
                        format!("a toggle's action reads form.pressed, not form.{f}"),
                    );
                    None
                }
            }
            Value::Form(f)
                if f.split_once('.').is_some_and(|(n, d)| {
                    ctx.picks.iter().any(|x| x == n)
                        && matches!(d, "count" | "agents" | "first" | "query")
                }) =>
            {
                if f.ends_with(".first") || f.ends_with(".query") {
                    Some("string".into())
                } else {
                    Some("int64".into())
                }
            }
            Value::Form(f) if ctx.form.as_deref() == Some(CHAT_FORM) => match f.as_str() {
                "text" | "to" | "emoji" | "choice" | "audio" | "duration" | "language"
                | "words" => Some("string".into()),
                "also" | "follow" => Some("bool".into()),
                _ => {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        p,
                        format!(
                            "a chat's composer has form.text, to, also, emoji, choice, audio, duration, language, \
                             words and follow, not form.{f}"
                        ),
                    );
                    None
                }
            },
            Value::Form(f) if ctx.form.as_deref() == Some(THREAD_COMPOSER) => {
                if f == "text" || f == "to" {
                    Some("string".into())
                } else {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        p,
                        format!("a conversation's message box has only form.text and form.to, not form.{f}"),
                    );
                    None
                }
            }
            Value::Form(f) if ctx.form.as_deref() == Some(MESSAGE_BOX) => {
                if f == "text" {
                    Some("string".into())
                } else {
                    self.err(
                        "E_UI_FIELD_UNKNOWN",
                        p,
                        format!("a message box has only form.text, not form.{f}"),
                    );
                    None
                }
            }
            Value::Form(f) => match &ctx.form {
                None => {
                    self.err(
                        "E_UI_REF_CONTEXT",
                        p,
                        format!("form.{f} is only available to a form's submit action"),
                    );
                    None
                }
                Some(m) => {
                    let m = m.clone();
                    match self.path(&m, f) {
                        Some(x) => Some(x.ty.clone()),
                        None => {
                            self.err("E_UI_FIELD_UNKNOWN", p, format!("{m} has no field {f}"));
                            None
                        }
                    }
                }
            },
            Value::Result(f) => match &ctx.result {
                None => {
                    self.err(
                        "E_UI_REF_CONTEXT",
                        p,
                        format!("result.{f} is only available after a call"),
                    );
                    None
                }
                Some(m) => {
                    let m = m.clone();
                    match self.path(&m, f) {
                        Some(x) => Some(x.ty.clone()),
                        None => {
                            self.err("E_UI_FIELD_UNKNOWN", p, format!("{m} has no field {f}"));
                            None
                        }
                    }
                }
            },
            Value::Error(_) => {
                if !ctx.error {
                    self.err(
                        "E_UI_REF_CONTEXT",
                        p,
                        "error values are only available in on_error",
                    );
                }
                None
            }
            Value::Caller(_) => Some("string".into()),
        }
    }

    fn arg(&mut self, a: &Arg, msg: &str, p: &str, sc: &Scope<'a>, ctx: &Ctx) {
        let field = self.path(msg, &a.field).cloned();
        if field.is_none() {
            self.err(
                "E_UI_FIELD_UNKNOWN",
                p,
                format!("{msg} has no field {}", a.field),
            );
        }
        let got = self.value(&a.value, p, sc, ctx);
        if let (Some(f), Some(g)) = (&field, &got)
            && !compatible(&f.ty, g)
        {
            self.err(
                "E_UI_ARG_TYPE",
                p,
                format!("{msg}.{} is {}, the value is {g}", a.field, f.ty),
            );
        }
        if let (Some(f), Value::Lit(l)) = (&field, &a.value)
            && !lit_fits(&f.ty, f.repeated, l)
        {
            self.err(
                "E_UI_ARG_TYPE",
                p,
                format!("{msg}.{} is {}, the literal is {l}", a.field, f.ty),
            );
        }
    }

    fn text(&mut self, t: &Text, p: &str) {
        if let Text::Plain(s) = t
            && s.trim().is_empty()
        {
            self.err("E_UI_TEXT", p, "a text is not empty");
        }
    }

    fn endpoint(&mut self, name: &str, p: &str) -> Option<&'a Endpoint> {
        match self.d.endpoints.iter().find(|e| e.name == name) {
            None => {
                self.err(
                    "E_UI_ENDPOINT_UNKNOWN",
                    p,
                    format!("endpoint {name} is not declared"),
                );
                None
            }
            Some(e) if !e.ui => {
                self.errors.push(
                    Error::new("E_UI_ENDPOINT_NOT_UI", p, format!("endpoint {name} is not marked ui"))
                        .help("mark it .ui() so screens may call it; other endpoints stay out of a document's reach"),
                );
                None
            }
            Some(e) => Some(e),
        }
    }

    /// The message of a query's rows.
    fn rows_message(&mut self, query: &str, sc: &Scope<'a>, p: &str) -> Option<String> {
        let Some(q) = sc.queries.get(query) else {
            self.err(
                "E_UI_QUERY_UNKNOWN",
                format!("{p}.query"),
                format!("query {query} is not declared here"),
            );
            return None;
        };
        let Some(rows) = &q.rows else {
            self.err(
                "E_UI_ROWS",
                format!("{p}.query"),
                format!("query {query} has no rows"),
            );
            return None;
        };
        let e = self.d.endpoints.iter().find(|e| e.name == q.endpoint)?;
        let f = self.path(&e.output, rows)?;
        match FieldType::parse(&f.ty) {
            Some(FieldType::Message(m)) => Some(m),
            _ => None,
        }
    }

    /// A sortable column: `by` is a string field of the request of the table's query (the client passes `asc` or
    /// `desc` in it).
    fn sort_field(&mut self, query: &str, by: &str, p: &str, sc: &Scope<'a>) {
        let Some(e) = sc
            .queries
            .get(query)
            .and_then(|q| self.d.endpoints.iter().find(|e| e.name == q.endpoint))
        else {
            return;
        };
        match self.path(&e.input, by) {
            None => self.err(
                "E_UI_FIELD_UNKNOWN",
                p,
                format!("{} has no field {by}", e.input),
            ),
            Some(f) if f.ty != "string" || f.repeated => self.errors.push(
                Error::new(
                    "E_UI_ARG_TYPE",
                    p,
                    format!("{}.{by} is {}; a sort field is a string", e.input, f.ty),
                )
                .help("the client passes asc or desc in it"),
            ),
            Some(_) => {}
        }
    }

    fn row_field(&mut self, msg: &str, f: &str, p: &str) -> Option<&'a Field> {
        let r = self.path(msg, f);
        if r.is_none() {
            self.err("E_UI_FIELD_UNKNOWN", p, format!("{msg} has no field {f}"));
        }
        r
    }

    /// Whether request `req` has every field of `key` with the same type (the key is passed as the request).
    fn covers(&self, req: &str, key: &str) -> bool {
        let Some(k) = self.message(key) else {
            return false;
        };
        k.fields.iter().all(|f| {
            self.path(req, &f.name)
                .is_some_and(|r| r.ty == f.ty && r.repeated == f.repeated)
        })
    }

    fn message(&self, name: &str) -> Option<&'a crate::Message> {
        self.d.messages.iter().find(|m| m.name == name)
    }

    /// A field by path (`customer.name`) through nested messages.
    fn path(&self, msg: &str, path: &str) -> Option<&'a Field> {
        let mut m = self.message(msg)?;
        let mut parts = path.split('.').peekable();
        while let Some(part) = parts.next() {
            let f = m.fields.iter().find(|f| f.name == part)?;
            if parts.peek().is_none() {
                return Some(f);
            }
            match FieldType::parse(&f.ty)? {
                FieldType::Message(n) => m = self.message(&n)?,
                _ => return None,
            }
        }
        None
    }
}

/// The scope an action reference resolves in: the screen, overlay or menu path before `.layout`/`.header`/….
fn scope_path(p: &str) -> String {
    for cut in [".layout", ".header", ".buttons", ".items"] {
        if let Some(i) = p.find(cut) {
            // an overlay inside a screen: its own scope
            return p[..i].to_string();
        }
    }
    p.to_string()
}

fn merge(ctxs: &[Ctx]) -> Ctx {
    let mut out = Ctx::default();
    for c in ctxs {
        if out.row.is_none() {
            out.row = c.row.clone();
            out.joins = c.joins.clone();
        }
        if out.form.is_none() {
            out.form = c.form.clone();
            out.picks = c.picks.clone();
        }
    }
    out
}

/// A name of a `viz` option or of a key of its rows: letters, digits and `_` (an index in a path is a name too);
/// never a name a JavaScript object treats specially (the client writes the bound data under it).
fn option_name(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !matches!(s, "__proto__" | "constructor" | "prototype")
}

fn snake(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// Whether a value of type `got` may fill a field of type `want` (numbers widen, refs are uuids).
fn compatible(want: &str, got: &str) -> bool {
    let norm = |t: &str| -> String {
        match t {
            "int32" | "int64" | "double" | "decimal" => "number".into(),
            t if t.starts_with("ref:") => "uuid".into(),
            t => t.to_string(),
        }
    };
    let (w, g) = (norm(want), norm(got));
    w == g || w == "string" && g != "bool" && !g.starts_with("message:") || w == "json"
}

/// A scalar as a link carries it (`30` and `"30"` are the same param value).
fn scalar_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn lit_fits(ty: &str, repeated: bool, v: &serde_json::Value) -> bool {
    use serde_json::Value as J;
    if repeated {
        return matches!(v, J::Array(_) | J::Null);
    }
    match (FieldType::parse(ty), v) {
        (_, J::Null) => true,
        (Some(FieldType::Scalar("bool")), J::Bool(_)) => true,
        (Some(FieldType::Scalar("bool")), _) => false,
        (Some(FieldType::Scalar("int32")), J::Number(n)) => n.is_i64() || n.is_u64(),
        (Some(FieldType::Scalar("int64")), J::Number(n)) => n.is_i64() || n.is_u64(),
        (Some(FieldType::Scalar("int64" | "decimal")), J::String(s)) => s.parse::<f64>().is_ok(),
        (Some(FieldType::Scalar("double" | "decimal")), J::Number(_)) => true,
        (Some(FieldType::Scalar("int32" | "int64" | "double" | "decimal")), _) => false,
        (
            Some(FieldType::Scalar("string" | "uuid" | "date" | "timestamp" | "bytes")),
            J::String(_),
        ) => true,
        (Some(FieldType::Scalar("string" | "uuid" | "date" | "timestamp" | "bytes")), _) => false,
        (Some(FieldType::Enum(_)), J::String(_)) => true,
        (Some(FieldType::Enum(_)), _) => false,
        (Some(FieldType::Ref { .. }), J::String(_)) => true,
        (Some(FieldType::Ref { .. }), _) => false,
        _ => true,
    }
}

/// Every text key used, with where.
fn collect_keys(ui: &Ui) -> Vec<(String, String)> {
    let mut out = vec![];
    let v = serde_json::to_value(ui).unwrap_or_default();
    fn walk(v: &serde_json::Value, path: String, viz: bool, out: &mut Vec<(String, String)>) {
        match v {
            serde_json::Value::Object(m) => {
                if m.len() <= 2
                    && let Some(serde_json::Value::String(k)) = m.get("key")
                    && m.keys().all(|x| x == "key" || x == "args")
                {
                    out.push((path.clone(), k.clone()));
                }
                for (k, x) in m {
                    // a viz's options and the key map of its rows are data of the plugin, not texts
                    if k == "texts" || viz && (k == "options" || k == "map") {
                        continue;
                    }
                    walk(x, format!("{path}.{k}"), viz || k == "viz", out);
                }
            }
            serde_json::Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    walk(x, format!("{path}[{i}]"), viz, out);
                }
            }
            _ => {}
        }
    }
    walk(&v, "ui".into(), false, &mut out);
    out
}
