//! Native Rust control flow over a single, retry-owned transaction journal.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::fold::Fold;
use syn::{Expr, Stmt};

/// Rewrites only the transaction body; nested callbacks and items keep their own `?`.
struct Lower {
    /// Hygienic journal bridge supplied for this attempt.
    context: Ident,
}
impl Fold for Lower {
    /// Bind transactions through the typed journal and preserve native guards and moves.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "Other syn expression variants retain native Rust syntax"
    )]
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Closure(_) | Expr::Async(_) => expr,
            Expr::Try(step) => {
                let program = self.fold_expr(*step.expr);
                let context = &self.context;
                syn::parse_quote_spanned!(step.question_token.span=> #context.bind(#program)?)
            }
            Expr::Return(step) => {
                let value = step
                    .expr
                    .map(|value| self.fold_expr(*value))
                    .unwrap_or_else(|| syn::parse_quote!(()));
                syn::parse_quote!(return Ok(#value))
            }
            Expr::While(mut step) => {
                step.cond = Box::new(self.fold_expr(*step.cond));
                step.body = self.fold_block(step.body);
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.checkpoint()?;));
                Expr::While(step)
            }
            Expr::ForLoop(mut step) => {
                step.expr = Box::new(self.fold_expr(*step.expr));
                step.body = self.fold_block(step.body);
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.checkpoint()?;));
                Expr::ForLoop(step)
            }
            Expr::Loop(mut step) => {
                step.body = self.fold_block(step.body);
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.checkpoint()?;));
                Expr::Loop(step)
            }
            other => syn::fold::fold_expr(self, other),
        }
    }
    /// Item bodies are separate language and lifetime boundaries.
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
}

/// Construct an inert transaction whose locals are rebuilt on every journal attempt.
/// The typed bridge rejects Effects in every position, including terminal branches.
pub(crate) fn expand(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let context = Ident::new("__transaction_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
    };
    let body = statements
        .into_iter()
        .map(|stmt| lower.fold_stmt(stmt))
        .collect::<Vec<_>>();
    Ok(
        quote!(#path::transactional::Transaction::from_body(move |#context| {
            #[allow(unreachable_code)] { Ok({ #(#body)* }) }
        })),
    )
}

/// Focused native lowering checks for domain control flow and lexical boundaries.
#[cfg(test)]
mod tests {
    use super::*;
    /// Generated binds retain guards and native callback boundaries while enforcing the domain bridge.
    #[test]
    fn native_lowering_contract() {
        let statements=syn::parse::Parser::parse_str(syn::Block::parse_within,"let value=step()?; match value {v if v>0=>v,_=>0}; while ready() {step()?;} let callback=||nested()?;").expect("Rust statements");
        let mut lower = Lower {
            context: Ident::new("transaction_context", Span::mixed_site()),
        };
        let statements = statements
            .into_iter()
            .map(|stmt| lower.fold_stmt(stmt))
            .collect::<Vec<_>>();
        let output = quote!(#(#statements)*).to_string();
        assert!(output.contains("if v > 0"));
        assert!(output.contains("checkpoint"));
        assert!(output.contains("nested () ?"));
    }
}
