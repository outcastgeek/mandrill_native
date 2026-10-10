//! Maya and Lee receive receipts; the café builds its receipt book and daily sales total.
//!
//! Run: `cargo run -p effect --example coffee_shop_receipts`.
//! CoffeeOrder, Receipt, and SalesBook are application models. Effect supplies
//! event codecs, journal commits, grouping, and atomic projection checkpoints.
//! The journal/store are local to this process; no payment is collected.
pub mod cafe_support;
use cafe_support::{Cafe, binary, binary_limits, key, nz, parse_policy};
use effect::eventlog::{
    AggregateId, EventCodecError, EventCommitPolicy, EventGroupId, EventHandlerContext, EventId,
    EventJournalId, EventRunError, EventRunOutcome, EventTag, EventVersion, JournalCursor,
    JournalEntry, JournalStream, MemoryJournalError, MemoryProjectionError, ProjectionAdvance,
    ProjectionAdvanceError, ProjectionCaseError, ProjectionId, ProjectionKey, ProjectionVersion,
};
use effect::{
    Effect, Exit, JsonValue, PureCodec, UtcMillis, effect, event, event_group, handled_event,
    json_i64, json_string, projection, schema, schema_enum,
};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::num::NonZeroU32;
use std::sync::Arc;

/// The real order information needed to issue a café receipt.
#[derive(Debug)]
pub struct CoffeeOrder {
    /// Pickup ticket number printed on the receipt.
    number: String,
    /// Customer's pickup name.
    customer: String,
    /// Name of the ordered drink.
    drink: String,
    /// Number of cups served.
    quantity: i64,
    /// Integral price of one cup, in cents.
    unit_cents: i64,
}
/// A customer's receipt is separate from event/journal metadata.
#[derive(Clone, Debug)]
pub struct Receipt {
    /// Original pickup ticket number.
    number: String,
    /// Customer to whom the receipt belongs.
    customer: String,
    /// Description of the served drink.
    drink: String,
    /// Cup count charged by this receipt.
    cups: i64,
    /// Integral amount, in cents.
    total_cents: i64,
}
/// Staff's reconstructed receipt book and its daily sales figures.
#[derive(Clone, Debug, Default)]
pub struct SalesBook {
    /// Actual customer receipts in journal order.
    receipts: Vec<Receipt>,
    /// Total cups served.
    drinks: i64,
    /// Sum of integral receipt amounts, in cents.
    total_cents: i64,
}
/// Business rejection carried by an event outcome.
#[derive(Debug)]
pub enum ReceiptProblem {
    /// A receipt cannot charge zero or a negative number of cups.
    InvalidQuantity,
    /// A receipt cannot use a negative price.
    InvalidPrice,
}
/// A projection retains malformed records and rejected receipts as distinct problems.
#[derive(Debug)]
pub enum BookProblem {
    /// The journal value does not match a registered receipt codec.
    Codec(EventCodecError),
    /// A rejected receipt cannot be counted as a sale.
    Receipt(ReceiptProblem),
}
/// Event and both projection forms preserve their concrete failure channels.
#[derive(Debug)]
pub enum SalesProblem {
    /// Committing a customer receipt failed.
    Receipt(EventRunError<MemoryJournalError, Infallible>),
    /// Reconstructing the raw receipt book failed.
    Book(ProjectionAdvanceError<MemoryJournalError, MemoryProjectionError, BookProblem>),
    /// Reconstructing typed daily sales failed.
    Sales(
        ProjectionAdvanceError<
            MemoryJournalError,
            MemoryProjectionError,
            ProjectionCaseError<BookProblem>,
        >,
    ),
}
/// R selects the authoritative local journal and a SalesBook projection store.
type CoffeeShop = Cafe<(), SalesBook>;
/// Real order payload shared by mobile and counter receipt events.
fn order_schema() -> PureCodec<CoffeeOrder, JsonValue> {
    schema! {
        struct CoffeeOrder { number: json_string(), customer: json_string(), drink: json_string(), quantity: json_i64(), unit_cents: json_i64() }
        policy: parse_policy()
    }.expect("receipt order schema")
}
/// Receipt success values include explicit money units and customer details.
fn receipt_schema() -> PureCodec<Receipt, JsonValue> {
    schema! {
        struct Receipt { number: json_string(), customer: json_string(), drink: json_string(), cups: json_i64(), total_cents: json_i64() }
        policy: parse_policy()
    }.expect("receipt schema")
}
/// Receipt business errors are recorded with their actual variant.
fn problem_schema() -> PureCodec<ReceiptProblem, JsonValue> {
    schema_enum! {
        enum ReceiptProblem { InvalidQuantity => "invalid-quantity", InvalidPrice => "invalid-price" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .expect("receipt problem schema")
}
/// Application calculation produces a receipt, without an executor or journal façade.
fn issue_receipt(order: &CoffeeOrder) -> Result<Receipt, ReceiptProblem> {
    if order.quantity <= 0 {
        return Err(ReceiptProblem::InvalidQuantity);
    }
    if order.unit_cents < 0 {
        return Err(ReceiptProblem::InvalidPrice);
    }
    Ok(Receipt {
        number: order.number.clone(),
        customer: order.customer.clone(),
        drink: order.drink.clone(),
        cups: order.quantity,
        total_cents: order.quantity * order.unit_cents,
    })
}
/// Pure business fold counts only successfully issued receipts.
fn add_receipt(
    book: &SalesBook,
    receipt: Result<Receipt, ReceiptProblem>,
) -> Result<SalesBook, BookProblem> {
    let receipt = receipt.map_err(BookProblem::Receipt)?;
    let mut next = book.clone();
    next.drinks += receipt.cups;
    next.total_cents += receipt.total_cents;
    next.receipts.push(receipt);
    Ok(next)
}
/// Typed-case helper keeps payload and decoded outcome visible to a staff reader.
fn count_sale(
    book: &SalesBook,
    _order: CoffeeOrder,
    receipt: Result<Receipt, ReceiptProblem>,
) -> Result<SalesBook, BookProblem> {
    add_receipt(book, receipt)
}
/// Daily aggregate joins the two customer channels into one café sales stream.
fn sales_day() -> AggregateId {
    AggregateId::from_stable(key("today-coffee-sales"))
}
/// Human-facing receipt formatting always names the cent-to-dollar conversion.
fn show_receipt(receipt: &Receipt) {
    println!(
        "Receipt {} — {}: {} {} cup(s), ${}.{:02}.",
        receipt.number,
        receipt.customer,
        receipt.cups,
        receipt.drink,
        receipt.total_cents / 100,
        receipt.total_cents % 100
    );
}
/// Commit two customer receipts and build independently checkpointed staff views.
fn receipt_book() -> Effect<(), SalesProblem, CoffeeShop> {
    effect! {
        let group = EventGroupId::new("coffee-shop-receipts").expect("receipt group");
        let mobile = event! {
            group: group.clone(), tag: EventTag::new("mobile-receipt").expect("event tag"), version: EventVersion::new(NonZeroU32::MIN),
            payload: order_schema(), success: receipt_schema(), error: problem_schema(), payload_identity: binary("receipt-order"),
            success_identity: binary("customer-receipt"), error_identity: binary("receipt-problem"), limits: binary_limits() };
        let mobile_receipt = handled_event! {
            definition: mobile.clone(), aggregate: |_: &CoffeeOrder| sales_day(), commit: EventCommitPolicy::SuccessOnly,
            handler |order: Arc<CoffeeOrder>| { issue_receipt(&order) }
        };
        let counter = event! {
            group: group.clone(), tag: EventTag::new("counter-receipt").expect("event tag"), version: EventVersion::new(NonZeroU32::MIN),
            payload: order_schema(), success: receipt_schema(), error: problem_schema(), payload_identity: binary("receipt-order"),
            success_identity: binary("customer-receipt"), error_identity: binary("receipt-problem"), limits: binary_limits(), aggregate: |_: &CoffeeOrder| sales_day() };
        let counter_receipt = handled_event! {
            definition: counter, commit: EventCommitPolicy::SuccessOnly,
            handler |context: EventHandlerContext, order: Arc<CoffeeOrder>| {
                println!("Sam files counter ticket {} after {} earlier sale(s).", order.number, context.expected.value());
                issue_receipt(&order)
            }
        };
        let channels = event_group! { group: group.clone(), events: [&mobile, &counter_receipt] }.expect("receipt channels");
        println!("The café accepts mobile and counter receipts in {}.", channels.id().as_str());
        let journal = EventJournalId::from_stable(key("coffee-register"));
        for outcome in [
            mobile_receipt.execute(journal.clone(), EventId::from_stable(key("receipt-104")), JournalCursor::new(0), UtcMillis::new(0),
                Arc::new(CoffeeOrder { number: "104".into(), customer: "Maya".into(), drink: "oat latte".into(), quantity: 2, unit_cents: 475 })).map_error(SalesProblem::Receipt)?,
            counter_receipt.execute(journal.clone(), EventId::from_stable(key("receipt-105")), JournalCursor::new(1), UtcMillis::new(1),
                Arc::new(CoffeeOrder { number: "105".into(), customer: "Lee".into(), drink: "americano".into(), quantity: 1, unit_cents: 325 })).map_error(SalesProblem::Receipt)?,
        ] {
            match outcome {
                EventRunOutcome::Committed { result: Ok(receipt), .. } => show_receipt(&receipt),
                _ => println!("The café could not issue this receipt."),
            }
        }
        let counter_definition = counter_receipt.definition().clone();
        let raw_mobile = mobile.clone(); let raw_counter = counter_definition.clone();
        let accepted = BTreeMap::from([
            ((group.clone(), EventTag::new("mobile-receipt").expect("tag")), EventVersion::new(NonZeroU32::MIN)),
            ((group, EventTag::new("counter-receipt").expect("tag")), EventVersion::new(NonZeroU32::MIN)),
        ]);
        let stream = JournalStream { journal, aggregate: sales_day() };
        let register_book = projection! {
            key: ProjectionKey { id: ProjectionId::from_stable(key("register-receipt-book")), stream: stream.clone() },
            version: ProjectionVersion::new(NonZeroU32::MIN), accepted: accepted, initial: SalesBook::default,
            fold: move |book: &SalesBook, entry: &JournalEntry| {
                let definition = if entry.event.tag.as_str() == "mobile-receipt" { &raw_mobile } else { &raw_counter };
                let receipt = definition.decode_outcome(&entry.event).map_err(BookProblem::Codec)?;
                add_receipt(book, receipt)
            }
        };
        let daily_sales = projection! {
            key: ProjectionKey { id: ProjectionId::from_stable(key("daily-drink-sales")), stream: stream },
            version: ProjectionVersion::new(NonZeroU32::MIN), initial: SalesBook::default,
            cases: [mobile => count_sale, counter_definition => |book: &SalesBook, _order: CoffeeOrder, receipt: Result<Receipt, ReceiptProblem>| add_receipt(book, receipt)]
        }.expect("daily sales cases");
        if let ProjectionAdvance::Advanced { snapshot, .. } = register_book.advance(nz(4)).map_error(SalesProblem::Book)? {
            println!("Sam's register book lists:"); for receipt in &snapshot.state.receipts { show_receipt(receipt); }
        }
        if let ProjectionAdvance::Advanced { snapshot, .. } = daily_sales.advance(nz(4)).map_error(SalesProblem::Sales)? {
            println!("Daily café sales: {} receipts, {} drinks, ${}.{:02}.", snapshot.state.receipts.len(), snapshot.state.drinks,
                snapshot.state.total_cents / 100, snapshot.state.total_cents % 100);
        }
    }
}
/// Execute the complete receipt story using local journal/projection capabilities.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open((), nz(16), nz(4));
    match futures_executor::block_on(receipt_book().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Café receipts failed: {exit:?}"),
    }
    cafe.base.finish();
}
