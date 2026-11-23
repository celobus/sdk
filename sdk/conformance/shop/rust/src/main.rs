//! The SDK conformance plugin `shop`: a UI with a list screen (a compact row view, a count), a record screen
//! (breadcrumbs, a record header, a titled card), a report screen of charts and figures on a `board` (`viz`: every
//! bind source, a selection, a link, a fallback) under a range switch (a `segmented` over a param with a default), a document-level
//! dialog form (a note, an alternate button), a sidebar group with a live alert badge, two commands (one opens the
//! report with args), a palette search source with an action and topics; an activity screen of catalog 3 (a fill
//! split with a header, a stream of note posts with a packing job's process, a strip of live jobs, a composer, a
//! side panel with a toggle and a searchable list) and an assign dialog with a pick input. Its `--describe` must equal
//! `../descriptor.json`, as the TypeScript and C# versions' must.

use celobus_sdk::prelude::*;
use celobus_sdk::ui::*;

pub const CLERK: Role = Role::new("clerk").title("Order clerk");

#[derive(Message, Clone, Debug, Default, PartialEq)]
pub enum OrderStatus {
    #[default]
    Placed,
    Paid,
    Cancelled,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Order {
    pub id: Uuid,
    pub number: String,
    pub customer: String,
    pub email: String,
    pub total: Decimal,
    pub currency: String,
    pub status: OrderStatus,
    pub placed_at: Timestamp,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ListOrdersRequest {
    pub status: Option<OrderStatus>,
    pub text: String,
    pub page_token: String,
    pub page_size: i32,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ListOrdersResponse {
    pub orders: Vec<Order>,
    pub next_page_token: String,
    pub total: i64,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetOrderRequest {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct CancelOrderRequest {
    pub id: Uuid,
    pub reason: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct CountOpenRequest {}

#[derive(Message, Clone, Debug, Default)]
pub struct CountResponse {
    pub count: i64,
}

#[derive(Message, Clone, Debug, Default)]
pub struct DaySales {
    pub day: Date,
    pub channel: String,
    pub total: Decimal,
    pub online: Decimal,
    pub retail: Decimal,
}

#[derive(Message, Clone, Debug, Default)]
pub struct SalesRequest {
    pub channel: String,
    pub range: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct SalesResponse {
    pub days: Vec<DaySales>,
    pub revenue: Decimal,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Product {
    pub name: String,
    pub revenue: Decimal,
    pub units: i32,
}

#[derive(Message, Clone, Debug, Default)]
pub struct TopProductsRequest {}

#[derive(Message, Clone, Debug, Default)]
pub struct TopProductsResponse {
    pub products: Vec<Product>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct OrderKey {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AllOrders {}

#[derive(Message, Clone, Debug, Default)]
pub struct OrdersParams {
    pub status: Option<OrderStatus>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct OrderParams {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct CancelParams {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ReportParams {
    pub product: Option<String>,
    pub range: Option<String>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub agent: bool,
    pub icon: String,
    pub engine: String,
    pub role: String,
    pub online: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Reaction {
    pub emoji: String,
    pub count: i32,
    pub mine: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct JobStep {
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Job {
    pub state: String,
    pub label: String,
    pub meta: String,
    pub started: i64,
    pub rows: Vec<JobStep>,
    pub steps: i32,
    pub retry_label: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Replies {
    pub count: i32,
    pub last: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Note {
    pub id: String,
    pub by: String,
    pub time: String,
    pub text: String,
    pub kind: String,
    pub edited: bool,
    pub mine: bool,
    pub pinned: bool,
    pub receipt: String,
    pub receipt_tone: String,
    pub reactions: Vec<Reaction>,
    pub replies: Option<Replies>,
    pub job: Option<Job>,
    pub job_id: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Packing {
    pub job: String,
    pub agent: String,
    pub icon: String,
    pub engine: String,
    pub state: String,
    pub since: i64,
    pub until: i64,
    pub sub: String,
    pub note: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetActivityRequest {
    pub order: String,
    pub page_token: String,
    pub tab: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ActivityView {
    pub title: String,
    pub status: String,
    pub ends: i64,
    pub notes: Vec<Note>,
    pub people: Vec<Person>,
    pub older_page_token: String,
    pub pinned: i32,
    pub closed: bool,
    pub placeholder: String,
    pub watching: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetLiveRequest {
    pub order: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct LiveView {
    pub notes: Vec<Note>,
    pub people: Vec<Person>,
    pub jobs: Vec<Packing>,
    pub label: String,
    pub hint: String,
    pub can_stop: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetWatchersRequest {
    pub order: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct WatchersView {
    pub people: Vec<Person>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct PreviewNoteRequest {
    pub order: String,
    pub text: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct PreviewView {
    pub hint: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetJobRequest {
    pub id: String,
    pub call: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct JobView {
    pub title: String,
    pub call: Option<JobStep>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AddNoteRequest {
    pub order: String,
    pub text: String,
    pub reply: String,
    pub urgent: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AddNoteResponse {
    pub note: Option<Note>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct NoteActionRequest {
    pub note: String,
    pub action: String,
    pub text: String,
    pub emoji: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct WatchRequest {
    pub order: String,
    pub watch: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AssignRequest {
    pub order: String,
    pub people: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Done {
    pub ok: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ActivityParams {
    pub order: Option<String>,
    pub job: Option<String>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AssignParams {
    pub order: Option<String>,
}

pub const ORDERS: Topic<AllOrders> = Topic::new("orders");
pub const ORDER: Topic<OrderKey> = Topic::new("order");
pub const ORDERS_SCREEN: ScreenKey<OrdersParams> = ScreenKey::new("orders");
pub const ORDER_SCREEN: ScreenKey<OrderParams> = ScreenKey::new("order");
pub const REPORT_SCREEN: ScreenKey<ReportParams> = ScreenKey::new("report");
pub const CANCEL: OverlayKey<CancelParams> = OverlayKey::new("cancel_form");
pub const ACTIVITY_SCREEN: ScreenKey<ActivityParams> = ScreenKey::new("activity");
pub const ASSIGN: OverlayKey<AssignParams> = OverlayKey::new("assign");

async fn list_orders(_: Ctx, _: ListOrdersRequest) -> Result<ListOrdersResponse> {
    Ok(ListOrdersResponse::default())
}
async fn get_order(_: Ctx, _: GetOrderRequest) -> Result<Order> {
    Ok(Order::default())
}
async fn cancel_order(_: Ctx, _: CancelOrderRequest) -> Result<Order> {
    Ok(Order::default())
}
async fn count_open(_: Ctx, _: CountOpenRequest) -> Result<CountResponse> {
    Ok(CountResponse::default())
}
async fn sales(_: Ctx, _: SalesRequest) -> Result<SalesResponse> {
    Ok(SalesResponse::default())
}
async fn top_products(_: Ctx, _: TopProductsRequest) -> Result<TopProductsResponse> {
    Ok(TopProductsResponse::default())
}

async fn get_activity(_: Ctx, _: GetActivityRequest) -> Result<ActivityView> {
    Ok(ActivityView::default())
}
async fn get_live(_: Ctx, _: GetLiveRequest) -> Result<LiveView> {
    Ok(LiveView::default())
}
async fn get_watchers(_: Ctx, _: GetWatchersRequest) -> Result<WatchersView> {
    Ok(WatchersView::default())
}
async fn preview_note(_: Ctx, _: PreviewNoteRequest) -> Result<PreviewView> {
    Ok(PreviewView::default())
}
async fn get_job(_: Ctx, _: GetJobRequest) -> Result<JobView> {
    Ok(JobView::default())
}
async fn add_note(_: Ctx, _: AddNoteRequest) -> Result<AddNoteResponse> {
    Ok(AddNoteResponse::default())
}
async fn note_action(_: Ctx, _: NoteActionRequest) -> Result<Done> {
    Ok(Done::default())
}
async fn watch(_: Ctx, _: WatchRequest) -> Result<Done> {
    Ok(Done::default())
}
async fn assign(_: Ctx, _: AssignRequest) -> Result<Done> {
    Ok(Done::default())
}

fn orders_screen() -> Screen {
    Screen::build(ORDERS_SCREEN, |s| {
        let status = s.param(OrdersParams::status);
        let orders = s.query(
            "orders",
            Query::call(list_orders)
                .arg(ListOrdersRequest::status, status)
                .rows(ListOrdersResponse::orders)
                .paged()
                .follow(ORDERS),
        );
        let open = s.row_action("open", &orders, |a, r| {
            a.navigate(ORDER_SCREEN, |p| p.arg(OrderParams::id, r.get(Order::id)))
        });
        let cancel = s.row_action("cancel", &orders, |a, r| {
            a.confirm(
                Confirm::danger("Cancel this order?")
                    .text("The customer is told.")
                    .cancel_label("Keep it")
                    .summary("receipt", r.get(Order::number)),
            )
            .call(cancel_order, |c| {
                c.arg(CancelOrderRequest::id, r.get(Order::id))
            })
            .toast(t("orders.cancelled", "Order cancelled"))
            .described(r.get(Order::customer))
            .with_button("Open", &open)
            .on_error(|e| e.toast_error())
        });
        s.title(t("orders.title", "Orders"));
        stack([
            filter_bar([status.select("Status")]),
            table(&orders)
                .mono(Order::number, "Order")
                .col(Order::customer, "Customer")
                .with_avatar(Order::customer)
                .second_line(Order::email)
                .money(Order::total, "Total", Order::currency)
                .status(
                    Order::status,
                    "Status",
                    [
                        (OrderStatus::Placed, Tone::Attention),
                        (OrderStatus::Paid, Tone::Ok),
                    ],
                )
                .relative(Order::placed_at, "Placed")
                .open(&open)
                .count(orders.field(ListOrdersResponse::total))
                .count_label(
                    t("orders.count", "{total} orders · newest first")
                        .arg("total", orders.field(ListOrdersResponse::total)),
                )
                .compact(|r| {
                    row_view(r.get(Order::customer))
                        .subtitle(r.get(Order::number))
                        .meta(r.get(Order::total))
                        .money(Order::currency)
                        .status(Order::status, [(OrderStatus::Paid, Tone::Ok)])
                })
                .row_button(|r| {
                    button("Cancel", &cancel)
                        .danger()
                        .visible(r.get(Order::status).eq(OrderStatus::Placed))
                })
                .into(),
        ])
    })
}

fn order_screen() -> Screen {
    Screen::build(ORDER_SCREEN, |s| {
        let id = s.param(OrderParams::id);
        let order = s.query(
            "order",
            Query::call(get_order)
                .arg(GetOrderRequest::id, id)
                .follow(ORDER.key(|k| k.set(OrderKey::id, id))),
        );
        let edit = s.action("cancel", |a| {
            a.open(CANCEL, |p| p.arg(CancelParams::id, id))
        });
        let back = s.action("back", |a| a.back());
        let remind = s.action("remind", |a| a.toast("Reminder sent"));
        s.title(order.field(Order::number));
        stack([
            breadcrumbs([
                crumb_to("Orders", ORDERS_SCREEN, |a| a),
                crumb_to(t("order.same_status", "Same status"), ORDERS_SCREEN, |a| {
                    a.arg(OrdersParams::status, order.field(Order::status))
                }),
                crumb(order.field(Order::number)),
            ]),
            record_header(order.field(Order::number))
                .subtitle(order.field(Order::customer))
                .tags([status(
                    order.field(Order::status),
                    [(OrderStatus::Paid, Tone::Ok)],
                )])
                .actions([button("Cancel order", &edit)
                    .danger()
                    .visible(order.field(Order::status).eq(OrderStatus::Placed))])
                .menu([menu_item("Send a reminder", &remind)])
                .close(&back)
                .into(),
            columns([
                fields([
                    item("Customer", order.field(Order::customer)),
                    item("Placed", order.field(Order::placed_at)).datetime(),
                ]),
                titled_card(
                    "Payment",
                    [status(
                        order.field(Order::status),
                        [(OrderStatus::Paid, Tone::Ok)],
                    )],
                )
                .trailing(order.field(Order::total))
                .money_in("USD"),
            ])
            .ratios([2, 1]),
        ])
    })
}

/// Charts and figures on a board: a line over dates, a ranked bar that selects, a metric, a pivot and several
/// series; over it the range switch, bound to a param with a default.
fn report_screen() -> Screen {
    Screen::build(REPORT_SCREEN, |s| {
        let product = s.param(ReportParams::product);
        let range = s.param_or(ReportParams::range, "30");
        let channel = s.state("channel", String::new());
        let by_day = s.query(
            "sales",
            Query::call(sales)
                .arg(SalesRequest::channel, &channel)
                .arg(SalesRequest::range, range)
                .rows(SalesResponse::days),
        );
        let top = s.query(
            "top",
            Query::call(top_products).rows(TopProductsResponse::products),
        );
        s.title("Report");
        let charts = board()
            .span(
                8,
                viz::line::line()
                    .title(t("report.revenue", "Revenue"))
                    .range("Last 30 days")
                    .options(json!({ "fmt": "money", "curve": "smooth" }))
                    .dates(&by_day, DaySales::day)
                    .values(&by_day, DaySales::total)
                    .height(280)
                    .open(ORDERS_SCREEN, |a| {
                        a.arg(OrdersParams::status, OrderStatus::Paid)
                    })
                    .fallback(
                        table(&by_day)
                            .date(DaySales::day, "Day")
                            .col(DaySales::total, "Revenue"),
                    ),
            )
            .third(
                viz::bar::bar()
                    .title("Top products")
                    .rows(
                        &top,
                        (
                            ("name", Product::name),
                            ("value", Product::revenue),
                            ("units", Product::units),
                        ),
                    )
                    .text_keys(["name"])
                    .option("fmt", "money")
                    .option("limit", 5)
                    .option("field", "product")
                    .select_param(&product),
            )
            .quarter(
                viz::figure::metric()
                    .title("Revenue")
                    .value("value", by_day.field(SalesResponse::revenue))
                    .value("target", 120000)
                    .value("caption", "this month")
                    .value("filter.channel", &channel),
            )
            .span(
                9,
                viz::bar::column()
                    .title("By channel")
                    .pivot(&by_day, DaySales::day, DaySales::channel, DaySales::total)
                    .as_date()
                    .option("stacked", true)
                    .select_state(&channel),
            )
            .full(heading("Channels"))
            .full(
                viz("line", "multiples")
                    .labels(&by_day, DaySales::channel)
                    .series(
                        &by_day,
                        (
                            (DaySales::online, "Online"),
                            (DaySales::retail, t("report.retail", "Retail")),
                        ),
                    )
                    .serie(&by_day, DaySales::total, "Total")
                    .column("notes", &by_day, DaySales::channel)
                    .as_text()
                    .rows(&top, [("name", Product::name)])
                    .to("parts")
                    .visible(product.val().empty()),
            )
            .fallback(table(&top).col(Product::name, "Product"));
        stack([
            segmented(
                range,
                [
                    ("7", t("report.week", "7 days")),
                    ("30", t("report.month", "30 days")),
                ],
            ),
            charts,
        ])
    })
}

/// The cancel dialog, document-level: the order screen and the palette's search action open it.
fn cancel_dialog() -> Overlay {
    Overlay::dialog(CANCEL, |o| {
        let oid = o.param(CancelParams::id);
        o.title("Cancel order");
        o.form(
            cancel_order,
            |f| {
                f.textarea(CancelOrderRequest::reason, "Why").required();
                f.cancel("Keep it");
                f.note("The customer gets an email.");
                f.alternate("Cancel quietly", |c| {
                    c.call_form(cancel_order, |c| c.arg(CancelOrderRequest::id, oid))
                        .close()
                });
            },
            "Cancel order",
            |a, _v| {
                a.call_form(cancel_order, |c| c.arg(CancelOrderRequest::id, oid))
                    .toast("Order cancelled")
                    .close()
            },
        )
    })
}

/// The activity of an order (catalog 3): notes and events read from the bottom, a packing job's work on a note, the
/// live jobs, a box to write a note, and a side panel of watchers.
fn activity_screen() -> Screen {
    Screen::build(ACTIVITY_SCREEN, |s| {
        let order = s.param(ActivityParams::order);
        let job = s.param(ActivityParams::job);
        let tab = s.state_reset("tab", "notes".to_string(), &[order.name()]);
        let wide = s.state_reset("wide", false, &[order.name()]);
        let draft = s.state("draft", String::new());
        let who = Via::<Note, Person>::new("who");
        let notes = s.query(
            "notes",
            Query::call(get_activity)
                .arg(GetActivityRequest::order, order.or_empty())
                .rows(ActivityView::notes)
                .older(
                    ActivityView::older_page_token,
                    GetActivityRequest::page_token,
                )
                .join(&who, ActivityView::people, Person::id, Note::by)
                .follow(ORDERS),
        );
        let live = s.query(
            "live",
            Query::call(get_live)
                .arg(GetLiveRequest::order, order.or_empty())
                .rows(LiveView::notes)
                .join(&who, LiveView::people, Person::id, Note::by),
        );
        let people = s.query(
            "people",
            Query::call(get_watchers)
                .arg(GetWatchersRequest::order, order.or_empty())
                .rows(WatchersView::people),
        );
        let hint = s.query(
            "hint",
            Query::call(preview_note)
                .arg(PreviewNoteRequest::order, order.or_empty())
                .arg(PreviewNoteRequest::text, draft.val())
                .debounce_ms(300)
                .when(not(draft.val().empty())),
        );
        let job_q = s.query(
            "job",
            Query::call(get_job)
                .arg(GetJobRequest::id, job.or_empty())
                .when(not(job.val().empty())),
        );
        let add = s.action("add", |a| {
            a.call(add_note, |c| {
                c.arg(AddNoteRequest::order, order.or_empty())
                    .arg(AddNoteRequest::text, form::text())
                    .arg(AddNoteRequest::reply, form::reply())
                    .arg(AddNoteRequest::urgent, form::option("urgent"))
            })
            .merge(&notes, result(AddNoteResponse::note))
        });
        let on_note = |s: &mut S<ActivityParams>, name: &str, form_value: Option<Val<String>>| {
            s.row_action(name, &notes, |a, r| {
                a.call(note_action, |c| {
                    c.arg(NoteActionRequest::note, r.get(Note::id))
                        .arg(NoteActionRequest::action, name);
                    match &form_value {
                        Some(v) if name == "react" => c.arg(NoteActionRequest::emoji, v.clone()),
                        Some(v) => c.arg(NoteActionRequest::text, v.clone()),
                        None => c,
                    }
                })
            })
        };
        let seen = on_note(s, "seen", None);
        let react = on_note(s, "react", Some(form::emoji()));
        let edit = on_note(s, "edit", Some(form::text()));
        let retry = on_note(s, "retry", None);
        let open_job = s.row_action("open_job", &notes, |a, r| {
            a.set_param(job, r.get(Note::job_id))
        });
        let close_job = s.action("close_job", |a| a.set_param(job, none()));
        let quote = s.row_action("quote", &notes, |a, r| {
            a.draft(
                t("activity.quote", "About {who}'s note: ").arg("who", r.at(who.f(Person::name))),
            )
        });
        let answer = s.row_action("answer", &notes, |a, r| {
            a.reply_to(r.at(who.f(Person::name)), r.get(Note::id))
        });
        let copy_link = s.action("copy_link", |a| a.copy_link(order.val()));
        let copy_title = s.action("copy_title", |a| a.copy(notes.field(ActivityView::title)));
        let toggle_watch = s.action("watch", |a| {
            a.call(watch, |c| {
                c.arg(WatchRequest::order, order.or_empty())
                    .arg(WatchRequest::watch, form::pressed())
            })
        });
        let mention = s.action("mention", |a| {
            a.toast(t("activity.mention", "Write to {to}").arg("to", form::to()))
        });
        let stop = s.action("stop", |a| a.toast("Stopping"));
        let stop_job = s.action("stop_job", |a| a.toast("Stopping the job"));
        let jump = s.action("jump", |a| a.scroll_to(live.field(LiveView::label)));
        let assign_a = s.action("assign", |a| {
            a.open(ASSIGN, |p| p.arg(AssignParams::order, order.or_empty()))
        });
        s.title(notes.field(ActivityView::title));
        let note = post::<Note>()
            .author(who.f(Person::name))
            .time(Note::time)
            .avatar(who.f(Person::avatar))
            .tile(who.f(Person::icon), who.f(Person::engine))
            .badge("AGENT", row_at(who.f(Person::agent)).eq(true))
            .markdown(Note::text)
            .edited(Note::edited)
            .pinned(Note::pinned, lit(true))
            .mine(Note::mine)
            .receipt(Note::receipt, Note::receipt_tone, [("done", Tone::Ok)])
            .reactions(Note::reactions, Reaction::emoji, Reaction::count, &react)
            .reactions_mine(Reaction::mine)
            .quick(["👍", "✅"])
            .summary(Note::replies.then(Replies::count), &open_job)
            .summary_label(
                t(
                    "activity.replies",
                    "{n, plural, one {# reply} other {# replies}}",
                )
                .arg("n", row_at(Note::replies.then(Replies::count))),
            )
            .process(Note::job, |w: ProcessB<Job>| {
                w.state(Job::state)
                    .label(Job::label)
                    .meta(Job::meta)
                    .started(Job::started)
                    .rows(Job::rows)
                    .steps(Job::steps)
                    .retry(&retry, Job::retry_label, row_value(Note::mine).eq(true))
                    .log(&open_job)
                    .fetch_call(&job_q, GetJobRequest::call, JobView::call, |a| {
                        a.arg(GetJobRequest::id, row_value(Note::job_id))
                    })
            })
            .hover([
                builtin::react(),
                builtin::reply(),
                entry(&quote, "Quote").text_style(),
                builtin::more(),
            ])
            .menu([
                builtin::copy_text(),
                builtin::copy_link(),
                builtin::edit(),
                builtin::separator(),
                entry(&answer, "Answer").icon("reply").danger(),
            ])
            .sheet([builtin::reply(), builtin::copy_text()])
            .edit(&edit, row_value(Note::mine).eq(true))
            .actions_when(notes.field(ActivityView::closed).ne(true))
            .card(
                card_of(&people, Person::name)
                    .avatar(Person::avatar)
                    .agent(Person::agent)
                    .tile(Person::icon, Person::engine)
                    .detail(Person::role)
                    .action(&mention)
                    .unknown("Not a watcher"),
            );
        let column = stack([
            header(notes.field(ActivityView::title))
                .title_icon(lit("package".to_string()))
                .leading_tile(lit("package".to_string()), lit("".to_string()))
                .leading_size(40)
                .subtitle([
                    seg_text(notes.field(ActivityView::status)),
                    seg_dot_live().visible(live.field(LiveView::can_stop).eq(true)),
                    seg_countdown(notes.field(ActivityView::ends)).prefix("closes in "),
                ])
                .subtitle_action(&jump)
                .faces(notes.field(ActivityView::people), "Watchers", &assign_a)
                .button("Assign", &assign_a)
                .button_when(notes.field(ActivityView::closed).ne(true))
                .menu([
                    menu_item("Copy link", &copy_link),
                    menu_item("Copy title", &copy_title),
                ])
                .tabs(
                    &tab,
                    [
                        header_tab("notes", "Notes"),
                        header_tab("pinned", "Pinned").badge(notes.field(ActivityView::pinned)),
                    ],
                )
                .phone_sub(notes.field(ActivityView::status))
                .phone_icons([&assign_a])
                .menu_tabs()
                .into(),
            stream(&notes)
                .key(Note::id)
                .post_with(note.clone())
                .kinds_line(
                    Note::kind,
                    "event",
                    who.f(Person::name),
                    Note::text,
                    Note::time,
                )
                .tail(&live)
                .days(Note::time)
                .jump_working(live.field(LiveView::label))
                .empty("No notes yet.")
                .phone_intro(notes.field(ActivityView::status))
                .on_seen(&seen)
                .on_seen_when(row_value(Note::mine).ne(true))
                .id("notes")
                .label("Notes")
                .visible(tab.val().eq("notes")),
            stream(&notes)
                .key(Note::id)
                .anchor_top()
                .older_button("Show more pinned notes")
                .post_with(note.no_summary())
                .empty("Nothing pinned.")
                .visible(tab.val().eq("pinned")),
            composer(&add)
                .for_stream("notes")
                .primary()
                .placeholder(notes.field(ActivityView::placeholder))
                .tools([ComposerTool::Emoji])
                .mentions(
                    mentions(&people, Person::name)
                        .avatar(Person::avatar)
                        .agent(Person::agent)
                        .tile(Person::icon, Person::engine)
                        .detail(Person::role)
                        .note("A mention tells them"),
                )
                .tags()
                .dictation()
                .vocabulary("DPD")
                .reply()
                .option("urgent", "Mark urgent")
                .option_when(notes.field(ActivityView::closed).ne(true))
                .caption_route(hint.field(PreviewView::hint))
                .caption_dot(live.field(LiveView::hint))
                .stop(&stop, live.field(LiveView::can_stop).eq(true))
                .stop_label("Stop")
                .readonly(
                    notes.field(ActivityView::closed).eq(true),
                    "This order is closed.",
                )
                .draft_state(&draft)
                .into(),
        ]);
        let panel = stack([
            header("Watchers")
                .context(notes.field(ActivityView::title))
                .icons([icon_button("eye", "Watching", &toggle_watch).toggle(
                    notes.field(ActivityView::watching),
                    "eye-off",
                    "Watch",
                )])
                .expand()
                .close(&close_job)
                .back("Activity")
                .into(),
            strip(&live, LiveView::jobs, Packing::job, Packing::agent)
                .label(live.field(LiveView::label))
                .tile(Packing::icon, Packing::engine)
                .state(
                    Packing::state,
                    &["working", "starting"],
                    &["waiting"],
                    &["starting"],
                )
                .since(Packing::since)
                .until(Packing::until)
                .sub_line(Packing::sub)
                .countdown_text("ends in {left}")
                .open(&open_job)
                .jump(Packing::note, "Jump to the note", "Jump to its last note")
                .button(
                    strip_button("Stop", &stop_job)
                        .icon("square")
                        .stop_style()
                        .visible(row_value(Packing::state).eq("working")),
                )
                .aria_label("Live jobs")
                .into(),
            process(&job_q, |w: ProcessB<JobView>| {
                w.state(JobView::title).label(JobView::title)
            }),
            list(&people, Person::name)
                .subtitle(Person::role)
                .search("Find a watcher")
                .online(Person::online)
                .limit(5)
                .group_label("Watchers")
                .into(),
            text(live.field(LiveView::label)).elapsed(),
        ])
        .visible(not(job.val().empty()));
        split(column, panel)
            .fill()
            .resize(340, 820, 420, 400)
            .expand(&wide)
    })
}

/// Assign people to an order: a pick over the watchers, a note while an agent is chosen.
fn assign_dialog() -> Overlay {
    Overlay::dialog(ASSIGN, |o| {
        let oid = o.param(AssignParams::order);
        o.title("Assign");
        let people = o.query(
            "directory",
            Query::call(get_watchers)
                .arg(GetWatchersRequest::order, oid.or_empty())
                .rows(WatchersView::people),
        );
        o.form(
            assign,
            |f| {
                f.pick(
                    AssignRequest::people,
                    "People",
                    &people,
                    Person::name,
                    Person::name,
                )
                .avatar(Person::avatar)
                .agent(Person::agent)
                .tile(Person::icon, Person::engine)
                .detail(Person::role)
                .exclude(lit(Vec::<String>::new()))
                .empty("Nobody matches", "Everyone is assigned")
                .placeholder("Find people")
                .required();
                f.note_when(
                    "An agent works on the order at once.",
                    picked_agents(AssignRequest::people).gt(0),
                );
            },
            t("assign.submit", "Assign {first}").arg("first", picked_first(AssignRequest::people)),
            |a, _| {
                a.call_form(assign, |c| c.arg(AssignRequest::order, oid.or_empty()))
                    .close()
            },
        )
    })
}

fn plugin() -> Plugin {
    Plugin::new("shop", "1.0.0")
        .role(CLERK)
        .agent(
            Agent::new("helper")
                .roles([CLERK])
                .title("Order helper")
                .describe("Answers questions about orders.")
                .icon("package")
                .engine(AgentEngine::Codex)
                .model("gpt-5.5")
                .session()
                .timeout(std::time::Duration::from_secs(900))
                .skills(["order-notes"])
                .topic(ORDERS)
                .delegates(["packer"])
                .connection("models")
                .ssh_key(),
        )
        .connection(
            Connection::llm("models")
                .title("Order models")
                .describe("The model provider the order helper runs on."),
        )
        .connection(Connection::stt("speech"))
        .agent(
            Agent::new("packer")
                .describe("Says how an order is packed.")
                .admin(),
        )
        .endpoint(Endpoint::new(list_orders).ui().read_only())
        .endpoint(Endpoint::new(get_order).ui().read_only())
        .endpoint(
            Endpoint::new(cancel_order)
                .ui()
                .roles([CLERK])
                .idempotency(Idempotency::Optional)
                .invalidates(ORDERS)
                .invalidates(ORDER.by(OrderKey::id, Order::id)),
        )
        .endpoint(Endpoint::new(count_open).ui().read_only())
        .endpoint(Endpoint::new(sales).ui().read_only())
        .endpoint(Endpoint::new(top_products).ui().read_only())
        .endpoint(Endpoint::new(get_activity).ui().read_only())
        .endpoint(Endpoint::new(get_live).ui().read_only())
        .endpoint(Endpoint::new(get_watchers).ui().read_only())
        .endpoint(Endpoint::new(preview_note).ui().read_only())
        .endpoint(Endpoint::new(get_job).ui().read_only())
        .endpoint(
            Endpoint::new(add_note)
                .ui()
                .idempotency(Idempotency::Optional)
                .invalidates(ORDERS),
        )
        .endpoint(
            Endpoint::new(note_action)
                .ui()
                .idempotency(Idempotency::Optional),
        )
        .endpoint(Endpoint::new(watch).ui().idempotency(Idempotency::Optional))
        .endpoint(
            Endpoint::new(assign)
                .ui()
                .idempotency(Idempotency::Optional),
        )
        .topic(ORDERS.roles([CLERK]))
        .topic(ORDER.decl().authorize(get_order))
        .ui(Ui::new()
            .nav(
                Nav::group("Orders").icon("shopping-bag").entry(
                    Entry::to(ORDERS_SCREEN, t("nav.all", "All orders"), |p| p)
                        .badge(Query::call(count_open).follow(ORDERS), CountResponse::count)
                        .alert(),
                ),
            )
            .landing(ORDERS_SCREEN)
            .command(Command::navigate("find", "Find an order", ORDERS_SCREEN).icon("search"))
            .command(
                Command::navigate_with("week", "Sales: last 7 days", REPORT_SCREEN, |a| {
                    a.arg(ReportParams::range, "7")
                })
                .icon("bar-chart"),
            )
            .screen(orders_screen())
            .screen(order_screen())
            .screen(report_screen())
            .screen(activity_screen())
            .home("sales", "Sales", |w| {
                let days = w.query("days", Query::call(sales).rows(SalesResponse::days));
                viz::line::line()
                    .dates(&days, DaySales::day)
                    .values(&days, DaySales::total)
                    .option("compact", true)
                    .open(REPORT_SCREEN, |a| a)
                    .fallback(metric("Revenue", days.field(SalesResponse::revenue)))
            })
            .overlay(cancel_dialog())
            .overlay(assign_dialog())
            .search(
                Search::by(
                    "Orders",
                    list_orders,
                    ListOrdersRequest::text,
                    ListOrdersResponse::orders,
                )
                .item(|r| row_view(r.get(Order::number)).subtitle(r.get(Order::customer)))
                .open(ORDER_SCREEN, |a, r| {
                    a.arg(OrderParams::id, r.get(Order::id))
                })
                .action("Cancel order", "x-circle", CANCEL, |a, r| {
                    a.arg(CancelParams::id, r.get(Order::id))
                }),
            ))
}

fn main() {
    plugin().serve()
}
