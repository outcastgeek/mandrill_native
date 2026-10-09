//! Stable Rust lowering of pull-driven yield bodies; no coroutine feature or producer task.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::fold::Fold;
use syn::{Expr, Stmt};

/// Rewrites only the stream body, preserving native control flow and nested callback boundaries.
struct Lower {
    /// Typed context for Effect/resource binds and one-value suspension.
    context: Ident,
    /// Diagnostic for an unsupported completion/yield form.
    error: Option<syn::Error>,
    /// Loop/label targets carry the resource checkpoint used by break and continue.
    targets: Vec<(Option<syn::Lifetime>, Ident, bool)>,
    /// Hygienic checkpoint ordinal has no durable or protocol identity meaning.
    checkpoints: usize,
}
impl Lower {
    /// Allocate a hygienic scope checkpoint name, never an application identity.
    fn checkpoint(&mut self) -> Ident {
        let index = self.checkpoints;
        self.checkpoints += 1;
        Ident::new(&format!("__stream_checkpoint_{index}"), Span::mixed_site())
    }
    /// Find the targeted loop/label without interpreting unrelated nested callbacks.
    fn target(&self, label: &Option<syn::Lifetime>, continuing: bool) -> Option<&Ident> {
        self.targets
            .iter()
            .rev()
            .find(|(candidate, _, is_loop)| {
                (!continuing || *is_loop)
                    && match label {
                        Some(label) => candidate.as_ref() == Some(label),
                        None => *is_loop,
                    }
            })
            .map(|(_, checkpoint, _)| checkpoint)
    }
    /// Lower a lexical fragment which holds resources across suspension and releases on completion.
    fn scoped_block(&mut self, block: syn::Block, checkpoint: Ident) -> syn::Block {
        let statements = block
            .stmts
            .into_iter()
            .map(|stmt| self.fold_stmt(stmt))
            .collect::<Vec<_>>();
        let context = &self.context;
        syn::parse_quote!({let #checkpoint=#context.checkpoint();let __stream_fragment={#(#statements)*};#context.release(#checkpoint).await?;__stream_fragment})
    }
}
impl Fold for Lower {
    /// Replace yields with suspension and Effects with lazy runtime binds.
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
                syn::parse_quote_spanned!(step.question_token.span=> #context.bind(#program).await?)
            }
            Expr::Yield(step) => {
                let value = step
                    .expr
                    .map(|value| self.fold_expr(*value))
                    .unwrap_or_else(|| syn::parse_quote!(()));
                let context = &self.context;
                syn::parse_quote_spanned!(step.yield_token.span=> #context.emit(#value).await)
            }
            Expr::Macro(invocation) if invocation.mac.path.is_ident("yield_from") => {
                match syn::parse2::<Expr>(invocation.mac.tokens.clone()) {
                    Ok(source) => {
                        let context = &self.context;
                        syn::parse_quote!(#context.append(#source).await?)
                    }
                    Err(error) => {
                        self.error = Some(error);
                        Expr::Macro(invocation)
                    }
                }
            }
            Expr::Return(step) => {
                if step.expr.is_some() {
                    self.error = Some(syn::Error::new_spanned(
                        &step,
                        "stream completion uses return; or unit completion",
                    ));
                }
                syn::parse_quote!(return Ok(()))
            }
            Expr::Break(mut step) => {
                step.expr = step.expr.map(|value| Box::new(self.fold_expr(*value)));
                if let Some(checkpoint) = self.target(&step.label, false) {
                    let context = &self.context;
                    let label = &step.label;
                    if let Some(value) = step.expr {
                        syn::parse_quote!({let __stream_break=#value;#context.release(#checkpoint).await?;break #label __stream_break})
                    } else {
                        syn::parse_quote!({#context.release(#checkpoint).await?;break #label})
                    }
                } else {
                    Expr::Break(step)
                }
            }
            Expr::Continue(step) => {
                if let Some(checkpoint) = self.target(&step.label, true) {
                    let context = &self.context;
                    syn::parse_quote!({#context.release(#checkpoint).await?;#step})
                } else {
                    Expr::Continue(step)
                }
            }
            Expr::While(mut step) => {
                step.cond = Box::new(self.fold_expr(*step.cond));
                let checkpoint = self.checkpoint();
                self.targets.push((
                    step.label.as_ref().map(|label| label.name.clone()),
                    checkpoint.clone(),
                    true,
                ));
                step.body = self.scoped_block(step.body, checkpoint);
                self.targets.pop();
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.cooperate().await?;));
                Expr::While(step)
            }
            Expr::ForLoop(mut step) => {
                step.expr = Box::new(self.fold_expr(*step.expr));
                let checkpoint = self.checkpoint();
                self.targets.push((
                    step.label.as_ref().map(|label| label.name.clone()),
                    checkpoint.clone(),
                    true,
                ));
                step.body = self.scoped_block(step.body, checkpoint);
                self.targets.pop();
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.cooperate().await?;));
                Expr::ForLoop(step)
            }
            Expr::Loop(mut step) => {
                let checkpoint = self.checkpoint();
                self.targets.push((
                    step.label.as_ref().map(|label| label.name.clone()),
                    checkpoint.clone(),
                    true,
                ));
                step.body = self.scoped_block(step.body, checkpoint);
                self.targets.pop();
                let context = &self.context;
                step.body
                    .stmts
                    .insert(0, syn::parse_quote!(#context.cooperate().await?;));
                Expr::Loop(step)
            }
            Expr::Block(mut step) => {
                let checkpoint = self.checkpoint();
                if let Some(label) = &step.label {
                    self.targets
                        .push((Some(label.name.clone()), checkpoint.clone(), false));
                }
                step.block = self.scoped_block(step.block, checkpoint);
                if step.label.is_some() {
                    self.targets.pop();
                }
                Expr::Block(step)
            }
            other => syn::fold::fold_expr(self, other),
        }
    }
    /// Handle the statement form of the local marker before it can become an exported macro call.
    fn fold_stmt(&mut self, stmt: Stmt) -> Stmt {
        if let Stmt::Macro(marker) = &stmt
            && marker.mac.path.is_ident("yield_from")
        {
            let source = match syn::parse2::<Expr>(marker.mac.tokens.clone()) {
                Ok(source) => source,
                Err(error) => {
                    self.error = Some(error);
                    return stmt;
                }
            };
            let context = &self.context;
            return syn::parse_quote!(#context.append(#source).await?;);
        }
        syn::fold::fold_stmt(self, stmt)
    }
    /// Nested items retain their own yield/Result semantics.
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
}
/// Construct a reusable stream factory; all body evaluation is deferred until pulling.
pub(crate) fn expand(statements: Vec<Stmt>) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let context = Ident::new("__stream_context", Span::mixed_site());
    let mut lower = Lower {
        context: context.clone(),
        error: None,
        targets: Vec::new(),
        checkpoints: 0,
    };
    let (prefix, statements) = crate::context_expand::split_prefix(statements);
    let statements = statements
        .into_iter()
        .map(|stmt| lower.fold_stmt(stmt))
        .collect::<Vec<_>>();
    if let Some(error) = lower.error {
        return Err(error);
    }
    Ok(
        quote!(#path::Stream::from_body(move |#context| {#(#prefix)* async move { #[allow(unreachable_code)] { let _: ()={#(#statements)*}; Ok(()) } } })),
    )
}

/// Focused native lowering checks for domain control flow and lexical boundaries.
#[cfg(test)]
mod tests {
    use super::*;
    /// Generated binds retain guards and native callback boundaries while enforcing the domain bridge.
    #[test]
    fn native_lowering_contract() {
        let statements = syn::parse::Parser::parse_str(
            syn::Block::parse_within,
            "yield_from!(helper()); for value in values {yield step(value)?;continue;}",
        )
        .expect("Rust statements");
        let mut lower = Lower {
            context: Ident::new("stream_context", Span::mixed_site()),
            error: None,
            targets: Vec::new(),
            checkpoints: 0,
        };
        let statements = statements
            .into_iter()
            .map(|stmt| lower.fold_stmt(stmt))
            .collect::<Vec<_>>();
        let output = quote!(#(#statements)*).to_string();
        assert!(output.contains("append"));
        assert!(output.contains("emit"));
        assert!(output.contains("release"));
        assert!(!output.contains("yield_from"));
        assert!(lower.error.is_none());
    }
}
