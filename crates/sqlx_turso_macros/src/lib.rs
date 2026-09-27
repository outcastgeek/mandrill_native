//! Checked query macros for sqlx-turso

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Group, Span, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use sqlx_macros_core::query::{QueryDriver, QueryMacroInput, expand_input};
use syn::{
    Expr, LitStr, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

struct QueryInput {
    source: Punctuated<LitStr, Token![+]>,
    args: Vec<Expr>,
}

struct QueryAsInput {
    record: Type,
    query: QueryInput,
}

impl Parse for QueryInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let source = Punctuated::<LitStr, Token![+]>::parse_separated_nonempty(input)?;
        let args = parse_args(input)?;

        Ok(Self { source, args })
    }
}

impl Parse for QueryAsInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let record = input.parse()?;
        input.parse::<Token![,]>()?;
        let query = input.parse()?;

        Ok(Self { record, query })
    }
}

fn parse_args(input: ParseStream<'_>) -> syn::Result<Vec<Expr>> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    input.parse::<Token![,]>()?;
    let args = Punctuated::<Expr, Token![,]>::parse_terminated(input)?;

    Ok(args.into_iter().collect())
}

fn query_tokens(input: QueryInput) -> TokenStream2 {
    let QueryInput { source, args } = input;

    quote! {
        source = #source,
        args = [#(#args),*]
    }
}

fn query_as_tokens(input: QueryAsInput) -> TokenStream2 {
    let QueryAsInput { record, query } = input;
    let QueryInput { source, args } = query;

    quote! {
        source = #source,
        args = [#(#args),*],
        record = #record
    }
}

fn query_file_tokens(input: QueryInput) -> TokenStream2 {
    let QueryInput { source, args } = input;

    quote! {
        source_file = #source,
        args = [#(#args),*]
    }
}

fn query_file_as_tokens(input: QueryAsInput) -> TokenStream2 {
    let QueryAsInput { record, query } = input;
    let QueryInput { source, args } = query;

    quote! {
        source_file = #source,
        args = [#(#args),*],
        record = #record
    }
}

fn expand_query(input: TokenStream2) -> TokenStream {
    let facade_paths = match facade_crate_paths() {
        Ok(paths) => paths,
        Err(error) => return error.to_compile_error().into(),
    };

    let input: QueryMacroInput = match syn_sqlx::parse2(input) {
        Ok(input) => input,
        Err(error) => return error.to_compile_error().into(),
    };

    let mut drivers = Vec::with_capacity(sqlx_macros_core::FOSS_DRIVERS.len() + 1);
    drivers.push(QueryDriver::new::<sqlx_turso_core::Turso>());
    drivers.extend_from_slice(sqlx_macros_core::FOSS_DRIVERS);

    match expand_input(input, drivers.iter()) {
        Ok(tokens) => rewrite_sqlx_paths(tokens, &facade_paths).into(),
        Err(error) => {
            let message = error.to_string();
            quote!(compile_error!(#message)).into()
        }
    }
}

struct FacadePaths {
    sqlx: TokenStream2,
    turso: TokenStream2,
}

fn facade_crate_paths() -> syn::Result<FacadePaths> {
    if let Ok(found) = crate_name("sqlx-turso-sqlx") {
        let path = found_crate_path(found, "sqlx");
        return Ok(FacadePaths {
            sqlx: path.clone(),
            turso: path,
        });
    }

    match crate_name("sqlx-turso") {
        Ok(found) => {
            let turso = found_crate_path(found, "sqlx_turso");
            Ok(FacadePaths {
                sqlx: quote!(#turso::sqlx),
                turso,
            })
        }
        Err(_) if std::env::var("CARGO_PKG_NAME").is_ok_and(|name| name == "sqlx_turso") => {
            Ok(FacadePaths {
                sqlx: quote!(::sqlx_turso::sqlx),
                turso: quote!(::sqlx_turso),
            })
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "sqlx-turso macros must be used through the `sqlx-turso` facade crate: {error}"
            ),
        )),
    }
}

fn found_crate_path(found: FoundCrate, itself: &str) -> TokenStream2 {
    match found {
        FoundCrate::Itself => {
            let ident = format_ident!("{itself}");
            quote!(::#ident)
        }
        FoundCrate::Name(name) => {
            let ident = format_ident!("{name}");
            quote!(::#ident)
        }
    }
}

fn rewrite_sqlx_paths(tokens: TokenStream2, facade_paths: &FacadePaths) -> TokenStream2 {
    let mut output = TokenStream2::new();
    let mut iter = tokens.into_iter().peekable();

    while let Some(token) = iter.next() {
        match token {
            TokenTree::Punct(first)
                if first.as_char() == ':' && iter.peek().is_some_and(is_colon_punct) =>
            {
                let Some(second) = iter.next() else {
                    output.extend([TokenTree::Punct(first)]);
                    continue;
                };
                if let Some(replacement) = absolute_path_replacement(iter.peek(), facade_paths) {
                    let _ = iter.next();
                    output.extend(replacement);
                } else {
                    output.extend([TokenTree::Punct(first), second]);
                }
            }
            TokenTree::Group(group) => {
                let mut rewritten = Group::new(
                    group.delimiter(),
                    rewrite_sqlx_paths(group.stream(), facade_paths),
                );
                rewritten.set_span(group.span());
                output.extend([TokenTree::Group(rewritten)]);
            }
            token @ (TokenTree::Ident(_) | TokenTree::Punct(_) | TokenTree::Literal(_)) => {
                output.extend([token]);
            }
        }
    }

    output
}

fn absolute_path_replacement(
    token: Option<&TokenTree>,
    facade_paths: &FacadePaths,
) -> Option<TokenStream2> {
    let TokenTree::Ident(ident) = token? else {
        return None;
    };

    if ident == "sqlx" {
        return Some(facade_paths.sqlx.clone());
    }

    if ident == "sqlx_turso" {
        return Some(facade_paths.turso.clone());
    }

    None
}

fn is_colon_punct(token: &TokenTree) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ':')
}

/// Expands to a checked Turso query
#[proc_macro]
pub fn query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    expand_query(query_tokens(input))
}

/// Expands to a checked Turso query mapped to a type
#[proc_macro]
pub fn query_as(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryAsInput);
    expand_query(query_as_tokens(input))
}

/// Expands to a checked Turso scalar query
#[proc_macro]
pub fn query_scalar(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_tokens(input);
    tokens.extend(quote!(, scalar = _));
    expand_query(tokens)
}

/// Expands to a checked Turso query loaded from a file
#[proc_macro]
pub fn query_file(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    expand_query(query_file_tokens(input))
}

/// Expands to a checked Turso file query mapped to a type
#[proc_macro]
pub fn query_file_as(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryAsInput);
    expand_query(query_file_as_tokens(input))
}

/// Expands to a checked Turso scalar query loaded from a file
#[proc_macro]
pub fn query_file_scalar(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_file_tokens(input);
    tokens.extend(quote!(, scalar = _));
    expand_query(tokens)
}

/// Expands to an unchecked Turso query
#[proc_macro]
pub fn query_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_tokens(input);
    tokens.extend(quote!(, checked = false));
    expand_query(tokens)
}

/// Expands to an unchecked Turso query mapped to a type
#[proc_macro]
pub fn query_as_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryAsInput);
    let mut tokens = query_as_tokens(input);
    tokens.extend(quote!(, checked = false));
    expand_query(tokens)
}

/// Expands to an unchecked Turso scalar query
#[proc_macro]
pub fn query_scalar_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_tokens(input);
    tokens.extend(quote!(, scalar = _, checked = false));
    expand_query(tokens)
}

/// Expands to an unchecked Turso query loaded from a file
#[proc_macro]
pub fn query_file_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_file_tokens(input);
    tokens.extend(quote!(, checked = false));
    expand_query(tokens)
}

/// Expands to an unchecked Turso file query mapped to a type
#[proc_macro]
pub fn query_file_as_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryAsInput);
    let mut tokens = query_file_as_tokens(input);
    tokens.extend(quote!(, checked = false));
    expand_query(tokens)
}

/// Expands to an unchecked Turso scalar query loaded from a file
#[proc_macro]
pub fn query_file_scalar_unchecked(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let mut tokens = query_file_tokens(input);
    tokens.extend(quote!(, scalar = _, checked = false));
    expand_query(tokens)
}
