//! Shared café menu models and native price lookup, in integer cents.
use std::sync::atomic::{AtomicI64, Ordering};
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
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Americano => "Americano (no milk)",
            Self::OatLatte => "Oat latte (dairy-free)",
            Self::Cappuccino => "Cappuccino (dairy)",
        }
    }
}
/// The café's price board; staff can change the oat-latte price in cents.
pub struct MenuBoard {
    /// Whether customers may currently read this board.
    pub open: bool,
    /// Current oat-latte price, explicitly expressed in cents.
    pub oat_latte_cents: AtomicI64,
}
impl MenuBoard {
    /// Look up an application price; a closed board would return MenuProblem.
    pub fn price(&self, drink: &Drink) -> Result<i64, MenuProblem> {
        if !self.open {
            return Err(MenuProblem::Closed);
        }
        Ok(match drink {
            Drink::Americano => 325,
            Drink::OatLatte => self.oat_latte_cents.load(Ordering::SeqCst),
            Drink::Cappuccino => 450,
        })
    }
}
/// A price lookup can reject a board that is unavailable to the visitor.
#[derive(Debug)]
pub enum MenuProblem {
    /// Staff have not opened the menu.
    Closed,
    /// The configured menu load bound has been reached.
    Busy,
}
