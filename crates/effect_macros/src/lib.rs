//! Procedural Rust block syntax for the executor-independent Effect kernel.
mod async_expand;
mod expand;
mod parse;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Construct a lazy reusable Effect with macro-local `?` binding.
#[proc_macro]
pub fn effect(input: TokenStream) -> TokenStream {
    let statements = parse_macro_input!(input with syn::Block::parse_within);
    match expand::expand(statements) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
