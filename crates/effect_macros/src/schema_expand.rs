//! Expansion of typed record schema declarations.

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use std::collections::BTreeSet;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, Path, Token, braced, parenthesized};

/// Parsed field codec plus mutually exclusive absence/default forms; native expansion consumes values and preserves independent default witnesses.
struct SchemaField {
    name: Ident,
    codec: Expr,
    optional: Option<(Expr, Expr, Option<Expr>)>,
    optional_attribute: bool,
    default_with: Option<Expr>,
}

impl Parse for SchemaField {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attributes = input.call(Attribute::parse_outer)?;
        let mut optional_attribute = false;
        let mut default_with = None;
        for attribute in attributes {
            if !attribute.path().is_ident("schema") {
                return Err(syn::Error::new(
                    attribute.span(),
                    "expected schema field attribute",
                ));
            }
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("optional") {
                    if optional_attribute {
                        return Err(meta.error("duplicate optional attribute"));
                    }
                    optional_attribute = true;
                    Ok(())
                } else if meta.path.is_ident("default_with") {
                    if default_with.is_some() {
                        return Err(meta.error("duplicate default_with attribute"));
                    }
                    default_with = Some(meta.value()?.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("expected optional or default_with"))
                }
            })?;
        }
        if optional_attribute && default_with.is_some() {
            return Err(input.error("optional and default_with are mutually exclusive"));
        }
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let codec = input.parse()?;
        let optional = if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            let content;
            parenthesized!(content in input);
            let missing = content.parse()?;
            content.parse::<Token![,]>()?;
            let omit = content.parse()?;
            let default = if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
                if content.is_empty() {
                    None
                } else {
                    Some(content.parse()?)
                }
            } else {
                None
            };
            if !content.is_empty() {
                return Err(
                    content.error("expected `(missing, omit)` or `(missing, omit, pure_default)`")
                );
            }
            Some((missing, omit, default))
        } else {
            None
        };
        Ok(Self {
            name,
            codec,
            optional,
            optional_attribute,
            default_with,
        })
    }
}

/// A record declaration with mandatory parse policy; native form requires representation and unknown-field policy together.
pub(crate) struct RecordSchemaInput {
    record: Path,
    fields: Vec<SchemaField>,
    policy: Expr,
    representation: Option<Expr>,
    unknown_fields: Option<Expr>,
}

/// Variants of schema input used by schema expand.
pub(crate) enum SchemaInput {
    Record(Box<RecordSchemaInput>),
    Enum(Box<crate::schema_enum_expand::SchemaEnumInput>),
}

impl Parse for SchemaInput {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![enum]) {
            return input.parse().map(|parsed| Self::Enum(Box::new(parsed)));
        }
        input
            .parse::<RecordSchemaInput>()
            .map(|parsed| Self::Record(Box::new(parsed)))
    }
}

impl Parse for RecordSchemaInput {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![struct]>()?;
        let record = input.parse()?;
        let content;
        braced!(content in input);
        let fields = Punctuated::<SchemaField, Token![,]>::parse_terminated(&content)?;
        let mut policy = None;
        let mut representation = None;
        let mut unknown_fields = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            let value: Expr = input.parse()?;
            match name.to_string().as_str() {
                "policy" if policy.is_none() => policy = Some(value),
                "representation" if representation.is_none() => representation = Some(value),
                "unknown_fields" if unknown_fields.is_none() => unknown_fields = Some(value),
                _ => {
                    return Err(syn::Error::new(
                        name.span(),
                        "unknown or duplicate schema option",
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected comma between schema options"));
            }
        }
        let policy = policy.ok_or_else(|| input.error("schema requires an explicit policy"))?;
        if representation.is_some() != unknown_fields.is_some() {
            return Err(input.error("native schemas require representation and unknown_fields"));
        }
        Ok(Self {
            record,
            fields: fields.into_iter().collect(),
            policy,
            representation,
            unknown_fields,
        })
    }
}

/// # Parameters
/// - `input` (`SchemaInput`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn expand(input: SchemaInput) -> syn::Result<TokenStream> {
    match input {
        SchemaInput::Record(record) => expand_record(*record),
        SchemaInput::Enum(variants) => crate::schema_enum_expand::expand(*variants),
    }
}

