//! The plugin's UI document (`ui` in the descriptor): contributions (sidebar
//! groups, home widgets, palette commands, record menus), screens and overlays built from the catalog's primitives,
//! queries of the plugin's `ui` endpoints, topics they follow, state, and action chains of closed steps. It is data:
//! no client runs plugin code, and nothing here styles anything (tones, emphasis and density are meanings).
//!
//! The JSON shape: a `Value` or a `Step` is an object with exactly one key naming its kind (`{"param": "status"}`,
//! `{"call": {...}}`); a node is one such key plus the common `visible`, `enabled`, `fallback`. Names of request and
//! response fields are the descriptor's field names (snake_case); a `path` may go into nested messages
//! (`customer.name`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The catalog revision this crate knows. Every primitive, option and step kind of revisions 1 to 3 is below
/// (revision 2 added `viz`, the charts and figures of the charts catalogue, and `board`, the dashboard layout;
/// revision 3 the app-like primitives `stream`, `post`, `process`, `strip`, `composer`, `header`, the `pick` input,
/// options of `split`, `list`, `icon_button`, `agent_run` and state, the steps `draft`, `reply_to`, `scroll_to`,
/// `copy`, `merge`, the query options `older`, `debounce_ms`, `join`, and the formats `elapsed`, `countdown`,
/// `span`; it deprecated `chat`).
pub const CATALOG: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Ui {
    /// The catalog revision the document is written for.
    #[serde(default = "one")]
    pub catalog: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nav: Vec<NavGroup>,
    /// The screen the plugin opens on (a screen id; it takes no required param). Without it, the client opens the
    /// screen of the navigation's first entry that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub home: Vec<Widget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menus: Vec<RecordMenu>,
    /// Queries used outside screens (badges, nav rows, home widgets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<Query>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<Screen>,
    /// Overlays opened from anywhere (a command's "New order" dialog).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
    /// Records the command palette finds in this plugin (each source is a read `ui` endpoint taking the typed text).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search: Vec<SearchSource>,
    /// Text catalogs: locale → key → ICU message (`en` is the fallback).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, BTreeMap<String, String>>,
}

fn one() -> u32 {
    1
}

/// A text: a literal, a catalog key with arguments, or a value shown as text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        args: BTreeMap<String, Value>,
    },
    Value(Value),
}

/// Where a value comes from. `{"lit": …}` is a literal of any JSON type.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Value {
    Lit(Json),
    /// A param of the screen or overlay.
    Param(String),
    /// A state value of the screen or overlay.
    State(String),
    /// A field of a query's response (one record), by path.
    Data {
        query: String,
        field: String,
    },
    /// A field of the current row (inside a table, list, chart, nav rows or a row menu).
    Row(String),
    /// A field of the enclosing form.
    Form(String),
    /// The member looking at the screen.
    Caller(CallerField),
    /// A field of the last `call` result of the running chain.
    Result(String),
    /// Inside `on_error`: what failed.
    Error(ErrorField),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallerField {
    UserId,
    DisplayName,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorField {
    /// The index of the failing step.
    Step,
    /// The error code (`not_found`, `orders_not_cancellable`).
    Code,
    /// `failed` or `unknown`.
    Outcome,
    /// The error's human message.
    Message,
}

/// A presentation condition (never access control).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Cond {
    Eq(Value, Value),
    Ne(Value, Value),
    In { value: Value, set: Vec<Json> },
    Empty(Value),
    Gt(Value, Value),
    Lt(Value, Value),
    And(Vec<Cond>),
    Or(Vec<Cond>),
    Not(Box<Cond>),
}

/// A request field set to a value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Arg {
    pub field: String,
    pub value: Value,
}

/// A typed input of a screen or overlay (in the URL for screens).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub name: String,
    /// A descriptor field type: `string`, `int64`, `uuid`, `enum:OrderStatus`, …
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Json>,
}

/// A named typed value of a screen or overlay that primitives read and `set` steps change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Json>,
    /// Params of the frame: when one of them changes, the state returns to its default (catalog 3; a tab that
    /// starts over in another conversation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reset: Vec<String>,
}

/// A named call of one of the plugin's `ui` endpoints.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub name: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    /// A repeated message field of the response: the rows tables, lists and charts iterate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    /// `page_token` / `page_size` in the request, `next_page_token` in the response.
    #[serde(default, skip_serializing_if = "is_false")]
    pub paged: bool,
    /// Append-only feed: on an invalidation the client asks for rows after the last cursor and appends them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed: Option<Feed>,
    /// Topics whose invalidations re-run it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow: Vec<Follow>,
    /// Re-run every N seconds (at least 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<u32>,
    /// The oldest the data may get while shown when the query follows topics (seconds; default 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u32>,
    /// The query runs only while this holds (a record panel reads once a row is selected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
    /// The page before the rows (catalog 3): the same endpoint and args read again with `arg` set to the answer's
    /// `token`; a `stream` loads older pages with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<Older>,
    /// Arguments that change wait this long before the read (ms, at most [`MAX_DEBOUNCE_MS`]; catalog 3): a preview
    /// of what is typed. An answer replaces the previous one only while its arguments are still current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u32>,
    /// Lookups the rows' templates read as `row.<as>.<field>` (catalog 3): each row gets the item of the response's
    /// repeated field `rows` whose `key` equals the row's `by`, from every answer the node holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub join: Vec<Join>,
}

/// The longest `debounce_ms` of a query.
pub const MAX_DEBOUNCE_MS: u32 = 2000;

/// How a query reads the page before its rows: the response field with the token, the request field it goes in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Older {
    pub token: String,
    pub arg: String,
}

/// A lookup of a query's rows: `row[as]` is the item of the response's repeated field `rows` whose `key` equals
/// `row[by]`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Join {
    #[serde(rename = "as")]
    pub name: String,
    pub rows: String,
    pub key: String,
    pub by: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Feed {
    /// The request field that takes the last cursor.
    pub after: String,
    /// The response field that carries the cursor to pass next time.
    pub cursor: String,
    /// The row field rows are deduplicated by.
    pub key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Follow {
    pub topic: String,
    /// The key's fields; none: any key of the topic.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key: Vec<Arg>,
    /// The least time between re-runs caused by this topic (ms; default 0 on screens, 2000 for nav and home).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_interval_ms: Option<u32>,
}

/// A chain of steps run on an event.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_error: Vec<Step>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Call(Call),
    Confirm(Confirm),
    Open(Target),
    Close {},
    Navigate(Target),
    Back {},
    Set(Set),
    Refresh {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        queries: Vec<String>,
    },
    Toast(Toast),
    OpenUrl {
        value: Value,
    },
    /// Puts the text into the composer of the item's stream (or the frame's primary composer), focused, the caret at
    /// the end (catalog 3).
    Draft {
        text: Text,
    },
    /// Adds `@<name> ` to that composer (unless there), focuses it, and sends `key` as its `reply` while the mention
    /// stays in the text (catalog 3).
    ReplyTo {
        name: Value,
        key: Value,
    },
    /// Scrolls the stream that holds an item with this key so the item is centred; nothing when none does
    /// (catalog 3).
    ScrollTo {
        key: Value,
    },
    /// The clipboard: a value, or the screen's link with `#<link>` (catalog 3).
    #[serde(rename = "copy")]
    CopyValue(CopyValue),
    /// Puts a row into the loaded rows of a query (by the stream's key) until the query's next answer (catalog 3).
    Merge {
        query: String,
        row: Value,
    },
}

