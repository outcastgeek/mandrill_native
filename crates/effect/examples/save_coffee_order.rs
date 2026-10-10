//! Maya corrects an online coffee order, saves her draft, and reloads it later.
//!
//! Run: `cargo run -p effect --example save_coffee_order`.
//! CoffeeOrder, Drink, Milk, Size, and OrderProblem are application models.
//! Effect's schemas validate their wire shape; the café validates business rules.
//! The draft store is local memory and does not survive restarting this example.
pub mod cafe_support;
use cafe_support::{Cafe, CafeKeys, limits, nz, parse_policy};
use effect::persistence::{
    MemoryStoreError, Persistable, PersistenceError, RecordPolicy, RecordVersion, StoredLookup,
    read, write,
};
use effect::{
    Codec, Effect, Exit, JsonRepresentation, JsonValue, KeyNamespace, PureCodec, SchemaIssue,
    StoredOutcome, UnknownFields, effect, json_i64, json_nullable, json_string, persistable,
    schema, schema_enum, service,
};
use std::sync::Arc;

/// Milk offered by this neighborhood café.
#[derive(Clone, Debug, PartialEq)]
pub enum Milk {
    /// Cow's milk.
    Dairy,
    /// Oat milk for customers avoiding dairy.
    Oat,
}
/// A customer's drink choice; variants intentionally have different payload shapes.
#[derive(Debug, PartialEq)]
pub enum Drink {
    /// Espresso and water need no milk selection.
    Americano,
    /// Latte carries a named milk choice.
    Latte {
        /// The customer's selected milk.
        milk: Milk,
    },
    /// Cappuccino carries its milk choice as a tuple payload.
    Cappuccino(
        /// The customer's selected milk.
        Milk,
    ),
}
/// Cup sizes printed on the café menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Size {
    /// The default 12-ounce cup.
    Regular,
    /// A 16-ounce cup.
    Large,
}
/// Owned draft; no Clone requirement is imposed on the customer's order.
#[derive(Debug, PartialEq)]
pub struct CoffeeOrder {
    /// Name called when the drink is ready.
    customer: String,
    /// Drink and any required milk choice.
    drink: Drink,
    /// Requested cup size.
    size: Size,
    /// Number of identical drinks; the café accepts one through four.
    quantity: i64,
    /// Optional preparation note, such as “less ice”.
    note: Option<String>,
}
/// Application-defined defaults supplied to the native form decoder through R.
pub struct CounterDefaults {
    /// Size chosen when the customer leaves the size field blank.
    size: Size,
}
/// Business feedback the customer can understand and correct.
#[derive(Debug)]
pub enum OrderProblem {
    /// The pickup name was blank.
    MissingName,
    /// This café accepts a small number of drinks in one draft.
    InvalidQuantity {
        /// Maximum drinks in one draft.
        maximum: i64,
    },
}
/// Errors retain shape validation, customer feedback, and provider details separately.
#[derive(Debug)]
pub enum DraftProblem {
    /// JSON does not match the declared application model.
    Form(SchemaIssue),
    /// An order violates a café rule.
    Order(OrderProblem),
    /// Reading or writing the customer's draft failed.
    Storage(PersistenceError<MemoryStoreError>),
}
/// Environment identifies which application's defaults and storage are selected.
type CoffeeShop = Cafe<CounterDefaults>;
/// Explicit milk tags make the wire model independent of Rust type names.
fn milk_schema() -> PureCodec<Milk, JsonValue> {
    schema_enum! {
        enum Milk { Dairy => "dairy", Oat => "oat" }
        tag_field: "kind", content_field: "details", policy: parse_policy()
    }
    .expect("milk schema")
}
/// Unit, named-field, and tuple variants describe actual drink choices.
fn drink_schema() -> PureCodec<Drink, JsonValue> {
    schema_enum! {
        enum Drink {
            Americano => "americano", Latte { milk: milk_schema() } => "latte", Cappuccino(milk_schema()) => "cappuccino"
        } tag_field: "drink", content_field: "options", policy: parse_policy()
    }.expect("drink schema")
}
/// Named cup sizes have explicit stable wire tags.
fn size_schema() -> PureCodec<Size, JsonValue> {
    schema_enum! {
        enum Size { Regular => "regular", Large => "large" }
        tag_field: "kind", content_field: "details", policy: parse_policy()
    }
    .expect("size schema")
}
/// Pure record codec is used to store and later borrow/decode a complete draft.
fn order_schema() -> PureCodec<CoffeeOrder, JsonValue> {
    schema! {
        struct CoffeeOrder {
            customer: json_string(), drink: drink_schema(), size: size_schema(),
            quantity: json_i64() => (|| 1, |quantity: &i64| *quantity == 1, Arc::new(JsonValue::from(1))),
            note: json_nullable(json_string()) => (|| None, Option::is_none)
        } policy: parse_policy()
    }.expect("stored order schema")
}
/// Share an immutable saved draft while decoding a fresh owned model from each stored record.
fn saved_draft_schema() -> PureCodec<Arc<CoffeeOrder>, JsonValue> {
    PureCodec::new(
        |json| order_schema().decode_ref(json).map(Arc::new),
        |order: &Arc<CoffeeOrder>| order_schema().encode_ref(order.as_ref()),
    )
}
/// The online editor defaults size through the café service and quantity to one.
fn order_form() -> Codec<CoffeeOrder, JsonValue, CoffeeShop, CoffeeShop> {
    schema! {
        struct CoffeeOrder {
            customer: json_string().into_effect::<CoffeeShop, CoffeeShop>(),
            drink: drink_schema().into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(default_with = || service! {
                select: |cafe: &CoffeeShop| cafe.services.clone(),
                on_error: |problem: SchemaIssue| problem,
                run |defaults: &Arc<CounterDefaults>| { defaults.size.clone() }
            })]
            size: size_schema().into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(default_with = || effect! { 1i64 })]
            quantity: json_i64().into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(optional)] note: json_nullable(json_string()).into_effect::<CoffeeShop, CoffeeShop>()
        } policy: parse_policy(), representation: JsonRepresentation, unknown_fields: UnknownFields::Reject
    }.expect("online order form")
}
/// Typed order failures can also be stored without reducing them to strings.
fn problem_schema() -> PureCodec<OrderProblem, JsonValue> {
    schema_enum! {
        enum OrderProblem { MissingName => "missing-name", InvalidQuantity { maximum: json_i64() } => "quantity" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }.expect("order feedback schema")
}
/// Keep stable draft identity separate from the order's mutable contents.
fn draft_identity(identity: &String) -> &str {
    identity.as_str()
}
/// Café validation belongs to the application, after schema shape validation.
fn accept_order(order: CoffeeOrder) -> Effect<CoffeeOrder, OrderProblem, CoffeeShop> {
    Effect::try_sync_once(
        move || {
            if order.customer.trim().is_empty() {
                return Err(OrderProblem::MissingName);
            }
            if !(1..=4).contains(&order.quantity) {
                return Err(OrderProblem::InvalidQuantity { maximum: 4 });
            }
            Ok(order)
        },
        |problem| problem,
    )
}
/// Maya fixes quantity zero, accepts the defaults, and reopens her saved draft.
fn save_mayas_order() -> Effect<(), DraftProblem, CoffeeShop> {
    effect! {
        let form = order_form();
        let accidental = form.as_effect_schema().parse(r#"{"customer":"Maya","drink":{"drink":"latte","options":{"milk":{"kind":"oat","details":null}}},"quantity":0}"#, parse_policy())
            .map_error(DraftProblem::Form)?;
        let feedback = accept_order(accidental).materialize_exit().map_error(|never| match never {})?;
        if let Exit::Fail(OrderProblem::InvalidQuantity { maximum }) = feedback {
            println!("Maya sees: choose between 1 and {maximum} drinks; quantity 0 cannot be saved.");
        }
        let edited = form.as_effect_schema().parse(r#"{"customer":"Maya","drink":{"drink":"latte","options":{"milk":{"kind":"oat","details":null}}},"quantity":2,"note":"extra hot"}"#, parse_policy())
            .map_error(DraftProblem::Form)?;
        let order = Arc::new(accept_order(edited).map_error(DraftProblem::Order)?);
        println!("Saving {}'s {:?} {:?}, quantity {}, note {:?}.", order.customer, order.size, order.drink, order.quantity, order.note);
        let drafts: Persistable<String, Arc<CoffeeOrder>, OrderProblem, CafeKeys> = persistable! {
            namespace: KeyNamespace::<CafeKeys>::new("coffee-drafts").expect("draft namespace"), request: String, key: draft_identity,
            success: saved_draft_schema(), error: problem_schema(), version: RecordPolicy::new(RecordVersion::new("coffee-order-v1").expect("draft version"), limits())
        };
        write(drafts.clone(), || "maya-morning-draft".to_owned(), move || StoredOutcome::Success(order.clone()), None).map_error(DraftProblem::Storage)?;
        let saved = read(drafts, || "maya-morning-draft".to_owned(), effect::UtcMillis::new(0)).map_error(DraftProblem::Storage)?;
        match saved {
            StoredLookup::Found { outcome: StoredOutcome::Success(order), .. } => println!("Welcome back, {}: your {} oat lattes and preparation note are restored.", order.customer, order.quantity),
            other => println!("Draft lookup returned {other:?}"),
        }
    }
}
/// Choose process-local providers; execute only the complete customer workflow.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        CounterDefaults {
            size: Size::Regular,
        },
        nz(16),
        nz(4),
    );
    match futures_executor::block_on(save_mayas_order().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Maya's draft failed: {exit:?}"),
    }
    cafe.base.finish();
}
