use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::fold::Fold;
use syn::spanned::Spanned;
use syn::{Expr, Stmt};

struct Target {
    label: Option<syn::Lifetime>,
    checkpoint: Ident,
    continuing: bool,
}
struct Lower {
    path: TokenStream,
    context: Ident,
    targets: Vec<Target>,
    checkpoints: usize,
}
impl Lower {
    fn checkpoint(&mut self) -> Ident {
        let ordinal = self.checkpoints;
        self.checkpoints += std::num::NonZeroUsize::MIN.get();
        Ident::new(
            &format!("__effect_checkpoint_{ordinal}"),
            Span::mixed_site(),
        )
    }
    fn close(&self, checkpoint: &Ident) -> TokenStream {
        let context = &self.context;
        quote!(if let Err(__effect_failure) = #context.release(#checkpoint).await { return __effect_failure; })
    }
    fn block(
        &mut self,
        block: syn::Block,
        checkpoint: Option<Ident>,
        cooperate: bool,
    ) -> syn::Block {
        let needs_scope = block.stmts.iter().any(|statement| match statement {
            Stmt::Local(local) => local
                .init
                .as_ref()
                .is_some_and(|init| crate::parse::has_effect_steps(&init.expr)),
            Stmt::Expr(expr, _) => crate::parse::has_effect_steps(expr),
            Stmt::Item(_) | Stmt::Macro(_) => false,
        });
        let checkpoint = checkpoint.or_else(|| needs_scope.then(|| self.checkpoint()));
        let statements = block
            .stmts
            .into_iter()
            .map(|statement| self.fold_stmt(statement))
            .collect::<Vec<_>>();
        let context = &self.context;
        let yielding = cooperate.then(|| quote!(if let Err(__effect_failure) = #context.cooperate().await { return __effect_failure; }));
        match checkpoint {
            Some(checkpoint) => {
                let close = self.close(&checkpoint);
                syn::parse_quote!({ let #checkpoint = #context.checkpoint(); #yielding let __effect_value = { #(#statements)* }; #close __effect_value })
            }
            None => syn::parse_quote!({ #yielding #(#statements)* }),
        }
    }
    fn target(&self, label: &Option<syn::Lifetime>, continuing: bool) -> Option<&Ident> {
        self.targets
            .iter()
            .rev()
            .find(|target| {
                (!continuing || target.continuing)
                    && match label {
                        Some(label) => target.label.as_ref() == Some(label),
                        None => target.continuing,
                    }
            })
            .map(|target| &target.checkpoint)
    }
}
impl Fold for Lower {
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "Delegate ordinary and future non-exhaustive syn syntax to syn folding"
    )]
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Closure(expr) => Expr::Closure(expr),
            Expr::Async(expr) => Expr::Async(expr),
            Expr::Try(expr) => {
                let program = self.fold_expr(*expr.expr);
                let context = &self.context;
                syn::parse_quote_spanned!(expr.question_token.span=> match #context.bind(#program).await { Ok(__effect_value) => __effect_value, Err(__effect_failure) => return __effect_failure })
            }
            Expr::Return(expr) => {
                let value = expr
                    .expr
                    .map(|expr| self.fold_expr(*expr))
                    .unwrap_or_else(|| syn::parse_quote!(()));
                let path = &self.path;
                syn::parse_quote_spanned!(expr.return_token.span=> return #path::Exit::Success(#value))
            }
            Expr::Break(mut expr) => {
                expr.expr = expr.expr.map(|value| Box::new(self.fold_expr(*value)));
                match self.target(&expr.label, false) {
                    Some(checkpoint) => {
                        let close = self.close(checkpoint);
                        let label = &expr.label;
                        match expr.expr {
                            Some(value) => {
                                syn::parse_quote!({ let __effect_break_value = #value; #close break #label __effect_break_value })
                            }
                            None => syn::parse_quote!({ #close break #label }),
                        }
                    }
                    None => Expr::Break(expr),
                }
            }
            Expr::Continue(expr) => match self.target(&expr.label, true) {
                Some(checkpoint) => {
                    let close = self.close(checkpoint);
                    syn::parse_quote_spanned!(expr.span()=> { #close #expr })
                }
                None => Expr::Continue(expr),
            },
            Expr::Loop(mut expr) => {
                let checkpoint = self.checkpoint();
                self.targets.push(Target {
                    label: expr.label.as_ref().map(|label| label.name.clone()),
                    checkpoint: checkpoint.clone(),
                    continuing: true,
                });
                expr.body = self.block(expr.body, Some(checkpoint), true);
                let _ = self.targets.pop();
                Expr::Loop(expr)
            }
            Expr::While(mut expr) => {
                expr.cond = Box::new(self.fold_expr(*expr.cond));
                let checkpoint = self.checkpoint();
                self.targets.push(Target {
                    label: expr.label.as_ref().map(|label| label.name.clone()),
                    checkpoint: checkpoint.clone(),
                    continuing: true,
                });
                expr.body = self.block(expr.body, Some(checkpoint), true);
                let _ = self.targets.pop();
                Expr::While(expr)
            }
            Expr::ForLoop(mut expr) => {
                expr.expr = Box::new(self.fold_expr(*expr.expr));
                let checkpoint = self.checkpoint();
                self.targets.push(Target {
                    label: expr.label.as_ref().map(|label| label.name.clone()),
                    checkpoint: checkpoint.clone(),
                    continuing: true,
                });
                expr.body = self.block(expr.body, Some(checkpoint), true);
                let _ = self.targets.pop();
                Expr::ForLoop(expr)
            }
            Expr::Block(mut expr) => {
                if let Some(label) = &expr.label {
                    let checkpoint = self.checkpoint();
                    self.targets.push(Target {
                        label: Some(label.name.clone()),
                        checkpoint: checkpoint.clone(),
                        continuing: false,
                    });
                    expr.block = self.block(expr.block, Some(checkpoint), false);
                    let _ = self.targets.pop();
                } else {
                    expr.block = self.fold_block(expr.block);
                }
                Expr::Block(expr)
            }
            other => syn::fold::fold_expr(self, other),
        }
    }
    fn fold_block(&mut self, block: syn::Block) -> syn::Block {
        self.block(block, None, false)
    }
}
pub(crate) fn expand(statements: Vec<Stmt>, path: &TokenStream) -> TokenStream {
    let context = Ident::new(stringify!(__effect_context), Span::mixed_site());
    let prefix_length = statements
        .iter()
        .take_while(|statement| {
            if crate::parse::has_loops(std::slice::from_ref(statement)) {
                return false;
            }
            match statement {
                Stmt::Local(local) => local.init.as_ref().is_none_or(|init| {
                    init.diverge.is_none() && crate::parse::pure(&init.expr).is_ok()
                }),
                Stmt::Expr(expr, Some(_)) => crate::parse::pure(expr).is_ok(),
                Stmt::Item(_) => true,
                Stmt::Expr(_, None) | Stmt::Macro(_) => false,
            }
        })
        .count();
    let mut remaining = statements;
    let body = remaining.split_off(prefix_length);
    let prefix = remaining;
    let mut lower = Lower {
        path: path.clone(),
        context: context.clone(),
        targets: Vec::new(),
        checkpoints: usize::default(),
    };
    let statements = body
        .into_iter()
        .map(|statement| lower.fold_stmt(statement))
        .collect::<Vec<_>>();
    quote_spanned!(Span::call_site()=> #path::reusable(move || { #(#prefix)* #path::async_workflow(move |#context| async move { #[allow(unreachable_code)] { #path::Exit::Success({ #(#statements)* }) } }) }))
}
