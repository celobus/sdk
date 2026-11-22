//! The JSON codec of derived messages: the protobuf JSON mapping.

use celobus_sdk::prelude::*;
use serde_json::json;

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderStatus {
    Placed,
    Paid,
    CancelledByCustomer,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Line {
    pub product_id: String,
    pub quantity: i32,
    pub unit_price: Decimal,
}

#[derive(Entity, Clone, Debug, PartialEq)]
#[entity(key = uuid)]
pub struct Order {
    pub id: Id<Order>,
    pub number: String,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct PlaceOrderRequest {
    pub order_id: Id<Order>,
    pub total_cents: i64,
    pub status: OrderStatus,
    pub previous: Option<OrderStatus>,
    pub note: Option<String>,
    pub lines: Vec<Line>,
    pub first: Option<Line>,
    pub attachment: Vec<u8>,
    pub placed_at: Timestamp,
    pub due: Date,
    pub extra: Json,
    pub urgent: bool,
    pub weight: f64,
    pub owner: Option<Id<platform::User>>,
    pub r#type: String,
}

fn sample() -> PlaceOrderRequest {
    PlaceOrderRequest {
        order_id: Id::new(Uuid::parse_str("0190b6e4-7c2a-7f00-8000-000000000001").unwrap()),
        total_cents: 9_007_199_254_740_993,
        status: OrderStatus::CancelledByCustomer,
        previous: None,
        note: None,
        lines: vec![Line {
            product_id: "CHAI".into(),
            quantity: 3,
            unit_price: "12.50".parse().unwrap(),
        }],
        first: None,
        attachment: b"hi!".to_vec(),
        placed_at: "2026-11-08T10:00:00.250Z".parse().unwrap(),
        due: Date::from_ymd_opt(2026, 12, 1).unwrap(),
        extra: Json(json!({"a": [1, 2]})),
        urgent: true,
        weight: 1.5,
        owner: Some(Id::new("u-1".to_string())),
        r#type: "retail".into(),
    }
}

#[test]
fn encodes_the_protobuf_json_mapping() {
    let value = serde_json::to_value(sample()).unwrap();
    assert_eq!(
        value,
        json!({
            "orderId": "0190b6e4-7c2a-7f00-8000-000000000001",
            "totalCents": "9007199254740993",
            "status": "cancelled_by_customer",
            "lines": [{ "productId": "CHAI", "quantity": 3, "unitPrice": "12.50" }],
            "attachment": "aGkh",
            "placedAt": "2026-11-08T10:00:00.250Z",
            "due": "2026-12-01",
            "extra": { "a": [1, 2] },
            "urgent": true,
            "weight": 1.5,
            "owner": "u-1",
            "type": "retail"
        })
    );
}

#[test]
fn round_trips() {
    let text = serde_json::to_string(&sample()).unwrap();
    let back: PlaceOrderRequest = serde_json::from_str(&text).unwrap();
    assert_eq!(back, sample());
}

#[test]
fn decodes_either_name_numbers_and_defaults() {
    let req: PlaceOrderRequest = serde_json::from_value(json!({
        "order_id": "0190b6e4-7c2a-7f00-8000-000000000001",
        "totalCents": 42,
        "status": 1,
        "previous": null,
        "lines": [{ "productId": "X", "quantity": "2", "unitPrice": 1.25 }],
        "placedAt": "2026-11-08T13:00:00+03:00",
        "due": "2026-12-01",
        "unknownField": "ignored"
    }))
    .unwrap();
    assert_eq!(req.total_cents, 42);
    assert_eq!(req.status, OrderStatus::Paid);
    assert_eq!(req.previous, None);
    assert_eq!(req.lines[0].quantity, 2);
    assert_eq!(req.lines[0].unit_price.to_string(), "1.25");
    assert_eq!(req.placed_at.to_rfc3339(), "2026-11-08T10:00:00+00:00");
    // missing fields take the protobuf defaults
    assert_eq!(req.r#type, "");
    assert!(!req.urgent);
    assert!(req.attachment.is_empty());
    assert_eq!(req.extra, Json(serde_json::Value::Null));
}

#[test]
fn reports_the_path_of_a_bad_field() {
    let value = json!({
        "orderId": "0190b6e4-7c2a-7f00-8000-000000000001",
        "placedAt": "2026-11-08T10:00:00Z",
        "due": "2026-12-01",
        "lines": [{ "quantity": 1 }, { "quantity": "many" }]
    });
    let err = PlaceOrderRequest::from_json(&value, "").unwrap_err();
    assert_eq!((err.status(), err.code()), (400, "invalid"));
    assert_eq!(
        err.fields().collect::<Vec<_>>(),
        vec![("lines[1].quantity", "must be a 32-bit integer")]
    );

    let err = PlaceOrderRequest::from_json(&json!({}), "").unwrap_err();
    assert_eq!(
        err.fields().collect::<Vec<_>>(),
        vec![("order_id", "required")]
    );

    let err = PlaceOrderRequest::from_json(
        &json!({"orderId": "0190b6e4-7c2a-7f00-8000-000000000001", "placedAt": "2026-11-08T10:00:00Z",
                "due": "2026-12-01", "status": "lost"}),
        "",
    )
    .unwrap_err();
    assert_eq!(
        err.fields().collect::<Vec<_>>(),
        vec![(
            "status",
            "must be one of placed, paid, cancelled_by_customer"
        )]
    );
}

#[test]
fn enums_know_their_values() {
    assert_eq!(
        OrderStatus::VALUES,
        &["placed", "paid", "cancelled_by_customer"]
    );
    assert_eq!(OrderStatus::Paid.as_str(), "paid");
    assert_eq!(
        serde_json::to_value(OrderStatus::Placed).unwrap(),
        json!("placed")
    );
}

#[test]
fn ids_are_typed_keys() {
    #[derive(Entity, Clone, Debug)]
    #[entity(key = i64)]
    pub struct Event {
        pub id: Id<Event>,
    }
    let id: Id<Event> = Id::new(7);
    assert_eq!(serde_json::to_value(id).unwrap(), json!("7"));
    assert_eq!(
        serde_json::from_value::<Id<Event>>(json!("8")).unwrap(),
        Id::new(8)
    );
    assert_eq!(Event::ENTITY, "event");
}

/// A face of a trimmed view: its empty fields are left out.
#[derive(Message, Clone, Debug, PartialEq, Default)]
#[message(omit_defaults)]
pub struct Face {
    pub id: String,
    pub agent: bool,
}

#[derive(Message, Clone, Debug, PartialEq)]
#[message(omit_defaults)]
pub struct Trimmed {
    pub text: String,
    pub count: i32,
    pub big: i64,
    pub weight: f64,
    pub flag: bool,
    pub tags: Vec<String>,
    pub face: Face,
    pub faces: Vec<Face>,
    pub maybe: Option<i32>,
    pub status: OrderStatus,
    pub line: Line,
}

#[test]
fn omit_defaults_leaves_out_empty_fields() {
    let empty = Trimmed::default_with_status();
    // an enum, a `Some` and a message that is not `omit_defaults` are written; everything else at its default is not
    assert_eq!(
        empty.to_json(),
        json!({
            "status": "placed",
            "maybe": 0,
            "line": {"productId": "", "quantity": 0, "unitPrice": "0"}
        })
    );
    assert_eq!(Trimmed::from_json(&empty.to_json(), "").unwrap(), empty);

    let full = Trimmed {
        text: "hi".into(),
        count: 2,
        big: 3,
        weight: 0.5,
        flag: true,
        tags: vec!["a".into()],
        face: Face {
            id: "u-1".into(),
            agent: false,
        },
        faces: vec![Face::default()],
        ..Trimmed::default_with_status()
    };
    assert_eq!(
        full.to_json(),
        json!({
            "text": "hi", "count": 2, "big": "3", "weight": 0.5, "flag": true, "tags": ["a"],
            "face": {"id": "u-1"}, "faces": [{}], "status": "placed", "maybe": 0,
            "line": {"productId": "", "quantity": 0, "unitPrice": "0"}
        })
    );
    assert_eq!(Trimmed::from_json(&full.to_json(), "").unwrap(), full);
}

impl Trimmed {
    fn default_with_status() -> Self {
        Trimmed {
            text: String::new(),
            count: 0,
            big: 0,
            weight: 0.0,
            flag: false,
            tags: vec![],
            face: Face::default(),
            faces: vec![],
            maybe: Some(0),
            status: OrderStatus::Placed,
            line: Line {
                product_id: String::new(),
                quantity: 0,
                unit_price: Decimal::ZERO,
            },
        }
    }
}