/// What a `copy` step copies: exactly one of a value and a link to an item.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CopyValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    /// Send the enclosing form's values (with `args` over them).
    #[serde(default, skip_serializing_if = "is_false")]
    pub form: bool,
    /// Show the form's values as a pending row of this query until a re-read returns the real one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_label: Option<Text>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub danger: bool,
    /// The member must type this value to confirm (a destructive change).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_label: Option<Text>,
    /// What is affected, shown as a record line (`Order #1048 · Hartwell & Co · $2,480.00`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_icon: Option<String>,
}

/// A screen or an overlay with its params.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Set {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    pub value: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Toast {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    /// Show the failing call's own message (in `on_error`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub error: bool,
    /// A second line (`#1048 · DPD 1Z84 0927`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    /// A button in the toast running an action of the same frame (`Undo`, `View label`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ToastAction>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToastAction {
    pub label: Text,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Ok,
    Attention,
    Danger,
}

/// A full page: `/w/{slug}/p/{plugin}/s/{id}?{params}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    pub id: String,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<State>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<Query>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
    /// Buttons and menus in the screen's header.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header: Vec<Node>,
    pub layout: Node,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKind {
    Dialog,
    SidePanel,
    Sheet,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

/// A dialog, side panel or sheet over a screen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    pub id: String,
    pub kind: OverlayKind,
    #[serde(default)]
    pub size: Size,
    pub title: Text,
    /// A small line above the title (`COMMERCE · ORDERS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kicker: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<State>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<Query>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Buttons in the overlay's header (a side panel) or footer (a dialog).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<Node>,
    pub layout: Node,
}

// ---------------------------------------------------------------------------------------------- contributions

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavGroup {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<NavEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavEntry {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The screen it opens (none: a section with children).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    /// A count from a document-level query (0 hides it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Value>,
    /// How the badge reads: a quiet count (the default) or an alert that asks for attention.
    #[serde(default, skip_serializing_if = "is_count")]
    pub badge_style: BadgeStyle,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NavEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<NavRows>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadgeStyle {
    #[default]
    Count,
    Alert,
}

fn is_count(b: &BadgeStyle) -> bool {
    *b == BadgeStyle::Count
}

/// Child rows from a query: label, per-row badge, a limit with "Show all", an add row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavRows {
    pub query: String,
    pub label: String,
    /// A row field: the unread count (the label turns bold).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// A row field: the mentions of the viewer (an amber pill instead of the unread count).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub screen: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    #[serde(default = "five")]
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<NavAdd>,
}

fn five() -> u32 {
    5
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavAdd {
    pub title: Text,
    pub overlay: String,
}

/// A workspace home widget: a node over document-level queries, optionally linking to a screen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Widget {
    pub id: String,
    pub title: Text,
    pub node: Node,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Target>,
}

/// A command-palette entry.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A screen to navigate to, or a document-level overlay to open.
    pub target: Target,
}

/// Records of the plugin the command palette finds: a read `ui` endpoint called with the typed text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchSource {
    /// The group the results are listed under (`Orders`).
    pub title: Text,
    pub endpoint: String,
    /// The request field that takes the typed text.
    pub text: String,
    /// The repeated response field with the results.
    pub rows: String,
    pub item: RowView,
    /// Where a result leads (`{row}` is the result).
    pub open: Target,
    /// What the palette offers on the top result (`New order for Kestrel Hotels`; `{row}` is the result).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SearchAction>,
}

/// An action on a found record: a screen, or a document-level overlay, with args from the record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchAction {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub target: Target,
}

/// Actions offered on the plugin's rows of a message type, wherever its own screens show them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordMenu {
    /// The row's message (`Order`).
    pub message: String,
    pub items: Vec<MenuItem>,
    /// The chains the items run (`{row}` is the record).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

// ---------------------------------------------------------------------------------------------- nodes

/// One primitive with the common options.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Node {
    #[serde(flatten)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Cond>,
    /// What an older client draws instead (display primitives only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Box<Node>>,
}

