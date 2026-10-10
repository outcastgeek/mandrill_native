//! Maya browses the café menu, chooses dairy-free drinks, and sees a price refresh.
//!
//! Run: `cargo run -p effect --example browse_cafe_menu`.
//! Drink, MenuBoard, and CounterTicket belong to this café. Effect supplies lazy
//! recipes, scoped caches, service selection, tracing, and pull-driven streams.
pub mod cafe_support;
use cafe_support::{Cafe, nz};
use effect::{
    CacheAdmission, CacheCapacity, CacheConfig, CacheOptions, CacheTtl, DependencyCacheError,
    DependencyCachePolicy, Effect, Exit, Release, ResourceBinding, Scope, ServiceOperation,
    SharedValue, Stream, cache, dependency_cache, effect, service, span, stream,
};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

/// A menu choice understood by the customer, independent of the Effect library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drink {
    /// Espresso with hot water; contains no milk.
    Americano,
    /// Espresso with oat milk.
    OatLatte,
    /// Espresso with dairy milk and foam.
    Cappuccino,
}
impl Drink {
    /// Customer-facing menu label and dietary information.
    fn label(&self) -> &'static str {
        match self {
            Self::Americano => "Americano (no milk)",
            Self::OatLatte => "Oat latte (dairy-free)",
            Self::Cappuccino => "Cappuccino (dairy)",
        }
    }
}
/// The café's price board; staff can change the oat-latte price in cents.
pub struct MenuBoard {
    /// Current oat-latte price, explicitly expressed in cents.
    oat_latte_cents: AtomicI64,
}
impl MenuBoard {
    /// Look up an application price; a closed board would return MenuProblem.
    fn price(&self, drink: &Drink) -> Result<i64, MenuProblem> {
        Ok(match drink {
            Drink::Americano => 325,
            Drink::OatLatte => self.oat_latte_cents.load(Ordering::SeqCst),
            Drink::Cappuccino => 450,
        })
    }
}
/// A customer's local pickup ticket, used as an already-selected service value.
struct CounterTicket {
    /// Name to call at the counter.
    customer: &'static str,
}
/// A stable application dependency: prices change when staff refresh the board.
#[derive(PartialEq, Eq)]
pub enum MenuDependency {
    /// The café's daily price board.
    PriceBoard,
}
/// Business failures from the menu service and its load admission.
#[derive(Debug)]
pub enum MenuProblem {
    /// Staff have not opened the menu yet.
    Closed,
    /// All four menu lookup workers are occupied.
    Busy,
}
/// Customer-visible errors retain the service and cache failure boundaries.
#[derive(Debug)]
pub enum BrowsingProblem {
    /// Menu service rejected a lookup.
    Menu(MenuProblem),
    /// A scoped menu cache failed.
    Cache(SharedValue<MenuProblem>),
    /// The retained price or its invalidation failed.
    Refresh(SharedValue<DependencyCacheError<MenuProblem>>),
}
/// Type alias makes the application-defined menu service explicit in R.
type CoffeeShop = Cafe<MenuBoard>;
/// Extracted business helper selects this café's MenuBoard at execution time.
fn menu_price(drink: Drink) -> Effect<i64, MenuProblem, CoffeeShop> {
    service! {
        select: |cafe: &CoffeeShop| cafe.services.clone(), on_error: |problem: MenuProblem| problem, run |board| {
            let drink = drink.clone();
            ServiceOperation::new(move |board: &Arc<MenuBoard>| {
                Effect::from_result_once(board.price(&drink))
            })?
        }
    }
}
/// Borrowed-key alternative shares the same menu helper and environment.
fn borrowed_menu_price(drink: &Drink) -> Effect<i64, MenuProblem, CoffeeShop> {
    menu_price(drink.clone())
}
/// A dependency-aware helper couples a price to the board that supplies it.
fn price_with_board(drink: &Drink) -> Effect<(i64, Vec<MenuDependency>), MenuProblem, CoffeeShop> {
    let drink = drink.clone();
    effect! { let drink = drink.clone(); let cents = menu_price(drink)?; (cents, vec![MenuDependency::PriceBoard]) }
}
/// Two espresso choices plus a rotating special, opened only when Maya pulls.
fn menu_choices() -> Stream<Drink, MenuProblem, CoffeeShop> {
    stream! {
        let board = ResourceBinding::new(Effect::sync(|| String::from("today's printed menu")), |_, _| Release::sync(|| println!("Maya returns the menu to the counter")))?;
        let _ = board.access(|_| ())?;
        for drink in [Drink::Americano, Drink::Cappuccino] { yield drink; }
        yield_from!(stream! { yield Drink::OatLatte; });
    }
}
/// The customer-facing workflow; services, caches, and streams compose with ?.
fn browse_menu() -> Effect<(), BrowsingProblem, CoffeeShop> {
    effect! {
        let customer = service! {
            handle: CounterTicket { customer: "Maya" }, run |ticket: &CounterTicket| {
                ticket.customer.to_owned()
            }
        }.map_error(BrowsingProblem::Menu)?;
        let drinks = menu_choices().run_collect(Some(3)).map_error(BrowsingProblem::Menu)?;
        println!("{customer} reads today's menu:");
        for drink in drinks {
            let label = drink.label(); let contains_dairy = drink == Drink::Cappuccino;
            let cents = span! {
                name: "read-menu-price", annotations: BTreeMap::from([(Arc::from("customer"), Arc::from("Maya"))]),
                body { let drink = drink.clone(); span! { name: "menu-board", effect: menu_price(drink) }? }
            }.map_error(BrowsingProblem::Menu)?;
            println!("  {}: ${}.{:02}", label, cents / 100, cents % 100);
            if contains_dairy { println!("    Maya skips dairy today."); }
        }
        let pickup_name = service! {
            project: |_: &CoffeeShop| CounterTicket { customer: "Maya" }, body {
                service! {
                    select: |ticket: &CounterTicket| ticket.customer.to_owned(),
                    on_error: |problem: MenuProblem| problem,
                    run |name: &String| { name.clone() }
                }?
            }
        }.map_error(BrowsingProblem::Menu)?;
        cached_menu().map_error(BrowsingProblem::Cache)?;
        refreshed_special().map_error(BrowsingProblem::Refresh)?;
        println!("Pickup ticket prepared for {pickup_name}.");
    }
}
/// A browsing session retains at most the three menu prices and four active loads.
fn cached_menu() -> Effect<(), SharedValue<MenuProblem>, CoffeeShop> {
    Scope::scoped(|scope| {
        effect! {
            let scope = scope.clone();
            let config = CacheConfig {
                equal: Arc::new(|left: &Drink, right| left == right), ttl: None,
                capacity: CacheCapacity::Bounded(nz(3)), cache_failure: Arc::new(|_: &MenuProblem| false) };
            let options = CacheOptions {
                ttl: CacheTtl::Fixed(None), admission: CacheAdmission::Bounded {
                    maximum: nz(4), capacity_error: Arc::new(|| MenuProblem::Busy) } };
            let prices = cache! {
                scope: scope.clone(), config: config.clone(), options: options.clone(),
                load |drink: Arc<Drink>| { menu_price((*drink).clone())? } }?;
            let familiar_prices = cache! { scope: scope.clone(), config: config, options: options, load: borrowed_menu_price }?;
            let first = prices.get(Drink::OatLatte)?;
            let americano = familiar_prices.get(Drink::Americano)?;
            let retained = prices.get(Drink::OatLatte)?;
            let cents = first.with(|cents| *cents).expect("live menu price");
            println!("Maya's oat-latte quote is ${}.{:02}; it is retained for this visit.", cents / 100, cents % 100);
            let familiar_cents = americano.with(|cents| *cents).expect("regular menu price");
            println!("Maya compares the regular americano: ${}.{:02}.", familiar_cents / 100, familiar_cents % 100);
            let retained_cents = retained.with(|cents| *cents).expect("retained quote");
            println!("Reopening the oat-latte quote still shows ${}.{:02}.", retained_cents / 100, retained_cents % 100);
        }
    })
}
/// Retain the three menu prices and their one price-board dependency per visit.
fn menu_retention() -> DependencyCachePolicy<Drink, MenuDependency, i64, MenuProblem> {
    DependencyCachePolicy {
        retention: nz(3),
        maximum_active: nz(4),
        maximum_dependencies: nz(1),
        key_equal: Arc::new(|left: &Drink, right| left == right),
        dependency_equal: Arc::new(|left: &MenuDependency, right| left == right),
        cache_failure: Arc::new(|_: &MenuProblem| false),
        ttl: CacheTtl::Fixed(None),
    }
}
/// Staff refresh the board; the next dependent quote reflects the new price.
fn refreshed_special() -> Effect<(), SharedValue<DependencyCacheError<MenuProblem>>, CoffeeShop> {
    Scope::scoped(|scope| {
        effect! {
            let scope = scope.clone();
            let specials = dependency_cache! {
                scope: scope.clone(), policy: menu_retention(),
                load |drink: Arc<Drink>| { price_with_board(&drink)? } }?;
            let _ = specials.get(Drink::OatLatte)?;
            service! {
                select: |cafe: &CoffeeShop| cafe.services.clone(),
                on_error: |problem: SharedValue<DependencyCacheError<MenuProblem>>| problem,
                run |board: &Arc<MenuBoard>| { board.oat_latte_cents.store(500, Ordering::SeqCst); }
            }?;
            specials.invalidate_dependency(MenuDependency::PriceBoard)?;
            let changed = specials.get(Drink::OatLatte)?;
            let cents = changed.with(|cents| *cents).expect("refreshed special");
            let reusable = dependency_cache! { scope: scope.clone(), policy: menu_retention(), load: price_with_board }?;
            let regular = reusable.get(Drink::Americano)?;
            let regular_cents = regular.with(|cents| *cents).expect("regular menu quote");
            println!("The regular americano remains ${}.{:02}.", regular_cents / 100, regular_cents % 100);
            println!("Staff refresh the special: oat latte now ${}.{:02}.", cents / 100, cents % 100);
        }
    })
}
/// Select local providers; only this boundary executes the customer recipe.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        MenuBoard {
            oat_latte_cents: AtomicI64::new(475),
        },
        nz(16),
        nz(4),
    );
    match futures_executor::block_on(browse_menu().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Maya could not browse: {exit:?}"),
    }
    cafe.base.finish();
}
