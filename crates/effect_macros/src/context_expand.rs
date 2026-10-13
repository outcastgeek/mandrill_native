//! Shared native control flow for typed domain contexts around the Effect runtime.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::fold::Fold;
use syn::{Expr, Stmt};
use syn::{parse_quote, parse_quote_spanned};

/// Rewrites domain binds without inspecting macro tokens, callbacks, or item bodies.
struct Lower {
    /// Execution-local typed context selected by the owning domain constructor.
    context: Ident,
}
impl Fold for Lower {
    /// Preserve guards, patterns and ownership while binding through the domain context.
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        if matches!(&expr, Expr::Closure(_) | Expr::Async(_)) {
            expr
        } else if let Expr::Try(step) = expr {
            let program = self.fold_expr(*step.expr);
            let context = &self.context;
            parse_quote_spanned!(step.question_token.span.resolved_at(proc_macro2::Span::mixed_site())=> #context.bind(#program).await?)
        } else if let Expr::Return(step) = expr {
            let value = step
                .expr
                .map(|value| self.fold_expr(*value))
                .unwrap_or_else(|| parse_quote!(()));
            parse_quote!(return Ok(#value))
        } else if let Expr::While(mut step) = expr {
            step.cond = Box::new(self.fold_expr(*step.cond));
            step.body = self.fold_block(step.body);
            let context = &self.context;
            step.body
                .stmts
                .insert(0, parse_quote!(#context.cooperate().await?;));
            Expr::While(step)
        } else if let Expr::ForLoop(mut step) = expr {
            step.expr = Box::new(self.fold_expr(*step.expr));
            step.body = self.fold_block(step.body);
            let context = &self.context;
            step.body
                .stmts
                .insert(0, parse_quote!(#context.cooperate().await?;));
            Expr::ForLoop(step)
        } else if let Expr::Loop(mut step) = expr {
            step.body = self.fold_block(step.body);
            let context = &self.context;
            step.body
                .stmts
                .insert(0, parse_quote!(#context.cooperate().await?;));
            Expr::Loop(step)
        } else {
            syn::fold::fold_expr(self, expr)
        }
    }
    /// Nested items retain their own language and failure boundary.
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
}

/// Return a deferred async callback whose constructor chooses the typed binder and scope.
/// All locals, including the pure prefix, live inside the per-evaluation future.
pub(crate) fn callback(statements: Vec<Stmt>) -> TokenStream {
    let context = Ident::new("__domain_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
    };
    let body = statements
        .into_iter()
        .map(|stmt| lower.fold_stmt(stmt))
        .collect::<Vec<_>>();
    quote!(move |#context| async move { #[allow(unreachable_code)] { Ok({ #(#body)* }) } })
}

/// Expose a borrowed execution context inside a restricted callback's future.
/// The original binder continues to own that context, so nested resource binds
/// retain the activation scope/fencing owner; the user borrow cannot escape it.
pub(crate) fn named_callback(
    statements: Vec<Stmt>,
    pattern: &syn::Pat,
) -> syn::Result<TokenStream> {
    let context = Ident::new("__domain_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
    };
    let body = statements
        .into_iter()
        .map(|stmt| lower.fold_stmt(stmt))
        .collect::<Vec<_>>();
    let path = crate::expand::effect_crate_path()?;
    Ok(
        quote!(move |#context: #path::cluster::EntityPlanContext<_, _>| async move {
            #[allow(unused_variables)] let #pattern = &#context;
            #[allow(unreachable_code)] { Ok({ #(#body)* }) }
        }),
    )
}

/// Wrap a body with a domain constructor resolved through the caller's Cargo dependency.
pub(crate) fn expand(statements: Vec<Stmt>, constructor: TokenStream) -> TokenStream {
    let callback = reusable_callback(statements);
    quote!(#constructor(#callback))
}

/// Split only native pure prefix statements so explicit shared-handle copies occur per evaluation
/// before the future captures them. The factory is still invoked lazily by its domain owner.
pub(crate) fn split_prefix(mut statements: Vec<Stmt>) -> (Vec<Stmt>, Vec<Stmt>) {
    let length = statements
        .iter()
        .take_while(|statement| {
            if crate::parse::has_loops(std::slice::from_ref(statement)) {
                return false;
            }
            match statement {
                Stmt::Local(local) => local.init.as_ref().is_none_or(|init| {
                    init.diverge.is_none()
                        && crate::parse::pure(&init.expr).is_ok()
                        && !has_suspension(&init.expr)
                }),
                Stmt::Expr(_, Some(_)) => false,
                Stmt::Item(_) => true,
                Stmt::Expr(_, None) | Stmt::Macro(_) => false,
            }
        })
        .count();
    let body = statements.split_off(length);
    (statements, body)
}
/// Prepare explicit per-evaluation captures for a reusable domain callback without payload cloning.
pub(crate) fn reusable_callback(statements: Vec<Stmt>) -> TokenStream {
    let (prefix, body) = split_prefix(statements);
    let context = Ident::new("__domain_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
    };
    let body = body
        .into_iter()
        .map(|statement| lower.fold_stmt(statement))
        .collect::<Vec<_>>();
    quote!(move |#context| {#(#prefix)* async move {#[allow(unreachable_code)] {Ok({#(#body)*})}}})
}

/// Keep native stream suspension inside the body future, including a yield in a local initializer.
/// Nested callback/async/item and unrelated macro tokens retain their own language boundary.
fn has_suspension(expression: &Expr) -> bool {
    /// Syntax-only observation; it never inspects a producer's runtime type or macro body tokens.
    struct Suspension {
        /// Whether a native yield or the known local append marker belongs to this body.
        found: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Suspension {
        /// A yield belongs to the current body unless a nested language boundary already stopped traversal.
        fn visit_expr_yield(&mut self, _: &'ast syn::ExprYield) {
            self.found = true;
        }
        /// Only the owning stream marker is recognized; other macro syntax remains opaque.
        fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
            self.found |= invocation.path.is_ident("yield_from");
        }
        /// A nested callback has independent capture and suspension semantics.
        fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
        /// A nested async block retains its native language boundary.
        fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
        /// An item's executable body is never part of the enclosing macro workflow.
        fn visit_item(&mut self, _: &'ast syn::Item) {}
    }
    let mut observation = Suspension { found: false };
    syn::visit::Visit::visit_expr(&mut observation, expression);
    observation.found
}
/// Prefix preparation must never turn stable macro-consumed yields into compiler coroutines.
#[cfg(test)]
mod tests {
    use super::*;
    /// An explicit capture copy may precede a local suspension, but suspension remains in the future.
    #[test]
    fn suspension_stays_in_future() -> syn::Result<()> {
        let statements = syn::parse::Parser::parse_str(
            syn::Block::parse_within,
            "let shared=shared.clone();let value={yield 1;2};yield value;",
        )?;
        let (prefix, body) = split_prefix(statements);
        assert_eq!(prefix.len(), 1);
        assert_eq!(body.len(), 2);
        Ok(())
    }
}
