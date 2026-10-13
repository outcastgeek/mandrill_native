//! Staff raise the oat-latte price while Maya retains her agreed quote.
//!
//! Run: `cargo +1.99.0 run -p effect --example update_menu_prices`.
//! Input: oat latte at 475 cents, then staff publish 500 cents.
//! Output: new visitors see 500 cents; Maya's visit quote remains 475 cents.
//! A weak-key cache retains the agreed quote while its visit key and quote owner live.
//! A dependency cache refreshes public prices when staff publish the changed board.

use effect::{effect_die, effect_map_error};
pub mod cafe_support;
use std::num::NonZeroUsize;
/// Shared menu data and native price lookup.
pub mod menu;
use cafe_support::Cafe;
use effect::{
    CacheTtl, DependencyCacheError, DependencyCachePolicy, Effect, Exit, Scope, SharedValue,
    WeakCache, dependency_cache, effect, effect_scope, effect_try_sync, service, weak_cache,
};
use menu::{Drink, MenuBoard, MenuProblem};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

/// Keep the same visit-owned quote alive across the staff update and read it again.
fn update_prices() -> Effect<(), SharedValue<DependencyCacheError<MenuProblem>>, CoffeeShop> {
    effect_scope! {
        body |scope: Scope<SharedValue<DependencyCacheError<MenuProblem>>>| {
            let public = dependency_cache! {
                scope: scope.clone(), policy: retention(), load: price_with_board,
            }?;
            let quotes = WeakCache::<String, i64, DependencyCacheError<MenuProblem>>::make(scope)?;
            let visit = Arc::new("Maya's visit".to_owned());
            let agreed = weak_cache! {
                cache: quotes.clone(), key: &visit,
                load { effect_map_error! { program: menu_price(Drink::OatLatte), map: DependencyCacheError::Load, }? },
            }?;
            let first_public = public.get(Drink::OatLatte)?;
            let initial = match first_public.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("initial public price: {error:?}")), }?, };
            assert_eq!(initial, 475);
            let initial_quote = match agreed.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("accepted quote: {error:?}")), }?, };
            assert_eq!(initial_quote, 475);
            service! {
                select: |cafe: &CoffeeShop| cafe.services.clone(),
                on_error: |error: SharedValue<DependencyCacheError<MenuProblem>>| error,
                run |board: &Arc<MenuBoard>| { board.oat_latte_cents.store(500, Ordering::SeqCst); },
            }?;
            public.invalidate_dependency(Dependency::Board)?;
            let current = public.get(Drink::OatLatte)?;
            let agreed_again = weak_cache! {
                cache: quotes, key: &visit,
                load { effect_map_error! { program: menu_price(Drink::OatLatte), map: DependencyCacheError::Load, }? },
            }?;
            let current = match current.with(|price| *price) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("updated public price: {error:?}")), }?, };
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
        select: |cafe: &CoffeeShop| cafe.services.clone(), on_error: |error: MenuProblem| error,
        run |board: &Arc<MenuBoard>| {
            let drink = drink.clone();
            let price = board.price(&drink);
            effect_try_sync! { once; body { price }, }?
        },
    }
}
/// A public price depends on the board staff can update.
fn price_with_board(drink: &Drink) -> Effect<(i64, Vec<Dependency>), MenuProblem, CoffeeShop> {
    let drink = drink.clone();
    effect! {
        let drink = drink.clone();
        let price = menu_price(drink)?;
        (price, vec![Dependency::Board])
    }
}
/// Retain the three public prices and one board dependency per price.
fn retention() -> DependencyCachePolicy<Drink, Dependency, i64, MenuProblem> {
    DependencyCachePolicy {
        retention: NonZeroUsize::MIN.saturating_add(2),
        maximum_active: NonZeroUsize::MIN.saturating_add(3),
        maximum_dependencies: NonZeroUsize::MIN,
        key_equal: Arc::new(|a: &Drink, b| a == b),
        dependency_equal: Arc::new(|a: &Dependency, b| a == b),
        cache_failure: Arc::new(|_| false),
        ttl: CacheTtl::Fixed(None),
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
/// Run one real price change and close its cache scopes before joining providers.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = Cafe::open(
        MenuBoard {
            open: true,
            oat_latte_cents: AtomicI64::new(475),
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(update_prices().run(env.clone()));
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
