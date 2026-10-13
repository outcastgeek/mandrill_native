//! Maya adds and removes drinks, reserves stock, and restores her saved basket.
//!
//! Run: `cargo +1.99.0 run -p effect --example live_coffee_basket`.
//! Basket, BasketTotal, and Stock are café models. Effect supplies tracked reads,
//! atomic stock commits, reactive query subscriptions, and hydration boundaries.
//! Expected totals: 3 drinks/$12.75, then 2/$8.00; restored basket is 2/$8.00.
//! Reservation leaves 3 americanos/2 oat lattes; restock adds one oat-latte portion.

use effect::{effect_die, effect_map, effect_map_error};
pub mod cafe_support;
use cafe_support::{Cafe, key, limits, parse_policy};
use effect::persistence::RecordVersion;
use effect::reactivity::{
    AsyncResultPhase, Atom, AtomError, AtomId, AtomProgram, AtomReadContext, AtomRef, AtomRefError,
    AtomRegistry, EffectStreamSource, HydrationConflictPolicy, HydrationError, HydrationSchema,
    HydrationSet, InvalidationKey, InvalidationRegistry, ReactiveQuery, ReactiveQueryError,
    ReactiveStateSource,
};
use effect::transaction;
use effect::transactional::{
    Transaction, TransactionDomain, TransactionError, TransactionPolicy, TxRef, TxValue,
};
use effect::{
    Chunk, Effect, Exit, PubSubError, PubSubOptions, PubSubOverflow, PubSubShutdown, ReplayPolicy,
    Scope, atom, effect, effect_scope, hydration, json_i64, json_string, reactive_bridge,
    reactive_derived, reactive_query, reactive_source, schema, service, stream,
};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Drinks Maya has selected; counts are quantities, never prices.
#[derive(Clone, Debug)]
pub struct Basket {
    /// Pickup name associated with this basket.
    customer: String,
    /// Americano cups at 325 cents each.
    americanos: i64,
    /// Oat latte cups at 475 cents each.
    oat_lattes: i64,
}
/// A tracked basket view with explicit monetary units.
#[derive(Debug)]
pub struct BasketTotal {
    /// Number of drinks selected.
    drinks: i64,
    /// Total price expressed in integral cents.
    cents: i64,
}
/// Available ingredients/cups at the café counter.
#[derive(Clone, Debug)]
pub struct Stock {
    /// Americano cups still available.
    americanos: i64,
    /// Oat latte portions still available.
    oat_lattes: i64,
}
/// Stock snapshots are immutable transaction values replaced by atomic commits.
impl TxValue for Stock {}
/// Customer feedback when a reservation cannot commit.
#[derive(Debug)]
pub enum StockProblem {
    /// Another customer has already taken the last requested drink.
    SoldOut,
}
/// A total calculation identifies a failed tracked basket dependency.
#[derive(Debug)]
pub enum BasketReadProblem {
    /// Original registry/read failure from this basket's dependency.
    Basket(AtomRefError),
}
/// Basket errors retain distinct model, hydration, query, and transaction channels.
#[derive(Debug)]
pub enum BasketProblem {
    /// Reading or changing the basket failed.
    Basket(AtomRefError),
    /// The mapped total view failed while retaining its original tracked read error.
    View(AtomError<BasketReadProblem>),
    /// The customer's label dependency failed.
    Label(AtomError<AtomRefError>),
    /// Saving or restoring state failed.
    Saved(HydrationError),
    /// Atomic stock reservation failed.
    Stock(TransactionError<StockProblem>),
    /// Stock availability subscription failed.
    Query(ReactiveQueryError),
    /// External display stream failed.
    Display(PubSubError),
}
/// This story needs no additional business service beyond its typed local values.
type CoffeeShop = Cafe;
/// Four queued basket updates are enough for this customer's finite visit.
fn basket_updates() -> PubSubOptions {
    PubSubOptions {
        capacity: NonZeroUsize::MIN.saturating_add(3),
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }
}
/// Extracted tracked helper observes the basket through the caller's read context.
fn total_for(basket: AtomRef<Basket>) -> AtomProgram<BasketTotal, BasketReadProblem, CoffeeShop> {
    let total: AtomProgram<BasketTotal, AtomRefError, CoffeeShop> = atom! {
        let basket = basket.clone();
        let selected = AtomProgram::read(basket)?;
        BasketTotal {
            drinks: selected.americanos + selected.oat_lattes,
            cents: selected.americanos * 325 + selected.oat_lattes * 475 }
    };
    atom! { map_error; program: total, map |error: AtomRefError| { BasketReadProblem::Basket(error) }, }
}
/// Reserve all requested drinks together; no partial reservation is committed.
fn reserve_stock(stock: TxRef<Stock>, basket: Arc<Basket>) -> Transaction<Stock, StockProblem> {
    let reservation: Transaction<Stock, &'static str> = transaction! {
        modify;
        reference: stock,
        modify |available: &Stock| {
            if basket.americanos > available.americanos || basket.oat_lattes > available.oat_lattes {
                return Err("requested drinks exceed stock");
            }
            let remaining = Stock {
                americanos: available.americanos - basket.americanos,
                oat_lattes: available.oat_lattes - basket.oat_lattes,
            };
            Ok((remaining.clone(), remaining))
        },
    };
    transaction! { map_error; program: reservation, map |_message: &'static str| { StockProblem::SoldOut }, }
}
/// Show a settled stock result, then close this preview subscription.
fn show_stock(
    query: ReactiveQuery<Stock, TransactionError<StockProblem>, CoffeeShop>,
) -> Effect<(), ReactiveQueryError, CoffeeShop> {
    effect_scope! {
        body |scope: Scope<ReactiveQueryError>| {
            let changes = query.stream();
            let scope = scope.clone(); let changes = changes.clone();
            let reader = changes.open(scope)?;
            let mut settled = false;
            while !settled {
                match reader.pull()? {
                    Chunk::Values(states) => for state in states {
                        if state.phase() == AsyncResultPhase::Settled {
                            settled = true;
                            if let Some(stock) = state.current_success() {
                                let text = match stock.with(|stock| format!("Counter stock: {} americanos, {} oat lattes.", stock.americanos, stock.oat_lattes)) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("stock snapshot: {error:?}")), }?, };
                                use std::io::Write; let written = writeln!(std::io::stdout().lock(), "{text}"); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                            }
                        }
                    },
                    Chunk::End => break,
                }
            }
        },
    }
}
/// Refresh the tracked total and show what the customer sees.
fn show_basket(
    total: Atom<BasketTotal, BasketReadProblem, CoffeeShop>,
) -> Effect<(), AtomError<BasketReadProblem>, CoffeeShop> {
    effect! {
        let total = total.clone();
        total.refresh()?;
        let state = total.read()?;
        let text = state.current_success().ok_or_else(|| effect::Defect::new("refreshed basket"))
            .and_then(|snapshot| snapshot.with(|total| format!("Maya's basket: {} drinks, ${}.{:02}.", total.drinks, total.cents / 100, total.cents % 100)));
        let text = match text { Ok(text) => text, Err(error) => effect_die! { defect: error, }?, };
        use std::io::Write;
        let written = writeln!(std::io::stdout().lock(), "{text}");
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
    }
}
/// Replay finite customer display changes from owned application snapshots.
fn basket_history(
    history: Arc<Vec<Arc<Basket>>>,
) -> effect::Stream<Arc<Basket>, PubSubError, CoffeeShop> {
    stream! { let history = history.clone(); for basket in history.iter() { yield basket.clone(); } }
}
/// Follow Maya's actual changes, save before checkout, and restore after clearing.
fn fill_basket() -> Effect<(), BasketProblem, CoffeeShop> {
    effect! {
        let registry = match AtomRegistry::new(basket_updates()) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket registry: {error:?}")), }?, };
        let basket_key = match key("maya-basket") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let basket = match reactive_source! {
            registry: registry.clone(), id: AtomId::from_stable(basket_key),
            initial: Basket { customer: "Maya".into(), americanos: 0, oat_lattes: 0 }, capacity: NonZeroUsize::MIN.saturating_add(3), overflow: PubSubOverflow::Backpressure
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket source: {error:?}")), }?, };
        let total_key = match key("maya-basket-total") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let total = match reactive_derived! {
            registry: registry.clone(), id: AtomId::from_stable(total_key), capacity: NonZeroUsize::MIN.saturating_add(3),
            overflow: PubSubOverflow::Backpressure, program: total_for(basket.clone())
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket total: {error:?}")), }?, };
        let selected = basket.clone();
        let label_key = match key("maya-basket-label") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let label = match reactive_derived! {
            registry: registry.clone(), id: AtomId::from_stable(label_key), capacity: NonZeroUsize::MIN.saturating_add(3),
            overflow: PubSubOverflow::Backpressure,
            evaluate: move |context: AtomReadContext| effect_map! { program: context.read::<Basket, CoffeeShop>(&selected), map: |basket| format!("{}'s coffee basket", basket.customer), }
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket label: {error:?}")), }?, };
        effect_map_error! { program: basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 1, oat_lattes: 2 }), map: BasketProblem::Basket, }?;
        effect_map_error! { program: show_basket(total.clone()), map: BasketProblem::View, }?;
        let before = effect_map_error! { program: basket.read::<CoffeeShop>(), map: BasketProblem::Basket, }?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya removes one oat latte before checkout."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_map_error! { program: basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 1, oat_lattes: 1 }), map: BasketProblem::Basket, }?;
        effect_map_error! { program: show_basket(total.clone()), map: BasketProblem::View, }?;
        effect_map_error! { program: label.refresh(), map: BasketProblem::Label, }?;
        let label_state = effect_map_error! { program: label.read(), map: BasketProblem::Label, }?;
        let text = label_state.current_success().ok_or_else(|| effect::Defect::new("basket label"))
            .and_then(|snapshot| snapshot.with(|label| format!("Customer display: {label}")));
        let text = match text { Ok(text) => text, Err(error) => effect_die! { defect: error, }?, };
        use std::io::Write;
        let written = writeln!(std::io::stdout().lock(), "{text}");
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        let after = effect_map_error! { program: basket.read::<CoffeeShop>(), map: BasketProblem::Basket, }?;
        let basket_version = match RecordVersion::new("coffee-basket-v1") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket version: {error:?}")), }?, };
        let basket_codec = match schema! { struct Basket { customer: json_string(), americanos: json_i64(), oat_lattes: json_i64() } policy: parse_policy() } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("basket schema: {error:?}")), }?, };
        let saved: HydrationSet<CoffeeShop> = match hydration! {
            registry: registry.clone(), limits: limits(), bindings: [basket.clone() => HydrationSchema::new(
                basket.id().clone(), basket_version,
                basket_codec, limits())]
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("saved basket binding: {error:?}")), }?, };
        let snapshot = effect_map_error! { program: saved.export_all(), map: BasketProblem::Saved, }?;
        let checkout_basket = after.clone();
        effect! {
        let after = checkout_basket.clone();
        let stock = service! {
            select: |cafe: &CoffeeShop| cafe.transactions.clone(),
            on_error: |problem: BasketProblem| problem,
            run |journal: &Arc<TransactionDomain>| {
                match journal.reference(Stock { americanos: 4, oat_lattes: 3 }) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("counter stock: {error:?}")), }?, }
            }
        }?;
        let observed_stock = stock.clone();
        let stock_registry = match InvalidationRegistry::new(basket_updates()) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("stock registry: {error:?}")), }?, };
        let stock_key = match key("counter-stock") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let availability: ReactiveQuery<Stock, TransactionError<StockProblem>, CoffeeShop> = reactive_query! {
            registry: stock_registry, dependencies: [InvalidationKey::from_stable(stock_key)],
            evaluate: move || effect_map! { program: observed_stock.read::<StockProblem>().atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }), map: |stock| (*stock).clone(), }
        };
        effect_map_error! { program: show_stock(availability.clone()), map: BasketProblem::Query, }?;
        let remaining = effect_map_error! { program: reserve_stock(stock.clone(), after.clone()).atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }), map: BasketProblem::Stock, }?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Reserved Maya's two drinks together; {} oat-latte portions remain.", remaining.oat_lattes); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_map_error! { program: show_stock(availability.clone()), map: BasketProblem::Query, }?;
        effect_map_error! { 
            program: transaction! {
            update; reference: stock.clone(),
            update |available: &Stock| { Ok::<_, StockProblem>(Stock { americanos: available.americanos, oat_lattes: available.oat_lattes + 1 }) },
        }.atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }), map: BasketProblem::Stock, }?;
        let replenished = effect_map_error! { program: stock.read::<StockProblem>().atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }), map: BasketProblem::Stock, }?;
        use std::io::Write;
        let (replenished, written) = {
            let written = writeln!(std::io::stdout().lock(), "Staff restocks one oat-latte portion; {} are now available.", replenished.oat_lattes);
            (replenished, written)
        };
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        let excessive = Arc::new(Basket { customer: "Maya".into(), americanos: 0, oat_lattes: 10 });
        let rejected = effect_map_error! { program: reserve_stock(stock.clone(), excessive).atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }).either(), map: BasketProblem::Stock, }?;
        assert!(matches!(rejected, Err(TransactionError::Domain(StockProblem::SoldOut))));
        let unchanged = effect_map_error! { program: stock.read::<StockProblem>().atomically(TransactionPolicy { max_conflicts: NonZeroUsize::MIN.saturating_add(3) }), map: BasketProblem::Stock, }?;
        assert_eq!((unchanged.americanos, unchanged.oat_lattes), (replenished.americanos, replenished.oat_lattes));
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "A larger request is rejected as SoldOut; stock remains {} americanos and {} oat-latte portions.", unchanged.americanos, unchanged.oat_lattes); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        }?;
        let display_basket = basket.clone();
        let history = Arc::new(vec![before, after]);
        let display: EffectStreamSource<Arc<Basket>, AtomRefError, PubSubError, CoffeeShop> = reactive_bridge! {
            read: move || display_basket.read::<CoffeeShop>(),
            changes: move || basket_history(history.clone())
        };
        let latest = effect_map_error! { program: display.read(), map: BasketProblem::Basket, }?;
        let displayed = effect_map_error! { program: display.changes().run_collect(Some(2)), map: BasketProblem::Display, }?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Customer display replayed {} basket changes; latest is {} drinks.", displayed.len(), latest.americanos + latest.oat_lattes); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_map_error! { program: basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 0, oat_lattes: 0 }), map: BasketProblem::Basket, }?;
        effect_map_error! { program: saved.import_all(snapshot, HydrationConflictPolicy::ForceMonotonic), map: BasketProblem::Saved, }?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya reopens the saved basket:"); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_map_error! { program: show_basket(total.clone()), map: BasketProblem::View, }?;
        effect_map_error! { program: label.dispose(PubSubShutdown::Discard), map: BasketProblem::Label, }?;
        effect_map_error! { program: total.dispose(PubSubShutdown::Discard), map: BasketProblem::View, }?;
    }
}
/// Select local providers and execute the complete customer visit once.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    match futures_executor::block_on(fill_basket().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("Maya's basket failed: {exit:?}").into());
        }
    }
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    Ok(())
}