///
/// # Parameters
/// - `input` (`RecordSchemaInput`): Owned input decoded or consumed by this operation.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
fn expand_record(input: RecordSchemaInput) -> syn::Result<TokenStream> {
    let effect = crate::expand::effect_crate_path()?;
    let record = &input.record;
    let policy = &input.policy;
    let mut names = BTreeSet::new();
    for field in &input.fields {
        if !names.insert(field.name.to_string()) {
            return Err(syn::Error::new(field.name.span(), "duplicate schema field"));
        }
    }
    let codec_vars: Vec<_> = input
        .fields
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("__effect_schema_codec_{index}"))
        .collect();
    let definitions = input.fields.iter().zip(&codec_vars).map(|(field, var)| {
        let codec = &field.codec;
        quote_spanned!(codec.span().resolved_at(proc_macro2::Span::mixed_site())=> let #var = #codec;)
    });
    if let Some(representation) = &input.representation {
        let unknown = input
            .unknown_fields
            .as_ref()
            .ok_or_else(|| syn::Error::new(record.span(), "missing unknown_fields"))?;
        let mut builder = quote!(#effect::NativeRecordBuilder::new());
        for (field, var) in input.fields.iter().zip(&codec_vars).rev() {
            let name = &field.name;
            builder = if let Some(default) = &field.default_with {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=> #builder.default_with_context(stringify!(#name), #var.clone(), #default, { let codec = #var; move |value, context| codec.validate_default_in_ref(value, context) }))
            } else if field.optional_attribute {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=> #builder.optional(stringify!(#name), #var, || None, |value| value.is_none()))
            } else if let Some((missing, omit, None)) = &field.optional {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=> #builder.optional(stringify!(#name), #var, #missing, #omit))
            } else if field.optional.is_some() {
                return Err(syn::Error::new(
                    name.span(),
                    "native defaults use #[schema(default_with = factory)]",
                ));
            } else {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=> #builder.field(stringify!(#name), #var))
            };
        }
        let names: Vec<_> = input.fields.iter().map(|field| &field.name).collect();
        let mut tuple = quote!(());
        for name in names.iter().rev() {
            tuple = quote!((#name,#tuple));
        }
        return Ok(quote!({
            let __effect_schema_policy = #policy;
            #(#definitions)*
            #builder.finish(#representation,__effect_schema_policy,#unknown, |#tuple| #record { #(#names),* }, |value: #record| { let #record { #(#names),* } = value; #tuple })
        }));
    }
    if let Some(field) = input
        .fields
        .iter()
        .find(|field| field.optional_attribute || field.default_with.is_some())
    {
        return Err(syn::Error::new(
            field.name.span(),
            "schema field attributes require an explicit representation",
        ));
    }
    let mut builder = quote!(#effect::RecordBuilder::<#record>::new());
    for (field, var) in input.fields.iter().zip(&codec_vars).rev() {
        let name = &field.name;
        builder = match &field.optional {
            Some((missing, omit, Some(default))) => {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=>
                    #builder.optional_with_default(
                        stringify!(#name), |value: &#record| &value.#name,
                        #var, #missing, #omit, #default,
                    )
                )
            }
            Some((missing, omit, None)) => {
                quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=>
                    #builder.optional(
                        stringify!(#name), |value: &#record| &value.#name,
                        #var, #missing, #omit,
                    )
                )
            }
            None => quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=>
                #builder.field(stringify!(#name), |value: &#record| &value.#name, #var)
            ),
        };
    }
    let field_names: Vec<_> = input.fields.iter().map(|field| &field.name).collect();
    let mut pattern = quote!(());
    for name in field_names.iter().rev() {
        pattern = quote!((#name, #pattern));
    }
    Ok(quote!({
        let __effect_schema_policy = #policy;
        #(#definitions)*
        #builder.finish(__effect_schema_policy, |#pattern| #record { #(#field_names),* })
    }))
}
