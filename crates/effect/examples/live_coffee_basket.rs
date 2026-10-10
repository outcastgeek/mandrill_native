//! Maya adds and removes drinks, reserves stock, and restores her saved basket.
//!
//! Run: `cargo run -p effect --example live_coffee_basket`.
//! Basket, BasketTotal, and Stock are café models. Effect supplies tracked reads,
//! atomic stock commits, reactive query subscriptions, and hydration boundaries.
pub mod cafe_support;
use cafe_support::{Cafe, key, limits, nz, parse_policy};
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
    Scope, atom, effect, hydration, json_i64, json_string, reactive_bridge, reactive_derived,
    reactive_query, reactive_source, schema, service, stream,
};
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
/// Basket errors retain distinct model, hydration, query, and transaction channels.
#[derive(Debug)]
pub enum BasketProblem {
    /// Reading or changing the basket failed.
    Basket(AtomRefError),
    /// A tracked view failed.
    View(AtomError<AtomRefError>),
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
        capacity: nz(4),
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }
}
/// Extracted tracked helper observes the basket through the caller's read context.
fn total_for(basket: AtomRef<Basket>) -> AtomProgram<BasketTotal, AtomRefError, CoffeeShop> {
    atom! {
        let basket = basket.clone();
        let selected = AtomProgram::read(basket)?;
        BasketTotal {
            drinks: selected.americanos + selected.oat_lattes,
            cents: selected.americanos * 325 + selected.oat_lattes * 475 }
    }
}
/// Reserve all requested drinks together; no partial reservation is committed.
fn reserve_stock(stock: TxRef<Stock>, basket: Arc<Basket>) -> Transaction<Stock, StockProblem> {
    transaction! {
        let stock = stock.clone(); let basket = basket.clone();
        let available = stock.read()?;
        if basket.americanos > available.americanos || basket.oat_lattes > available.oat_lattes {
            Transaction::<(), StockProblem>::fail_with(|| StockProblem::SoldOut)?;
        }
        let remaining = Stock {
            americanos: available.americanos - basket.americanos,
            oat_lattes: available.oat_lattes - basket.oat_lattes };
        stock.write(Arc::new(remaining.clone()))?;
        remaining
    }
}
/// Query preview closes its subscription after Initial, Running, and one result.
fn show_stock(
    query: ReactiveQuery<Stock, TransactionError<StockProblem>, CoffeeShop>,
) -> Effect<(), ReactiveQueryError, CoffeeShop> {
    Scope::scoped(move |scope| {
        let changes = query.stream();
        effect! {
            let scope = scope.clone(); let changes = changes.clone();
            let reader = changes.open(scope)?;
            for _ in 0..3 {
                match reader.pull()? {
                    Chunk::Values(states) => for state in states {
                        if state.phase() == AsyncResultPhase::Settled {
                            if let Some(stock) = state.current_success() {
                                stock.with(|stock| println!("Counter stock: {} americanos, {} oat lattes.", stock.americanos, stock.oat_lattes)).expect("stock snapshot");
                            }
                        }
                    },
                    Chunk::End => break,
                }
            }
        }
    })
}
/// Refresh the tracked total and show what the customer sees.
fn show_basket(
    total: Atom<BasketTotal, AtomRefError, CoffeeShop>,
) -> Effect<(), AtomError<AtomRefError>, CoffeeShop> {
    effect! {
        let total = total.clone();
        total.refresh()?;
        let state = total.read()?;
        state.current_success().expect("refreshed basket").with(|total|
            println!("Maya's basket: {} drinks, ${}.{:02}.", total.drinks, total.cents / 100, total.cents % 100)).expect("basket total");
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
        let registry = AtomRegistry::new(basket_updates()).expect("basket registry");
        let basket = reactive_source! {
            registry: registry.clone(), id: AtomId::from_stable(key("maya-basket")),
            initial: Basket { customer: "Maya".into(), americanos: 0, oat_lattes: 0 }, capacity: nz(4), overflow: PubSubOverflow::Backpressure
        }.expect("basket source");
        let total = reactive_derived! {
            registry: registry.clone(), id: AtomId::from_stable(key("maya-basket-total")), capacity: nz(4),
            overflow: PubSubOverflow::Backpressure, program: total_for(basket.clone())
        }.expect("basket total");
        let selected = basket.clone();
        let label = reactive_derived! {
            registry: registry.clone(), id: AtomId::from_stable(key("maya-basket-label")), capacity: nz(4),
            overflow: PubSubOverflow::Backpressure,
            evaluate: move |context: AtomReadContext| context.read::<Basket, CoffeeShop>(&selected).map(|basket| format!("{}'s coffee basket", basket.customer))
        }.expect("basket label");
        basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 1, oat_lattes: 2 }).map_error(BasketProblem::Basket)?;
        show_basket(total.clone()).map_error(BasketProblem::View)?;
        let before = basket.read::<CoffeeShop>().map_error(BasketProblem::Basket)?;
        println!("Maya removes one oat latte before checkout.");
        basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 1, oat_lattes: 1 }).map_error(BasketProblem::Basket)?;
        show_basket(total.clone()).map_error(BasketProblem::View)?;
        label.refresh().map_error(BasketProblem::View)?;
        let label_state = label.read().map_error(BasketProblem::View)?;
        label_state.current_success().expect("basket label").with(|label| println!("Customer display: {label}")).expect("label snapshot");
        let after = basket.read::<CoffeeShop>().map_error(BasketProblem::Basket)?;
        let saved: HydrationSet<CoffeeShop> = hydration! {
            registry: registry.clone(), limits: limits(), bindings: [basket.clone() => HydrationSchema::new(
                basket.id().clone(), RecordVersion::new("coffee-basket-v1").expect("basket version"),
                schema! { struct Basket { customer: json_string(), americanos: json_i64(), oat_lattes: json_i64() } policy: parse_policy() }.expect("basket schema"), limits())]
        }.expect("saved basket binding");
        let snapshot = saved.export_all().map_error(BasketProblem::Saved)?;
        let stock = service! {
            select: |cafe: &CoffeeShop| cafe.transactions.clone(),
            on_error: |problem: BasketProblem| problem,
            run |journal: &Arc<TransactionDomain>| {
                journal.reference(Stock { americanos: 4, oat_lattes: 3 }).expect("counter stock")
            }
        }?;
        let observed_stock = stock.clone();
        let availability: ReactiveQuery<Stock, TransactionError<StockProblem>, CoffeeShop> = reactive_query! {
            registry: InvalidationRegistry::new(basket_updates()).expect("stock registry"), dependencies: [InvalidationKey::from_stable(key("counter-stock"))],
            evaluate: move || observed_stock.read::<StockProblem>().atomically(TransactionPolicy { max_conflicts: nz(4) }).map(|stock| (*stock).clone())
        };
        show_stock(availability.clone()).map_error(BasketProblem::Query)?;
        let remaining = reserve_stock(stock, after.clone()).atomically(TransactionPolicy { max_conflicts: nz(4) }).map_error(BasketProblem::Stock)?;
        println!("Reserved Maya's two drinks together; {} oat-latte portions remain.", remaining.oat_lattes);
        show_stock(availability).map_error(BasketProblem::Query)?;
        let display_basket = basket.clone();
        let history = Arc::new(vec![before, after]);
        let display: EffectStreamSource<Arc<Basket>, AtomRefError, PubSubError, CoffeeShop> = reactive_bridge! {
            read: move || display_basket.read::<CoffeeShop>(),
            changes: move || basket_history(history.clone())
        };
        let latest = display.read().map_error(BasketProblem::Basket)?;
        let displayed = display.changes().run_collect(Some(2)).map_error(BasketProblem::Display)?;
        println!("Customer display replayed {} basket changes; latest is {} drinks.", displayed.len(), latest.americanos + latest.oat_lattes);
        basket.set::<CoffeeShop>(Basket { customer: "Maya".into(), americanos: 0, oat_lattes: 0 }).map_error(BasketProblem::Basket)?;
        saved.import_all(snapshot, HydrationConflictPolicy::ForceMonotonic).map_error(BasketProblem::Saved)?;
        println!("Maya reopens the saved basket:");
        show_basket(total.clone()).map_error(BasketProblem::View)?;
        label.dispose(PubSubShutdown::Discard).map_error(BasketProblem::View)?;
        total.dispose(PubSubShutdown::Discard).map_error(BasketProblem::View)?;
    }
}
/// Select local providers and execute the complete customer visit once.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open((), nz(16), nz(4));
    match futures_executor::block_on(fill_basket().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Maya's basket failed: {exit:?}"),
    }
    cafe.base.finish();
}
