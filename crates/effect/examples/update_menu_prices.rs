//! Staff raise the oat-latte price while Maya retains her agreed quote.
//!
//! Run: `cargo +1.100.0 run -p effect --example update_menu_prices`.
//! Input: oat latte at 475 cents, then staff publish 500 cents.
//! Output: new visitors see 500 cents; Maya's visit quote remains 475 cents.
//! A weak-key cache retains the agreed quote while its visit key and quote owner live.
//! A dependency cache refreshes public prices when staff publish the changed board.

use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::{effect_die, effect_map_error, effect_sync};
pub mod cafe_support;
use std::num::NonZeroUsize;
/// Shared menu data and native price lookup.
pub mod menu;
use cafe_support::Cafe;
use effect::{
    CacheAdmission, CacheCapacity, CacheConfig, CacheKey, CacheOptions, CacheTtl,
    DependencyCacheError, Effect, Exit, PreparedCacheCapacity, PreparedCacheRefusals,
    PreparedDependencyPolicy, PreparedDependencyPool, PreparedDependencyStorage,
    PreparedDependencyWriter, PreparedWeakCacheCapacity, PreparedWeakCacheStorage, Scope,
    SharedValue, dependency_cache, effect, effect_scope, effect_try_sync, memory, service,
    weak_cache,
};
use menu::{Drink, MenuBoard, MenuProblem};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

type PublicPrices = PreparedDependencyStorage<Drink, Dependency, i64, MenuProblem>;
type VisitQuotes = PreparedWeakCacheStorage<String, i64, DependencyCacheError<MenuProblem>>;
type PriceRefusals = PreparedCacheRefusals<DependencyCacheError<MenuProblem>>;
const RETENTION: NonZeroUsize = NonZeroUsize::MIN.saturating_add(2);
const ACTIVE: NonZeroUsize = NonZeroUsize::MIN.saturating_add(3);
const CELLS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(7);
const PRICE_REGION: RegionId = RegionId::named("prices");

/// Keep the same visit-owned quote alive across the staff update and read it again.
fn update_prices(
    prices: PublicPrices,
    quotes: VisitQuotes,
    dependencies: Arc<PreparedDependencyPool<Dependency>>,
    price_refusals: PriceRefusals,
    quote_refusals: PriceRefusals,
) -> Effect<(), SharedValue<DependencyCacheError<MenuProblem>>, CoffeeShop> {
    let storage = effect_sync! { once; body: move || (prices, quotes), };
    effect_scope! {
        body |scope: Scope<SharedValue<DependencyCacheError<MenuProblem>>>| {
            let storage = storage.clone();
            let dependencies = dependencies.clone();
            let price_refusals = price_refusals.clone();
            let quote_refusals = quote_refusals.clone();
            let (prices, quotes) = storage?;
            let public = dependency_cache! {
                scope: scope.clone(), policy: retention(),
                memory: prices, dependency_pool: dependencies, refusals: price_refusals,
                load: price_with_board,
            }?;
            let quotes = weak_cache! {
                scope: scope, memory: quotes, refusals: quote_refusals,
            }?;
            let visit = Arc::new("Maya's visit".to_owned());
            let agreed = weak_cache! {
                cache: quotes.clone(), key: &visit,
                load { effect_map_error! { program: menu_price(Drink::OatLatte), map: DependencyCacheError::Load, }? },
            }?;
            let first_public = public.get(Drink::OatLatte)?;
            let initial = match first_public.with(|entry| entry.value.with(|price| *price)) { Ok(Ok(value)) => value, Ok(Err(error)) | Err(error) => effect_die! { defect: effect::Defect::new(format!("initial public price: {error:?}")), }?, };
            assert_eq!(initial, 475);
            let initial_quote = match agreed.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("accepted quote: {error:?}")), }?, };
            assert_eq!(initial_quote, 475);
            service! {
                select: |cafe: &CoffeeShop| cafe.services.clone(),
                run |board: &Arc<MenuBoard>| { board.oat_latte_cents.store(500, Ordering::SeqCst); },
            }?;
            public.invalidate_dependency(Dependency::Board)?;
            let current = public.get(Drink::OatLatte)?;
            let agreed_again = weak_cache! {
                cache: quotes, key: &visit,
                load { effect_map_error! { program: menu_price(Drink::OatLatte), map: DependencyCacheError::Load, }? },
            }?;
            let current = match current.with(|entry| entry.value.with(|price| *price)) { Ok(Ok(value)) => value, Ok(Err(error)) | Err(error) => effect_die! { defect: effect::Defect::new(format!("updated public price: {error:?}")), }?, };
            let accepted = match agreed_again.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("same retained visit quote: {error:?}")), }?, };
            let retained = match agreed.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("original retained quote: {error:?}")), }?, };
            assert_eq!(retained, 475);
            assert_eq!((accepted, current), (475, 500));
            { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "New oat-latte price: {current} cents; {} retains its accepted {accepted}-cent quote.", visit); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        },
    }
}
/// Select this application's price board when the cache actually loads a choice.
fn menu_price(drink: Drink) -> Effect<i64, MenuProblem, CoffeeShop> {
    service! {
        select: |cafe: &CoffeeShop| cafe.services.clone(),
        run |board: &Arc<MenuBoard>| {
            let drink = drink.clone();
            let price = board.price(&drink);
            effect_try_sync! { once; body { price }, }?
        },
    }
}
/// A public price depends on the board staff can update.
fn price_with_board(
    drink: CacheKey<Drink>,
    mut writer: PreparedDependencyWriter<Dependency>,
) -> Effect<(i64, PreparedDependencyWriter<Dependency>), MenuProblem, CoffeeShop> {
    let drink = (*drink).clone();
    // The writer records any overflow and refuses to publish it.
    let _ = writer.push(Dependency::Board);
    let writer = effect_sync! { once; body: move || writer, };
    effect! {
        let drink = drink.clone();
        let writer = writer.clone();
        let price = menu_price(drink)?;
        (price, writer?)
    }
}

