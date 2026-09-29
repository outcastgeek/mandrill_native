//! Expansion of the Effect computation macro.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Stmt};

// The package identity from crates/effect/Cargo.toml, required by
// proc_macro_crate to resolve consumer dependency aliases. The effect crate
// also declares `extern crate self as effect` for its own expansions.
const EFFECT_PACKAGE_NAME: &str = "effect";

/// # Parameters
/// - `statements` (`Vec<Stmt>`): Rust statements parsed and lowered by the procedural macro.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn expand(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let needs_native_control =
        crate::parse::validate(&statements).is_err() || crate::parse::has_loops(&statements);
    let path = effect_crate_path()?;
    let path = quote!(#path::__private);
    if needs_native_control {
        return Ok(crate::async_expand::expand(statements, &path));
    }
    let body = lower(&statements, &path);
    Ok(quote!(#path::reusable(move || { #body })))
}

/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn effect_crate_path() -> syn::Result<TokenStream> {
    let name = match crate_name(EFFECT_PACKAGE_NAME)
        .map_err(|error| syn::Error::new(Span::call_site(), error))?
    {
        FoundCrate::Itself => String::from(EFFECT_PACKAGE_NAME),
        FoundCrate::Name(name) => name,
    };
    let ident = Ident::new(&name, Span::call_site());
    Ok(quote!(::#ident))
}

///
/// # Parameters
/// - `statements` (`&[Stmt]`): Rust statements parsed and lowered by the procedural macro.
/// - `path` (`&TokenStream`): Location of the field, file, or configuration value being processed.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
fn lower(statements: &[Stmt], path: &TokenStream) -> TokenStream {
    let Some((statement, remaining)) = statements.split_first() else {
        return quote!(#path::success(()));
    };
    let rest = lower(remaining, path);
    match statement {
        Stmt::Local(local) => {
            if let Some(init) = &local.init
                && let Expr::Try(expr) = init.expr.as_ref()
            {
                let pattern = &local.pat;
                let attrs = &local.attrs;
                let program = &expr.expr;
                return quote_spanned!(statement.span()=> #path::bind(#program, move |#(#attrs)* #pattern| { #rest }));
            }
            if let Some(init) = &local.init
                && is_control_flow(&init.expr)
            {
                let pattern = &local.pat;
                let attrs = &local.attrs;
                let program = expression(&init.expr, path);
                return quote_spanned!(statement.span()=> #path::bind(#program, move |#(#attrs)* #pattern| { #rest }));
            }
            quote!(#local #rest)
        }
        Stmt::Expr(Expr::Try(expr), semi) => {
            let program = &expr.expr;
            if semi.is_none() && remaining.is_empty() {
                quote_spanned!(statement.span()=> #path::bind(#program, move |value| #path::success(value)))
            } else {
                quote_spanned!(statement.span()=> #path::bind(#program, move |_| { #rest }))
            }
        }
        Stmt::Expr(Expr::Return(expr), _) => match &expr.expr {
            Some(value) => quote_spanned!(statement.span()=> #path::success(#value)),
            None => quote_spanned!(statement.span()=> #path::success(())),
        },
        Stmt::Expr(expr, semi) if is_control_flow(expr) => {
            let program = expression(expr, path);
            if semi.is_none() && remaining.is_empty() {
                program
            } else {
                quote_spanned!(statement.span()=> #path::bind(#program, move |_| { #rest }))
            }
        }
        Stmt::Expr(expr, None) if remaining.is_empty() => {
            quote_spanned!(statement.span()=> #path::success(#expr))
        }
        other @ (Stmt::Item(_) | Stmt::Expr(..) | Stmt::Macro(_)) => quote!(#other #rest),
    }
}

///
/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
fn is_control_flow(expr: &Expr) -> bool {
    crate::parse::has_effect_steps(expr)
        && (matches!(expr, Expr::If(_) | Expr::Match(_))
            || matches!(expr, Expr::Block(block) if block.label.is_none()))
}

///
/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
/// - `path` (`&TokenStream`): Location of the field, file, or configuration value being processed.
///
/// # Returns
/// Type: `TokenStream`.
/// Returns the expanded Rust tokens.
fn expression(expr: &Expr, path: &TokenStream) -> TokenStream {
    if let Expr::If(expr) = expr {
        let attrs = &expr.attrs;
        let condition = &expr.cond;
        let then_branch = lower(&expr.then_branch.stmts, path);
        let else_branch = expr.else_branch.as_ref().map_or_else(
            || quote!(#path::success(())),
            |(_, other)| expression(other, path),
        );
        return quote_spanned!(expr.span()=> #(#attrs)* if #condition { #then_branch } else { #else_branch });
    }
    if let Expr::Match(expr) = expr {
        let attrs = &expr.attrs;
        let value = &expr.expr;
        let arms = expr.arms.iter().map(|arm| {
            let attrs = &arm.attrs;
            let pattern = &arm.pat;
            let body = expression(&arm.body, path);
            quote_spanned!(arm.span()=> #(#attrs)* #pattern => { #body },)
        });
        return quote_spanned!(expr.span()=> #(#attrs)* match #value { #(#arms)* });
    }
    if let Expr::Block(expr) = expr
        && expr.label.is_none()
    {
        let attrs = &expr.attrs;
        let body = lower(&expr.block.stmts, path);
        return quote_spanned!(expr.span()=> #(#attrs)* #path::defer(move || { #body }));
    }
    if let Expr::Try(expr) = expr {
        let program = &expr.expr;
        return quote_spanned!(expr.span()=> #path::bind(#program, move |value| #path::success(value)));
    }
    quote_spanned!(expr.span()=> #path::success(#expr))
}
