//! A plugin with a UI described by the Rust SDK validates with the platform's own validator.

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
    pub total: Decimal,
    pub currency: String,
    pub status: OrderStatus,
    pub placed_at: Timestamp,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ListOrdersRequest {
    pub status: Option<OrderStatus>,
    pub page_token: String,
    pub page_size: i32,
    pub channels: String,
    pub sort_total: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ListOrdersResponse {
    pub orders: Vec<Order>,
    pub next_page_token: String,
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
pub struct OrderKey {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct AllOrders {}

#[derive(Message, Clone, Debug, Default)]
pub struct OrdersParams {
    pub status: Option<OrderStatus>,
    pub channels: Option<String>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct OrderParams {
    pub id: Uuid,
}

#[derive(Message, Clone, Debug, Default)]
pub struct CancelParams {
    pub id: Uuid,
}

pub const ORDERS: Topic<AllOrders> = Topic::new("orders");
pub const ORDER: Topic<OrderKey> = Topic::new("order");
pub const ORDERS_SCREEN: ScreenKey<OrdersParams> = ScreenKey::new("orders");
pub const ORDER_SCREEN: ScreenKey<OrderParams> = ScreenKey::new("order");
pub const CANCEL: OverlayKey<CancelParams> = OverlayKey::new("cancel_form");

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

fn orders_screen() -> Screen {
    Screen::build(ORDERS_SCREEN, |s| {
        let status = s.param(OrdersParams::status);
        let channels = s.param(OrdersParams::channels);
        let orders = s.query(
            "orders",
            Query::call(list_orders)
                .arg(ListOrdersRequest::status, status)
                .arg(ListOrdersRequest::channels, channels.or_empty())
                .rows(ListOrdersResponse::orders)
                .paged()
                .follow(ORDERS),
        );
        let open = s.row_action("open", &orders, |a, r| {
            a.navigate(ORDER_SCREEN, |p| p.arg(OrderParams::id, r.get(Order::id)))
        });
        let cancel = s.row_action("cancel", &orders, |a, r| {
            a.confirm(Confirm::danger("Cancel this order?").text("The customer is told."))
                .call(cancel_order, |c| {
                    c.arg(CancelOrderRequest::id, r.get(Order::id))
                })
                .toast(t("orders.cancelled", "Order cancelled"))
                .on_error(|e| e.toast_error())
        });
        s.title(t("orders.title", "Orders"));
        stack([
            filter_bar([
                status.select("Status"),
                channels
                    .choices("Channel")
                    .option("web", "Web")
                    .option("store", "Store"),
            ]),
            table(&orders)
                .mono(Order::number, "Order")
                .col(Order::customer, "Customer")
                .money(Order::total, "Total", Order::currency)
                .sortable(ListOrdersRequest::sort_total)
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
        s.overlay(Overlay::dialog(CANCEL, |o| {
            let oid = o.param(CancelParams::id);
            o.title("Cancel order");
            o.form(
                cancel_order,
                |f| {
                    f.textarea(CancelOrderRequest::reason, "Why").required();
                    f.cancel("Keep it");
                },
                "Cancel order",
                |a, _v| {
                    a.call_form(cancel_order, |c| c.arg(CancelOrderRequest::id, oid))
                        .toast("Order cancelled")
                        .close()
                },
            )
        }));
        s.title(order.field(Order::number))
            .header([button("Cancel order", &edit)
                .danger()
                .visible(order.field(Order::status).eq(OrderStatus::Placed))]);
        columns([
            fields([
                item("Customer", order.field(Order::customer)),
                item("Total", order.field(Order::total)).money_in("USD"),
                item("Placed", order.field(Order::placed_at)).datetime(),
            ]),
            status(order.field(Order::status), [(OrderStatus::Paid, Tone::Ok)]),
        ])
        .ratios([2, 1])
    })
}

fn plugin() -> Plugin {
    Plugin::new("shop", "1.0.0")
        .role(CLERK)
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
        .topic(ORDERS.roles([CLERK]))
        .topic(ORDER.decl().authorize(get_order))
        .ui(Ui::new()
            .nav(
                Nav::group("Orders").icon("shopping-bag").entry(
                    Entry::to(ORDERS_SCREEN, t("nav.all", "All orders"), |p| p)
                        .badge(Query::call(count_open).follow(ORDERS), CountResponse::count),
                ),
            )
            .command(Command::navigate("find", "Find an order", ORDERS_SCREEN).icon("search"))
            .screen(orders_screen())
            .screen(order_screen()))
}

#[test]
fn a_plugin_ui_describes_and_validates() {
    let json = plugin().describe_json().expect("describes");
    let d = celobus_descriptor::parse(json.as_bytes()).expect("parses");
    let errors = celobus_descriptor::validate(&d);
    assert!(errors.is_empty(), "{errors:#?}\n{json}");
    let ui = d.ui.as_ref().unwrap();
    assert_eq!(ui.screens.len(), 2);
    assert_eq!(ui.texts["en"]["nav.all"], "All orders");
    assert_eq!(ui.queries[0].name, "badge_count");
    assert!(d.endpoints.iter().all(|e| e.ui));
    assert!(
        d.endpoints
            .iter()
            .filter(|e| e.name != "CancelOrder")
            .all(|e| e.read_only)
    );
    assert_eq!(d.topics.len(), 2);
    assert_eq!(celobus_descriptor::ui_check::ui_warnings(&d), vec![]);
    // a sortable column names its request field; a filter with several values is a multi-select
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let children = &v["ui"]["screens"][0]["layout"]["stack"]["children"];
    assert_eq!(children[1]["table"]["columns"][2]["sort"], "sort_total");
    assert_eq!(
        children[0]["filter_bar"]["inputs"][1]["control"],
        "multi_select"
    );
    assert_eq!(
        children[0]["filter_bar"]["inputs"][1]["options"][1]["value"],
        "store"
    );
}

#[test]
fn a_plugin_without_ui_describes_as_before() {
    let p = Plugin::new("plain", "1.0.0").endpoint(Endpoint::new(get_order).read_only());
    let json = p.describe_json().unwrap();
    assert!(!json.contains("\"ui\""), "{json}");
    assert!(!json.contains("read_only"), "{json}");
    assert!(!json.contains("topics"), "{json}");
}

// ---------------------------------------------------------------------------------------------- charts and figures

#[derive(Message, Clone, Debug, Default)]
pub struct Day {
    pub day: Date,
    pub channel: String,
    pub total: Decimal,
    pub online: Decimal,
    pub retail: Decimal,
    pub orders: i32,
}

#[derive(Message, Clone, Debug, Default)]
pub struct SalesRequest {
    pub period: String,
    pub channel: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct SalesResponse {
    pub days: Vec<Day>,
    pub revenue: Decimal,
    pub orders: i64,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Product {
    pub title: String,
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
pub struct ReportParams {
    pub period: Option<String>,
}

pub const REPORT: ScreenKey<ReportParams> = ScreenKey::new("report");

async fn sales(_: Ctx, _: SalesRequest) -> Result<SalesResponse> {
    Ok(SalesResponse::default())
}
async fn top_products(_: Ctx, _: TopProductsRequest) -> Result<TopProductsResponse> {
    Ok(TopProductsResponse::default())
}

fn report_screen() -> Screen {
    Screen::build(REPORT, |s| {
        let period = s.param(ReportParams::period);
        let channel = s.state("channel", String::new());
        let by_day = s.query(
            "sales",
            Query::call(sales)
                .arg(SalesRequest::period, period.or_empty())
                .arg(SalesRequest::channel, &channel)
                .rows(SalesResponse::days),
        );
        let top = s.query(
            "top",
            Query::call(top_products).rows(TopProductsResponse::products),
        );
        s.title("Report");
        stack([
            // every source of the contract, in its order
            viz::line::line()
                .title("Revenue")
                .range("Last 30 days")
                .options(serde_json::json!({ "fmt": "money", "curve": "smooth" }))
                .dates(&by_day, Day::day)
                .values(&by_day, Day::total)
                .serie(&by_day, Day::online, "Online store")
                .pivot(&by_day, Day::day, Day::channel, Day::total)
                .as_date()
                .rows(
                    &top,
                    (("name", Product::title), ("value", Product::revenue)),
                )
                .text_keys(["name"])
                .value("value", by_day.field(SalesResponse::revenue))
                .height(280)
                .select_state(&channel)
                .open(ORDERS_SCREEN, |a| a)
                .into(),
            // fields of different types in one series and one rows bind; other destinations
            viz::bar::column()
                .labels(&by_day, Day::channel)
                .series(
                    &by_day,
                    (
                        (Day::online, "Online"),
                        (Day::orders, t("report.orders", "Orders")),
                    ),
                )
                .serie(&by_day, Day::retail, "Retail")
                .serie(&by_day, Day::total, "Total")
                .column("targets", &by_day, Day::orders)
                .column("notes", &by_day, Day::channel)
                .as_text()
                .rows(&top, [("name", Product::title)])
                .to("parts")
                .option("stacked", true)
                .option("target", 120)
                .select_param(&period)
                .fallback(table(&top).col(Product::title, "Product")),
            // one value from a query, a state, a param and literals
            viz::figure::metric()
                .title("Revenue")
                .value("value", by_day.field(SalesResponse::revenue))
                .value("center.value", by_day.field(SalesResponse::orders))
                .value("channel", &channel)
                .value("period", period)
                .value("target", 12.5)
                .value("unit", "orders")
                .into(),
            viz("meter", "meterList")
                .rows(&top, vec![("value", Product::units)])
                .visible(channel.val().ne("")),
            viz::meter::meter_list().into(),
        ])
    })
}

pub const DASHBOARD: ScreenKey<ReportParams> = ScreenKey::new("dashboard");

/// A dashboard: cards of several widths on the board, a heading between groups, a fallback for an older client.
fn dashboard_screen() -> Screen {
    Screen::build(DASHBOARD, |s| {
        let by_day = s.query("sales", Query::call(sales).rows(SalesResponse::days));
        let top = s.query(
            "top",
            Query::call(top_products).rows(TopProductsResponse::products),
        );
        let revenue = || {
            viz::figure::metric()
                .title("Revenue")
                .value("value", by_day.field(SalesResponse::revenue))
        };
        s.title("Dashboard");
        board()
            .quarter(revenue())
            .quarter(revenue().title("Orders"))
            .half(viz::meter::gauge().value("value", by_day.field(SalesResponse::orders)))
            .full(heading("Sales"))
            .span(
                8,
                viz::line::line()
                    .dates(&by_day, Day::day)
                    .values(&by_day, Day::total),
            )
            .third(
                viz::part::donut()
                    .rows(
                        &top,
                        (("name", Product::title), ("value", Product::revenue)),
                    )
                    .to("parts"),
            )
            .full(table(&top).col(Product::title, "Product"))
            .fallback(table(&top).col(Product::title, "Product"))
    })
}

fn report_plugin() -> Plugin {
    Plugin::new("shop", "1.0.0")
        .endpoint(Endpoint::new(list_orders).ui().read_only())
        .endpoint(Endpoint::new(sales).ui().read_only())
        .endpoint(Endpoint::new(top_products).ui().read_only())
        .ui(Ui::new()
            .screen(orders_screen_plain())
            .screen(report_screen())
            .screen(dashboard_screen())
            .home("sales", "Sales", |w| {
                let days = w.query("days", Query::call(sales).rows(SalesResponse::days));
                viz::line::line()
                    .dates(&days, Day::day)
                    .values(&days, Day::total)
                    .value("value", days.field(SalesResponse::revenue))
                    .open(REPORT, |a| a.arg(ReportParams::period, "month"))
                    .fallback(metric("Revenue", days.field(SalesResponse::revenue)))
            }))
}

/// The orders screen without its actions (the report plugin only links to it).
fn orders_screen_plain() -> Screen {
    Screen::build(ORDERS_SCREEN, |s| {
        let orders = s.query(
            "orders",
            Query::call(list_orders).rows(ListOrdersResponse::orders),
        );
        s.title("Orders");
        table(&orders).col(Order::number, "Order").into()
    })
}

#[test]
fn charts_and_figures_describe_and_validate() {
    use serde_json::json;
    let text = report_plugin().describe_json().expect("describes");
    let d = celobus_descriptor::parse(text.as_bytes()).expect("parses");
    let errors = celobus_descriptor::validate(&d);
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["ui"]["catalog"], 3);
    let nodes = &v["ui"]["screens"][1]["layout"]["stack"]["children"];
    // the contract's node, key for key
    assert_eq!(
        nodes[0],
        json!({ "viz": {
          "family": "line", "kind": "line", "title": "Revenue", "range": "Last 30 days",
          "options": { "fmt": "money", "curve": "smooth" },
          "bind": [
            { "to": "labels", "from": { "column": { "query": "sales", "field": "day", "as": "date" } } },
            { "to": "values", "from": { "column": { "query": "sales", "field": "total" } } },
            { "to": "series", "from": { "series": { "query": "sales",
                                                      "fields": [{ "field": "online", "label": "Online store" }] } } },
            { "to": "series", "from": { "pivot": { "query": "sales", "x": "day", "series": "channel",
                                                     "value": "total", "as": "date" } } },
            { "to": "rows", "from": { "rows": { "query": "top", "map": { "name": "title", "value": "revenue" },
                                                  "text": ["name"] } } },
            { "to": "value", "from": { "value": { "data": { "query": "sales", "field": "revenue" } } } }
          ],
          "height": 280,
          "select": { "state": "channel" },
          "open": { "screen": "orders" }
        } })
    );
    let column = &nodes[1]["viz"];
    assert_eq!(
        (column["family"].as_str(), column["kind"].as_str()),
        (Some("bar"), Some("column"))
    );
    assert_eq!(column["options"], json!({ "stacked": true, "target": 120 }));
    assert_eq!(
        column["bind"],
        json!([
          { "to": "labels", "from": { "column": { "query": "sales", "field": "channel", "as": "text" } } },
          // one series bind: two fields of different types, and the two added one by one after them
          { "to": "series", "from": { "series": { "query": "sales", "fields": [
              { "field": "online", "label": "Online" },
              { "field": "orders", "label": { "key": "report.orders" } },
              { "field": "retail", "label": "Retail" }, { "field": "total", "label": "Total" }] } } },
          { "to": "targets", "from": { "column": { "query": "sales", "field": "orders" } } },
          { "to": "notes", "from": { "column": { "query": "sales", "field": "channel", "as": "text" } } },
          { "to": "parts", "from": { "rows": { "query": "top", "map": { "name": "title" } } } }
        ])
    );
    assert_eq!(column["select"], json!({ "param": "period" }));
    assert!(nodes[1]["fallback"]["table"].is_object(), "{}", nodes[1]);
    assert_eq!(
        nodes[2]["viz"]["bind"],
        json!([
          { "to": "value", "from": { "value": { "data": { "query": "sales", "field": "revenue" } } } },
          { "to": "center.value", "from": { "value": { "data": { "query": "sales", "field": "orders" } } } },
          { "to": "channel", "from": { "value": { "state": "channel" } } },
          { "to": "period", "from": { "value": { "param": "period" } } },
          { "to": "target", "from": { "value": { "lit": 12.5 } } },
          { "to": "unit", "from": { "value": { "lit": "orders" } } }
        ])
    );
    assert!(nodes[2]["viz"].get("options").is_none(), "{}", nodes[2]);
    assert_eq!(nodes[3]["viz"]["kind"], "meterList");
    assert!(nodes[3]["visible"].is_object());
    assert_eq!(
        nodes[4],
        json!({ "viz": { "family": "meter", "kind": "meterList" } })
    );
    // a home widget binds its own document-level queries
    let home = &v["ui"]["home"][0];
    assert_eq!(
        home["node"]["viz"]["bind"][0]["from"]["column"]["query"],
        "home_sales_days"
    );
    assert_eq!(
        home["node"]["viz"]["open"],
        json!({ "screen": "report", "args": [{ "field": "period", "value": { "lit": "month" } }] })
    );
    assert!(home["node"]["fallback"]["metric"].is_object());
    // an older client draws the fallbacks; the screen without a viz is whole for it
    let r = celobus_descriptor::ui_check::requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["report"].compat(1), "degraded");
    assert_eq!(r["home"].compat(1), "degraded");
    assert_eq!(r["orders"].compat(1), "full");
    assert_eq!(r["report"].compat(2), "full");
}

#[test]
fn a_board_describes_and_validates() {
    let text = report_plugin().describe_json().expect("describes");
    let d = celobus_descriptor::parse(text.as_bytes()).expect("parses");
    assert_eq!(celobus_descriptor::validate(&d), vec![]);
    assert_eq!(celobus_descriptor::ui_check::ui_warnings(&d), vec![]);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let screen = &v["ui"]["screens"][2];
    assert_eq!(screen["id"], "dashboard");
    let items = screen["layout"]["board"]["children"].as_array().unwrap();
    // the spans of the items; the full width is the default and is left out
    let spans: Vec<_> = items.iter().map(|i| i["span"].as_u64()).collect();
    assert_eq!(
        spans,
        [Some(3), Some(3), Some(6), None, Some(8), Some(4), None]
    );
    let kinds: Vec<_> = items
        .iter()
        .map(|i| {
            i["node"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone()
        })
        .collect();
    assert_eq!(kinds, ["viz", "viz", "viz", "text", "viz", "viz", "table"]);
    assert_eq!(items[1]["node"]["viz"]["title"], "Orders");
    assert_eq!(items[5]["node"]["viz"]["bind"][0]["to"], "parts");
    assert!(screen["layout"]["fallback"]["table"].is_object());
    // a display layout of catalog 2: an older client draws the fallback
    let r = celobus_descriptor::ui_check::requirements(d.ui.as_ref().unwrap());
    assert_eq!(r["dashboard"].compat(1), "degraded");
    assert_eq!(r["dashboard"].compat(2), "full");
}

#[test]
fn every_kind_of_the_catalogue_has_a_constructor() {
    // the constructors come from the same table the validator reads
    let d = |v: VizB| {
        let n: N = v.into();
        let v = serde_json::to_value(&n.0).unwrap();
        (
            v["viz"]["family"].as_str().unwrap().to_string(),
            v["viz"]["kind"].as_str().unwrap().to_string(),
        )
    };
    for (v, family, kind) in [
        (viz::line::line(), "line", "line"),
        (viz::bar::column(), "bar", "column"),
        (viz::bar::funnel(), "bar", "funnel"),
        (viz::flow::funnel(), "flow", "funnel"),
        (viz::figure::metric(), "figure", "metric"),
        (viz::meter::meter_list(), "meter", "meterList"),
        (viz::part::donut(), "part", "donut"),
        (viz::dist::histogram(), "dist", "histogram"),
        (viz::table::pivot(), "table", "pivot"),
        (viz::time::heatmap(), "time", "heatmap"),
    ] {
        let (f, k) = d(v);
        assert_eq!((f.as_str(), k.as_str()), (family, kind));
        assert!(celobus_descriptor::viz_kinds::known(family, kind));
    }
}

#[test]
#[should_panic(expected = "JSON object")]
fn the_options_of_a_viz_are_an_object() {
    let _ = viz::line::line().options(serde_json::json!(["fmt", "money"]));
}

// ---------------------------------------------------------------------------------------------- report screens

#[derive(Message, Clone, Debug, Default, PartialEq)]
pub enum Grain {
    #[default]
    Day,
    Week,
}

#[derive(Message, Clone, Debug, Default)]
pub struct TrendParams {
    pub range: Option<String>,
    pub grain: Option<Grain>,
    pub top: i32,
    pub status: Option<OrderStatus>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct NoteParams {
    pub kind: Option<String>,
}

pub const TREND: ScreenKey<TrendParams> = ScreenKey::new("trend");
pub const NOTE: OverlayKey<NoteParams> = OverlayKey::new("note");

fn trend_screen() -> Screen {
    Screen::build(TREND, |s| {
        // a param with a default has a value on every link: its handle is of the plain type
        let range: ParamRef<String> = s.param_or(TrendParams::range, "30");
        let grain: ParamRef<Grain> = s.param_or(TrendParams::grain, Grain::Day);
        let top: ParamRef<i32> = s.param_or(TrendParams::top, 5);
        let view = s.state("view", "chart".to_string());
        let sales = s.query(
            "sales",
            Query::call(sales)
                .arg(SalesRequest::period, range)
                .rows(SalesResponse::days),
        );
        let week = s.action("week", |a| a.set_param(range, "7"));
        s.title("Trend");
        s.header([button("Last week", &week)]);
        stack([
            row([
                segmented(range, [("7", "7 days"), ("30", "30 days")]),
                segmented(grain, [(Grain::Day, "Days"), (Grain::Week, "Weeks")]),
                segmented(
                    top,
                    [
                        (5, t("trend.top5", "Top 5")),
                        (10, t("trend.top10", "Top 10")),
                    ],
                ),
                segmented(&view, [("chart", "Chart"), ("table", "Table")]),
            ]),
            viz::line::line()
                .dates(&sales, Day::day)
                .values(&sales, Day::total)
                // a literal, an enum value for an optional param, a param: as in a link
                .open(TREND, |a| {
                    a.arg(TrendParams::range, "7")
                        .arg(TrendParams::status, OrderStatus::Paid)
                        .arg(TrendParams::grain, grain)
                })
                .into(),
            link("Paid orders", TREND, |a| {
                a.arg(TrendParams::status, OrderStatus::Paid)
            }),
            breadcrumbs([
                crumb_to("Trend", TREND, |a| a),
                crumb_to(t("trend.paid", "Paid"), TREND, |a| {
                    a.arg(TrendParams::status, OrderStatus::Paid)
                        .arg(TrendParams::grain, grain)
                }),
                crumb("Week"),
            ]),
        ])
    })
}

fn trend_plugin() -> Plugin {
    Plugin::new("shop", "1.0.0")
        .endpoint(Endpoint::new(sales).ui().read_only())
        .ui(Ui::new()
            .screen(trend_screen())
            .overlay(Overlay::dialog(NOTE, |o| {
                o.title("Note");
                text("A note")
            }))
            .command(Command::navigate("trend", "Trend", TREND))
            .command(
                Command::navigate_with("trend_week", "Trend: last 7 days", TREND, |a| {
                    a.arg(TrendParams::range, "7")
                        .arg(TrendParams::grain, Grain::Week)
                })
                .icon("chart-line"),
            )
            .command(Command::open_with("note", "New note", NOTE, |a| {
                a.arg(NoteParams::kind, "plain")
            })))
}

#[test]
fn a_report_head_has_defaults_a_range_switch_breadcrumbs_and_commands_with_args() {
    use serde_json::json;
    let text = trend_plugin().describe_json().expect("describes");
    let d = celobus_descriptor::parse(text.as_bytes()).expect("parses");
    let errors = celobus_descriptor::validate(&d);
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let screen = &v["ui"]["screens"][0];
    // a default makes a param optional, whatever its field is; the others are as before
    assert_eq!(
        screen["params"],
        json!([
            { "name": "range", "type": "string", "required": false, "default": "30" },
            { "name": "grain", "type": "enum:Grain", "required": false, "default": "day" },
            { "name": "top", "type": "int32", "required": false, "default": 5 },
            { "name": "status", "type": "enum:OrderStatus", "required": false },
        ])
    );
    assert_eq!(
        screen["queries"][0]["args"],
        json!([{ "field": "period", "value": { "param": "range" } }])
    );
    let switches = &screen["layout"]["stack"]["children"][0]["stack"]["children"];
    assert_eq!(
        switches[0]["segmented"],
        json!({ "bind": { "param": "range" }, "options": [
            { "value": "7", "label": "7 days" }, { "value": "30", "label": "30 days" }] })
    );
    assert_eq!(
        switches[1]["segmented"],
        json!({ "bind": { "param": "grain" }, "options": [
            { "value": "day", "label": "Days" }, { "value": "week", "label": "Weeks" }] })
    );
    assert_eq!(switches[2]["segmented"]["bind"], json!({ "param": "top" }));
    assert_eq!(
        switches[2]["segmented"]["options"][1],
        json!({ "value": 10, "label": { "key": "trend.top10" } })
    );
    assert_eq!(
        switches[3]["segmented"],
        json!({ "bind": { "state": "view" }, "options": [
            { "value": "chart", "label": "Chart" }, { "value": "table", "label": "Table" }] })
    );
    assert_eq!(
        screen["layout"]["stack"]["children"][1]["viz"]["open"],
        json!({ "screen": "trend", "args": [
            { "field": "range", "value": { "lit": "7" } },
            { "field": "status", "value": { "lit": "paid" } },
            { "field": "grain", "value": { "param": "grain" } }] })
    );
    // a breadcrumb trail: crumbs that link to screens, with args like a link, and the last one a text
    assert_eq!(
        screen["layout"]["stack"]["children"][3]["breadcrumbs"],
        json!({ "items": [
            { "text": "Trend", "target": { "screen": "trend" } },
            { "text": { "key": "trend.paid" }, "target": { "screen": "trend", "args": [
                { "field": "status", "value": { "lit": "paid" } },
                { "field": "grain", "value": { "param": "grain" } }] } },
            { "text": "Week" }] })
    );
    assert_eq!(
        v["ui"]["commands"],
        json!([
            { "id": "trend", "title": "Trend", "target": { "screen": "trend" } },
            { "id": "trend_week", "title": "Trend: last 7 days", "icon": "chart-line",
              "target": { "screen": "trend", "args": [
                  { "field": "range", "value": { "lit": "7" } },
                  { "field": "grain", "value": { "lit": "week" } }] } },
            { "id": "note", "title": "New note",
              "target": { "overlay": "note", "args": [{ "field": "kind", "value": { "lit": "plain" } }] } },
        ])
    );
}
