//! Maya corrects an online coffee order, saves her draft, and reloads it later.
//!
//! Run: `cargo +1.99.0 run -p effect --example save_coffee_order`.
//! CoffeeOrder, Drink, Milk, Size, and OrderProblem are application models.
//! Effect's schemas validate their wire shape; the café validates business rules.
//! The draft store is local memory and does not survive restarting this example.
//! Input: quantity 0, corrected to 2 oat lattes with an extra-hot note.
//! Expected: validation feedback, then the same two drinks and note after reload.

use effect::{effect_die, effect_map_error};
pub mod cafe_support;
use cafe_support::{Cafe, CafeKeys, limits, parse_policy};
use effect::codec;
use effect::effect_schema_parse;
use effect::effect_try_sync;
use effect::persistence::{
    MemoryStoreError, Persistable, PersistenceError, RecordPolicy, RecordVersion, StoredLookup,
    read, write,
};
use effect::{
    Codec, Effect, Exit, JsonRepresentation, JsonValue, KeyNamespace, PureCodec, SchemaIssue,
    StoredOutcome, UnknownFields, effect, json_i64, json_nullable, json_string, persistable,
    schema, schema_enum, service,
};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Maya fixes quantity zero, accepts the defaults, and reopens her saved draft.
fn save_mayas_order() -> Effect<(), DraftProblem, CoffeeShop> {
    effect! {
        let parse = parse_policy();
        // Reuse the café's existing depth and byte envelope. Scalar and digit
        // counts are bounded by input bytes; pattern_steps is the café's explicit
        // search-work budget chosen from that same envelope.
        let form = match match order_form() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }.configured(effect::SchemaPolicy {
            parse, json_depth: parse.limits.depth, string_length: parse.limits.bytes,
            integer_digits: parse.limits.bytes, pattern_steps: parse.limits.bytes,
        }) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("bounded online order form: {error:?}")), }?, };
        let accidental = effect_schema_parse! {
            codec: &form, text: r#"{"customer":"Maya","drink":{"drink":"latte","options":{"milk":{"kind":"oat","details":null}}},"quantity":0}"#,
            on_error |problem| { DraftProblem::Form(problem) },
        }?;
        let feedback = effect_map_error! { program: accept_order(accidental).materialize_exit(), map: |never| match never {}, }?;
        if let Exit::Fail(OrderProblem::InvalidQuantity { maximum }) = feedback {
            { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya sees: choose between 1 and {maximum} drinks; quantity 0 cannot be saved."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        }
        let edited = effect_schema_parse! {
            codec: &form, text: r#"{"customer":"Maya","drink":{"drink":"latte","options":{"milk":{"kind":"oat","details":null}}},"quantity":2,"note":"extra hot"}"#,
            on_error |problem| { DraftProblem::Form(problem) },
        }?;
        let order = Arc::new(effect_map_error! { program: accept_order(edited), map: DraftProblem::Order, }?);
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Saving {}'s {:?} {:?}, quantity {}, note {:?}.", order.customer, order.size, order.drink, order.quantity, order.note); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        let draft_namespace = match KeyNamespace::<CafeKeys>::new("coffee-drafts") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("draft namespace: {error:?}")), }?, };
        let draft_codec = match saved_draft_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let feedback_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let draft_version = match RecordVersion::new("coffee-order-v1") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("draft version: {error:?}")), }?, };
        let drafts: Persistable<String, Arc<CoffeeOrder>, OrderProblem, CafeKeys> = persistable! {
            namespace: draft_namespace, request: String, key: draft_identity::<String>,
            success: draft_codec, error: feedback_codec, version: RecordPolicy::new(draft_version, limits())
        };
        effect_map_error! { program: write(drafts.clone(), || "maya-morning-draft".to_owned(), move || StoredOutcome::Success(order.clone()), None), map: DraftProblem::Storage, }?;
        let saved = effect_map_error! { program: read(drafts, || "maya-morning-draft".to_owned(), effect::UtcMillis::new(0)), map: DraftProblem::Storage, }?;
        match saved {
            StoredLookup::Found { outcome: StoredOutcome::Success(order), .. } => { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Welcome back, {}: your {} oat lattes and preparation note are restored.", order.customer, order.quantity); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
            other => { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Draft lookup returned {other:?}"); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
        }
    }
}
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
fn milk_schema() -> Result<PureCodec<Milk, JsonValue>, effect::Defect> {
    schema_enum! {
        enum Milk { Dairy => "dairy", Oat => "oat" }
        tag_field: "kind", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("milk schema: {error:?}")))
}
/// Unit, named-field, and tuple variants describe actual drink choices.
fn drink_schema() -> Result<PureCodec<Drink, JsonValue>, effect::Defect> {
    let milk = milk_schema()?;
    schema_enum! {
        enum Drink {
            Americano => "americano", Latte { milk: milk.clone() } => "latte", Cappuccino(milk) => "cappuccino"
        } tag_field: "drink", content_field: "options", policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("drink schema: {error:?}")))
}
/// Named cup sizes have explicit stable wire tags.
fn size_schema() -> Result<PureCodec<Size, JsonValue>, effect::Defect> {
    schema_enum! {
        enum Size { Regular => "regular", Large => "large" }
        tag_field: "kind", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("size schema: {error:?}")))
}
/// Pure record codec is used to store and later borrow/decode a complete draft.
fn order_schema() -> Result<PureCodec<CoffeeOrder, JsonValue>, effect::Defect> {
    let drink = drink_schema()?;
    let size = size_schema()?;
    schema! {
        struct CoffeeOrder {
            customer: json_string(), drink: drink, size: size,
            quantity: json_i64() => (|| 1, |quantity: &i64| *quantity == 1, Arc::new(JsonValue::from(1))),
            note: json_nullable(json_string()) => (|| None, Option::is_none)
        } policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("stored order schema: {error:?}")))
}
/// Share an immutable saved draft while decoding a fresh owned model from each stored record.
fn saved_draft_schema() -> Result<PureCodec<Arc<CoffeeOrder>, JsonValue>, effect::Defect> {
    let schema = order_schema()?;
    let decode = schema.clone();
    Ok(codec! {
        pure;
        decode |json: &JsonValue| { decode.decode_ref(json).map(Arc::new) },
        encode |order: &Arc<CoffeeOrder>| { schema.encode_ref(order.as_ref()) },
    })
}
/// The online editor defaults size through the café service and quantity to one.
fn order_form() -> Result<Codec<CoffeeOrder, JsonValue, CoffeeShop, CoffeeShop>, effect::Defect> {
    let drink = drink_schema()?;
    let size = size_schema()?;
    schema! {
        struct CoffeeOrder {
            customer: json_string().into_effect::<CoffeeShop, CoffeeShop>(),
            drink: drink.into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(default_with = || service! {
                select: |cafe: &CoffeeShop| cafe.services.clone(),
                on_error: |problem: SchemaIssue| problem,
                run |defaults: &Arc<CounterDefaults>| { defaults.size.clone() }
            })]
            size: size.into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(default_with = || effect! { 1i64 })]
            quantity: json_i64().into_effect::<CoffeeShop, CoffeeShop>(),
            #[schema(optional)] note: json_nullable(json_string()).into_effect::<CoffeeShop, CoffeeShop>()
        } policy: parse_policy(), representation: JsonRepresentation, unknown_fields: UnknownFields::Reject
    }.map_err(|error| effect::Defect::new(format!("online order form: {error:?}")))
}
/// Typed order failures can also be stored without reducing them to strings.
fn problem_schema() -> Result<PureCodec<OrderProblem, JsonValue>, effect::Defect> {
    schema_enum! {
        enum OrderProblem { MissingName => "missing-name", InvalidQuantity { maximum: json_i64() } => "quantity" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("order feedback schema: {error:?}")))
}
/// Keep stable draft identity separate from the order's mutable contents.
fn draft_identity<T: AsRef<str> + ?Sized>(identity: &T) -> &str {
    identity.as_ref()
}
/// Café validation belongs to the application, after schema shape validation.
fn accept_order(order: CoffeeOrder) -> Effect<CoffeeOrder, OrderProblem, CoffeeShop> {
    effect_try_sync! {
        once;
        body {
            if order.customer.trim().is_empty() {
                return Err(OrderProblem::MissingName);
            }
            if !(1..=4).contains(&order.quantity) {
                return Err(OrderProblem::InvalidQuantity { maximum: 4 });
            }
            Ok(order)
        },
    }
}
/// Choose process-local providers; execute only the complete customer workflow.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        CounterDefaults {
            size: Size::Regular,
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    match futures_executor::block_on(save_mayas_order().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("Maya's draft failed: {exit:?}").into());
        }
    }
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    Ok(())
}