impl From<Kind> for Node {
    fn from(kind: Kind) -> Self {
        Node {
            kind,
            visible: None,
            enabled: None,
            fallback: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    // structure
    Stack(Stack),
    Columns(Columns),
    Section(Section),
    Card(Card),
    Tabs(Tabs),
    Divider {},
    Split(Split),
    Board(Board),
    // navigation
    Segmented(Segmented),
    Link(Link),
    Breadcrumbs(Breadcrumbs),
    // data
    Table(Table),
    List(List),
    Fields(Fields),
    Metric(Metric),
    Chart(Chart),
    Viz(Viz),
    Timeline(Timeline),
    Status(StatusNode),
    Tag(Tag),
    Avatar(Avatar),
    AvatarStack(AvatarStack),
    Meter(Meter),
    Progress(Progress),
    Text(TextNode),
    CopyField(CopyField),
    Empty(Empty),
    // input
    Form(Form),
    FilterBar(FilterBar),
    // actions
    Button(Button),
    IconButton(IconButton),
    Menu(Menu),
    // feedback
    Notice(Notice),
    // records
    RecordHeader(RecordHeader),
    AgentRun(AgentRun),
    // conversations
    Conversation(Conversation),
    /// Deprecated in catalog 3 (`W_UI_DEPRECATED`): `stream`, `post`, `composer`, `header`, `strip` and a `fill`
    /// split replace it.
    Chat(Chat),
    // catalog 3
    Header(Header),
    Stream(Stream),
    Process(Process),
    Strip(Strip),
    Composer(ComposerNode),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    Tight,
    #[default]
    Normal,
    Loose,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stack {
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub gap: Gap,
    /// How a horizontal stack spreads its children (`between`: first left, last right).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<Justify>,
    pub children: Vec<Node>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    Start,
    Between,
    End,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Columns {
    /// Relative widths (`[2, 1]`); equal when empty. Columns stack on a phone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ratios: Vec<u32>,
    pub children: Vec<Node>,
}

/// A dashboard (catalog revision 2): items on one shared 12-column grid, flowing left to right and wrapping when a
/// row is full. Every row is as tall as its tallest item. At most [`BOARD_MAX_ITEMS`] items; not nested in another
/// board.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub children: Vec<BoardItem>,
}

/// The most items of a `board`.
pub const BOARD_MAX_ITEMS: usize = 60;
/// The columns of a `board`'s grid: the widest span, and the default one.
pub const BOARD_COLUMNS: u32 = 12;

/// One item of a `board`: a node and the columns it takes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoardItem {
    /// 1 to 12 columns (12, the full width, when absent). A narrower screen widens the item.
    #[serde(default = "board_columns", skip_serializing_if = "is_board_columns")]
    pub span: u32,
    pub node: Node,
}

fn board_columns() -> u32 {
    BOARD_COLUMNS
}

fn is_board_columns(span: &u32) -> bool {
    *span == BOARD_COLUMNS
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Node>,
    pub children: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer: Vec<Node>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Card {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    /// A value at the end of the title line (a total).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_format: Option<Format>,
    pub children: Vec<Node>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tabs {
    /// The state (or param) holding the selected tab's id; none: the client keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<Bind>,
    pub tabs: Vec<Tab>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub id: String,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Value>,
    pub children: Vec<Node>,
}

/// A state or a param a control writes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Bind {
    State(String),
    Param(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Split {
    pub list: Box<Node>,
    pub detail: Box<Node>,
    /// An app-like screen (catalog 3): the screen takes the viewport's height under the shell's header and nothing
    /// scrolls the page; in each side the first `stream` takes the free height and scrolls itself. The detail is an
    /// in-page side panel shown while its `visible` holds (a full-screen page on a phone).
    #[serde(default, skip_serializing_if = "is_false")]
    pub fill: bool,
    /// The panel's width is dragged by its border (px), kept per viewer (catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<Resize>,
    /// A bool state: true, the detail fills the screen (catalog 3). The detail's `header.expand` toggles it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand: Option<StateName>,
}

/// The side panel's width: at least `min`, at most `max`, `default` at first and on reset (px, 240 to 1200); the
/// column keeps at least `keep`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Resize {
    pub min: u32,
    pub max: u32,
    pub default: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<u32>,
}

/// The narrowest and the widest a resizable panel may be set (px).
pub const RESIZE_MIN: u32 = 240;
pub const RESIZE_MAX: u32 = 1200;

/// A state of the frame, by name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateName {
    pub state: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Segmented {
    pub bind: Bind,
    pub options: Vec<Opt>,
}

/// A choice: a value and its label.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Opt {
    pub value: Json,
    pub label: Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub text: Text,
    pub target: Target,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Breadcrumbs {
    pub items: Vec<Crumb>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Crumb {
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

/// How a value is shown (normative semantics in the proposal §4.6).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Format {
    Text {},
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimals: Option<u8>,
    },
    /// A decimal amount; `currency` is a row or record field with an ISO 4217 code, or `code` a literal one.
    Money {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        currency: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Percent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimals: Option<u8>,
    },
    Date {},
    Datetime {},
    /// `5 min ago`; `short`: `5 min`, `2 h`, `Fri`.
    Relative {
        #[serde(default, skip_serializing_if = "is_false")]
        short: bool,
    },
    Bool {},
    /// Counts up, live, from a moment in ms (`0:48`, `12:05`, `1:02:05`; catalog 3).
    Elapsed {},
    /// Counts down, live, to a moment in ms (`8 min`, `<1 min`; nothing once past; catalog 3).
    Countdown {},
    /// A duration in ms (`<1s`, `6s`, `1m 05s`, `1h 02m`; catalog 3).
    Span {},
    /// An enum value with its catalog label and a tone per value.
    Status {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        tones: BTreeMap<String, Tone>,
    },
    Member {},
    Tags {},
    Avatar {},
    Mono {},
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Start,
    End,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// A row field path.
    pub field: String,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// The request field that sorts by this column (the client passes `asc`/`desc` in it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    /// A second, quieter line under the value (a row field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// An avatar before the value: a row field with a picture URL or initials (`HW`), else the value's initials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub query: String,
    pub columns: Vec<Column>,
    /// What opening a row does (a chain; `{row}` is the row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    /// Buttons or a menu per row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_actions: Vec<Node>,
    /// Buttons, search and filters above the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toolbar: Vec<Node>,
    /// The state that holds the selected row's key field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    /// A tone per row (the row's left rail and fill), by condition, first match wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_tone: Vec<ToneWhen>,
    /// Rows can be checked; the state holds the checked rows' keys (a list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Selection>,
    /// How many rows there are in all (`Showing 1–10 of 1,248`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<Value>,
    /// The line over the compact rows (`412 of 1,248 orders · newest first`); the count alone when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_label: Option<Text>,
    /// One row as a list item, where a table does not fit (a phone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<RowView>,
}

/// A row drawn as a list item: title, subtitle, an amount and a status at the end, an avatar or icon at the start.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RowView {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_format: Option<Format>,
    /// A row field: a picture URL, else initials of the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusField>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// A state, or a param of the screen (the selection is then in the link).
    pub state: String,
    pub key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToneWhen {
    pub when: Cond,
    pub tone: Tone,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub query: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_format: Option<Format>,
    /// A member id or a name field drawn as an avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A row field with the icon's name (one icon per row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_field: Option<String>,
    /// A quiet text before the meta (`×2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aside: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_actions: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    /// A filter box over the rows' titles with this placeholder; it filters the loaded rows, no request (catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Text>,
    /// A bool row field: a green dot after the title (catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<String>,
    /// On a phone, the first N rows, then `Show all N` (1 to 100; catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// On a phone, a label over the rows in a card (`People · 9`; catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_label: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct StatusField {
    pub field: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tones: BTreeMap<String, Tone>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    pub items: Vec<FieldItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldItem {
    pub label: Text,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// The value links to a screen (drawn as a chip with an arrow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Target>,
    /// A chip's picture URL or initials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub title: Text,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// A change shown next to the value (`+12 %`), toned up/down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_icon: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Line,
    Area,
    Pie,
}

/// The first charts (catalog revision 1), kept for compatibility: new screens use [`Viz`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Chart {
    pub kind: ChartKind,
    pub query: String,
    /// The row field on the x axis (or the slice label of a pie).
    pub x: String,
    pub series: Vec<Series>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Series {
    pub field: String,
    pub label: Text,
}

/// A chart or a figure of the charts catalogue (catalog revision 2): the kind `kind` of the family `family`
/// ([`crate::viz_kinds`]), drawn from `options` after the binds have written the data into them. A display
/// primitive: an older client draws its `fallback`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Viz {
    pub family: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    /// The muted text at the right of the head (`Last 30 days`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Text>,
    /// The static options of the kind (`{"fmt": "money", "curve": "smooth"}`), at most [`VIZ_MAX_OPTIONS`] bytes.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub options: serde_json::Map<String, Json>,
    /// Data written into the options before drawing, in order (at most [`VIZ_MAX_BINDS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bind: Vec<VizBind>,
    /// The height in px ([`VIZ_MIN_HEIGHT`]…[`VIZ_MAX_HEIGHT`]); the kind's own height when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The state or param of the screen that receives the picked value (null when nothing is picked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Bind>,
    /// Where the card's link leads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Target>,
}

/// The largest `options` of a `viz`, as compact JSON.
pub const VIZ_MAX_OPTIONS: usize = 16 << 10;
/// The most binds of a `viz`.
pub const VIZ_MAX_BINDS: usize = 16;
pub const VIZ_MIN_HEIGHT: u32 = 16;
pub const VIZ_MAX_HEIGHT: u32 = 1200;

/// One piece of data of a `viz`: `from` is written to the option `to` (a name or a dotted path: `center.value`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VizBind {
    pub to: String,
    pub from: VizSource,
}

/// Where a `viz` option's data comes from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VizSource {
    /// An array: the field of every row of the query.
    Column {
        query: String,
        field: String,
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        cast: Option<VizAs>,
    },
    /// `[{name: label, values: [...]}, …]`: one series per field.
    Series { query: String, fields: Vec<Series> },
    /// Long rows to wide: `options.labels` gets the distinct `x` in row order (formatted by `as`), the option gets
    /// `[{name, values}]` per distinct value of `series` (a missing cell is null).
    Pivot {
        query: String,
        x: String,
        series: String,
        value: String,
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        cast: Option<VizAs>,
    },
    /// An array of objects, one per row: key → row field. Numeric strings become numbers except under the keys
    /// listed in `text`.
    Rows {
        query: String,
        map: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        text: Vec<String>,
    },
    /// One value.
    Value(Value),
}

/// How a `column`'s cells (or a `pivot`'s labels) are read.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VizAs {
    /// Numbers and numeric strings become numbers, anything else stays (the default).
    Number,
    /// Kept as a string.
    Text,
    /// An ISO date or datetime becomes a short label (`Nov 4`, with the year when the rows span more than one).
    Date,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub query: String,
    pub text: String,
    pub time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// A row field: the actor's picture URL (else their initials).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// A row field with an icon name, for actors without a face (an agent, a plugin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<StatusField>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StatusNode {
    pub value: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tones: BTreeMap<String, Tone>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub text: Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Avatar {
    /// A member id (drawn with the member's name and picture) or a name.
    pub value: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AvatarStack {
    /// A list of member ids.
    pub value: Value,
    #[serde(default = "five")]
    pub max: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub value: Value,
    pub max: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    /// A fraction 0..1.
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    #[default]
    Body,
    Heading,
    Caption,
    Mono,
    /// A small uppercase label over a group (`ACTIVITY`).
    Label,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextNode {
    pub text: Text,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CopyField {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Empty {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Box<Node>>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Text,
    Textarea,
    Number,
    Money,
    Date,
    Datetime,
    Select,
    MultiSelect,
    Checkbox,
    Switch,
    RadioCards,
    Member,
    Upload,
    Repeat,
    Search,
    /// Chips over a directory (catalog 3): the input's `pick` names the rows; the value is the chosen values joined
    /// with `, `.
    Pick,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// The request field it fills (a form) or the param it sets (a filter bar).
    pub name: String,
    pub control: Control,
    pub label: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Text>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<Value>,
    /// Literal options of a select, multi-select or radio cards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Opt>,
    /// Options read from a query's rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_from: Option<OptionsFrom>,
    /// The inputs of one row of a `repeat` (a repeated message field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Input>,
    /// `repeat`: rows at least / at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    /// A filter chip's icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `half`: shares its line with the next half input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<InputWidth>,
    /// A number drawn with − and + buttons.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stepper: bool,
    /// `repeat`: a value shown at the end of each row, from the i-th row of a query (a line total from a quote).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aside: Option<RowAside>,
    /// `repeat`: the label of the add row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_label: Option<Text>,
    /// `pick`: the directory the chips are chosen from (catalog 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<Pick>,
}

/// The directory of a `pick` input: rows of a query (its `rows`, or the repeated field `rows`), each chosen by its
/// `value` and drawn with its `label`, avatar or agent tile, `detail`, grouped by `section`. `exclude` is a list
/// value of values not offered (members already in). Conditions and texts read `form.<name>.count`,
/// `form.<name>.agents` (the chosen rows whose `agent` is true) and `form.<name>.first` (the first word of the first
/// chosen label).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Pick {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Value>,
    /// The line when nothing matches the typed text (its texts read the typed text as `form.<name>.query`), and
    /// when every row is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_done: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputWidth {
    Half,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RowAside {
    pub title: Text,
    pub query: String,
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OptionsFrom {
    pub query: String,
    pub value: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    pub inputs: Vec<Input>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Form {
    /// The endpoint whose request the form fills (its fields name the request's fields).
    pub endpoint: String,
    pub sections: Vec<FormSection>,
    /// The chain run on submit (its `call` with `form: true` sends the values).
    pub submit: String,
    pub submit_label: Text,
    /// The submit button's label on a phone, where the form is a sheet (absent: `submit_label`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_submit_label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_label: Option<Text>,
    /// A total under the inputs (from a quote query that reads the form's values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<FormSummary>,
    /// A line next to the buttons (what happens after submitting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Text>,
    /// A second way to send the values (`Save draft`): its chain, like `submit`, sends them with `form: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate: Option<FormButton>,
    /// Lines under the inputs, each shown while its condition (over the form's values) holds (catalog 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<FormNote>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormNote {
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormButton {
    pub label: Text,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormSummary {
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<Text>,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FilterBar {
    /// Inputs that set params (`name` is the param).
    pub inputs: Vec<Input>,
}

/// The head of a record (a panel or a page): title, tags, a subtitle, actions; on a phone the title moves to the
/// top bar (`short`), the actions to the bottom bar and the menu to an action sheet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordHeader {
    pub title: Text,
    /// The top bar's title on a phone (`#1048`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Text>,
    /// The heading and the line under it on a phone, when they differ (`Hartwell & Co`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_title: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_subtitle: Option<Text>,
    /// Status chips next to the title.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Node>,
    /// Buttons under the title.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Node>,
    /// More actions (`…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Menu>,
    /// Actions of the panel's corner buttons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<String>,
}

/// An agent's run on a record: who, its state, its steps, Stop and a message box. The data is the plugin's (a
/// read endpoint), the buttons run the plugin's actions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentRun {
    pub query: String,
    /// Record fields: the agent's name and the line under it.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: StatusField,
    /// The repeated field of steps, each with a text and a state (`done`, `running`, `pending`).
    pub steps: String,
    pub step_text: String,
    pub step_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_when: Option<Cond>,
    /// A message box: the action gets the text as `{form: text}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Composer>,
    /// What the run needs from the viewer: an amber line first, with one button; shown while its text is not empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<RunNeeds>,
    /// Opens the run's session page ("Open session"). With it the card shows the current step, not the step list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<RunLink>,
    /// The changes the run made to this record: a repeated field and each row's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<RunChanges>,
    /// The agent's earlier sessions on this record: a record field with the line ("2 earlier sessions") and its
    /// action; hidden while the line is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earlier: Option<RunEarlier>,
    /// The run's full log in a panel (catalog 3): a head with the agent's tile (`icon`, `engine`), the title, the
    /// `subtitle`, the status with `status_label`, `step_label` and `elapsed`, Stop; then every step (`step_text` its
    /// title, `step_state` one of `done`, `now`, `wait`, `todo`, and `step_detail`, `step_time`, `step_tool` — a
    /// call as a process row's — `step_evidence`, `step_lines`) and the `note` while the run is not live
    /// (`live_when`). Response fields but the `step_*`, which are fields of a step.
    #[serde(default, skip_serializing_if = "is_false")]
    pub log: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_lines: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_when: Option<Cond>,
}

/// A conversation (a channel, a thread, a direct message): its messages oldest first, each with its author (a face
/// or an icon), a badge (`AGENT`), a quiet line (`via Commerce`), a chip before the text (`To people`), a receipt
/// under it (what an agent did with the message) and a card (a session, a question, an approval) with one button;
/// a composer under them, optionally with the addressees a message can go to. All names but `query` are row fields.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub query: String,
    pub author: String,
    pub text: String,
    pub time: String,
    /// The author's picture URL (else their initials).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// An icon name, for authors without a face (an agent, a plugin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<MessageCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer: Option<ThreadComposer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
}

/// A messenger screen (Pencil, `S06 Chat`): a channel or a direct message with its header, members and tabs, its
/// messages (mentions, reactions, dictated audio, agents' result cards, approvals, thread summaries), the strip of an
/// agent at work and the composer; a side panel shows the open thread, an agent run's log or the members. The data is
/// the plugin's: `channel` reads one conversation (its `messages` rows have at least `id`, `author`, `text`, `time`),
/// `thread` the open thread (`root`, `replies`), `run` a run's log (`steps`), `members` the people and agents (`people`,
/// `agents`). The actions run on a message (`row`) or read
/// `{form: text, to, also, emoji}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Chat {
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<String>,
    /// A query of the agent a direct message is with (`name`, what it can do, its access, channels and runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// A query of whom a new message can go to (`people`, `agents`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    pub actions: ChatActions,
}

/// What a chat screen can do: `send` posts in the conversation (`{form: text}`, and a dictated message's recording:
/// `audio` as a data URL, `duration`, `language`, `words` as JSON), `reply` in the open thread (`{form: text, to,
/// also}`, the same recording); `thread`, `run`, `approve`, `decline`, `undo`, `card` and `react` (`{form: emoji}`) run on a
/// message; `members` opens the members, `close` closes the side panel; `stop` and `tell` (`{form: text}`) act on the
/// open run; `follow` (`{form: follow}`) follows the open thread or stops following it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ChatActions {
    pub send: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub react: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tell: Option<String>,
    /// On a message: edit its text (`{form: text}`), delete it, pin or unpin it, choose an option of its decision
    /// (`{form: choice}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choose: Option<String>,
    /// Starts a direct message or a group (`{form: to, text}`: the names, comma-separated, and the first message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// Pauses or resumes the agent of a direct message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause: Option<String>,
    /// The open thread's Following bell: follows the thread (`{form: follow}` true) or stops following it (false);
    /// the thread query's `following` says whether the reader follows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<String>,
}

/// The line under a message: its text and a tone per value of a field (ok: done, attention: needs someone,
/// neutral: waiting); hidden while the text is empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<StatusField>,
}

/// A card under a message, shown while its title is not empty: a band (`Needs you · you decide`) with an aside
/// (`by 11:30`) toned by a field, a title, a text, a footer line and one button whose label is a row field (hidden
/// while empty); the button runs an action on the row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MessageCard {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aside: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<StatusField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// The message box under a conversation. Its action reads `{form: text}` and, with addressees, `{form: to}` (the
/// chosen option's value; the first is the default).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ThreadComposer {
    pub placeholder: Text,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<Opt>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunNeeds {
    /// Record field: the request ("Needs your approval: …"); empty when nothing is needed.
    pub text: String,
    pub label: Text,
    pub action: String,
    /// Record field: the deadline next to the button ("by 11:30").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunLink {
    pub label: Text,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunChanges {
    pub title: Text,
    /// The repeated field of changes and each row's text.
    pub field: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunEarlier {
    pub text: String,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Composer {
    pub placeholder: Text,
    pub action: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Emphasis {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub label: Text,
    /// The action (chain) it runs.
    pub action: String,
    #[serde(default)]
    pub emphasis: Emphasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IconButton {
    pub icon: String,
    /// The accessible label (and the tooltip).
    pub label: Text,
    pub action: String,
    /// A toggle (catalog 3): whether it is on. A press shows the other state at once and runs the action with
    /// `{form: pressed}` (the new state); the pressed state wins until the action has gone through and the value's
    /// query has answered once more; a failed action reverts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<Value>,
    /// The icon and the label while off (the icon and the label above are the pressed ones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_off: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_off: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub items: Vec<MenuItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub label: Text,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub danger: bool,
    /// Starts a new group with this title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    #[serde(default)]
    pub tone: NoticeTone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Box<Node>>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoticeTone {
    #[default]
    Neutral,
    Ok,
    Attention,
    Danger,
}

// ---------------------------------------------------------------------------------------------- catalog 3

/// A head of a screen's column or of a side panel (catalog 3): a leading tile or faces, the title with a glyph, a
/// subtitle line of segments, a faces pill, text pills, icon buttons, a `…` menu, tabs, and the panel's expand and
/// close. On a phone a column's header is the shell's top bar (the title, `crumb_icon`, `phone.sub`, `phone.icons`,
/// the menu with the tabs first); a panel's header is a back bar.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub title: Text,
    /// An icon name before the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_icon: Option<Value>,
    /// An icon the breadcrumb and the phone's top bar put before the screen's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crumb_icon: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading: Option<Leading>,
    /// One line, segments joined with ` · `; a segment with an empty value is left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitle: Vec<Segment>,
    /// The subtitle line is a button running this action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_action: Option<String>,
    /// A muted text after the title (a panel's `# returns`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<HeaderFaces>,
    /// Text pills (at most [`HEADER_MAX_BUTTONS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<HeaderButton>,
    /// `icon_button` nodes (at most [`HEADER_MAX_ICONS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<Node>,
    /// The `…` menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Menu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<HeaderTabs>,
    /// The enclosing `split`'s expand button (desktop).
    #[serde(default, skip_serializing_if = "is_false")]
    pub expand: bool,
    /// The panel's close button (desktop) and back button (phone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<String>,
    /// The phone back button's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<HeaderPhone>,
}

pub const HEADER_MAX_TABS: usize = 8;
pub const HEADER_MAX_BUTTONS: usize = 4;
pub const HEADER_MAX_ICONS: usize = 4;

/// The header's leading: an agent tile (the engine's mark with the glyph) or a stack of faces; `size` in px.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Leading {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<Tile>,
    /// A list of faces (`name`, `avatar`, `agent`, `icon`, `engine`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

/// An agent tile: the glyph and the engine (`claude`, `codex`, `pi`; none: the ink cell). Drawn while `icon` is not
/// empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tile {
    pub icon: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Value>,
}

/// A piece of a subtitle line: exactly one of a text, a dot (`live`, `waiting`), a value with a format
/// (`countdown`, `elapsed`) after an optional `prefix`, and a tone (`live`, `waiting`): the line's colour as a dot
/// gives it, with no dot drawn.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dot: Option<Dot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Dot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dot {
    /// Amber: at work.
    Live,
    /// Hollow, muted: waiting for someone.
    Waiting,
}

/// A faces pill: a list of faces and a label (`9 members`), running an action.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeaderFaces {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    /// The pill's accessible name (absent: `{label}: open`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeaderButton {
    pub label: Text,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

/// The header's tabs, bound to a string state (or param) whose default is one of the ids.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeaderTabs {
    pub bind: Bind,
    pub items: Vec<HeaderTab>,
    /// The tab bar's accessible label (absent: the header's title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
}

/// A tab: selecting it sets the bound value to `id`; with `action` it runs the action instead. `badge` draws
/// ` · N` while above 0.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeaderTab {
    pub id: String,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// A column header on a phone: the line under the top bar's title, actions as top-bar icon buttons (each the
/// action of one of the header's `icons`, or of its `faces`), and the tabs first in the `…` menu.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct HeaderPhone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub menu_tabs: bool,
}

/// A live list of a query's rows (catalog 3), the newest at the bottom by default: older pages by the query's
/// `older`, day dividers, follow mode and jump to the latest, a `tail` query's rows merged after the rows by key, a
/// `head` row, an action when the newest row is seen, an item template per row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    pub query: String,
    /// The row field that identifies an item (pages, the tail, `merge` and `scroll_to` go by it).
    pub key: String,
    pub item: StreamItem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Kinds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<Tail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<StreamHead>,
    #[serde(default, skip_serializing_if = "is_bottom")]
    pub anchor: Anchor,
    #[serde(default, skip_serializing_if = "is_scroll")]
    pub older_load: OlderLoad,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_label: Option<Text>,
    /// A divider before each day (`Today`, `Yesterday`, the date) of this row field's moments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<String>,
    /// A `New` divider after the item with this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<Value>,
    /// The jump button's text when nothing new came (`Assistant is working`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_working: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    /// A muted line above the first item on a phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_intro: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_seen: Option<OnSeen>,
    /// The name composers (`for`) refer to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The list's accessible name (`Messages`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
}

/// The template of a stream's rows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamItem {
    Post(Box<Post>),
}

/// Rows drawn otherwise by the value of a row field: `line` for the rows whose `field` equals `when` (one centred
/// muted line: `author` bold, the text, ` · time`; no avatar, no actions).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Kinds {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<KindLine>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KindLine {
    pub when: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
}

/// Another query of the frame whose rows (of the stream's row message) come after the rows; a tail row whose key
/// is among the rows is dropped. `hold`: a tail row gone from the next tail answer stays until the main query has
/// answered once more.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tail {
    pub query: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold: bool,
}

/// A row before the rows, from the main query's answer (a response field of the row message, drawn with the
/// template but its summary), and a rule under it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StreamHead {
    pub row: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The newest at the bottom, follow mode, jump to the latest.
    #[default]
    Bottom,
    /// A list read from the top.
    Top,
}

fn is_bottom(a: &Anchor) -> bool {
    *a == Anchor::Bottom
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OlderLoad {
    /// Near the top, and a button.
    #[default]
    Scroll,
    /// A button only.
    Button,
}

fn is_scroll(a: &OlderLoad) -> bool {
    *a == OlderLoad::Scroll
}

/// Runs `action` with `{row: <the main query's newest row>}` once the reader is at the end and the newest row
/// changed (`when` read in that row).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OnSeen {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
}

/// The template of an item (catalog 3): who wrote it and when, its text and recording, marks and lines under it, a
/// slot for a run of work, reactions, a summary of what follows from it, and what can be done with it. Every
/// string is a row field (a path; `who.name` through a query's `join`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Post {
    pub author: String,
    pub time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<PostAvatar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<PostBadge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<PostText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<PostAudio>,
    /// A bool field: `(edited)` after the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<PostPinned>,
    /// A bool field: the viewer's own row (no Reply to it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<PostReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<PostRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<PostCorrection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactions: Option<PostReactions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<PostSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<Process>,
    /// The rail on hover (desktop), the `…` menu, the phone's long-press sheet: entries in the order drawn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hover: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheet: Vec<Entry>,
    /// The inline editor of the text: its action reads `{form: text}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<PostEdit>,
    /// While false: no rail, no menu, no sheet, no inline correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions_when: Option<Cond>,
    /// The identity card a mention, the lead or the name opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<PostCard>,
}

