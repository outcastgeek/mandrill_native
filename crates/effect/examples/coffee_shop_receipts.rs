//! Maya and Lee receive receipts; the café builds its receipt book and daily sales total.
//!
//! Run: `cargo +1.99.0 run -p effect --example coffee_shop_receipts`.
//! Add `-- --book` to print the detailed receipt book instead of the daily total.
//! Input: Maya buys two 475-cent lattes; Lee buys one 325-cent americano.
//! Expected daily total: two receipts, three drinks, 1275 cents ($12.75).
//! CoffeeOrder, Receipt, and SalesBook are application models. Effect supplies
//! event codecs, journal commits, grouping, and atomic projection checkpoints.
//! The journal/store are local to this process; no payment is collected.

use effect::{effect_die, effect_map_error};
pub mod cafe_support;
use cafe_support::{Cafe, binary, binary_limits, key, parse_policy};
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
use std::num::NonZeroUsize;
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
fn order_schema() -> Result<PureCodec<CoffeeOrder, JsonValue>, effect::Defect> {
    schema! {
        struct CoffeeOrder { number: json_string(), customer: json_string(), drink: json_string(), quantity: json_i64(), unit_cents: json_i64() }
        policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("receipt order schema: {error:?}")))
}
/// Receipt success values include explicit money units and customer details.
fn receipt_schema() -> Result<PureCodec<Receipt, JsonValue>, effect::Defect> {
    schema! {
        struct Receipt { number: json_string(), customer: json_string(), drink: json_string(), cups: json_i64(), total_cents: json_i64() }
        policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("receipt schema: {error:?}")))
}
/// Receipt business errors are recorded with their actual variant.
fn problem_schema() -> Result<PureCodec<ReceiptProblem, JsonValue>, effect::Defect> {
    schema_enum! {
        enum ReceiptProblem { InvalidQuantity => "invalid-quantity", InvalidPrice => "invalid-price" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("receipt problem schema: {error:?}")))
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
fn sales_day() -> Result<AggregateId, effect::Defect> {
    key("today-coffee-sales").map(AggregateId::from_stable)
}
/// Human-facing receipt formatting always names the cent-to-dollar conversion.
fn show_receipt(receipt: &Receipt) -> Result<(), effect::Defect> {
    {
        use std::io::Write;
        writeln!(
            std::io::stdout().lock(),
            "Receipt {} — {}: {} {} cup(s), ${}.{:02}.",
            receipt.number,
            receipt.customer,
            receipt.cups,
            receipt.drink,
            receipt.total_cents / 100,
            receipt.total_cents % 100
        )
        .map_err(effect::Defect::from_io)?;
    };
    Ok(())
}
/// Commit two customer receipts and build independently checkpointed staff views.
fn receipt_book(detailed: bool) -> Effect<(), SalesProblem, CoffeeShop> {
    effect! {
        let day = match sales_day() { Ok(day) => day, Err(error) => effect_die! { defect: error, }?, };
        let mobile_day = day.clone(); let counter_day = day.clone();
        let group = match EventGroupId::new("coffee-shop-receipts") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("receipt group: {error:?}")), }?, };
        let mobile_tag = match EventTag::new("mobile-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("event tag: {error:?}")), }?, };
        let mobile_order_codec = match order_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile_receipt_codec = match receipt_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile_problem_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile_payload_identity = match binary("receipt-order") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile_success_identity = match binary("customer-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile_error_identity = match binary("receipt-problem") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let mobile = event! {
            group: group.clone(), tag: mobile_tag, version: EventVersion::new(NonZeroU32::MIN),
            payload: mobile_order_codec, success: mobile_receipt_codec, error: mobile_problem_codec, payload_identity: mobile_payload_identity,
            success_identity: mobile_success_identity, error_identity: mobile_error_identity, limits: binary_limits() };
        let mobile_receipt = handled_event! {
            definition: mobile.clone(), aggregate: move |_: &CoffeeOrder| mobile_day.clone(), commit: EventCommitPolicy::SuccessOnly,
            handler |order: Arc<CoffeeOrder>| { issue_receipt(&order) }
        };
        let counter_tag = match EventTag::new("counter-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("event tag: {error:?}")), }?, };
        let counter_order_codec = match order_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter_receipt_codec = match receipt_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter_problem_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter_payload_identity = match binary("receipt-order") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter_success_identity = match binary("customer-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter_error_identity = match binary("receipt-problem") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let counter = event! {
            group: group.clone(), tag: counter_tag, version: EventVersion::new(NonZeroU32::MIN),
            payload: counter_order_codec, success: counter_receipt_codec, error: counter_problem_codec, payload_identity: counter_payload_identity,
            success_identity: counter_success_identity, error_identity: counter_error_identity, limits: binary_limits(), aggregate: move |_: &CoffeeOrder| counter_day.clone() };
        let counter_receipt = handled_event! {
            definition: counter, commit: EventCommitPolicy::SuccessOnly,
            handler |context: EventHandlerContext, order: Arc<CoffeeOrder>| {
                let result = issue_receipt(&order);
                { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Sam files counter ticket {} after {} earlier sale(s).", order.number, context.expected.value()); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
                result
            }
        };
        let channels = match event_group! { group: group.clone(), events: [&mobile, &counter_receipt] } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("receipt channels: {error:?}")), }?, };
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "The café accepts mobile and counter receipts in {}.", channels.id().as_str()); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        let journal = EventJournalId::from_stable(match key("coffee-register") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
        for outcome in [
            {
                let receipt_journal = journal.clone();
                let mobile_event = EventId::from_stable(match key("receipt-104") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
                effect_map_error! {
                program: mobile_receipt.execute(receipt_journal, mobile_event, JournalCursor::new(0), UtcMillis::new(0),
                Arc::new(CoffeeOrder { number: "104".into(), customer: "Maya".into(), drink: "oat latte".into(), quantity: 2, unit_cents: 475 })), map: SalesProblem::Receipt, }?
            },
            {
                let receipt_journal = journal.clone();
                let counter_event = EventId::from_stable(match key("receipt-105") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
                effect_map_error! {
                program: counter_receipt.execute(receipt_journal, counter_event, JournalCursor::new(1), UtcMillis::new(1),
                Arc::new(CoffeeOrder { number: "105".into(), customer: "Lee".into(), drink: "americano".into(), quantity: 1, unit_cents: 325 })), map: SalesProblem::Receipt, }?
            },
        ] {
            match outcome {
                EventRunOutcome::Committed { result: Ok(receipt), .. } => match show_receipt(&receipt) { Ok(()) => (), Err(error) => effect_die! { defect: error, }?, },
                _ => { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "The café could not issue this receipt."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
            }
        }
        let counter_definition = counter_receipt.definition().clone();
        let raw_mobile = mobile.clone(); let raw_counter = counter_definition.clone();
        let accepted = BTreeMap::from([
            ((group.clone(), match EventTag::new("mobile-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("tag: {error:?}")), }?, }), EventVersion::new(NonZeroU32::MIN)),
            ((group, match EventTag::new("counter-receipt") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("tag: {error:?}")), }?, }), EventVersion::new(NonZeroU32::MIN)),
        ]);
        let stream = JournalStream { journal, aggregate: day };
        let receipt_book_key = match key("register-receipt-book") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let register_book = projection! {
            key: ProjectionKey { id: ProjectionId::from_stable(receipt_book_key), stream: stream.clone() },
            version: ProjectionVersion::new(NonZeroU32::MIN), accepted: accepted, initial: SalesBook::default,
            fold |book: &SalesBook, entry: &JournalEntry| {
                let definition = if entry.event.tag.as_str() == "mobile-receipt" { &raw_mobile } else { &raw_counter };
                let receipt = definition.decode_outcome(&entry.event).map_err(BookProblem::Codec)?;
                add_receipt(book, receipt)
            }
        };
        let daily_sales_key = match key("daily-drink-sales") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let daily_sales = match projection! {
            key: ProjectionKey { id: ProjectionId::from_stable(daily_sales_key), stream },
            version: ProjectionVersion::new(NonZeroU32::MIN), initial: SalesBook::default,
            cases: [mobile => count_sale, counter_definition => |book: &SalesBook, _order: CoffeeOrder, receipt: Result<Receipt, ReceiptProblem>| add_receipt(book, receipt)]
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("daily sales cases: {error:?}")), }?, };
        if detailed {
            if let ProjectionAdvance::Advanced { snapshot, .. } = effect_map_error! { program: register_book.advance(NonZeroUsize::MIN.saturating_add(3)), map: SalesProblem::Book, }? {
                { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Sam's register book lists:"); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } }; for receipt in &snapshot.state.receipts { match show_receipt(receipt) { Ok(()) => (), Err(error) => effect_die! { defect: error, }?, }; }
            }
        } else {
            if let ProjectionAdvance::Advanced { snapshot, .. } = effect_map_error! { program: daily_sales.advance(NonZeroUsize::MIN.saturating_add(3)), map: SalesProblem::Sales, }? {
                {
                    use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Daily café sales: {} receipts, {} drinks, ${}.{:02}.", snapshot.state.receipts.len(), snapshot.state.drinks,
                    snapshot.state.total_cents / 100, snapshot.state.total_cents % 100); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
            }
        }
    }
}
/// Execute the complete receipt story using local journal/projection capabilities.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    match futures_executor::block_on(
        receipt_book(std::env::args().skip(1).any(|arg| arg == "--book")).run(cafe.clone()),
    ) {
        Exit::Success(()) => (),
        exit @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("Café receipts failed: {exit:?}").into());
        }
    }
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    Ok(())
}
