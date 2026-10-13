//! Native control flow for restricted durable descriptions, with evaluation-local ownership.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::fold::Fold;
use syn::{Expr, Stmt};
use syn::{parse_quote, parse_quote_spanned};

/// Keeps binds on the durable boundary without traversing nested callbacks or items.
struct Lower {
    /// Typed durable context for one evaluation.
    context: Ident,
    /// Rejection of unsupported durable looping syntax, if observed.
    error: Option<syn::Error>,
}
impl Fold for Lower {
    /// Preserve native Rust branches and guards while replacing only domain binds.
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        if matches!(&expr, Expr::Closure(_) | Expr::Async(_)) {
            expr
        } else if let Expr::Try(step) = expr {
            let program = self.fold_expr(*step.expr);
            let context = &self.context;
            parse_quote_spanned!(step.question_token.span.resolved_at(proc_macro2::Span::mixed_site())=> match #context.bind(#program).await {
                Ok(value) => value, Err(failure) => return Err(failure),
            })
        } else if let Expr::Return(step) = expr {
            let value = step
                .expr
                .map(|value| self.fold_expr(*value))
                .unwrap_or_else(|| parse_quote!(()));
            parse_quote!(return ::core::result::Result::Ok(#value))
        } else if matches!(&expr, Expr::While(_) | Expr::Loop(_)) {
            self.error = Some(syn::Error::new_spanned(
                &expr,
                "durable coordination supports deterministic for iteration, not while or loop",
            ));
            expr
        } else {
            syn::fold::fold_expr(self, expr)
        }
    }
    /// Item bodies keep their own error and execution boundaries.
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
}

/// Build a reusable, deferred durable body; no pre-bind initializer runs at construction.
pub(crate) fn expand(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let context = Ident::new("__durable_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
        error: None,
    };
    let (prefix, statements) = crate::context_expand::split_prefix(statements);
    let body = statements
        .into_iter()
        .map(|stmt| lower.fold_stmt(stmt))
        .collect::<Vec<_>>();
    if let Some(error) = lower.error {
        return Err(error);
    }
    Ok(
        quote!(#path::workflow::Durable::from_body(move |#context| {#(#prefix)* async move {
            #[allow(unreachable_code)] { Ok({ #(#body)* }) }
        } })),
    )
}

/// Lower a workflow callback's owned input without adding a hidden payload clone.
/// The workflow executor rebuilds this callback for each durable evaluation/replay.
pub(crate) fn expand_once(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let tokens = expand(statements)?;
    let expression: syn::Expr = syn::parse2(tokens)?;
    let syn::Expr::Call(mut call) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected durable body constructor",
        ));
    };
    let path = crate::expand::effect_crate_path()?;
    *call.func = parse_quote!(#path::workflow::Durable::from_body_once);
    Ok(quote!(#call))
}

/// Focused native lowering checks for domain control flow and lexical boundaries.
#[cfg(test)]
mod tests {
    use super::*;
    /// Generated binds retain guards and native callback boundaries while enforcing the domain bridge.
    #[test]
    fn native_lowering_contract() -> syn::Result<()> {
        let statements = syn::parse::Parser::parse_str(
            syn::Block::parse_within,
            "let value=step()?; match value {v if v>0=>v,_=>0}; let callback=||nested()?;",
        )?;
        let mut lower = Lower {
            context: Ident::new("durable_context", Span::mixed_site()),
            error: None,
        };
        let statements = statements
            .into_iter()
            .map(|stmt| lower.fold_stmt(stmt))
            .collect::<Vec<_>>();
        let output = quote!(#(#statements)*).to_string();
        assert!(output.contains("if v > 0"));
        assert!(output.contains("return Err (failure)"));
        assert!(output.contains("nested () ?"));
        assert!(lower.error.is_none());
        Ok(())
    }
}