/// The lead: the agent tile while `tile.when` holds (default: `agent` true or `icon` not empty), else the
/// picture `url`, else the author's initials.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PostAvatar {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<PostTile>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PostTile {
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostBadge {
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
}

/// The text: Markdown (`writing` true adds the caret), or plain with `@[Name]`/`#[x]` chips and the words lit while
/// the recording plays.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PostText {
    pub field: String,
    #[serde(default, skip_serializing_if = "is_markdown")]
    pub format: TextFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    #[default]
    Markdown,
    Plain,
}

fn is_markdown(f: &TextFormat) -> bool {
    *f == TextFormat::Markdown
}

/// A recording: its duration (s), label, URL (a data URL) and timed words (`word`, `start`, `end`; evenly timed
/// when absent).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PostAudio {
    pub duration: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<String>,
}

/// A bool field; `mark` true: `Pinned` above the head of a pinned row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostPinned {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<Value>,
}

/// A line under the text with the tone's icon (ok ✓, attention ⚡, neutral 👁); drawn while `route.to` is empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostReceipt {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<StatusField>,
}

/// `→ <to> · <status>`, with `why` as its tooltip; drawn while `to` is not empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostRoute {
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A correction the writer may make: inline for `window_ms` after the row's moment `from` (a draining button with
/// `lead`, the `icon` — `undo`, `arrow-right` — the `label` and the seconds left), then an entry of the menu and the
/// sheet (`menu_label`, else `label`). Drawn while `label` is not empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostCorrection {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub action: String,
    pub from: String,
    #[serde(default = "thirty_seconds")]
    pub window_ms: u32,
}

