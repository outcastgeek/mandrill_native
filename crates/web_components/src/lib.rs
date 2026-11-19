pub mod components;
pub use web_components_macros::html;

extern crate self as web_components;

#[doc(hidden)]
pub mod __private {
    pub use crate::html_spread::AttributeSlots;
    pub use maud::html as maud_html;
}

mod html_spread;