/// Retain the three public prices and one board dependency per price.
fn retention() -> PreparedDependencyPolicy<Drink, Dependency, i64, MenuProblem> {
    PreparedDependencyPolicy {
        config: CacheConfig {
            capacity: CacheCapacity::Bounded(RETENTION),
            equal: Arc::new(|a: &Drink, b| a == b),
            ttl: None,
            cache_failure: Arc::new(|_| false),
        },
        options: CacheOptions {
            ttl: CacheTtl::Fixed(None),
            admission: CacheAdmission::Bounded {
                maximum: ACTIVE,
                capacity_error: Arc::new(|| DependencyCacheError::ActiveCapacity {
                    maximum: ACTIVE.get(),
                }),
            },
        },
        dependency_equal: Arc::new(|a: &Dependency, b| a == b),
    }
}

/// Environment selects the price board and bounded native providers.
type CoffeeShop = Cafe<MenuBoard>;
/// Dependency identity distinguishes the board from individual drink keys.
#[derive(PartialEq, Eq)]
pub enum Dependency {
    /// The mutable public price board.
    Board,
}
fn prepare_refusals(
    memory: &PreparedMemory<'static>,
    active: NonZeroUsize,
) -> Result<PriceRefusals, MemoryError> {
    let active_error = SharedValue::prepare_region(
        memory,
        PRICE_REGION,
        DependencyCacheError::ActiveCapacity {
            maximum: active.get(),
        },
    )?;
    let busy = SharedValue::prepare_region(
        memory,
        PRICE_REGION,
        DependencyCacheError::Memory(MemoryError::Busy),
    )?;
    let capacity = SharedValue::prepare_region(
        memory,
        PRICE_REGION,
        DependencyCacheError::Memory(MemoryError::NoSlot),
    )?;
    Ok(PreparedCacheRefusals::new(
        move |error| {
            if error == MemoryError::Busy {
                busy.clone()
            } else {
                capacity.clone()
            }
        },
        move || active_error.clone(),
    ))
}

/// Run one real price change and close its cache scopes before joining providers.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let price_limits = PreparedCacheCapacity {
        retention: CacheCapacity::Bounded(RETENTION),
        active: ACTIVE,
        lookups: ACTIVE,
        diagnostics: ACTIVE,
        observers: ACTIVE,
        keys: CELLS,
        entries: CELLS,
        values: CELLS,
        errors: NonZeroUsize::MIN.saturating_add(19),
        cleanup_outcomes: NonZeroUsize::MIN.saturating_add(11),
        maintenance_outcomes: NonZeroUsize::MIN.saturating_add(1),
        cleanup_entries: 2,
    };
    let quote_limits = PreparedWeakCacheCapacity {
        associations: 1,
        active: NonZeroUsize::MIN,
        lookups: NonZeroUsize::MIN,
        diagnostics: NonZeroUsize::MIN,
        observers: NonZeroUsize::MIN,
    };
    let price_bytes = PublicPrices::storage_bytes(price_limits, CELLS)?
        .checked_add(PreparedDependencyPool::<Dependency>::storage_bytes(
            CELLS.get(),
            1,
        )?)?
        .checked_add(VisitQuotes::storage_bytes(quote_limits)?)?
        .checked_add(Bytes::new(16 * 1024))?;
    let total = price_bytes.checked_add(Bytes::new(64 * 1024))?;
    let memory = memory! {
        total: total, mode: prepared, metadata: metadata,
        partition metadata: 32 KiB, partition prices: price_bytes,
    }?
    .prepare()?;
    let public = PublicPrices::prepare(&memory, PRICE_REGION, price_limits, CELLS)?;
    let dependencies = Arc::new(PreparedDependencyPool::prepare(
        &memory,
        PRICE_REGION,
        CELLS.get(),
        1,
    )?);
    let quotes = VisitQuotes::prepare(&memory, PRICE_REGION, quote_limits)?;
    let price_refusals = prepare_refusals(&memory, ACTIVE)?;
    let quote_refusals = prepare_refusals(&memory, quote_limits.active)?;
    let _memory = memory.seal();
    let env = Cafe::open(
        MenuBoard {
            open: true,
            oat_latte_cents: AtomicI64::new(475),
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(
        effect! {
            run;
            program: update_prices(public, quotes, dependencies, price_refusals, quote_refusals),
            environment: env.clone(),
        },
    );
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("price update failed: {other:?}").into());
        }
    }
    Ok(())
}
