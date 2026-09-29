//! Lowering of restricted transactional composition.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Stmt};

/// # Parameters
/// - `statements` (`Vec<Stmt>`): Rust statements parsed and lowered by the procedural macro.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn expand(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let body = lower(&statements, &path)?;
    Ok(quote!({ #body }))
}

///
/// # Parameters
/// - `statements` (`&[Stmt]`): Rust statements parsed and lowered by the procedural macro.
/// - `path` (`&TokenStream`): Location of the field, file, or configuration value being processed.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
fn lower(statements: &[Stmt], path: &TokenStream) -> syn::Result<TokenStream> {
    let Some((statement, remaining)) = statements.split_first() else {
        return Ok(quote!(#path::transactional::Transaction::succeed_with(|| ())));
    };
    match statement {
        Stmt::Local(local) => {
            if let Some(init) = &local.init
                && let Expr::Try(expr) = init.expr.as_ref()
            {
                let pattern = &local.pat;
                let program = &expr.expr;
                let rest = lower(remaining, path)?;
                return Ok(quote_spanned!(statement.span()=>
                    (#program).flat_map(move |#pattern| { #rest })
                ));
            }
            if let Some(init) = &local.init
                && is_control_flow(&init.expr)
            {
                let pattern = &local.pat;
                let program = expression(&init.expr, path)?;
                let rest = lower(remaining, path)?;
                return Ok(quote_spanned!(statement.span()=>
                    (#program).flat_map(move |#pattern| { #rest })
                ));
            }
            let rest = lower(remaining, path)?;
            Ok(quote_spanned!(statement.span()=> #local #rest))
        }
        Stmt::Expr(Expr::Try(expr), semi) => {
            let program = &expr.expr;
            if semi.is_none() && remaining.is_empty() {
                Ok(quote_spanned!(statement.span()=> #program))
            } else {
                let rest = lower(remaining, path)?;
                Ok(quote_spanned!(statement.span()=>
                    (#program).flat_map(move |_| { #rest })
                ))
            }
        }
        Stmt::Expr(Expr::Return(expr), _) => {
            let value = expr
                .expr
                .as_ref()
                .map_or_else(|| quote!(()), |value| quote!(#value));
            Ok(quote_spanned!(statement.span()=>
                #path::transactional::Transaction::succeed_with(move || #value)
            ))
        }
        Stmt::Expr(expr, semi) if is_control_flow(expr) => {
            let program = expression(expr, path)?;
            if semi.is_none() && remaining.is_empty() {
                Ok(program)
            } else {
                let rest = lower(remaining, path)?;
                Ok(quote_spanned!(statement.span()=>
                    (#program).flat_map(move |_| { #rest })
                ))
            }
        }
        Stmt::Expr(expr, None) if remaining.is_empty() => Ok(quote_spanned!(statement.span()=>
            #path::transactional::Transaction::succeed_with(move || #expr)
        )),
        Stmt::Expr(expr, None) => Err(syn::Error::new_spanned(
            expr,
            "only the final expression may omit a semicolon",
        )),
        Stmt::Expr(expr, Some(_)) => {
            let rest = lower(remaining, path)?;
            Ok(quote_spanned!(statement.span()=> #expr; #rest))
        }
        Stmt::Item(item) => {
            let rest = lower(remaining, path)?;
            Ok(quote_spanned!(statement.span()=> #item #rest))
        }
        Stmt::Macro(item) => {
            let rest = lower(remaining, path)?;
            Ok(quote_spanned!(statement.span()=> #item #rest))
        }
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
        && (matches!(expr, Expr::If(_) | Expr::Match(_) | Expr::ForLoop(_))
            || matches!(expr, Expr::Block(block) if block.label.is_none()))
}

///
/// # Parameters
/// - `expr` (`&Expr`): Rust syntax expression parsed or expanded by this macro.
/// - `path` (`&TokenStream`): Location of the field, file, or configuration value being processed.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
fn expression(expr: &Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    if let Expr::If(branch) = expr {
        let attrs = &branch.attrs;
        let condition = &branch.cond;
        let then_branch = lower(&branch.then_branch.stmts, path)?;
        let else_branch = match &branch.else_branch {
            Some((_, other)) => expression(other, path)?,
            None => quote!(#path::transactional::Transaction::succeed_with(|| ())),
        };
        return Ok(quote_spanned!(expr.span()=>
            #(#attrs)* if #condition { #then_branch } else { #else_branch }
        ));
    }
    if let Expr::Match(branch) = expr {
        let attrs = &branch.attrs;
        let value = &branch.expr;
        let arms = branch
            .arms
            .iter()
            .map(|arm| {
                let attrs = &arm.attrs;
                let pattern = &arm.pat;
                let body = expression(&arm.body, path)?;
                Ok(quote_spanned!(arm.span()=>
                    #(#attrs)* #pattern => { #body },
                ))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        return Ok(quote_spanned!(expr.span()=>
            #(#attrs)* match #value { #(#arms)* }
        ));
    }
    if let Expr::Block(block) = expr
        && block.label.is_none()
    {
        let body = lower(&block.block.stmts, path)?;
        return Ok(quote_spanned!(expr.span()=> #body));
    }
    if let Expr::ForLoop(loop_expr) = expr {
        crate::parse::pure(&loop_expr.expr)?;
        let items = &loop_expr.expr;
        let pattern = &loop_expr.pat;
        let body = lower(&loop_expr.body.stmts, path)?;
        return Ok(quote_spanned!(expr.span()=>
            #path::transactional::Transaction::for_each(
                move || (#items).into_iter(),
                move |#pattern| { (#body).map(|_| ()) },
            )
        ));
    }
    if let Expr::Try(step) = expr {
        let program = &step.expr;
        return Ok(quote_spanned!(expr.span()=> #program));
    }
    Ok(quote_spanned!(expr.span()=>
        #path::transactional::Transaction::succeed_with(move || #expr)
    ))
}