fn thirty_seconds() -> u32 {
    30_000
}

pub const CORRECTION_MAX_WINDOW_MS: u32 = 120_000;

/// Chips `emoji count` (pressed when `mine`); a click runs `action` with `{form: emoji}`; the hover's React offers
/// the `quick` emoji.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostReactions {
    pub field: String,
    pub emoji: String,
    pub count: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<String>,
}

pub const POST_MAX_QUICK: usize = 8;

/// What follows from the item (replies): faces, `label` (`{count} replies`), `working` with a live dot and the time
/// since `since`, `waiting` with a hollow dot, else `last`; the whole runs `action`. Drawn while `count` > 0 or
/// `working` or `waiting` is not empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostSummary {
    pub count: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<String>,
}

/// An entry of a post's hover rail, menu or sheet: a built-in gesture (`react`, `reply`, `copy_text`, `copy_link`,
/// `play`, `edit`, `correction`, `more`, `separator`) or an action with its label.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<EntryStyle>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub danger: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

/// The built-in entries of a post.
pub const POST_BUILTINS: &[&str] = &[
    "react",
    "reply",
    "copy_text",
    "copy_link",
    "play",
    "edit",
    "correction",
    "more",
    "separator",
];
pub const POST_MAX_ENTRIES: usize = 12;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryStyle {
    /// An icon button (the label is its tooltip).
    Icon,
    /// A text button.
    Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PostEdit {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
}

/// The identity card: a name looked up (case-insensitive) in `query`'s rows (its `rows`): `avatar`, `agent`, `icon`,
/// `engine`, `detail`; a button runs `action` with `{form: to}` (the name); `unknown` when the name is not there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PostCard {
    pub query: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Text>,
}

