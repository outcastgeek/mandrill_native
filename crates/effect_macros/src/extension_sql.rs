//! Typed inert SQL descriptions and row/SQL Effect projection over shared lowering.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::fold::Fold;
use syn::{
    Expr, Ident, Stmt, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};
use syn::{parenthesized, parse_quote};

struct PreparedRowsInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for PreparedRowsInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_rows" {
            return Err(syn::Error::new(mode.span(), "expected prepared_rows"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn prepared_rows(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedRowsInput { mut fields } = syn::parse2(input)?;
    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
    let region = fields.take("region")?;
    let capacity = fields.take("capacity")?;
    fields.finish(quote!(#path::PreparedSqlRowsPool::prepare(#memory, #region, #capacity)))
}

struct AtomicQueryInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for AtomicQueryInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "atomic_query" {
            return Err(syn::Error::new(mode.span(), "expected atomic_query"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn atomic_query(input: TokenStream) -> syn::Result<TokenStream> {
    let AtomicQueryInput { fields } = syn::parse2(input)?;
    atomic_rows(
        fields,
        "statement",
        quote!(query_into_rows_atomic),
        quote!(query_bytes_into_rows_atomic),
    )
}

fn atomic_rows(
    mut fields: crate::domain_expand::Fields,
    source_field: &str,
    typed_method: TokenStream,
    bytes_method: TokenStream,
) -> syn::Result<TokenStream> {
    let client = fields.transform("client", |value, _| Ok(quote!(&(#value))))?;
    let source = fields.take(source_field)?;
    let output = if !fields.contains("decoder")
        && ["column", "kind", "buffers"].iter().any(|name| fields.contains(name))
    {
        let column = fields.take("column")?;
        let kind = fields.take("kind")?;
        let rows = fields.take("rows")?;
        let buffers = fields.take("buffers")?;
        quote!(#client.#bytes_method(#source, #column, #kind, #rows, #buffers))
    } else {
        let decoder = fields.take("decoder")?;
        let rows = fields.take("rows")?;
        quote!(#client.#typed_method(#source, #decoder, #rows))
    };
    fields.finish(output)
}

struct PreparedStatementInput {
    description: Description,
}
impl Parse for PreparedStatementInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_statement" {
            return Err(syn::Error::new(mode.span(), "expected prepared_statement"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            description: input.parse()?,
        })
    }
}
fn prepared_statement(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedStatementInput {
        description: Description(entries),
    } = syn::parse2(input)?;
    let builder = Ident::new("__prepared_sql_builder", Span::mixed_site());
    let mut body = Vec::new();
    let mut pool = false;
    for entry in entries {
        match entry {
            Entry::Local(local) => body.push(quote!(#local)),
            Entry::Field(name, value) if name == "pool" && !pool => {
                pool = true;
                body.push(quote!(let #builder = (#value).reserve().map_err(#path::PreparedSqlStatementError::Memory)?;));
            }
            Entry::Field(name, _) => {
                return Err(syn::Error::new(
                    name.span(),
                    "expected one pool before SQL operations",
                ));
            }
            Entry::Call(name, values) => {
                if !pool {
                    return Err(syn::Error::new(
                        name.span(),
                        "pool must precede SQL operations",
                    ));
                }
                let method = name.to_string();
                if method == "null" {
                    arity(&name, &values, 0)?;
                    body.push(quote!(let #builder = #builder.null();));
                    continue;
                }
                arity(&name, &values, 1)?;
                let [value] = values.as_slice() else {
                    return Err(syn::Error::new(name.span(), "expected one argument"));
                };
                let operation = match method.as_str() {
                    "text" => quote!(text(&(#value))),
                    "identifier" => quote!(identifier(&(#value))),
                    "text_param" => quote!(text_param(&(#value))),
                    "blob" => quote!(blob(&(#value))),
                    "integer" => quote!(integer(#value)),
                    "real" => quote!(real(#value)),
                    _ => {
                        return Err(syn::Error::new(
                            name.span(),
                            "expected text, identifier, null, integer, real, text_param or blob",
                        ));
                    }
                };
                body.push(quote!(let #builder = #builder.#operation;));
            }
        }
    }
    if !pool {
        return Err(syn::Error::new(Span::call_site(), "missing pool"));
    }
    Ok(
        quote!((|| -> ::core::result::Result<_, #path::PreparedSqlStatementError> { #(#body)* #builder.finish() })()),
    )
}
struct PreparedStatementPoolInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for PreparedStatementPoolInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_statement_pool" {
            return Err(syn::Error::new(
                mode.span(),
                "expected prepared_statement_pool",
            ));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn prepared_statement_pool(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedStatementPoolInput { mut fields } = syn::parse2(input)?;
    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
    let region = fields.take("region")?;
    let capacity = fields.take("capacity")?;
    let pool = quote!(#path::PreparedSqlStatementPool::prepare(#memory, #region, #capacity));
    let output = if fields.contains("materialization_admission") {
        let admission = fields.take("materialization_admission")?;
        quote!(#pool.and_then(|pool| pool.with_materialization_admission(#admission)))
    } else {
        pool
    };
    fields.finish(output)
}

struct ClientInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for ClientInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "client" {
            return Err(syn::Error::new(mode.span(), "expected client"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn client(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let ClientInput { mut fields } = syn::parse2(input)?;
    let pool = fields.take("pool")?;
    let mut client = quote!(#path::SqlClient::new(#pool));
    if fields.contains("row_lanes") {
        let lanes = fields.take("row_lanes")?;
        client = quote!(#client.with_row_lanes(#lanes));
    }
    let output = if fields.contains("pending_checkout_admission") {
        let admission = fields.take("pending_checkout_admission")?;
        quote!(#client.with_pending_checkout_admission(#admission))
    } else {
        quote!(::core::result::Result::<_, #path::__private::effect::memory::MemoryError>::Ok(#client))
    };
    fields.finish(output)
}

struct PreparedNativeRowsInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for PreparedNativeRowsInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_native_rows" {
            return Err(syn::Error::new(
                mode.span(),
                "expected prepared_native_rows",
            ));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn prepared_native_rows(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedNativeRowsInput { mut fields } = syn::parse2(input)?;
    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
    let region = fields.take("region")?;
    let capacity = fields.take("capacity")?;
    let output = if fields.contains("lanes") {
        let lanes = fields.take("lanes")?;
        quote!(#path::PreparedSqlRowLanes::prepare(#memory, #region, #lanes, #capacity))
    } else {
        quote!(#path::prepare_native_rows(#memory, #region, #capacity))
    };
    fields.finish(output)
}

struct PreparedSessionPoolInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for PreparedSessionPoolInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_session_pool" {
            return Err(syn::Error::new(
                mode.span(),
                "expected prepared_session_pool",
            ));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn prepared_session_pool(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedSessionPoolInput { mut fields } = syn::parse2(input)?;
    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
    let region = fields.take("region")?;
    let capacity = fields.take("capacity")?;
    fields.finish(quote!(#path::PreparedSqlSessionPool::prepare(#memory, #region, #capacity)))
}

struct BatchPoolInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for BatchPoolInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_batch_pool" {
            return Err(syn::Error::new(mode.span(), "expected prepared_batch_pool"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn prepared_batch_pool(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let BatchPoolInput { mut fields } = syn::parse2(input)?;
    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
    let region = fields.take("region")?;
    let capacity = fields.take("capacity")?;
    fields.finish(quote!(#path::PreparedSqlBatchPool::prepare(#memory, #region, #capacity)))
}
struct PreparedBatchInput {
    description: Description,
}
impl Parse for PreparedBatchInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "prepared_batch" {
            return Err(syn::Error::new(mode.span(), "expected prepared_batch"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            description: input.parse()?,
        })
    }
}
fn prepared_batch(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let PreparedBatchInput {
        description: Description(entries),
    } = syn::parse2(input)?;
    let builder = Ident::new("__prepared_batch_builder", Span::mixed_site());
    let mut body = Vec::new();
    let mut pool = false;
    for entry in entries {
        match entry {
            Entry::Local(local) => body.push(quote!(#local)),
            Entry::Field(name, value) if name == "pool" && !pool => {
                pool = true;
                body.push(quote!(let mut #builder = (#value).reserve().map_err(#path::PreparedSqlStatementError::Memory)?;));
            }
            Entry::Field(name, _) => {
                return Err(syn::Error::new(
                    name.span(),
                    "expected one pool before batch statements",
                ));
            }
            Entry::Call(name, values) => {
                if !pool {
                    return Err(syn::Error::new(
                        name.span(),
                        "pool must precede batch statements",
                    ));
                }
                if name != "statement" {
                    return Err(syn::Error::new(name.span(), "expected statement(...)"));
                }
                arity(&name, &values, 1)?;
                let [value] = values.as_slice() else {
                    return Err(syn::Error::new(name.span(), "expected one statement"));
                };
                body.push(quote!(
                    if #builder.len() == #builder.capacity() {
                        return Err(#path::PreparedSqlStatementError::Memory(#path::__private::effect::memory::MemoryError::NoSlot));
                    }
                    #builder.try_push(#value).map_err(|_| #path::PreparedSqlStatementError::Memory(#path::__private::effect::memory::MemoryError::NoSlot))?;
                ));
            }
        }
    }
    if !pool {
        return Err(syn::Error::new(Span::call_site(), "missing pool"));
    }
    Ok(
        quote!((|| -> ::core::result::Result<_, #path::PreparedSqlStatementError> { #(#body)* Ok(#builder.finish()) })()),
    )
}
struct AtomicBatchInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for AtomicBatchInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "atomic_batch" {
            return Err(syn::Error::new(mode.span(), "expected atomic_batch"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn atomic_batch(input: TokenStream) -> syn::Result<TokenStream> {
    let AtomicBatchInput { fields } = syn::parse2(input)?;
    atomic_rows(
        fields,
        "batch",
        quote!(batch_into_rows_atomic),
        quote!(batch_bytes_into_rows_atomic),
    )
}

struct ReserveSessionInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for ReserveSessionInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "reserve_session" {
            return Err(syn::Error::new(mode.span(), "expected reserve_session"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn reserve_session(input: TokenStream) -> syn::Result<TokenStream> {
    let ReserveSessionInput { mut fields } = syn::parse2(input)?;
    let client = fields.transform("client", |value, _| Ok(quote!(&(#value))))?;
    fields.finish(quote!(#client.kernel().reserve_prepared_session()))
}

struct ReserveRowsInput {
    fields: crate::domain_expand::Fields,
}
impl Parse for ReserveRowsInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mode: Ident = input.parse()?;
        if mode != "reserve_rows" {
            return Err(syn::Error::new(mode.span(), "expected reserve_rows"));
        }
        input.parse::<Token![;]>()?;
        Ok(Self {
            fields: input.parse()?,
        })
    }
}
fn reserve_rows(input: TokenStream) -> syn::Result<TokenStream> {
    let ReserveRowsInput { mut fields } = syn::parse2(input)?;
    let client = fields.transform("client", |value, _| Ok(quote!(&(#value))))?;
    fields.finish(quote!(#client.kernel().reserve_prepared_rows()))
}

/// One ordered descriptor operation, retaining caller spans and evaluation order.
enum Entry {
    /// Pure local bindings stay in the inert description's ordinary Rust block.
    Local(Box<Stmt>),
    /// Named configuration is evaluated once at its declared position.
    Field(Ident, Box<Expr>),
    /// Ordered call-shaped operation with native typed expressions.
    Call(Ident, Vec<Expr>),
}
/// Parsed statement/mutation configuration; effectful construction is never inferred.
struct Description(Vec<Entry>);
impl Parse for Description {
    /// Parse only known descriptor shapes; semantic diagnostics retain original field spans.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            if input.peek(Token![let]) {
                entries.push(Entry::Local(Box::new(input.parse()?)));
                continue;
            }
            let name = Ident::parse_any(input)?;
            if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                let value = input.parse()?;
                entries.push(Entry::Field(name, Box::new(value)));
                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }
            } else {
                let content;
                parenthesized!(content in input);
                let values = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .collect();
                entries.push(Entry::Call(name, values));
                input.parse::<Token![;]>()?;
            }
        }
        Ok(Self(entries))
    }
}
/// Enforce a fixed native operation arity at the call's source span.
fn arity(name: &Ident, values: &[Expr], expected: usize) -> syn::Result<()> {
    if values.len() != expected {
        return Err(syn::Error::new(
            name.span(),
            format!("`{name}` requires {expected} argument(s)"),
        ));
    }
    Ok(())
}
/// Validate static identifier structure where known, leaving dynamic identifiers to typed runtime checks.
fn literal_identifier(value: &Expr) -> syn::Result<()> {
    if let Expr::Lit(literal) = value
        && let syn::Lit::Str(value) = &literal.lit
    {
        let text = value.value();
        if text.contains('\0') || text.split('.').any(|part| part.trim().is_empty()) {
            return Err(syn::Error::new(
                value.span(),
                "identifier parts must be nonempty and contain no NUL",
            ));
        }
    }
    Ok(())
}
/// Lower D02 with a first-error builder, moving native bindings without cloning application payloads.
fn statement(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let Description(entries) = syn::parse2(input)?;
    let mut body = Vec::new();
    let mut predicate = false;
    let builder = Ident::new("__sql_builder", Span::mixed_site());
    for entry in entries {
        match entry {
            Entry::Local(local) => body.push(quote!(#local)),
            Entry::Field(name, value) => {
                if name != "text" {
                    return Err(syn::Error::new(
                        name.span(),
                        "SQL named configuration supports `text:`; use ordered calls for identifier/fragment/where/param",
                    ));
                }
                body.push(quote!(let #builder = #builder.text(#value);));
            }
            Entry::Call(name, values) => {
                arity(&name, &values, 1)?;
                let [value] = values.as_slice() else {
                    return Err(syn::Error::new(name.span(), "expected one argument"));
                };
                let operation = match name.to_string().as_str() {
                    "text" => Ident::new("text", name.span().resolved_at(Span::mixed_site())),
                    "identifier" => {
                        literal_identifier(value)?;
                        Ident::new("identifier", name.span().resolved_at(Span::mixed_site()))
                    }
                    "fragment" => {
                        Ident::new("fragment", name.span().resolved_at(Span::mixed_site()))
                    }
                    "param" => Ident::new("param", name.span().resolved_at(Span::mixed_site())),
                    "where" => {
                        if predicate {
                            return Err(syn::Error::new(
                                name.span(),
                                "SQL statement has more than one WHERE predicate",
                            ));
                        }
                        predicate = true;
                        Ident::new("predicate", name.span().resolved_at(Span::mixed_site()))
                    }
                    _ => {
                        return Err(syn::Error::new(
                            name.span(),
                            "unknown SQL operation; expected text/identifier/fragment/where/param",
                        ));
                    }
                };
                body.push(quote!(let #builder = #builder.#operation(#value);));
            }
        }
    }
    Ok(quote!({ let #builder = #path::SqlStatement::builder(); #(#body)* #builder.finish() }))
}
/// Lower D03/D04 with nonempty assignments, literal duplicate diagnostics and exactly one guard.
fn mutation(input: TokenStream, path: &TokenStream, update: bool) -> syn::Result<TokenStream> {
    let Description(entries) = syn::parse2(input)?;
    let mut table = false;
    let mut count = 0;
    let mut predicate = None;
    let mut body = Vec::new();
    let mut columns = std::collections::HashSet::new();
    let builder = Ident::new("__sql_mutation", Span::mixed_site());
    let guard = Ident::new("__sql_predicate", Span::mixed_site());
    for entry in entries {
        match entry {
            Entry::Local(local) => body.push(quote!(#local)),
            Entry::Field(name, value) if name == "table" => {
                if table {
                    return Err(syn::Error::new(name.span(), "duplicate table field"));
                }
                if count > 0 {
                    return Err(syn::Error::new(
                        name.span(),
                        "table must precede assignments",
                    ));
                }
                literal_identifier(&value)?;
                table = true;
                body.push(quote!(let #builder = #path::MutationBuilder::new(#value);));
            }
            Entry::Call(name, values) if name == "value" && !update || name == "set" && update => {
                if !table {
                    return Err(syn::Error::new(
                        name.span(),
                        "table is required before assignments",
                    ));
                }
                if predicate.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "WHERE must follow all assignments",
                    ));
                }
                arity(&name, &values, 2)?;
                let [column, value] = values.as_slice() else {
                    return Err(syn::Error::new(name.span(), "expected column and value"));
                };
                literal_identifier(column)?;
                if let Expr::Lit(literal) = column
                    && let syn::Lit::Str(column) = &literal.lit
                    && !columns.insert(column.value())
                {
                    return Err(syn::Error::new(column.span(), "duplicate mutation column"));
                }
                body.push(quote!(let #builder = #builder.value(#column, #value);));
                count += 1;
            }
            Entry::Call(name, values) if name == "where" && update => {
                arity(&name, &values, 1)?;
                if predicate.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "guarded update requires exactly one WHERE predicate",
                    ));
                }
                if count == 0 {
                    return Err(syn::Error::new(
                        name.span(),
                        "UPDATE assignments must precede WHERE",
                    ));
                }
                let [value] = values.as_slice() else {
                    return Err(syn::Error::new(name.span(), "expected one argument"));
                };
                body.push(quote!(let #guard = #value;));
                predicate = Some(name.span());
            }
            Entry::Field(name, _) | Entry::Call(name, _) => {
                return Err(syn::Error::new(
                    name.span(),
                    if update {
                        "expected table:, set(column,value); or where(typed_predicate);"
                    } else {
                        "expected table: or value(column,value);"
                    },
                ));
            }
        }
    }
    if !table {
        return Err(syn::Error::new(Span::call_site(), "missing table field"));
    }
    if count == 0 {
        return Err(syn::Error::new(
            Span::call_site(),
            "mutation requires at least one assignment",
        ));
    }
    let finish = if update {
        if predicate.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "guarded update requires exactly one WHERE predicate",
            ));
        }
        quote!(#builder.update(#guard))
    } else {
        quote!(#builder.insert())
    };
    Ok(quote!({#(#body)* #finish}))
}

/// Rewrites only D05-local operations while preserving ordinary nested callbacks/items/macros.
struct RowLocals {
    /// Hygienic renamed extension path.
    path: TokenStream,
}
impl Fold for RowLocals {
    /// Give native reads/refinements a typed row binding; arbitrary Results remain ordinary Rust.
    fn fold_expr(&mut self, expr: Expr) -> Expr {
        if matches!(&expr, Expr::Closure(_) | Expr::Async(_)) {
            expr
        } else if let Expr::Call(mut call) = expr {
            call.args = call
                .args
                .into_iter()
                .map(|expr| self.fold_expr(expr))
                .collect();
            if let Expr::Path(function) = call.func.as_ref()
                && function.path.segments.len() == 1
            {
                let Some(segment) = function.path.segments.first() else {
                    return Expr::Call(call);
                };
                let name = segment.ident.to_string();
                if matches!(
                    name.as_str(),
                    "column"
                        | "optional_column"
                        | "native_column"
                        | "refine"
                        | "validate"
                        | "choice_text"
                ) {
                    let path = &self.path;
                    let Some(segment) = function.path.segments.first() else {
                        return Expr::Call(call);
                    };
                    let name = &segment.ident;
                    let arguments = &segment.arguments;
                    call.func = Box::new(parse_quote!(#path::__private::#name #arguments));
                }
            }
            Expr::Call(call)
        } else {
            syn::fold::fold_expr(self, expr)
        }
    }
    /// Nested items retain their own reader/error/lifetime language boundary.
    fn fold_item(&mut self, item: syn::Item) -> syn::Item {
        item
    }
}
/// Lower D05 row bodies through the shared context callback and owned row-program carrier.
fn row_program(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let mut tokens = input.clone().into_iter();
    if matches!((tokens.next(), tokens.next()),
        (Some(proc_macro2::TokenTree::Ident(name)), Some(proc_macro2::TokenTree::Punct(colon)))
        if name == "project_with_env" && colon.as_char() == ':')
    {
        let input = syn::parse2::<crate::callback_expand::Input>(input)?;
        input.mode(&[], "default")?;
        let mut fields = input.fields;
        let project = fields.role(
            "project_with_env",
            2,
            crate::domain_expand::BodyCategory::Native,
        )?;
        return fields.finish_inferred(
            quote!(A, R, F),
            &[("project_with_env", quote!(F))],
            quote!(#path::RowDecoder<A, R>),
            quote!(A: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&#path::sqlx::TursoRow, &R) -> ::core::result::Result<A, #path::RowError> + Send + Sync + 'static),
            quote!(#path::RowDecoder::pure_with_env(#project)),
        );
    }
    let mut tokens = input.clone().into_iter();
    if matches!((tokens.next(), tokens.next()),
        (Some(proc_macro2::TokenTree::Ident(name)), Some(proc_macro2::TokenTree::Punct(colon)))
        if name == "project" && colon.as_char() == ':')
    {
        let input = syn::parse2::<crate::callback_expand::Input>(input)?;
        input.mode(&[], "default")?;
        let mut fields = input.fields;
        let project = fields.role("project", 1, crate::domain_expand::BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(A, R, F),
            &[("project", quote!(F))],
            quote!(#path::RowDecoder<A, R>),
            quote!(A: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&#path::sqlx::TursoRow) -> ::core::result::Result<A, #path::RowError> + Send + Sync + 'static),
            quote!(#path::RowDecoder::pure(#project)),
        );
    }
    let statements = syn::parse::Parser::parse2(syn::Block::parse_within, input)?;
    let mut locals = RowLocals { path: path.clone() };
    let statements = statements
        .into_iter()
        .map(|statement| locals.fold_stmt(statement))
        .collect();
    Ok(crate::context_expand::expand(
        statements,
        quote!(#path::RowDecoder::from_program),
    ))
}

/// Projection grammar keeps selection, handle and body evaluation distinct and inert.
struct WithInput {
    /// Ordered fields retain configuration evaluation order.
    fields: Vec<(Ident, Expr)>,
    /// Optional inline run argument and body.
    inline: Option<(syn::Pat, Vec<Stmt>)>,
}
impl Parse for WithInput {
    /// Parse named select/handle/project/on_error/errors/effect/run and inline run |client| { body }.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut fields = Vec::new();
        let mut inline = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            if name == "run" && input.peek(Token![|]) {
                if inline.is_some() {
                    return Err(syn::Error::new(name.span(), "duplicate inline run body"));
                }
                input.parse::<Token![|]>()?;
                let arg = syn::Pat::parse_single(input)?;
                input.parse::<Token![|]>()?;
                let block: syn::Block = input.parse()?;
                inline = Some((arg, block.stmts));
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            } else {
                input.parse::<Token![:]>()?;
                fields.push((name, input.parse()?));
                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }
            }
        }
        Ok(Self { fields, inline })
    }
}
/// Lower D06 explicit selection/projection with mutually exclusive inline/extracted bodies.
fn with_sql(input: TokenStream, path: &TokenStream) -> syn::Result<TokenStream> {
    let WithInput { fields, inline } = syn::parse2(input)?;
    let mut selected = None;
    let mut effect = None;
    let mut helper = None;
    let mut errors = None;
    let mut body = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, (name, value)) in fields.into_iter().enumerate() {
        if !seen.insert(name.to_string()) {
            return Err(syn::Error::new(name.span(), "duplicate sql_with field"));
        }
        let variable = Ident::new(&format!("__sql_field_{index}"), Span::mixed_site());
        body.push(quote!(let #variable = #value;));
        match name.to_string().as_str() {
            "select" | "project" => {
                if selected.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "select/project/handle forms are mutually exclusive",
                    ));
                }
                selected = Some(quote!(#variable));
            }
            "handle" => {
                if selected.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "select/project/handle forms are mutually exclusive",
                    ));
                }
                selected = Some(quote!(move |_| #variable.clone()));
            }
            "effect" => effect = Some(quote!(#variable)),
            "run" => helper = Some(quote!(#variable)),
            "errors" | "on_error" => {
                if errors.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "provide only one errors/on_error mapper",
                    ));
                }
                errors = Some(quote!(#variable));
            }
            _ => {
                return Err(syn::Error::new(
                    name.span(),
                    "unknown sql_with field; expected select/handle/project/on_error/errors/effect/run",
                ));
            }
        }
    }
    if usize::from(effect.is_some()) + usize::from(helper.is_some()) + usize::from(inline.is_some())
        != 1
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "sql_with requires exactly one inline run, run: helper or effect: program",
        ));
    }
    let select = selected.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "sql_with requires select:, project: or handle:",
        )
    })?;
    let result = if let Some(program) = effect {
        quote!(#path::sql_with(#select, #program))
    } else if let Some(helper) = helper {
        quote!(#path::sql_with(#select, #path::__private::effect::Effect::service(|client: &#path::SqlClient| client.clone()).flat_map(#helper)))
    } else {
        let Some((arg, statements)) = inline else {
            return Err(syn::Error::new(
                Span::call_site(),
                "missing inline SQL body",
            ));
        };
        // The body has the reference's explicit SqlClient environment, while
        // provide_some retains the caller's outer R and execution context.
        // Its captures move once through the existing core callback lowering.
        let selection: Stmt = parse_quote!(let #arg = #path::__private::effect::Effect::service(|client: &#path::SqlClient| client.clone())?;);
        let statements = std::iter::once(selection).chain(statements).collect();
        let program = crate::async_expand::expand_once(
            statements,
            &quote!(#path::__private::effect::__private),
        );
        quote!(#path::sql_with(#select, #program))
    };
    let result = match errors {
        Some(errors) => quote!((#result).map_error(#errors)),
        None => result,
    };
    Ok(quote!({ #(#body)* #result }))
}

/// Dispatch only the five approved SQL entry points through the existing shared proc-macro registry.
pub(crate) fn expand(name: &str, input: TokenStream) -> syn::Result<TokenStream> {
    let path = crate::extension_support::crate_path("effect-sql")?;
    match name {
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_rows") => {
            prepared_rows(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "atomic_query") => {
            atomic_query(input)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_statement_pool") => {
            prepared_statement_pool(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "client") => {
            client(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_session_pool") => {
            prepared_session_pool(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_native_rows") => {
            prepared_native_rows(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_batch_pool") => {
            prepared_batch_pool(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_batch") => {
            prepared_batch(input, &path)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "atomic_batch") => {
            atomic_batch(input)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "reserve_rows") => {
            reserve_rows(input)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "reserve_session") => {
            reserve_session(input)
        }
        "sql" if matches!(input.clone().into_iter().next(), Some(proc_macro2::TokenTree::Ident(mode)) if mode == "prepared_statement") => {
            prepared_statement(input, &path)
        }
        "sql" => statement(input, &path),
        "sql_insert" => mutation(input, &path, false),
        "sql_update" => mutation(input, &path, true),
        "row" => row_program(input, &path),
        "sql_with" => with_sql(input, &path),
        _ => Err(syn::Error::new(Span::call_site(), "unknown SQL macro")),
    }
}
