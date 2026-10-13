//! Named callback blocks over existing Effect constructors and typed operators.
//! Fields are evaluated once in source order. Bodies retain their owning language,
//! reusable/once captures and error channels; no runtime or provider is selected here.

use crate::domain_expand::{BodyCategory, Fields};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::parse_quote;
use syn::{Expr, Token};

/// Optional leading mode followed by ordered fields; a mode requires its semicolon.
/// This leaves ordinary Rust identifiers unambiguous in existing body macros.
pub(crate) struct Input {
    /// Explicit mode, such as once, contextual, pure, modify or merge.
    pub(crate) mode: Option<Ident>,
    /// Source-ordered role expressions and metadata.
    pub(crate) fields: Fields,
}
impl Parse for Input {
    /// Recognize only an identifier followed immediately by a semicolon as a mode.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let fork = input.fork();
        let mode = if fork.call(Ident::parse_any).is_ok() && fork.peek(Token![;]) {
            let mode = input.call(Ident::parse_any)?;
            input.parse::<Token![;]>()?;
            Some(mode)
        } else {
            None
        };
        Ok(Self {
            mode,
            fields: input.parse()?,
        })
    }
}
impl Input {
    /// Validate an owner's finite modes at the original keyword span.
    pub(crate) fn mode(&self, allowed: &[&str], default: &str) -> syn::Result<String> {
        let Some(mode) = &self.mode else {
            return Ok(default.into());
        };
        let name = mode.to_string();
        if allowed.contains(&name.as_str()) {
            Ok(name)
        } else {
            Err(syn::Error::new(
                mode.span(),
                format!("unsupported mode: {name}"),
            ))
        }
    }
}
/// A continuation factory returns an existing Effect description. Its ordinary
/// Rust body runs after the source succeeds, with the native reusable Fn contract.
/// Passing the factory directly preserves owned captures and adds no execution scope.
pub(crate) fn continuation(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let program = fields.take("program")?;
    let body = fields.role("body", 1, BodyCategory::Native)?;
    fields.finish_inferred(
        quote!(A, B, E, R, F),
        &[
            ("program", quote!(#path::Effect<A, E, R>)),
            ("body", quote!(F)),
        ],
        quote!(#path::Effect<B, E, R>),
        quote!(A: Send + 'static, B: Send + 'static, E: Send + 'static,
            R: Send + Sync + 'static, F: Fn(A) -> #path::Effect<B, E, R> + Send + Sync + 'static),
        quote!((#program).flat_map(#body)),
    )
}

/// Retain native lazy guard and body factories without inserting another loop owner.
pub(crate) fn while_loop(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let guard = fields.role("guard", 0, BodyCategory::Native)?;
    let body = fields.role("body", 0, BodyCategory::Native)?;
    fields.finish_inferred(
        quote!(E, R, F, G),
        &[("guard", quote!(F)), ("body", quote!(G))],
        quote!(#path::Effect<(), E, R>),
        quote!(E: Send + 'static, R: Send + Sync + 'static,
            F: Fn() -> bool + Send + Sync + 'static,
            G: Fn() -> #path::Effect<(), E, R> + Send + Sync + 'static),
        quote!(#path::Effect::while_loop(#guard, #body)),
    )
}

/// Lower a deferred reusable body description or reuse an explicitly typed program.
pub(crate) fn program(fields: &mut Fields, name: &str) -> syn::Result<TokenStream> {
    fields.transform(name, |expr, inline| {
        if !inline {
            return Ok(quote!(#expr));
        }
        let Expr::Block(block) = expr else {
            return Err(syn::Error::new_spanned(expr, "Effect body must be braced"));
        };
        crate::expand::expand(block.block.stmts)
    })
}
/// Select exactly one program expression or zero-input Effect body.
pub(crate) fn body(fields: &mut Fields) -> syn::Result<TokenStream> {
    match (fields.contains("program"), fields.contains("body")) {
        (true, false) => fields.take("program"),
        (false, true) => program(fields, "body"),
        _ => Err(syn::Error::new(
            Span::call_site(),
            "specify exactly one of program or body",
        )),
    }
}
/// Native leaf lowering: `body` returns a value, Result, or one native Future.
/// Reusable factories are the default; once forms move owned captures without cloning.
pub(crate) fn leaf(name: &str, input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "once",
            "with_env",
            "with_context",
            "once_with_context",
            "exit_with_context",
        ],
        "reusable",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let fallible = name.starts_with("effect_try_");
    let kind = name
        .trim_start_matches("effect_")
        .trim_start_matches("try_");
    let base = if kind == "future" {
        "from_future"
    } else {
        kind
    };
    let base = if fallible {
        format!("try_{kind}")
    } else {
        base.into()
    };
    let method = match mode.as_str() {
        "reusable" => base,
        "once" => format!("{base}_once"),
        "exit_with_context" if kind == "future" && !fallible => {
            "from_exit_future_with_context".into()
        }
        "with_env" if kind == "future" => format!("{base}_with_env"),
        "with_context" if kind == "cpu" || kind == "blocking" => format!("{base}_with_context"),
        "once_with_context" if !fallible && (kind == "cpu" || kind == "blocking") => {
            format!("{base}_once_with_context")
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "mode is not supported by this native constructor",
            ));
        }
    };
    let arity = if kind == "sync" {
        0
    } else if mode == "with_env" || mode.ends_with("with_context") {
        2
    } else {
        1
    };
    let callback = fields.role("body", arity, BodyCategory::Native)?;
    let method = Ident::new(&method, Span::mixed_site());
    let explicit_mapper = fields.contains("on_error");
    let output = if fallible {
        let mapper = if fields.contains("on_error") {
            fields.role("on_error", 1, BodyCategory::Native)?
        } else {
            quote!(::core::convert::identity)
        };
        quote!(#path::Effect::#method(#callback, #mapper))
    } else {
        quote!(#path::Effect::#method(#callback))
    };
    let once = mode.starts_with("once");
    let callable = if once { quote!(FnOnce) } else { quote!(Fn) };
    let shared = if once { quote!() } else { quote!(+ Sync) };
    let arguments = if kind == "sync" {
        quote!()
    } else if mode == "exit_with_context" {
        quote!(::std::sync::Arc<R>, #path::NativeContext)
    } else if mode == "with_env" {
        quote!(::std::sync::Arc<R>, #path::Cancellation)
    } else if mode.ends_with("with_context") {
        quote!(#path::Cancellation, #path::TraceContext)
    } else {
        quote!(#path::Cancellation)
    };
    let raw_error = if explicit_mapper {
        quote!(X)
    } else {
        quote!(E)
    };
    let value = if mode == "exit_with_context" {
        quote!(#path::Exit<A, E>)
    } else if fallible {
        quote!(::core::result::Result<A, #raw_error>)
    } else {
        quote!(A)
    };
    let mut generics = vec![quote!(A), quote!(E), quote!(R), quote!(F)];
    let mut bounds = vec![
        quote!(A: Send + 'static),
        quote!(E: Send + 'static),
        quote!(R: Send + Sync + 'static),
    ];
    let mut types = vec![("body", quote!(F))];
    let result = if kind == "future" {
        generics.push(quote!(Pending));
        bounds.push(quote!(Pending: ::core::future::Future<Output = #value> + Send + 'static));
        if fallible {
            bounds.push(quote!(#raw_error: Send + 'static));
        }
        quote!(Pending)
    } else {
        value
    };
    bounds.push(quote!(F: #callable(#arguments) -> #result + Send #shared + 'static));
    if fallible && explicit_mapper {
        generics.extend([quote!(X), quote!(M)]);
        types.push(("on_error", quote!(M)));
        bounds.push(quote!(M: #callable(X) -> E + Send #shared + 'static));
    }
    match kind {
        "cpu" => bounds.push(quote!(R: #path::HasCpuExecutor)),
        "blocking" => bounds.push(quote!(R: #path::HasBlockingExecutor)),
        _ => {}
    }
    fields.finish_inferred(
        quote!(#(#generics),*),
        &types,
        quote!(#path::Effect<A, E, R>),
        quote!(#(#bounds),*),
        output,
    )
}
/// Construct the existing defect channel from one owned, once-evaluated expression.
pub(crate) fn defect(input: Input) -> syn::Result<TokenStream> {
    input.mode(&[], "default")?;
    let mut fields = input.fields;
    let value = fields.take("defect")?;
    let path = crate::expand::effect_crate_path()?;
    fields.finish(quote!(#path::Effect::die(#value)))
}
/// Assemble inert collection descriptions with local item/items_from markers.
/// Ordinary control flow is retained; `?` belongs to Rust construction, never Effect execution.
fn collection(fields: &mut Fields) -> syn::Result<TokenStream> {
    fields.transform("items", |expr, inline| {
        if !inline {
            return Ok(quote!(#expr));
        }
        let Expr::Block(block) = expr else {
            return Err(syn::Error::new_spanned(
                expr,
                "items assembly must be braced",
            ));
        };
        use syn::fold::Fold;
        /// Replace only owner-local collection markers, preserving nested Rust closures/items.
        struct Assemble {
            /// Hygienic local receiving descriptions in the written collection order.
            values: Ident,
            /// First malformed owner-local marker, returned after folding the block.
            error: Option<syn::Error>,
        }
        impl Fold for Assemble {
            /// Local markers push/extend descriptions in written order.
            fn fold_stmt(&mut self, stmt: syn::Stmt) -> syn::Stmt {
                if let syn::Stmt::Macro(marker) = &stmt {
                    let method = if marker.mac.path.is_ident("item") {
                        Some(quote!(push))
                    } else if marker.mac.path.is_ident("items_from") {
                        Some(quote!(extend))
                    } else {
                        None
                    };
                    if let Some(method) = method {
                        match syn::parse2::<Expr>(marker.mac.tokens.clone()) {
                            Ok(value) => {
                                let values = &self.values;
                                return parse_quote!(#values.#method(#value););
                            }
                            Err(error) => self.error = Some(error),
                        }
                    } else {
                        self.error =
                            Some(syn::Error::new_spanned(marker, "unknown collection marker"));
                    }
                }
                syn::fold::fold_stmt(self, stmt)
            }
            /// Closures keep their independent Rust grammar and markers.
            fn fold_expr_closure(&mut self, expr: syn::ExprClosure) -> syn::ExprClosure {
                expr
            }
            /// Native async blocks retain their Result/Future boundary.
            fn fold_expr_async(&mut self, expr: syn::ExprAsync) -> syn::ExprAsync {
                expr
            }
            /// Nested items are ordinary Rust.
            fn fold_item(&mut self, item: syn::Item) -> syn::Item {
                item
            }
        }
        let values = Ident::new("__effect_items", Span::mixed_site());
        let mut lower = Assemble {
            values: values.clone(),
            error: None,
        };
        let block = lower.fold_block(block.block);
        if let Some(error) = lower.error {
            return Err(error);
        }
        Ok(quote!({let mut #values = ::std::vec::Vec::new(); #block #values}))
    })
}
/// Lower transforms and policies through existing typed operators, retaining cleanup and cancellation.
pub(crate) fn operator(name: &str, input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(&["once", "shared"], "default")?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let produces = fields.contains("produce");
    let output = match name {
        "effect_map"
        | "effect_map_error"
        | "effect_provide_some"
        | "effect_sandbox"
        | "effect_recover" => {
            if mode != "default" {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "mode does not apply to this operator",
                ));
            }
            let source = fields.take("program")?;
            let (role, method, category) = match name {
                "effect_map" => ("map", quote!(map), BodyCategory::Native),
                "effect_map_error" => ("map", quote!(map_error), BodyCategory::Native),
                "effect_provide_some" => ("project", quote!(provide_some), BodyCategory::Native),
                "effect_sandbox" => ("defect", quote!(sandbox), BodyCategory::Native),
                _ => ("recover", quote!(catch_all), BodyCategory::Effect),
            };
            let callback = fields.role(role, 1, category)?;
            let (returns, callback_bound, extra_bounds) = match name {
                "effect_map" => (
                    quote!(#path::Effect<B, E, R>),
                    quote!(F: Fn(A) -> B),
                    quote!(B: Send + 'static),
                ),
                "effect_map_error" => (
                    quote!(#path::Effect<A, B, R>),
                    quote!(F: Fn(E) -> B),
                    quote!(B: Send + 'static),
                ),
                "effect_provide_some" => (
                    quote!(#path::Effect<A, E, B>),
                    quote!(F: Fn(&B) -> R),
                    quote!(B: Send + Sync + 'static),
                ),
                "effect_sandbox" => (
                    quote!(#path::Effect<A, E, R>),
                    quote!(F: Fn(#path::Defect) -> E),
                    quote!(),
                ),
                _ => (
                    quote!(#path::Effect<A, E, R>),
                    quote!(F: Fn(E) -> #path::Effect<A, E, R>),
                    quote!(),
                ),
            };
            let generics = if matches!(name, "effect_sandbox" | "effect_recover") {
                quote!(A, E, R, F)
            } else {
                quote!(A, E, R, B, F)
            };
            return fields.finish_inferred(
                generics,
                &[
                    ("program", quote!(#path::Effect<A, E, R>)),
                    (role, quote!(F)),
                ],
                returns,
                quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                    #callback_bound + Send + Sync + 'static, #extra_bounds),
                quote!((#source).#method(#callback)),
            );
        }
        "effect_from_option" => {
            let missing = fields.role("missing", 0, BodyCategory::Native)?;
            if !fields.inline("value") {
                let value = fields.take("value")?;
                let method = if mode == "once" {
                    quote!(from_option_once)
                } else {
                    quote!(from_option)
                };
                quote!(#path::Effect::#method(#value, #missing))
            } else {
                if mode == "once" {
                    return Err(syn::Error::new(
                        Span::call_site(),
                        "once option requires value",
                    ));
                }
                let value = fields.role("value", 0, BodyCategory::Native)?;
                quote!(#path::Effect::try_sync(move || (#value)().ok_or_else(&#missing), ::core::convert::identity))
            }
        }
        "effect_all" | "effect_all_concurrent" => {
            let items = collection(&mut fields)?;
            if name == "effect_all" {
                quote!(#path::Effect::all(#items))
            } else {
                let concurrency = fields.take("concurrency")?;
                quote!(#path::Effect::all_concurrent(#concurrency, #items))
            }
        }
        "effect_for_each" | "effect_for_each_concurrent" => {
            let callback = fields.role(
                "body",
                1,
                if name == "effect_for_each_concurrent" {
                    BodyCategory::EffectFactory
                } else {
                    BodyCategory::Effect
                },
            )?;
            if name == "effect_for_each" {
                if fields.contains("produce") {
                    let values = fields.role("produce", 0, BodyCategory::Native)?;
                    quote!(#path::Effect::for_each_sequential(#values, #callback))
                } else {
                    let values = fields.take("values")?;
                    quote!(#path::Effect::for_each(#values, #callback))
                }
            } else {
                let values = fields.take("values")?;
                let concurrency = fields.take("concurrency")?;
                let method = if mode == "shared" {
                    quote!(for_each_concurrent_shared)
                } else {
                    quote!(for_each_concurrent)
                };
                quote!(#path::Effect::#method(#concurrency, #values, #callback))
            }
        }
        _ => {
            let source = body(&mut fields)?;
            match name {
                "effect_timeout" => {
                    let duration = fields.take("duration")?;
                    let timeout = fields.role("on_timeout", 0, BodyCategory::Native)?;
                    quote!((#source).timeout(#duration, #timeout))
                }
                "effect_provided" => {
                    let env = fields.take("environment")?;
                    quote!((#source).provide(#env))
                }
                "effect_race" => {
                    let right = fields.take("right")?;
                    quote!((#source).race(#right))
                }
                "effect_uninterruptible" => quote!((#source).uninterruptible()),
                "effect_on_interrupt" => {
                    let finalizer = program(&mut fields, "finalizer")?;
                    quote!((#source).on_interrupt(#finalizer))
                }
                "effect_retry" | "effect_repeat" => {
                    let schedule = fields.take("schedule")?;
                    let method = Ident::new(name.trim_start_matches("effect_"), Span::mixed_site());
                    quote!((#source).#method(#schedule))
                }
                _ => {
                    return Err(syn::Error::new(
                        Span::call_site(),
                        "unknown Effect operator",
                    ));
                }
            }
        }
    };
    let allowed = match name {
        "effect_from_option" => mode == "default" || mode == "once",
        "effect_for_each_concurrent" => mode == "default" || mode == "shared",
        _ => mode == "default",
    };
    if !allowed {
        return Err(syn::Error::new(
            Span::call_site(),
            "mode does not apply to this operator",
        ));
    }
    if name == "effect_for_each_concurrent" {
        let values = if mode == "shared" {
            quote!(::std::sync::Arc<[T]>)
        } else {
            quote!(::std::vec::Vec<T>)
        };
        return fields.finish_inferred(
            quote!(A, E, R, T, F),
            &[("concurrency", quote!(::core::num::NonZeroUsize)), ("values", values), ("body", quote!(F))],
            quote!(#path::Effect<::std::vec::Vec<A>, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: #path::HasTaskSpawner + Send + Sync + 'static,
                T: Send + Sync + 'static,
                F: Fn(&T) -> #path::Effect<A, E, R> + Send + Sync + 'static), output,
        );
    }
    if name == "effect_for_each" {
        if produces {
            return fields.finish_inferred(
                quote!(E, R, Items, F, G),
                &[("produce", quote!(F)), ("body", quote!(G))],
                quote!(#path::Effect<(), E, R>),
                quote!(E: Send + 'static, R: Send + Sync + 'static,
                    Items: IntoIterator + 'static, Items::Item: Send + 'static,
                    Items::IntoIter: Send + 'static, F: Fn() -> Items + Send + Sync + 'static,
                    G: Fn(Items::Item) -> #path::Effect<(), E, R> + Send + Sync + 'static),
                output,
            );
        }
        return fields.finish_inferred(
            quote!(A, E, R, T, F),
            &[("values", quote!(::std::vec::Vec<T>)), ("body", quote!(F))],
            quote!(#path::Effect<::std::vec::Vec<A>, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                T: Clone + Send + Sync + 'static,
                F: Fn(T) -> #path::Effect<A, E, R> + Send + Sync + 'static),
            output,
        );
    }
    fields.finish(output)
}
