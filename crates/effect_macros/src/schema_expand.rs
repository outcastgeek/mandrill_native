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
    prepared_options: Option<Vec<(Ident, Expr)>>,
}

pub(crate) struct EffectRecordInput {
    record: Path,
    fields: Vec<SchemaField>,
    options: Vec<(Ident, Expr)>,
    decode_env: Option<syn::Type>,
    encode_env: Option<syn::Type>,
}

fn expand_effect_record(input: EffectRecordInput) -> syn::Result<TokenStream> {
    let effect = crate::expand::effect_crate_path()?;
    let record = &input.record;
    let decode = input.decode_env.map_or_else(|| quote!(_), |ty| quote!(#ty));
    let encode = input.encode_env.map_or_else(|| quote!(_), |ty| quote!(#ty));
    let mut seen = BTreeSet::new();
    let mut declarations = Vec::new();
    let mut fields = Vec::new();
    for (index, field) in input.fields.iter().enumerate() {
        if !seen.insert(field.name.to_string()) {
            return Err(syn::Error::new(field.name.span(), "duplicate schema field"));
        }
        if field.optional.is_some() && (field.optional_attribute || field.default_with.is_some()) {
            return Err(syn::Error::new(
                field.name.span(),
                "conflicting field absence policies",
            ));
        }
        let name = &field.name;
        let codec = format_ident!(
            "__effect_schema_codec_{index}",
            span = proc_macro2::Span::mixed_site()
        );
        let default_factory = if let Some(factory) = &field.default_with {
            let factory = crate::prepared_program_expand::loop_factory(factory.clone(), &effect)?;
            let local = format_ident!(
                "__effect_schema_factory_{index}",
                span = proc_macro2::Span::mixed_site()
            );
            declarations.push(quote!(let #local = #factory;));
            Some(local)
        } else {
            None
        };
        let expression = &field.codec;
        declarations.push(quote!(let #codec = #expression;));
        let policy = if field.optional_attribute {
            let factory = quote!(|| #effect::effect_prepare! { description; program: effect_sync! { once; body: || None, }, });
            Some((false, factory, quote!(|value| value.is_none())))
        } else if let Some(local) = default_factory {
            Some((true, quote!(#local), quote!(())))
        } else if let Some((factory, omit, annotation)) = &field.optional {
            if annotation.is_some() {
                return Err(syn::Error::new(
                    name.span(),
                    "prepared effect records use default_with for defaults; a pure-default tuple is not supported",
                ));
            }
            let factory = crate::prepared_program_expand::loop_factory(factory.clone(), &effect)?;
            let factory_local = format_ident!(
                "__effect_schema_factory_{index}",
                span = proc_macro2::Span::mixed_site()
            );
            let omit_local = format_ident!(
                "__effect_schema_omit_{index}",
                span = proc_macro2::Span::mixed_site()
            );
            declarations.push(quote!(let #factory_local = #factory; let #omit_local = #omit;));
            Some((false, quote!(#factory_local), quote!(#omit_local)))
        } else {
            None
        };
        fields.push((name, codec, policy));
    }
    let option_declarations = input.options.iter().map(|(name, expression)| {
        let var = format_ident!(
            "__effect_schema_{name}",
            span = proc_macro2::Span::mixed_site()
        );
        quote!(let #var = #expression;)
    });
    let mut builder =
        quote!(#effect::PreparedEffectRecordBuilder::<#record, #decode, #encode, _>::new());
    for (name, codec, policy) in fields.iter().rev() {
        builder = match policy {
            None => {
                quote!(#builder.field(stringify!(#name), |value: &#record| &value.#name, #codec))
            }
            Some((true, factory, _)) => {
                quote!(#builder.default_with(stringify!(#name), |value: &#record| &value.#name, #codec, #factory, None))
            }
            Some((false, factory, omit)) => {
                quote!(#builder.optional(stringify!(#name), |value: &#record| &value.#name, #codec, #factory, #omit, None))
            }
        };
    }
    let names: Vec<_> = input.fields.iter().map(|field| &field.name).collect();
    let mut tuple = quote!(());
    for name in names.iter().rev() {
        tuple = quote!((#name, #tuple));
    }
    let ident = |name: &str| {
        format_ident!(
            "__effect_schema_{name}",
            span = proc_macro2::Span::mixed_site()
        )
    };
    let policy = ident("policy");
    let operations = ident("operations");
    let output = ident("output");
    let issues = ident("issues");
    let validation = ident("validation");
    let (map, classify) = if input.options.iter().any(|(name, _)| name == "on_error") {
        let on_error = ident("on_error");
        let into_schema = ident("into_schema_error");
        (quote!(#on_error), quote!(#into_schema))
    } else {
        (
            quote!(::core::convert::identity),
            quote!(::core::result::Result::Ok),
        )
    };
    let finish = if input
        .options
        .iter()
        .any(|(name, _)| name == "representation")
    {
        let representation = ident("representation");
        let fields = ident("fields");
        quote!(#builder.finish_in(#policy, #effect::PreparedGenericRecordResources {
            operations: #operations,
            backing: ::std::sync::Arc::new(#effect::PreparedGenericRecordBacking::new(#representation, #fields)),
            issues: #issues, validation: #validation },
            |#tuple| #record { #(#names),* }, |value: #record| { let #record { #(#names),* } = value; #tuple }, #map, #classify))
    } else {
        quote!(#builder.finish(#policy, #effect::PreparedEffectRecordResources { operations: #operations, output: #output, issues: #issues, validation: #validation },
            |#tuple| #record { #(#names),* }, |value: #record| { let #record { #(#names),* } = value; #tuple }, #map, #classify))
    };
    Ok(quote!({ #(#declarations)* #(#option_declarations)* #finish }))
}

/// Variants of schema input used by schema expand.
pub(crate) enum SchemaInput {
    EffectArray(crate::domain_expand::Fields),
    EffectMap(crate::domain_expand::Fields),
    PreparedSequence(crate::domain_expand::Fields),
    PreparedObject(crate::domain_expand::Fields),
    PreparedBinary(crate::domain_expand::Fields),
    EffectRecord(Box<EffectRecordInput>),
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
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_effect"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return EffectRecordInput::parse(input)
                .map(|record| Self::EffectRecord(Box::new(record)));
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_binary"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return input.parse().map(Self::PreparedBinary);
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_object"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return input.parse().map(Self::PreparedObject);
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_sequence"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return input.parse().map(Self::PreparedSequence);
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_effect_map"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return input.parse().map(Self::EffectMap);
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared_effect_array"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            return input.parse().map(Self::EffectArray);
        }
        let lookahead = input.fork();
        if let Ok(mode) = lookahead.parse::<Ident>()
            && mode == "prepared"
            && lookahead.peek(Token![;])
        {
            input.parse::<Ident>()?;
            input.parse::<Token![;]>()?;
            if input.peek(Token![enum]) {
                return crate::schema_enum_expand::SchemaEnumInput::parse_prepared(input)
                    .map(|value| Self::Enum(Box::new(value)));
            }
            return RecordSchemaInput::parse_mode(input, true)
                .map(|parsed| Self::Record(Box::new(parsed)));
        }
        if input.peek(Token![enum]) {
            return input.parse().map(|parsed| Self::Enum(Box::new(parsed)));
        }
        input
            .parse::<RecordSchemaInput>()
            .map(|parsed| Self::Record(Box::new(parsed)))
    }
}

impl Parse for EffectRecordInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![struct]>()?;
        let record = input.parse()?;
        let content;
        braced!(content in input);
        let fields = Punctuated::<SchemaField, Token![,]>::parse_terminated(&content)?;
        let mut options: Vec<(Ident, Expr)> = Vec::new();
        let mut decode_env = None;
        let mut encode_env = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match name.to_string().as_str() {
                "decode_env" if decode_env.is_none() => decode_env = Some(input.parse()?),
                "encode_env" if encode_env.is_none() => encode_env = Some(input.parse()?),
                "policy" | "operations" | "output" | "issues" | "validation" | "on_error"
                | "into_schema_error" | "representation" | "fields"
                    if !options.iter().any(|(key, _)| key == &name) =>
                {
                    options.push((name, input.parse()?))
                }
                _ => {
                    return Err(syn::Error::new(
                        name.span(),
                        "unknown or duplicate prepared effect record option",
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        for name in ["policy", "operations", "issues", "validation"] {
            if !options.iter().any(|(key, _)| key == name) {
                return Err(input.error(format!("prepared effect record requires {name}")));
            }
        }
        let has_representation = options.iter().any(|(key, _)| key == "representation");
        let has_fields = options.iter().any(|(key, _)| key == "fields");
        let has_output = options.iter().any(|(key, _)| key == "output");
        if has_representation != has_fields || has_output == has_representation {
            return Err(input.error(
                "supply output for JSON or representation and fields together for generic records",
            ));
        }
        if decode_env.is_some() != encode_env.is_some() {
            return Err(input.error("decode_env and encode_env must be supplied together"));
        }
        if options.iter().any(|(key, _)| key == "on_error")
            != options.iter().any(|(key, _)| key == "into_schema_error")
        {
            return Err(input.error("on_error and into_schema_error must be supplied together"));
        }
        Ok(Self {
            record,
            fields: fields.into_iter().collect(),
            options,
            decode_env,
            encode_env,
        })
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
        Self::parse_mode(input, false)
    }
}
impl RecordSchemaInput {
    fn parse_mode(input: ParseStream<'_>, prepared: bool) -> syn::Result<Self> {
        input.parse::<Token![struct]>()?;
        let record = input.parse()?;
        let content;
        braced!(content in input);
        let fields = Punctuated::<SchemaField, Token![,]>::parse_terminated(&content)?;
        let mut policy = None;
        let mut representation = None;
        let mut unknown_fields = None;
        let mut prepared_options = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            let value: Expr = input.parse()?;
            if prepared {
                if !matches!(
                    name.to_string().as_str(),
                    "policy" | "output" | "issues" | "validation"
                ) || prepared_options
                    .iter()
                    .any(|(key, _): &(Ident, Expr)| key == &name)
                {
                    return Err(syn::Error::new(
                        name.span(),
                        "unknown or duplicate prepared schema option",
                    ));
                }
                prepared_options.push((name, value));
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                } else if !input.is_empty() {
                    return Err(input.error("expected comma between schema options"));
                }
                continue;
            }
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
        if prepared {
            for required in ["policy", "output", "issues", "validation"] {
                if !prepared_options.iter().any(|(key, _)| key == required) {
                    return Err(input.error(format!("prepared schema requires {required}")));
                }
            }
            let policy = prepared_options
                .iter()
                .find(|(key, _)| key == "policy")
                .map(|(_, value)| value.clone())
                .ok_or_else(|| input.error("schema requires an explicit policy"))?;
            return Ok(Self {
                record,
                fields: fields.into_iter().collect(),
                policy,
                representation: None,
                unknown_fields: None,
                prepared_options: Some(prepared_options),
            });
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
            prepared_options: None,
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
        SchemaInput::EffectRecord(record) => expand_effect_record(*record),
        SchemaInput::PreparedBinary(mut fields) => {
            let path = crate::expand::effect_crate_path()?;
            let binary = fields.take("binary")?;
            let codec = fields.take("codec")?;
            let bytes = fields.take("bytes")?;
            let errors = fields.take("errors")?;
            let banks = fields.take("banks")?;
            let output = fields.take("output")?;
            let policy = fields.take("policy")?;
            let result = if fields.contains("on_error") {
                let on_error =
                    fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
                quote!(#path::encoding::PreparedBinaryCodec::new_mapped(
                    #binary, #codec, #path::encoding::PreparedBinaryCodecResources {
                        bytes: #bytes, errors: #errors, banks: #banks, output: #output, policy: #policy
                    }, #on_error))
            } else {
                quote!(#path::encoding::PreparedBinaryCodec::new(
                    #binary, #codec, #bytes, #errors, #banks, #output, #policy))
            };
            fields.finish(result)
        }
        SchemaInput::PreparedObject(mut fields) => {
            let path = crate::expand::effect_crate_path()?;
            let codec = fields.take("codec")?;
            let representation = fields.take("representation")?;
            let policy = fields.take("policy")?;
            let entries = fields.take("entries")?;
            let input_fields = fields.take("fields")?;
            let operations = fields.take("operations")?;
            let issues = fields.take("issues")?;
            let validation = fields.take("validation")?;
            let resources = quote!(#path::PreparedObjectCodecResources { entries: #entries, fields: #input_fields, operations: #operations, issues: #issues, validation: #validation });
            let result = match (
                fields.contains("on_error"),
                fields.contains("into_schema_error"),
            ) {
                (false, false) => {
                    quote!(#path::PreparedObjectCodec::new(#codec, #representation, #policy, #resources))
                }
                (true, true) => {
                    let on_error =
                        fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
                    let into_schema = fields.role(
                        "into_schema_error",
                        1,
                        crate::domain_expand::BodyCategory::Native,
                    )?;
                    quote!(#path::PreparedObjectCodec::new_mapped(#codec, #representation, #policy, #resources, #on_error, #into_schema))
                }
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "on_error and into_schema_error must be supplied together",
                    ));
                }
            };
            fields.finish(result)
        }
        SchemaInput::PreparedSequence(mut fields) => {
            let path = crate::expand::effect_crate_path()?;
            let codec = fields.take("codec")?;
            let representation = fields.take("representation")?;
            let policy = fields.take("policy")?;
            let arrays = fields.take("arrays")?;
            let operations = fields.take("operations")?;
            let issues = fields.take("issues")?;
            let validation = fields.take("validation")?;
            let resources = quote!(#path::PreparedSequenceCodecResources { arrays: #arrays, operations: #operations, issues: #issues, validation: #validation });
            let result = match (
                fields.contains("on_error"),
                fields.contains("into_schema_error"),
            ) {
                (false, false) => {
                    quote!(#path::PreparedSequenceCodec::new(#codec, #representation, #policy, #resources))
                }
                (true, true) => {
                    let on_error =
                        fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
                    let into_schema = fields.role(
                        "into_schema_error",
                        1,
                        crate::domain_expand::BodyCategory::Native,
                    )?;
                    quote!(#path::PreparedSequenceCodec::new_mapped(#codec, #representation, #policy, #resources, #on_error, #into_schema))
                }
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "on_error and into_schema_error must be supplied together",
                    ));
                }
            };
            fields.finish(result)
        }
        SchemaInput::EffectMap(mut fields) => {
            let path = crate::expand::effect_crate_path()?;
            let codec = fields.take("codec")?;
            let policy = fields.take("policy")?;
            let entries = fields.take("entries")?;
            let input_fields = fields.take("fields")?;
            let names = fields.take("names")?;
            let output = fields.take("output")?;
            let operations = fields.take("operations")?;
            let issues = fields.take("issues")?;
            let validation = fields.take("validation")?;
            let resources = quote!(#path::PreparedJsonMapCodecResources { entries: #entries, fields: #input_fields, names: #names, output: #output, operations: #operations, issues: #issues, validation: #validation });
            let result = match (
                fields.contains("on_error"),
                fields.contains("into_schema_error"),
            ) {
                (false, false) => {
                    quote!(#path::PreparedJsonMapCodec::new(#codec, #policy, #resources))
                }
                (true, true) => {
                    let on_error =
                        fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
                    let into_schema = fields.role(
                        "into_schema_error",
                        1,
                        crate::domain_expand::BodyCategory::Native,
                    )?;
                    quote!(#path::PreparedJsonMapCodec::new_mapped(#codec, #policy, #resources, #on_error, #into_schema))
                }
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "on_error and into_schema_error must be supplied together",
                    ));
                }
            };
            fields.finish(result)
        }
        SchemaInput::EffectArray(mut fields) => {
            let path = crate::expand::effect_crate_path()?;
            let codec = fields.take("codec")?;
            let policy = fields.take("policy")?;
            let maximum = fields.take("maximum")?;
            let arrays = fields.take("arrays")?;
            let output = fields.take("output")?;
            let operations = fields.take("operations")?;
            let issues = fields.take("issues")?;
            let validation = fields.take("validation")?;
            let resources = quote!(#path::PreparedJsonArrayCodecResources { arrays: #arrays, output: #output,
                operations: #operations, issues: #issues, validation: #validation });
            let result = match (
                fields.contains("on_error"),
                fields.contains("into_schema_error"),
            ) {
                (false, false) => {
                    quote!(#path::PreparedJsonArrayCodec::new(#codec, #policy, #maximum, #resources))
                }
                (true, true) => {
                    let on_error =
                        fields.role("on_error", 1, crate::domain_expand::BodyCategory::Native)?;
                    let into_schema = fields.role(
                        "into_schema_error",
                        1,
                        crate::domain_expand::BodyCategory::Native,
                    )?;
                    quote!(#path::PreparedJsonArrayCodec::new_mapped(#codec, #policy, #maximum, #resources, #on_error, #into_schema))
                }
                _ => {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "on_error and into_schema_error must be supplied together",
                    ));
                }
            };
            fields.finish(result)
        }
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
    if let Some(options) = &input.prepared_options {
        if let Some(field) = input
            .fields
            .iter()
            .find(|field| field.optional_attribute || field.default_with.is_some())
        {
            return Err(syn::Error::new(
                field.name.span(),
                "prepared pure schemas use optional tuple forms",
            ));
        }
        let mut declarations = Vec::new();
        let mut fields = Vec::new();
        for (index, field) in input.fields.iter().enumerate() {
            let codec = format_ident!(
                "__effect_schema_codec_{index}",
                span = proc_macro2::Span::mixed_site()
            );
            let expression = &field.codec;
            declarations.push(quote!(let #codec = #expression;));
            let name = &field.name;
            let optional = if let Some((missing, omit, default)) = &field.optional {
                let missing_var = format_ident!(
                    "__effect_schema_missing_{index}",
                    span = proc_macro2::Span::mixed_site()
                );
                let omit_var = format_ident!(
                    "__effect_schema_omit_{index}",
                    span = proc_macro2::Span::mixed_site()
                );
                declarations.push(quote!(let #missing_var = #missing; let #omit_var = #omit;));
                let default_var = default.as_ref().map(|expression| {
                    let var = format_ident!(
                        "__effect_schema_default_{index}",
                        span = proc_macro2::Span::mixed_site()
                    );
                    declarations.push(quote!(let #var = #expression;));
                    var
                });
                Some((missing_var, omit_var, default_var))
            } else {
                None
            };
            fields.push((name, codec, optional));
        }
        let option_declarations = options.iter().map(|(name, expression)| {
            let var = format_ident!(
                "__effect_schema_{}",
                name,
                span = proc_macro2::Span::mixed_site()
            );
            quote!(let #var = #expression;)
        });
        let mut builder = quote!(#effect::PreparedRecordBuilder::<#record>::new());
        for (name, codec, optional) in fields.iter().rev() {
            builder = match optional {
                Some((missing, omit, Some(default))) => {
                    quote!(#builder.optional_with_default(stringify!(#name), |value: &#record| &value.#name, #codec, #missing, #omit, #default))
                }
                Some((missing, omit, None)) => {
                    quote!(#builder.optional(stringify!(#name), |value: &#record| &value.#name, #codec, #missing, #omit))
                }
                None => {
                    quote!(#builder.field(stringify!(#name), |value: &#record| &value.#name, #codec))
                }
            };
        }
        let names: Vec<_> = input.fields.iter().map(|field| &field.name).collect();
        let mut pattern = quote!(());
        for name in names.iter().rev() {
            pattern = quote!((#name, #pattern));
        }
        let policy_var = format_ident!(
            "__effect_schema_policy",
            span = proc_macro2::Span::mixed_site()
        );
        let output_var = format_ident!(
            "__effect_schema_output",
            span = proc_macro2::Span::mixed_site()
        );
        let issues_var = format_ident!(
            "__effect_schema_issues",
            span = proc_macro2::Span::mixed_site()
        );
        let validation_var = format_ident!(
            "__effect_schema_validation",
            span = proc_macro2::Span::mixed_site()
        );
        return Ok(
            quote!({ #(#declarations)* #(#option_declarations)* #builder.finish(#policy_var, #output_var, #issues_var, #validation_var, |#pattern| #record { #(#names),* }) }),
        );
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