/// A block of a run of work (catalog 3): live (`starting`, `working`), done, or ended (`stopped`, `failed`,
/// `timed_out`, `gone`); no block while `state` is empty. Its rows (`kind`: `tool`, `note`, `taken`; `title`,
/// `detail`, `state`, `time`, `at`, `calls`, `tools`) and their calls (`call`, `detail`, `state`, `ms`, `input`,
/// `output`, `stderr`, `exit`, `diff`, `truncated`) have these field names. As a `post`'s `process` the paths are
/// row fields; standalone they are fields of `query`'s answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Process {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    /// A moment in ms the live block counts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    /// The number of steps (`View N steps`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<String>,
    /// The text being written (with a caret).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saying: Option<String>,
    /// Retry on an ended block: its action, a field with its label, a condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<ProcessAction>,
    /// `Open full log`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<ProcessAction>,
    /// How a truncated call is read whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_call: Option<FetchCall>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessAction {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
}

/// A query of the frame read once more with `args` and the request field `call` set to the opened call's `call`
/// id; the answer's `field` replaces the call when its `call` is not empty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FetchCall {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    pub call: String,
    pub field: String,
}

/// A pinned list of live items (catalog 3): a tile, the title, a dot (live, waiting), `Starting` or the time since
/// `since`, a sub line (and for a waiting item `countdown_text` with the time left until `until`), a jump to the
/// item's place in a stream and buttons. Hidden while there are no rows. `rows`: a repeated field of the answer
/// other than the query's rows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    pub key: String,
    pub title: String,
    /// Above the items (`2 live runs`); none while empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<PostTile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StripState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// After a waiting item's sub line, with `{left}` the countdown (`ends in {left}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub countdown_text: Option<Text>,
    /// The tile and the title run it (row context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<StripJump>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<StripButton>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<Text>,
}

