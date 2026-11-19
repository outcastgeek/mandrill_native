//! Existing computation grammar lowered to inferred typed descriptors BEFORE
//! ordinary Effect's Arc erasure. Native role bodies retain their Rust language.
use crate::callback_expand::Input;
use crate::domain_expand::BodyCategory;
use crate::prepared_resource_expand::resource_program;
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Stmt};

pub(crate) fn root(expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    if let Expr::Macro(expression) = &expression
        && is_owned_macro_path(&expression.mac.path, path)
    {
        let name = expression
            .mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        if (name == "effect" || name == "e")
            && let crate::parse::EffectInput::Statements(statements) =
                syn::parse2::<crate::parse::EffectInput>(expression.mac.tokens.clone())?
        {
            return statements_lower(&statements, path);
        }
    }
    program(expression, path)
}
pub(crate) fn program(expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    if let Expr::MethodCall(call) = &expression
        && call.method == "either"
        && call.args.is_empty()
    {
        let source = program((*call.receiver).clone(), path)?;
        let graph = quote!(#path::__private::prepared_graph);
        return Ok(quote!(#graph::either(#source)));
    }
    if let Expr::MethodCall(call) = &expression
        && call.method == "race"
        && call.args.len() == 1
    {
        let left = program((*call.receiver).clone(), path)?;
        let right = program(
            call.args
                .first()
                .cloned()
                .ok_or_else(|| syn::Error::new_spanned(call, "race needs one right program"))?,
            path,
        )?;
        let graph = quote!(#path::__private::prepared_graph);
        return Ok(quote!(#graph::race(#left,#right)));
    }
    if let Expr::MethodCall(call) = &expression
        && (call.method == "fork" || call.method == "fork_on")
    {
        let mut call = call.clone();
        call.args = call
            .args
            .into_iter()
            .map(|argument| program(argument, path).and_then(syn::parse2::<Expr>))
            .collect::<syn::Result<_>>()?;
        return Ok(quote!(#call));
    }
    if let Expr::Macro(expression) = &expression {
        if !is_owned_macro_path(&expression.mac.path, path) {
            return Ok(quote!(#expression));
        }
        let name = expression
            .mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        if name == "effect_map_error" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            let _mode = input.mode(&[], "default")?;
            let mut fields = input.fields;
            let source =
                fields.transform("program", |expr, inline| program_field(expr, inline, path))?;
            let map = fields.role("map", 1, BodyCategory::Native)?;
            let graph = quote!(#path::__private::prepared_graph);
            return fields.finish_inferred(
                quote!(N, F),
                &[("program", quote!(N)), ("map", quote!(F))],
                quote!(#graph::Outcome<N,#graph::ErrorTransform<F>>),
                quote!(N: #graph::Description,F: Fn(N::Error)->N::Error + Send + 'static),
                quote!(#graph::map_error(#source,#map)),
            );
        }
        if name == "effect_ref" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            if is_prepared_ref(&input) {
                return prepared_ref(input, path);
            }
        }
        if name == "effect_scope_acquire" || name == "effect_finalizer" || name == "effect_scope" {
            return crate::prepared_scope_expand::scope_program(
                &name,
                syn::parse2(expression.mac.tokens.clone())?,
                path,
            );
        }
        if name == "layer" || name == "effect_with_layer" {
            return crate::prepared_layer_expand::layer_program(
                &name,
                syn::parse2(expression.mac.tokens.clone())?,
                path,
            );
        }
        if name == "effect_resource" || name == "effect_using" {
            return resource_program(&name, syn::parse2(expression.mac.tokens.clone())?, path);
        }
        if name == "effect" || name == "e" {
            let input = syn::parse2::<crate::parse::EffectInput>(expression.mac.tokens.clone())?;
            return match input {
                crate::parse::EffectInput::Statements(statements) => {
                    let body = statements_lower(&statements, path)?;
                    let graph = quote!(#path::__private::prepared_graph);
                    Ok(quote!(#graph::defer(move||{#body})))
                }
                crate::parse::EffectInput::Interpret(mode, _) => Err(syn::Error::new(
                    mode.span(),
                    "run the prepared description at its selected interpreter boundary",
                )),
                crate::parse::EffectInput::Sleep(_) => Err(syn::Error::new_spanned(
                    expression,
                    "prepared sleep requires an explicitly reserved timer descriptor",
                )),
                crate::parse::EffectInput::Either(mut fields) => {
                    let source = fields.transform("program", |expr, _| program(expr, path))?;
                    let graph = quote!(#path::__private::prepared_graph);
                    fields.finish(quote!(#graph::either(#source)))
                }
                crate::parse::EffectInput::Log(_) => Err(syn::Error::new_spanned(
                    expression,
                    "prepared logging requires a selected telemetry descriptor",
                )),
                crate::parse::EffectInput::ConsoleWrite(_) => Err(syn::Error::new_spanned(
                    expression,
                    "prepared console writes require a selected provider descriptor",
                )),
                crate::parse::EffectInput::NativePrimitive(mode, _) => Err(syn::Error::new(
                    mode.span(),
                    "this operation requires an explicitly selected prepared descriptor",
                )),
                crate::parse::EffectInput::Defer(mut fields) => {
                    let body = fields.transform("body", |expression, inline| {
                        let expression = if inline {
                            syn::parse2(crate::domain_expand::role(expression, 0, BodyCategory::Native)?)?
                        } else {
                            expression
                        };
                        loop_factory(expression, path)
                    })?;
                    let graph = quote!(#path::__private::prepared_graph);
                    fields.finish(quote!(#graph::defer(#body)))
                }
                crate::parse::EffectInput::WhileLoop(mut fields) => {
                    let guard = fields.role("guard", 0, BodyCategory::Native)?;
                    let on_capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
                    let body = fields.transform("body", |expression, inline| {
                        let expression = if inline {
                            syn::parse2(crate::domain_expand::role(
                                expression,
                                0,
                                BodyCategory::Native,
                            )?)?
                        } else {
                            expression
                        };
                        loop_factory(expression, path)
                    })?;
                    let graph = quote!(#path::__private::prepared_graph);
                    fields.finish(quote!(#graph::while_loop(#guard, #body, #on_capacity)))
                }
                crate::parse::EffectInput::Continuation(mut fields) => {
                    let source = fields.transform("program", |expr, _| program(expr, path))?;
                    let body = fields.role("body", 1, BodyCategory::Native)?;
                    let graph = quote!(#path::__private::prepared_graph);
                    fields.finish_inferred(quote!(L,F,N),
                        &[("program",quote!(L)),("body",quote!(F))],
                        quote!(L::Bound<F,N>),
                        quote!(L: #graph::Bindable,N: #graph::Description<Error=L::Error,Env=L::Env>,F: FnOnce(L::Value)->N + Send + 'static),
                        quote!(#graph::bind(#source,#body)))
                }
            };
        }
        if name == "effect_provide_some" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            let _mode = input.mode(&[], "default")?;
            let mut fields = input.fields;
            let source = fields.transform("program", |expr, _| program(expr, path))?;
            let project = fields.role("project", 1, BodyCategory::Native)?;
            let graph = quote!(#path::__private::prepared_graph);
            return fields.finish_inferred(quote!(N,F,I),
                &[("program",quote!(N)),("project",quote!(F))],
                quote!(#graph::Provided<N,F,I>),
                quote!(N: #graph::Description,I: Send + Sync + 'static,F: FnOnce(&I)->N::Env + Send + 'static),
                quote!(#graph::provide_some(#source,#project)));
        }
        if name == "service" {
            let mut fields =
                syn::parse2::<crate::domain_expand::Fields>(expression.mac.tokens.clone())?;
            if fields.contains("env") {
                let environment = if fields.contains("environment") {
                    Some(fields.raw("environment")?)
                } else {
                    None
                };
                let select = fields.role("env", 1, BodyCategory::Native)?;
                let graph = quote!(#path::__private::prepared_graph);
                if let Some(environment) = environment {
                    return fields.finish_inferred(quote!(F,A,E),&[("env",quote!(F))],
                        quote!(impl #graph::Description<Value=A,Error=E,Env=#environment>),
                        quote!(F: FnOnce(&#environment)->A + Send + 'static,A: Send + 'static,E: Send + 'static,#environment: Send + Sync + 'static),
                        quote!(#graph::service(#select)));
                }
                return fields.finish_inferred(quote!(F,A,E,R),&[("env",quote!(F))],
                    quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
                    quote!(F: FnOnce(&R)->A + Send + 'static,A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static),
                    quote!(#graph::service(#select)));
            }
        }
        if matches!(
            name.as_str(),
            "effect_cpu" | "effect_try_cpu" | "effect_blocking" | "effect_try_blocking"
        ) {
            return crate::prepared_future_expand::job(
                &name,
                syn::parse2(expression.mac.tokens.clone())?,
                path,
            );
        }
        if name == "effect_race" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            let _mode = input.mode(&["once", "shared"], "default")?;
            let mut fields = input.fields;
            let source = match (fields.contains("program"), fields.contains("body")) {
                (true, false) => "program",
                (false, true) => "body",
                _ => {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "specify exactly one of program or body",
                    ));
                }
            };
            let left =
                fields.transform(source, |expr, inline| program_field(expr, inline, path))?;
            let right = fields.transform("right", |expr, _| program(expr, path))?;
            let graph = quote!(#path::__private::prepared_graph);
            return fields.finish_inferred(quote!(L,R),&[(source,quote!(L)),("right",quote!(R))],quote!(#graph::Race<L,R>),quote!(L: #graph::Description,R: #graph::Description<Value=L::Value,Error=L::Error,Env=L::Env>,L::Env: #path::prepared_provider::HasPreparedProvider<L::Value,L::Error,#path::prepared_provider::TaskProvider>),quote!(#graph::race(#left,#right)));
        }
        if name == "effect_future" || name == "effect_try_future" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            return crate::prepared_future_expand::expand(&name, input, path);
        }
        if name == "effect_sync" || name == "effect_try_sync" {
            let input = syn::parse2::<Input>(expression.mac.tokens.clone())?;
            let _mode = input.mode(&["once"], "reusable")?;
            let mut fields = input.fields;
            let callback = fields.role("body", 0, BodyCategory::Native)?;
            let graph = quote!(#path::__private::prepared_graph);
            if name == "effect_sync" {
                return fields.finish_inferred(quote!(F,A,E,R),&[("body",quote!(F))],
                    quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
                    quote!(F: FnOnce()->A + Send + 'static,A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static),
                    quote!(#graph::sync(#callback)));
            }
            if fields.contains("on_error") {
                let mapper = fields.role("on_error", 1, BodyCategory::Native)?;
                return fields.finish_inferred(quote!(F,M,A,X,E,R),
                    &[("body",quote!(F)),("on_error",quote!(M))],
                    quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
                    quote!(F: FnOnce()->::core::result::Result<A,X> + Send + 'static,M: FnOnce(X)->E + Send + 'static,A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static),
                    quote!(#graph::try_sync(#callback,#mapper)));
            }
            return fields.finish_inferred(quote!(F,A,E,R),
                &[("body",quote!(F))],
                quote!(impl #graph::Description<Value=A,Error=E,Env=R>),
                quote!(F: FnOnce()->::core::result::Result<A,E> + Send + 'static,A: Send + 'static,E: Send + 'static,R: Send + Sync + 'static),
                quote!(#graph::try_sync(#callback,::core::convert::identity)));
        }
    }
    // Explicit concrete descriptors remain typed. An opaque ordinary Effect does
    // not implement Description and cannot masquerade as a strict capture proof.
    Ok(quote!(#expression))
}
// A loop factory is reusable native Rust: its prefix runs on every invocation.
// Only descriptor-return positions are unified as graph branches.
pub(crate) fn loop_factory(mut expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    if let Expr::Block(block) = &mut expression
        && block.label.is_none()
    {
        if let Some(Stmt::Expr(value, None)) = block.block.stmts.last_mut() {
            *value = syn::parse2(loop_factory(value.clone(), path)?)?;
        }
        return native_descriptors(expression, path);
    }
    if let Expr::Paren(value) = &mut expression {
        *value.expr = syn::parse2(loop_factory((*value.expr).clone(), path)?)?;
        return Ok(quote!(#expression));
    }
    if let Expr::Group(value) = &mut expression {
        *value.expr = syn::parse2(loop_factory((*value.expr).clone(), path)?)?;
        return Ok(quote!(#expression));
    }
    let Expr::Closure(mut closure) = expression else {
        return native_descriptors(expression, path);
    };
    struct Count(usize);
    impl<'ast> syn::visit::Visit<'ast> for Count {
        fn visit_expr_return(&mut self, value: &'ast syn::ExprReturn) {
            if !value
                .expr
                .as_ref()
                .is_some_and(|expr| loop_always_returns(expr))
            {
                self.0 += 1;
            }
            syn::visit::visit_expr_return(self, value);
        }
        fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
        fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
        fn visit_item(&mut self, _: &'ast syn::Item) {}
    }
    let tail = if let Expr::Block(block) = closure.body.as_ref() {
        block.block.stmts.last().is_some_and(|statement| {
            matches!(statement, Stmt::Expr(value, None) if !loop_always_returns(value))
                || matches!(statement, Stmt::Macro(value) if value.semi_token.is_none())
        })
    } else {
        !loop_always_returns(&closure.body)
    };
    let mut count = Count(0);
    syn::visit::Visit::visit_expr(&mut count, &closure.body);
    let total = count.0 + usize::from(tail);
    struct LowerReturns<'a> {
        path: &'a TokenStream,
        total: usize,
        next: usize,
        error: Option<syn::Error>,
    }
    impl syn::fold::Fold for LowerReturns<'_> {
        fn fold_item(&mut self, item: syn::Item) -> syn::Item {
            item
        }
        fn fold_expr(&mut self, expression: Expr) -> Expr {
            if matches!(expression, Expr::Closure(_) | Expr::Async(_)) {
                return expression;
            }
            let unreachable_return = matches!(&expression, Expr::Return(value) if value.expr.as_ref().is_some_and(|expr| loop_always_returns(expr)));
            let expression = syn::fold::fold_expr(self, expression);
            let Expr::Return(mut value) = expression else {
                return expression;
            };
            if unreachable_return {
                return Expr::Return(value);
            }
            let index = self.next;
            self.next += 1;
            let actual = value
                .expr
                .take()
                .map_or_else(|| syn::parse_quote!(()), |v| *v);
            match loop_factory_return(actual, self.path).and_then(|value| {
                syn::parse2(loop_factory_choice(value, index, self.total, self.path))
            }) {
                Ok(actual) => value.expr = Some(Box::new(actual)),
                Err(error) => self.error = Some(error),
            }
            Expr::Return(value)
        }
    }
    let mut lower = LowerReturns {
        path,
        total,
        next: 0,
        error: None,
    };
    let mut body = syn::fold::Fold::fold_expr(&mut lower, (*closure.body).clone());
    if let Some(error) = lower.error {
        return Err(error);
    }
    if tail {
        if let Expr::Block(block) = &mut body {
            if let Some(statement) = block.block.stmts.last_mut() {
                let expression = if let Stmt::Expr(expression, None) = statement {
                    Some(expression.clone())
                } else if let Stmt::Macro(invocation) = statement
                    && invocation.semi_token.is_none()
                {
                    Some(Expr::Macro(syn::ExprMacro {
                        attrs: invocation.attrs.clone(),
                        mac: invocation.mac.clone(),
                    }))
                } else {
                    None
                };
                if let Some(expression) = expression {
                    let value = loop_factory_return(expression, path)?;
                    *statement = Stmt::Expr(
                        syn::parse2(loop_factory_choice(value, count.0, total, path))?,
                        None,
                    );
                }
            }
            body = syn::parse2(native_descriptors(body, path)?)?;
        } else {
            let value = loop_factory_return(body, path)?;
            body = syn::parse2(loop_factory_choice(value, count.0, total, path))?;
        }
    } else {
        body = syn::parse2(native_descriptors(body, path)?)?;
    }
    *closure.body = body;
    Ok(quote!(#closure))
}
fn loop_always_returns(expression: &Expr) -> bool {
    if matches!(expression, Expr::Return(_)) {
        return true;
    }
    if let Expr::Paren(value) = expression {
        return loop_always_returns(&value.expr);
    }
    if let Expr::Group(value) = expression {
        return loop_always_returns(&value.expr);
    }
    if let Expr::Block(block) = expression {
        return block.block.stmts.last().is_some_and(
            |statement| matches!(statement, Stmt::Expr(value, _) if loop_always_returns(value)),
        );
    }
    if let Expr::If(branch) = expression {
        let left = branch.then_branch.stmts.last().is_some_and(
            |statement| matches!(statement, Stmt::Expr(value, _) if loop_always_returns(value)),
        );
        return left
            && branch
                .else_branch
                .as_ref()
                .is_some_and(|(_, right)| loop_always_returns(right));
    }
    if let Expr::Match(branch) = expression {
        return !branch.arms.is_empty()
            && branch.arms.iter().all(|arm| loop_always_returns(&arm.body));
    }
    false
}
fn loop_factory_choice(
    mut body: TokenStream,
    index: usize,
    count: usize,
    path: &TokenStream,
) -> TokenStream {
    let graph = quote!(#path::__private::prepared_graph);
    if index + 1 < count {
        body = quote!(#graph::branch_left(#body));
    }
    for _ in 0..index {
        body = quote!(#graph::branch_right(#body));
    }
    body
}
fn loop_factory_return(mut expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    let graph = quote!(#path::__private::prepared_graph);
    if let Expr::Paren(value) = &mut expression {
        *value.expr = syn::parse2(loop_factory_return((*value.expr).clone(), path)?)?;
        return Ok(quote!(#expression));
    }
    if let Expr::Group(value) = &mut expression {
        *value.expr = syn::parse2(loop_factory_return((*value.expr).clone(), path)?)?;
        return Ok(quote!(#expression));
    }
    if let Expr::Block(block) = &mut expression
        && block.label.is_none()
    {
        if let Some(tail) = block.block.stmts.last_mut() {
            if let Stmt::Expr(value, None) = tail {
                *value = syn::parse2(loop_factory_return(value.clone(), path)?)?;
            } else if let Stmt::Macro(invocation) = tail
                && invocation.semi_token.is_none()
            {
                let value = Expr::Macro(syn::ExprMacro {
                    attrs: invocation.attrs.clone(),
                    mac: invocation.mac.clone(),
                });
                *tail = Stmt::Expr(syn::parse2(loop_factory_return(value, path)?)?, None);
            }
        }
        return native_descriptors(expression, path);
    }
    if let Expr::If(branch) = &expression
        && branch.else_branch.is_some()
    {
        let attrs = &branch.attrs;
        let condition = &branch.cond;
        let left_returns = branch.then_branch.stmts.last().is_some_and(
            |statement| matches!(statement, Stmt::Expr(value, _) if loop_always_returns(value)),
        );
        let right_returns = branch
            .else_branch
            .as_ref()
            .is_some_and(|(_, value)| loop_always_returns(value));
        let left = loop_factory_return(
            Expr::Block(syn::ExprBlock {
                attrs: Vec::new(),
                label: None,
                block: branch.then_branch.clone(),
            }),
            path,
        )?;
        let Some((_, right)) = &branch.else_branch else {
            return Err(syn::Error::new_spanned(
                condition,
                "loop factory branch requires an else",
            ));
        };
        let right = loop_factory_return((**right).clone(), path)?;
        return if left_returns || right_returns {
            Ok(quote!(#(#attrs)* if #condition { #left } else { #right }))
        } else {
            Ok(
                quote!(#(#attrs)* if #condition { #graph::branch_left(#left) } else { #graph::branch_right(#right) }),
            )
        };
    }
    if let Expr::Match(branch) = &expression {
        let attrs = &branch.attrs;
        let value = &branch.expr;
        let count = branch
            .arms
            .iter()
            .filter(|arm| !loop_always_returns(&arm.body))
            .count();
        let mut index = 0;
        let mut arms = Vec::with_capacity(branch.arms.len());
        for arm in &branch.arms {
            let attrs = &arm.attrs;
            let pattern = &arm.pat;
            let returns = loop_always_returns(&arm.body);
            let mut body = loop_factory_return((*arm.body).clone(), path)?;
            if !returns {
                body = loop_factory_choice(body, index, count, path);
                index += 1;
            }
            arms.push(quote!(#(#attrs)* #pattern => { #body },));
        }
        return Ok(quote!(#(#attrs)* match #value { #(#arms)* }));
    }
    program(expression, path)
}

/// Lower concrete descriptor expressions in this native syntax region. Nested
/// Rust callbacks, async blocks and items keep their own expansion boundaries.
pub(crate) fn native_descriptors(expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    struct Lower<'a> {
        path: &'a TokenStream,
        error: Option<syn::Error>,
    }
    impl syn::fold::Fold for Lower<'_> {
        fn fold_item(&mut self, item: syn::Item) -> syn::Item {
            item
        }

        fn fold_block(&mut self, mut block: syn::Block) -> syn::Block {
            // syn represents a braced macro in tail position as a statement.
            // Treat its owned form as a value, leaving native statements alone.
            if let Some(tail) = block.stmts.last_mut()
                && let Stmt::Macro(invocation) = tail
                && invocation.semi_token.is_none()
                && is_owned_macro_path(&invocation.mac.path, self.path)
            {
                *tail = Stmt::Expr(
                    Expr::Macro(syn::ExprMacro {
                        attrs: invocation.attrs.clone(),
                        mac: invocation.mac.clone(),
                    }),
                    None,
                );
            }
            syn::fold::fold_block(self, block)
        }

        fn fold_expr(&mut self, expression: Expr) -> Expr {
            if self.error.is_some()
                || matches!(expression, Expr::Closure(_) | Expr::Async(_) | Expr::Const(_))
            {
                return expression;
            }
            if matches!(expression, Expr::Macro(_)) {
                return match program(expression.clone(), self.path).and_then(syn::parse2) {
                    Ok(lowered) => lowered,
                    Err(error) => {
                        self.error = Some(error);
                        expression
                    }
                };
            }
            syn::fold::fold_expr(self, expression)
        }
    }
    let mut lower = Lower { path, error: None };
    let expression = syn::fold::Fold::fold_expr(&mut lower, expression);
    if let Some(error) = lower.error {
        Err(error)
    } else {
        Ok(quote!(#expression))
    }
}

/// Enter the syntactically supplied descriptor factory, keeping creation of
/// the factory and any nested native callbacks under their original scopes.
pub(crate) fn native_descriptor_callback(
    mut expression: Expr,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mut target = &mut expression;
    loop {
        if let Expr::Closure(callback) = target
            && callback.asyncness.is_none()
        {
            *callback.body = syn::parse2(native_descriptors((*callback.body).clone(), path)?)?;
            break;
        } else if let Expr::Paren(parenthesized) = target {
            target = &mut parenthesized.expr;
        } else if let Expr::Group(grouped) = target {
            target = &mut grouped.expr;
        } else if let Expr::Block(block) = target
            && block.label.is_none()
            && let Some(Stmt::Expr(tail, None)) = block.block.stmts.last_mut()
        {
            target = tail;
        } else {
            break;
        }
    }
    Ok(quote!(#expression))
}
pub(crate) fn program_field(
    expression: Expr,
    inline: bool,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    if inline {
        expression_lower(&expression, path)
    } else {
        program(expression, path)
    }
}
pub(crate) fn effect_callback(expression: Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    effect_callback_arity(expression, 1, path)
}
pub(crate) fn effect_callback_arity(
    expression: Expr,
    arity: usize,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Expr::Closure(mut callback) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "inline body requires a Rust closure",
        ));
    };
    if callback.inputs.len() != arity
        || callback.asyncness.is_some()
        || !matches!(callback.output, syn::ReturnType::Default)
        || !matches!(callback.body.as_ref(), Expr::Block(_))
    {
        return Err(syn::Error::new_spanned(
            callback,
            "inline body has the wrong parameters or requires a braced body",
        ));
    }
    callback.capture.get_or_insert_with(Default::default);
    let body = expression_lower(&callback.body, path)?;
    *callback.body = syn::parse2(body)?;
    Ok(quote!(#callback))
}
fn statements_lower(statements: &[Stmt], path: &TokenStream) -> syn::Result<TokenStream> {
    crate::parse::validate(statements)?;
    let graph = quote!(#path::__private::prepared_graph);
    let Some((statement, remaining)) = statements.split_first() else {
        return Ok(quote!(#graph::success(())));
    };
    let rest = statements_lower(remaining, path)?;
    match statement {
        Stmt::Local(local) => {
            if let Some(init) = &local.init
                && let Expr::Try(value) = init.expr.as_ref()
            {
                let source = program((*value.expr).clone(), path)?;
                let pattern = &local.pat;
                let attrs = &local.attrs;
                return Ok(
                    quote_spanned!(statement.span()=> #graph::bind(#source,move|#(#attrs)* #pattern|{#rest})),
                );
            }
            if let Some(init) = &local.init
                && control(&init.expr)
            {
                let source = expression_lower(&init.expr, path)?;
                let pattern = &local.pat;
                let attrs = &local.attrs;
                return Ok(
                    quote_spanned!(statement.span()=> #graph::bind(#source,move|#(#attrs)* #pattern|{#rest})),
                );
            }
            let mut local = local.clone();
            if let Some(init) = &mut local.init {
                *init.expr = syn::parse2(native_descriptors((*init.expr).clone(), path)?)?;
            }
            Ok(quote!(#local #rest))
        }
        Stmt::Expr(Expr::Try(value), semi) => {
            let source = program((*value.expr).clone(), path)?;
            if semi.is_none() && remaining.is_empty() {
                Ok(quote!(#graph::bind(#source,move|value|#graph::success(value))))
            } else {
                Ok(quote!(#graph::bind(#source,move|_|{#rest})))
            }
        }
        Stmt::Expr(Expr::Return(value), _) => {
            let value = value
                .expr
                .as_ref()
                .map_or_else(|| quote!(()), |value| quote!(#value));
            Ok(quote!(#graph::success(#value)))
        }
        Stmt::Expr(value, semi) if control(value) => {
            let source = expression_lower(value, path)?;
            if semi.is_none() && remaining.is_empty() {
                Ok(source)
            } else {
                Ok(quote!(#graph::bind(#source,move|_|{#rest})))
            }
        }
        Stmt::Expr(value, None) if remaining.is_empty() => Ok(quote!(#graph::success(#value))),
        Stmt::Macro(invocation) if invocation.semi_token.is_none() && remaining.is_empty() => {
            Ok(quote!(#graph::success({ #invocation })))
        }
        Stmt::Expr(_, _) | Stmt::Item(_) | Stmt::Macro(_) => Ok(quote!(#statement #rest)),
    }
}

fn control(expression: &Expr) -> bool {
    crate::parse::has_effect_steps(expression)
        && (matches!(expression, Expr::If(_) | Expr::Match(_))
            || matches!(expression,Expr::Block(block) if block.label.is_none()))
}
fn expression_lower(expression: &Expr, path: &TokenStream) -> syn::Result<TokenStream> {
    let graph = quote!(#path::__private::prepared_graph);
    if let Expr::If(branch) = expression {
        let attrs = &branch.attrs;
        let condition = &branch.cond;
        let left = statements_lower(&branch.then_branch.stmts, path)?;
        let right = match &branch.else_branch {
            Some((_, right)) => expression_lower(right, path)?,
            None => quote!(#graph::success(())),
        };
        return Ok(
            quote!(#(#attrs)* if #condition { #graph::branch_left(#left) } else { #graph::branch_right(#right) }),
        );
    }
    if let Expr::Match(branch) = expression {
        let attrs = &branch.attrs;
        let value = &branch.expr;
        let mut arms = Vec::new();
        for (index, arm) in branch.arms.iter().enumerate() {
            let attrs = &arm.attrs;
            let pattern = &arm.pat;
            let mut body = expression_lower(&arm.body, path)?;
            if index + 1 < branch.arms.len() {
                body = quote!(#graph::branch_left(#body));
            }
            for _ in 0..index {
                body = quote!(#graph::branch_right(#body));
            }
            // syn 3 stores a match guard in Pat::Guard, not a separate Arm
            // field. Quoting the complete pattern preserves its guard/spans.
            arms.push(quote_spanned!(arm.span()=> #(#attrs)* #pattern => {#body},));
        }
        return Ok(quote!(#(#attrs)* match #value {#(#arms)*}));
    }
    if let Expr::Block(block) = expression
        && block.label.is_none()
    {
        let attrs = &block.attrs;
        let body = statements_lower(&block.block.stmts, path)?;
        return Ok(quote!(#(#attrs)* #graph::defer(move||{#body})));
    }
    if let Expr::Try(value) = expression {
        let source = program((*value.expr).clone(), path)?;
        return Ok(quote!(#graph::bind(#source,move|value|#graph::success(value))));
    }
    Ok(quote!(#graph::success(#expression)))
}

/// Existing reference grammar, selecting the startup-backed native service owner.
pub(crate) fn prepared_ref(input: Input, path: &TokenStream) -> syn::Result<TokenStream> {
    if input.mode.as_ref().is_some_and(|mode| {
        matches!(
            mode.to_string().as_str(),
            "prepared_subscription_read" | "prepared_subscription_snapshot"
        )
    }) {
        return prepared_subscription_ref(input, path);
    }
    if input.mode.as_ref().is_some_and(|mode| {
        matches!(
            mode.to_string().as_str(),
            "prepared_synchronized" | "prepared_synchronized_pure"
        )
    }) {
        return prepared_synchronized_ref(input, path);
    }
    let mode = input.mode(
        &["prepared_modify", "prepared_update", "prepared_in_place"],
        "prepared_modify",
    )?;
    let mut fields = input.fields;
    let reference = fields.role("reference", 1, BodyCategory::Native)?;
    let role = if mode == "prepared_update" {
        "update"
    } else {
        "modify"
    };
    let callback = fields.role(role, 1, BodyCategory::Native)?;
    let graph = quote!(#path::__private::prepared_graph);
    let (value, callback_bound, invoke) = match mode.as_str() {
        "prepared_modify" => (
            quote!(B),
            quote!(F: FnOnce(&T)->(B,T) + Send + 'static),
            quote!((#reference)(environment).modify_now(#callback)),
        ),
        "prepared_update" => (
            quote!(()),
            quote!(F: FnOnce(&T)->T + Send + 'static),
            quote!((#reference)(environment).update_now(#callback)),
        ),
        "prepared_in_place" => (
            quote!(B),
            quote!(F: FnOnce(&mut T)->B + Send + 'static),
            quote!((#reference)(environment).modify_in_place_now(#callback)),
        ),
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "invalid prepared reference mode",
            ));
        }
    };
    let (generics, value_bound) = if mode == "prepared_update" {
        (quote!(S, F, T, E, R), quote!())
    } else {
        (quote!(S, F, T, B, E, R), quote!(B: Send + 'static,))
    };
    fields.finish_inferred(generics, &[("reference",quote!(S)),("modify",quote!(F)),("update",quote!(F))],
        quote!(impl #graph::Description<Value=#value,Error=E,Env=R>),
        quote!(T: Send + 'static,#value_bound E: Send + 'static,R: Send + Sync + 'static,S: for<'a> FnOnce(&'a R)-> &'a #path::PreparedAtomicRef<T> + Send + 'static,#callback_bound),
        quote!(#graph::service(move |environment: &R| { #invoke })))
}
pub(crate) fn is_prepared_ref(input: &Input) -> bool {
    input.mode.as_ref().is_some_and(|mode| {
        matches!(
            mode.to_string().as_str(),
            "prepared_modify"
                | "prepared_update"
                | "prepared_in_place"
                | "prepared_synchronized"
                | "prepared_synchronized_pure"
                | "prepared_subscription_read"
                | "prepared_subscription_snapshot"
        )
    })
}
fn prepared_subscription_ref(input: Input, path: &TokenStream) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "prepared_subscription_read",
            "prepared_subscription_snapshot",
        ],
        "prepared_subscription_read",
    )?;
    let mut fields = input.fields;
    let reference = fields.role("reference", 1, BodyCategory::Native)?;
    let mapper = fields.role("on_error", 1, BodyCategory::Native)?;
    let graph = quote!(#path::__private::prepared_graph);
    let (value, method) = if mode == "prepared_subscription_read" {
        (quote!(#path::PreparedPayloadView<T>), quote!(read_now))
    } else {
        (
            quote!(#path::PreparedSubscriptionSnapshot<T>),
            quote!(subscribe_now),
        )
    };
    fields.finish_inferred(quote!(S,M,T,E,R), &[("reference",quote!(S)),("on_error",quote!(M))],
        quote!(impl #graph::Description<Value=#value,Error=E,Env=R>),
        quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
            S: for<'a> FnOnce(&'a R)-> &'a #path::PreparedSubscriptionRef<T> + Send + 'static,
            M: FnOnce(#path::PreparedPubSubError)->E + Send + 'static),
        quote!(#graph::ready(move |environment: &R| match (#reference)(environment).#method() {
            Ok(value) => #path::Exit::Success(value), Err(error) => #path::Exit::Fail((#mapper)(error)),
        })))
}
pub(crate) fn is_owned_macro_path(invocation: &syn::Path, effect: &TokenStream) -> bool {
    if invocation.segments.len() == 1 {
        return invocation.leading_colon.is_none();
    }
    let prefix = || invocation.segments.iter().take(invocation.segments.len() - 1);
    if let Ok(owner) = syn::parse2::<syn::Path>(effect.clone())
        && prefix()
            .map(|segment| &segment.ident)
            .eq(owner.segments.iter().map(|segment| &segment.ident))
    {
        return true;
    }
    let Some(first) = invocation.segments.first() else {
        return false;
    };
    let Some(last) = invocation.segments.last() else {
        return false;
    };
    // All five facades re-export the same callback macros at their root.
    // The base `effect!` and `service!` are available through their hidden
    // Effect re-export, with the entire prefix required for that route.
    let facade = match invocation.segments.len() {
        2 => matches!(
            last.ident.to_string().as_str(),
            "effect_map_error" | "effect_ref" | "effect_scope_acquire" | "effect_finalizer"
                | "effect_scope" | "layer" | "effect_with_layer" | "effect_resource"
                | "effect_using" | "effect_provide_some" | "effect_cpu" | "effect_try_cpu"
                | "effect_blocking" | "effect_try_blocking" | "effect_race" | "effect_future"
                | "effect_try_future" | "effect_sync" | "effect_try_sync"
        ),
        4 => prefix()
            .skip(1)
            .map(|segment| &segment.ident)
            .eq(["__private", "effect"]),
        _ => false,
    };
    facade && crate::extension_support::EFFECT_FACADE_PACKAGES.into_iter().any(|package| {
        crate::extension_support::crate_path(package)
            .and_then(syn::parse2::<syn::Path>)
            .is_ok_and(|resolved| resolved.segments.first().is_some_and(|owner| owner.ident == first.ident))
    })
}

/// Existing synchronized reference grammar composed from canonical prepared Resource and Bind nodes.
pub(crate) fn prepared_synchronized_ref(
    input: Input,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &["prepared_synchronized", "prepared_synchronized_pure"],
        "prepared_synchronized",
    )?;
    let mut fields = input.fields;
    let reference = fields.role("reference", 1, BodyCategory::Native)?;
    let mapper = fields.role("on_error", 1, BodyCategory::Native)?;
    let pure = mode == "prepared_synchronized_pure";
    let modify = if pure {
        fields.role("modify", 1, BodyCategory::Native)?
    } else {
        fields.transform("modify", |expression, inline| {
            if inline {
                effect_callback(expression, path)
            } else {
                crate::prepared_resource_expand::native_release(expression, path)
            }
        })?
    };
    let graph = quote!(#path::__private::prepared_graph);
    let write = quote!(#path::__private::PreparedSynchronizedWrite<T,M,E>);
    let body = if pure {
        quote!(#graph::sync(move || (#modify)(snapshot)))
    } else {
        quote!((#modify)(snapshot))
    };
    let (generics, modify_bound) = if pure {
        (
            quote!(S, M, F, T, B, E, R),
            quote!(F: FnOnce(#path::PreparedPayloadView<T>)->(B,T) + Send + 'static,),
        )
    } else {
        (
            quote!(S, M, F, N, T, B, E, R),
            quote!(N: #graph::Description<Value=(B,T),Error=E,Env=R>, F: FnOnce(#path::PreparedPayloadView<T>)->N + Send + 'static,),
        )
    };
    fields.finish_inferred(generics, &[("reference",quote!(S)),("on_error",quote!(M)),("modify",quote!(F))],
        quote!(impl #graph::Description<Value=B,Error=E,Env=R>),
        quote!(T: Send + Sync + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
            S: for<'a> FnOnce(&'a R)-> &'a #path::PreparedSynchronizedRef<T> + Send + 'static,
            M: Fn(#path::PreparedSemaphoreError)->E + Send + Sync + 'static, #modify_bound),
        quote!({
            let acquire = #graph::native_future(move |environment: #graph::Environment<R>, context: #graph::FutureContext<E>| async move {
                match environment.with(|env| (#reference)(env).admit_write(context, #mapper)) {
                    Ok(Ok(pending)) => pending.await,
                    Ok(Err((error, _context, mapper))) => #path::Exit::Fail(mapper(error)),
                    Err(defect) => #path::Exit::Die(defect),
                }
            });
            let resource = #graph::resource(acquire, move |write: #write, _primary| {
                #graph::try_sync(move || write.release(), ::core::convert::identity)
            });
            #graph::bind(resource, move |owner: #graph::ResourceValue<#write>| {
                let snapshot = #graph::native_future(move |_environment: #graph::Environment<R>, _context: #graph::FutureContext<E>| async move {
                    match owner.with(|write| write.snapshot()) {
                        Ok(Ok(snapshot)) => #path::Exit::Success((owner, snapshot)),
                        Ok(Err(error)) => #path::Exit::Fail(error),
                        Err(defect) => #path::Exit::Die(defect),
                    }
                });
                #graph::bind(snapshot, move |(owner, snapshot)| {
                    let body = #body;
                    #graph::bind(body, move |(result, next)| {
                        #graph::native_future(move |_environment: #graph::Environment<R>, context: #graph::FutureContext<E>| async move {
                            if context.is_requested() { drop((result, next)); return #path::Exit::Interrupted; }
                            match owner.with(|write| write.commit(next)) {
                                Ok(Ok(())) => #path::Exit::Success(result),
                                Ok(Err(error)) => #path::Exit::Fail(error),
                                Err(defect) => #path::Exit::Die(defect),
                            }
                        })
                    })
                })
            })
        }))
}
