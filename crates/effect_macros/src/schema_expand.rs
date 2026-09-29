use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use std::collections::BTreeSet;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Path, Token, braced, parenthesized};

struct SchemaField {
    name: Ident,
    codec: Expr,
    optional: Option<(Expr, Expr, Option<Expr>)>,
}

impl Parse for SchemaField {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
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
        })
    }
}

pub(crate) struct RecordSchemaInput {
    record: Path,
    fields: Vec<SchemaField>,
    policy: Expr,
}

pub(crate) enum SchemaInput {
    Record(Box<RecordSchemaInput>),
    Enum(Box<crate::schema_enum_expand::SchemaEnumInput>),
}

impl Parse for SchemaInput {
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
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![struct]>()?;
        let record = input.parse()?;
        let content;
        braced!(content in input);
        let fields = Punctuated::<SchemaField, Token![,]>::parse_terminated(&content)?;
        let policy_name: Ident = input.parse()?;
        if policy_name != "policy" {
            return Err(syn::Error::new(policy_name.span(), "expected `policy`"));
        }
        input.parse::<Token![:]>()?;
        let policy = input.parse()?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after schema policy"));
        }
        Ok(Self {
            record,
            fields: fields.into_iter().collect(),
            policy,
        })
    }
}

pub(crate) fn expand(input: SchemaInput) -> syn::Result<TokenStream> {
    match input {
        SchemaInput::Record(record) => expand_record(*record),
        SchemaInput::Enum(variants) => crate::schema_enum_expand::expand(*variants),
    }
}

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
        quote_spanned!(codec.span()=> let #var = #codec;)
    });
    let mut builder = quote!(#effect::RecordBuilder::<#record>::new());
    for (field, var) in input.fields.iter().zip(&codec_vars).rev() {
        let name = &field.name;
        builder = match &field.optional {
            Some((missing, omit, Some(default))) => quote_spanned!(name.span()=>
                #builder.optional_with_default(
                    stringify!(#name), |value: &#record| &value.#name,
                    #var, #missing, #omit, #default,
                )
            ),
            Some((missing, omit, None)) => quote_spanned!(name.span()=>
                #builder.optional(
                    stringify!(#name), |value: &#record| &value.#name,
                    #var, #missing, #omit,
                )
            ),
            None => quote_spanned!(name.span()=>
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