/// Which values of a field are live, waiting, starting.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct StripState {
    pub field: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starting: Vec<String>,
}

/// The jump button: `scroll_to` the key in the row field `field`; its label by state.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StripJump {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_waiting: Option<Text>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StripButton {
    pub label: Text,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StripStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StripStyle {
    /// The red-outlined Stop with the filled square.
    Stop,
    /// A muted text button.
    Text,
}

/// A message box (catalog 3): `send` runs with `{form: text, audio, duration, language, words, reply}` and each of
/// the `options` (a bool); mentions over a directory, tags, dictation, a reply reference, caption lines, Send turned
/// Stop, a read-only reason.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ComposerNode {
    pub send: String,
    /// The `id` of the stream it belongs to (jumps to its end on send; `draft` and `reply_to` from its items).
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Text>,
    /// Owns ⌥ Space and the steps of rows outside a stream; one per frame.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
    /// The panel's box.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact: bool,
    /// `add`, `emoji`, `attach`; none when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ComposerTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Mentions>,
    /// `#word` becomes `#[word]` on send.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tags: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictation: Option<Dictation>,
    /// Takes `reply_to`; the form's `reply` is the key.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reply: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ComposerOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caption: Vec<Caption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<ComposerStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readonly: Option<Readonly>,
    /// A string state the text is mirrored into, for queries that read what is typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_state: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComposerTool {
    Add,
    Emoji,
    Attach,
}

/// The mention picker's directory: rows of a query (its `rows`, or the repeated field `rows`), in the order given,
/// grouped by `section` on a desktop, with a `note` in the footer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Mentions {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Text>,
}

/// The Dictate button, shown when the workspace has a speech connection; `vocabulary`: words for the recogniser
/// besides the mentionable names.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Dictation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<Text>,
}

/// A checkbox under the box: `{form: <name>}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComposerOption {
    pub name: String,
    pub label: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Cond>,
}

/// A caption line: the first with a value not empty is drawn.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Caption {
    pub value: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<CaptionStyle>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptionStyle {
    /// Who will answer (with the icon).
    Route,
    /// A plain hint.
    Dot,
}

/// While `when` holds and the box is empty, Send is a labelled Stop running `action` (and Esc runs it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComposerStop {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Cond>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    /// The button's accessible name (absent: its label): `Stop the agent` beside a visible `Stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<Text>,
}

/// While `when` holds, the reason stands where the box would be.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Readonly {
    pub when: Cond,
    pub reason: Text,
}

