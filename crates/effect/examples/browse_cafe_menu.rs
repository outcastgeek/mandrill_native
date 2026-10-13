//! Maya reads the menu and compares dairy-free choices with their actual prices.
//!
//! Run: `cargo +1.99.0 run -p effect --example browse_cafe_menu`.
//! Input: an open board with americano 325, oat latte 475, cappuccino 450 cents.
//! Output: all three priced choices, with cappuccino identified as unsuitable for Maya.
//! A lazy stream supplies choices; the selected price board supplies each price.

use effect::{effect_die, effect_map_error};
pub mod cafe_support;
use cafe_support::Cafe;
use std::num::NonZeroUsize;
/// Menu choices and price-board data used by the browsing and update recipes.
pub mod menu;
use effect::{
    CacheAdmission, CacheCapacity, CacheConfig, CacheOptions, CacheTtl, Effect, Exit, Scope,
    SharedValue, Stream, cache, effect_for_each, effect_scope, effect_try_sync, service, span,
    stream,
};
use menu::{Drink, MenuBoard, MenuProblem};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicI64},
};

/// Pull the menu once and print the dietary information beside each selected price.
fn browse_menu() -> Effect<(), SharedValue<MenuProblem>, CoffeeShop> {
    effect_scope! {
        body |scope: Scope<SharedValue<MenuProblem>>| {
        let prices = cache! {
            scope: scope, config: CacheConfig {
                equal: Arc::new(|a: &Drink, b| a == b), ttl: None,
                capacity: CacheCapacity::Bounded(NonZeroUsize::MIN.saturating_add(2)), cache_failure: Arc::new(|_| false) },
            options: CacheOptions {
                ttl: CacheTtl::Fixed(None), admission: CacheAdmission::Bounded {
                maximum: NonZeroUsize::MIN.saturating_add(3), capacity_error: Arc::new(|| MenuProblem::Busy) } },
            load |drink: Arc<Drink>| { menu_price((*drink).clone())? },
        }?;
        let drinks = effect_map_error! { program: menu_choices().run_collect(Some(3)), map: SharedValue::new, }?;
        let browse_prices = prices.clone();
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya reads today's menu:"); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_for_each! {
            values: drinks,
            body |drink: Drink| {
                let prices = browse_prices.clone();
                let label = drink.label(); let contains_dairy = drink == Drink::Cappuccino;
                let cents = span! {
                    name: "read-menu-price", annotations: BTreeMap::from([(Arc::from("customer"), Arc::from("Maya"))]),
                    body { let drink = drink.clone(); span! { name: "menu-board", effect: prices.get(drink) }? }
                }?;
                let cents = match cents.with(|cents| *cents) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("visit-owned price: {error:?}")), }?, };
                { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "  {label}: ${}.{:02}", cents / 100, cents % 100); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
                if contains_dairy { { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "    Maya skips dairy today."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } }; }
            },
        }?;
        let selected = prices.get(Drink::OatLatte)?;
        let selected_cents = match selected.with(|cents| *cents) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("retained menu price: {error:?}")), }?, };
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya confirms her oat-latte choice at {} cents from the same visit menu.", selected_cents); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        },
    }
}
/// Type alias makes the application-defined menu service explicit in R.
type CoffeeShop = Cafe<MenuBoard>;
/// Extracted business helper selects this café's MenuBoard at execution time.
fn menu_price(drink: Drink) -> Effect<i64, MenuProblem, CoffeeShop> {
    service! {
        select: |cafe: &CoffeeShop| cafe.services.clone(), on_error: |problem: MenuProblem| problem, run |board| {
            let drink = drink.clone();
            service! {
                operation |board: &Arc<MenuBoard>| {
                    let price = board.price(&drink);
                    effect_try_sync! { once; body { price }, }?
                },
            }?
        }
    }
}
/// Supply the advertised choices in board order when the customer pulls them.
fn menu_choices() -> Stream<Drink, MenuProblem, CoffeeShop> {
    stream! {
        for drink in [Drink::Americano, Drink::Cappuccino] { yield drink; }
        yield_from!(stream! { yield Drink::OatLatte; });
    }
}
/// Select local providers and execute the customer's menu visit.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe = Cafe::open(
        MenuBoard {
            open: true,
            oat_latte_cents: AtomicI64::new(475),
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(browse_menu().run(cafe.clone()));
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("menu visit failed: {other:?}").into());
        }
    }
    Ok(())
}