impl Kind {
    /// The primitive's name in the document (`table`, `icon_button`).
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Stack(_) => "stack",
            Kind::Columns(_) => "columns",
            Kind::Section(_) => "section",
            Kind::Card(_) => "card",
            Kind::Tabs(_) => "tabs",
            Kind::Divider {} => "divider",
            Kind::Split(_) => "split",
            Kind::Board(_) => "board",
            Kind::Segmented(_) => "segmented",
            Kind::Link(_) => "link",
            Kind::Breadcrumbs(_) => "breadcrumbs",
            Kind::Table(_) => "table",
            Kind::List(_) => "list",
            Kind::Fields(_) => "fields",
            Kind::Metric(_) => "metric",
            Kind::Chart(_) => "chart",
            Kind::Viz(_) => "viz",
            Kind::Timeline(_) => "timeline",
            Kind::Status(_) => "status",
            Kind::Tag(_) => "tag",
            Kind::Avatar(_) => "avatar",
            Kind::AvatarStack(_) => "avatar_stack",
            Kind::Meter(_) => "meter",
            Kind::Progress(_) => "progress",
            Kind::Text(_) => "text",
            Kind::CopyField(_) => "copy_field",
            Kind::Empty(_) => "empty",
            Kind::Form(_) => "form",
            Kind::FilterBar(_) => "filter_bar",
            Kind::Button(_) => "button",
            Kind::IconButton(_) => "icon_button",
            Kind::Menu(_) => "menu",
            Kind::Notice(_) => "notice",
            Kind::RecordHeader(_) => "record_header",
            Kind::AgentRun(_) => "agent_run",
            Kind::Conversation(_) => "conversation",
            Kind::Chat(_) => "chat",
            Kind::Header(_) => "header",
            Kind::Stream(_) => "stream",
            Kind::Process(_) => "process",
            Kind::Strip(_) => "strip",
            Kind::Composer(_) => "composer",
        }
    }

    /// Interactive primitives: an older client that cannot draw one must not show the screen (fail closed).
    pub fn interactive(&self) -> bool {
        matches!(
            self,
            Kind::Form(_)
                | Kind::FilterBar(_)
                | Kind::Button(_)
                | Kind::IconButton(_)
                | Kind::Menu(_)
                | Kind::Segmented(_)
                | Kind::Tabs(_)
                | Kind::Link(_)
                | Kind::RecordHeader(_)
                | Kind::AgentRun(_)
                | Kind::Conversation(_)
                | Kind::Chat(_)
                | Kind::Header(_)
                | Kind::Stream(_)
                | Kind::Process(_)
                | Kind::Strip(_)
                | Kind::Composer(_)
        )
    }

    /// The catalog revision the primitive needs: the one it appeared in, or that of the newest option it uses.
    pub fn revision(&self) -> u32 {
        let new_format = |f: &Option<Format>| {
            matches!(
                f,
                Some(Format::Elapsed {} | Format::Countdown {} | Format::Span {})
            )
        };
        match self {
            Kind::Header(_)
            | Kind::Stream(_)
            | Kind::Process(_)
            | Kind::Strip(_)
            | Kind::Composer(_) => 3,
            Kind::Split(s) if s.fill || s.resize.is_some() || s.expand.is_some() => 3,
            Kind::List(l)
                if l.search.is_some()
                    || l.online.is_some()
                    || l.limit.is_some()
                    || l.group_label.is_some() =>
            {
                3
            }
            Kind::IconButton(b) if b.pressed.is_some() => 3,
            Kind::AgentRun(r) if r.log => 3,
            Kind::Text(t) if new_format(&t.format) => 3,
            Kind::Metric(m) if new_format(&m.format) => 3,
            Kind::Form(f)
                if !f.notes.is_empty()
                    || f.sections
                        .iter()
                        .flat_map(|s| &s.inputs)
                        .any(|i| i.control == Control::Pick) =>
            {
                3
            }
            Kind::Viz(_) | Kind::Board(_) => 2,
            _ => 1,
        }
    }
}

impl Step {
    pub fn name(&self) -> &'static str {
        match self {
            Step::Draft { .. } => "draft",
            Step::ReplyTo { .. } => "reply_to",
            Step::ScrollTo { .. } => "scroll_to",
            Step::CopyValue(_) => "copy",
            Step::Merge { .. } => "merge",
            Step::Call(_) => "call",
            Step::Confirm(_) => "confirm",
            Step::Open(_) => "open",
            Step::Close {} => "close",
            Step::Navigate(_) => "navigate",
            Step::Back {} => "back",
            Step::Set(_) => "set",
            Step::Refresh { .. } => "refresh",
            Step::Toast(_) => "toast",
            Step::OpenUrl { .. } => "open_url",
        }
    }

    /// The catalog revision the step kind appeared in.
    pub fn revision(&self) -> u32 {
        match self {
            Step::Draft { .. }
            | Step::ReplyTo { .. }
            | Step::ScrollTo { .. }
            | Step::CopyValue(_)
            | Step::Merge { .. } => 3,
            _ => 1,
        }
    }
}

// ---------------------------------------------------------------------------------------------- conditions

/// The normative reading of a condition, which every client implements the same way. `resolve` answers a value's
/// JSON, `None` when it is absent (a missing field, null).
///
/// - `eq`, `ne`, `in` compare like values: numbers by value when either side is a number (an int64 travels as a
///   string), objects and arrays by their JSON, anything else as text, case-insensitively. **An absent value equals
///   the protobuf default** (catalog 3): it is equal to another absent value and to `""`, `0` (or a numeric string
///   of zero) and `false`, since a view that omits empty fields sends nothing for them.
/// - `empty` holds for an absent value, `""` and `[]`.
/// - `gt`, `lt` compare numbers (else text); an absent side makes them false.
pub fn holds(c: &Cond, resolve: &dyn Fn(&Value) -> Option<Json>) -> bool {
    match c {
        Cond::Eq(a, b) => same(resolve(a).as_ref(), resolve(b).as_ref()),
        Cond::Ne(a, b) => !same(resolve(a).as_ref(), resolve(b).as_ref()),
        Cond::In { value, set } => {
            let v = resolve(value);
            set.iter().any(|x| same(v.as_ref(), Some(x)))
        }
        Cond::Empty(v) => match resolve(v) {
            None | Some(Json::Null) => true,
            Some(Json::String(s)) => s.is_empty(),
            Some(Json::Array(a)) => a.is_empty(),
            Some(_) => false,
        },
        Cond::Gt(a, b) | Cond::Lt(a, b) => {
            let (Some(x), Some(y)) = (present(resolve(a)), present(resolve(b))) else {
                return false;
            };
            let ord = match (number(&x), number(&y)) {
                (Some(m), Some(n)) => m.partial_cmp(&n),
                _ => Some(text_of(&x).cmp(&text_of(&y))),
            };
            match c {
                Cond::Gt(..) => ord == Some(std::cmp::Ordering::Greater),
                _ => ord == Some(std::cmp::Ordering::Less),
            }
        }
        Cond::And(cs) => cs.iter().all(|c| holds(c, resolve)),
        Cond::Or(cs) => cs.iter().any(|c| holds(c, resolve)),
        Cond::Not(c) => !holds(c, resolve),
    }
}

fn present(v: Option<Json>) -> Option<Json> {
    v.filter(|v| !v.is_null())
}

/// The protobuf default a view leaves out: `""`, `0`, `false`.
fn is_default(v: &Json) -> bool {
    match v {
        Json::Bool(b) => !b,
        Json::String(s) => s.is_empty() || s.trim().parse::<f64>().is_ok_and(|n| n == 0.0),
        Json::Number(n) => n.as_f64() == Some(0.0),
        _ => false,
    }
}

fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => n.as_f64(),
        Json::String(s) if !s.trim().is_empty() => {
            s.trim().parse::<f64>().ok().filter(|n| n.is_finite())
        }
        _ => None,
    }
}

fn text_of(v: &Json) -> String {
    match v {
        Json::String(s) => s.to_lowercase(),
        v => v.to_string().to_lowercase(),
    }
}

fn same(a: Option<&Json>, b: Option<&Json>) -> bool {
    let a = a.filter(|v| !v.is_null());
    let b = b.filter(|v| !v.is_null());
    match (a, b) {
        (None, None) => true,
        (None, Some(x)) | (Some(x), None) => is_default(x),
        (Some(x), Some(y)) => {
            if (x.is_number() || y.is_number())
                && let (Some(m), Some(n)) = (number(x), number(y))
            {
                return m == n;
            }
            if x.is_object() || x.is_array() || y.is_object() || y.is_array() {
                return x == y;
            }
            text_of(x) == text_of(y)
        }
    }
}
