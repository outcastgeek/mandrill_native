//! Expansion of tagged-enum schema declarations.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Path, Token, braced, parenthesized};

/// Typed named field used by schema enum expand.
struct NamedField {
    name: Ident,
    codec: Expr,
}

impl Parse for NamedField {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        Ok(Self {
            name,
            codec: input.parse()?,
        })
    }
}

/// Variants of payload used by schema enum expand.
enum Payload {
    Unit,
    Tuple(Box<Expr>),
    Named(Vec<NamedField>),
}

/// Typed variant used by schema enum expand.
struct Variant {
    name: Ident,
    payload: Payload,
    tag: Expr,
}

impl Parse for Variant {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        let payload = if input.peek(syn::token::Paren) {
            let content;
            parenthesized!(content in input);
            let codec = content.parse()?;
            if !content.is_empty() {
                return Err(content.error("one payload codec is required"));
            }
            Payload::Tuple(Box::new(codec))
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            let fields = Punctuated::<NamedField, Token![,]>::parse_terminated(&content)?;
            Payload::Named(fields.into_iter().collect())
        } else {
            Payload::Unit
        };
        input.parse::<Token![=>]>()?;
        Ok(Self {
            name,
            payload,
            tag: input.parse()?,
        })
    }
}

/// Typed schema enum input used by schema enum expand.
pub(crate) struct SchemaEnumInput {
    enum_path: Path,
    variants: Vec<Variant>,
    tag_field: Expr,
    content_field: Expr,
    policy: Expr,
    prepared_options: Option<Vec<(Ident, Expr)>>,
}

impl Parse for SchemaEnumInput {
    ///
    /// # Parameters
    /// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `syn::Result<Self>`.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![enum]>()?;
        let enum_path = input.parse()?;
        let content;
        braced!(content in input);
        let variants = Punctuated::<Variant, Token![,]>::parse_terminated(&content)?;
        let tag_field = named_expr(input, "tag_field")?;
        let content_field = named_expr(input, "content_field")?;
        let policy = named_expr(input, "policy")?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after enum schema policy"));
        }
        Ok(Self {
            enum_path,
            variants: variants.into_iter().collect(),
            tag_field,
            content_field,
            policy,
            prepared_options: None,
        })
    }
}
impl SchemaEnumInput {
    pub(crate) fn parse_prepared(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![enum]>()?;
        let enum_path = input.parse()?;
        let content;
        braced!(content in input);
        let variants = Punctuated::<Variant, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        let mut options = Vec::<(Ident, Expr)>::new();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            if !matches!(
                name.to_string().as_str(),
                "tag_field" | "content_field" | "policy" | "output" | "validation"
            ) || options.iter().any(|(key, _)| key == &name)
            {
                return Err(syn::Error::new(
                    name.span(),
                    "unknown or duplicate prepared enum option",
                ));
            }
            input.parse::<Token![:]>()?;
            options.push((name, input.parse()?));
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected comma between enum options"));
            }
        }
        for name in [
            "tag_field",
            "content_field",
            "policy",
            "output",
            "validation",
        ] {
            if !options.iter().any(|(key, _)| key == name) {
                return Err(input.error(format!("prepared enum requires {name}")));
            }
        }
        let get = |name: &str| {
            options
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| input.error("missing prepared enum option"))
        };
        Ok(Self {
            enum_path,
            variants,
            tag_field: get("tag_field")?,
            content_field: get("content_field")?,
            policy: get("policy")?,
            prepared_options: Some(options),
        })
    }
}

///
/// # Parameters
/// - `input` (`ParseStream<'_>`): Owned input decoded or consumed by this operation.
/// - `expected` (`&str`): Value or revision required for the proposed state transition.
///
/// # Returns
/// Type: `syn::Result<Expr>`.
fn named_expr(input: ParseStream<'_>, expected: &str) -> syn::Result<Expr> {
    let name: Ident = input.parse()?;
    if name != expected {
        return Err(syn::Error::new(
            name.span(),
            format!("expected `{expected}`"),
        ));
    }
    input.parse::<Token![:]>()?;
    let value = input.parse()?;
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    }
    Ok(value)
}

/// Expand tagged variants, anchoring named encoders in JsonValue::Object before
/// property insertion so consumers need no map type annotation or serde_json import.
/// Mixed-site encoder locals remain distinct from every user payload field.
///
/// # Parameters
/// - `input` (`SchemaEnumInput`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `syn::Result<TokenStream>`.
pub(crate) fn expand(input: SchemaEnumInput) -> syn::Result<TokenStream> {
    if input.prepared_options.is_some() {
        return expand_prepared(input);
    }
    let effect = crate::expand::effect_crate_path()?;
    let enum_path = &input.enum_path;
    let tag_field = &input.tag_field;
    let content_field = &input.content_field;
    let policy = &input.policy;
    let builder = Ident::new("__effect_builder", Span::mixed_site());
    if input.variants.is_empty() {
        return Err(syn::Error::new(enum_path.span(), "enum needs variants"));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut registrations = Vec::new();
    for variant in &input.variants {
        let name = &variant.name;
        if !names.insert(name.to_string()) {
            return Err(syn::Error::new(name.span(), "duplicate enum variant"));
        }
        let tag = &variant.tag;
        let (preparation, codec, project, construct) = match &variant.payload {
            Payload::Unit => (
                quote!(),
                quote!(#effect::json_null()),
                quote!(|value: &#enum_path| {
                    if matches!(value, #enum_path::#name) { Some(&()) } else { None }
                }),
                quote!(|_| #enum_path::#name),
            ),
            Payload::Tuple(codec) => (
                quote!(),
                quote!(#codec),
                quote!(|value: &#enum_path| {
                    if let #enum_path::#name(inner) = value {
                        Some(inner)
                    } else { None }
                }),
                quote!(|inner| #enum_path::#name(inner)),
            ),
            Payload::Named(fields) => {
                let encoded_value = Ident::new("__effect_variant_encoded", Span::mixed_site());
                let object_map = Ident::new("__effect_variant_object", Span::mixed_site());
                let mut seen = std::collections::BTreeSet::new();
                for field in fields {
                    if !seen.insert(field.name.to_string()) {
                        return Err(syn::Error::new(field.name.span(), "duplicate enum field"));
                    }
                }
                let field_names: Vec<_> = fields.iter().map(|field| &field.name).collect();
                let codecs: Vec<_> = fields
                    .iter()
                    .enumerate()
                    .map(|(index, _)| format_ident!("__effect_variant_codec_{index}"))
                    .collect();
                let decoders: Vec<_> = fields
                    .iter()
                    .enumerate()
                    .map(|(index, _)| format_ident!("__effect_variant_decoder_{index}"))
                    .collect();
                let definitions = fields.iter().zip(&codecs).map(|(field, codec)| {
                    let value = &field.codec;
                    quote_spanned!(value.span().resolved_at(proc_macro2::Span::mixed_site())=> let #codec = #value;)
                });
                let decoder_definitions = codecs
                    .iter()
                    .zip(&decoders)
                    .map(|(codec, decoder)| quote!(let #decoder = #codec.clone();));
                let properties = field_names.iter().zip(&codecs).map(|(field, codec)| {
                    quote!(#effect::SchemaProperty::new(
                        stringify!(#field), #codec.ast().clone(), true, None,
                    )?)
                });
                let decoded = field_names.iter().zip(&decoders).map(|(field, codec)| {
                    quote!(let #field = #codec.decode_ref(
                        object.get(stringify!(#field)).ok_or_else(||
                            #effect::SchemaIssue::new(
                                #effect::SchemaIssueKind::MissingField(
                                    ::std::sync::Arc::from(stringify!(#field)),
                                )
                            )
                        )?
                    ).map_err(|issue| issue.at(#effect::IssueSegment::Field(
                        ::std::sync::Arc::from(stringify!(#field)),
                    )))?;)
                });
                let encoded = field_names.iter().zip(&codecs).map(|(field, codec)| {
                    quote!(#object_map.insert(
                        stringify!(#field).into(),
                        #codec.encode_ref(#field).map_err(|issue|
                            issue.at(#effect::IssueSegment::Field(
                                ::std::sync::Arc::from(stringify!(#field)),
                            ))
                        )?,
                    );)
                });
                let preparation = quote! {
                    #(#definitions)*
                    let __effect_variant_ast = #effect::SchemaAst::object(
                        vec![#(#properties),*], __effect_schema_policy.unknown_fields,
                    )?;
                    #(#decoder_definitions)*
                    let __effect_variant_payload = #effect::PureCodec::with_ast(
                        __effect_variant_ast,
                        move |encoded| {
                            let #effect::JsonValue::Object(object) = encoded else {
                                return Err(#effect::SchemaIssue::new(
                                    #effect::SchemaIssueKind::TypeMismatch {
                                        expected: #effect::ValueKind::Record,
                                        actual: #effect::value_kind(encoded),
                                    }
                                ));
                            };
                            if __effect_schema_policy.unknown_fields
                                == #effect::UnknownFields::Reject {
                                for key in object.keys() {
                                    if ![#(stringify!(#field_names)),*]
                                        .contains(&key.as_str()) {
                                        return Err(#effect::SchemaIssue::new(
                                            #effect::SchemaIssueKind::UnknownField(
                                                ::std::sync::Arc::from(key.as_str()),
                                            )
                                        ));
                                    }
                                }
                            }
                            #(#decoded)*
                            Ok(#enum_path::#name { #(#field_names),* })
                        },
                        move |value: &#enum_path| {
                            let #enum_path::#name { #(#field_names),* } = value else {
                                return Err(#effect::SchemaIssue::new(
                                    #effect::SchemaIssueKind::InvalidTag
                                ));
                            };
                            let mut #encoded_value = #effect::JsonValue::Object(
                                ::core::default::Default::default(),
                            );
                            if let #effect::JsonValue::Object(#object_map) = &mut #encoded_value {
                                #(#encoded)*
                            }
                            Ok(#encoded_value)
                        },
                    );
                };
                (
                    preparation,
                    quote!(__effect_variant_payload),
                    quote!(|value: &#enum_path| {
                        if matches!(value, #enum_path::#name { .. }) {
                            Some(value)
                        } else { None }
                    }),
                    quote!(|value| value),
                )
            }
        };
        registrations.push(
            quote_spanned!(name.span().resolved_at(proc_macro2::Span::mixed_site())=> {
                #preparation
                #builder = #builder.variant(
                    #tag, #codec, #project, #construct,
                )?;
            }),
        );
    }
    Ok(
        quote!((|| -> ::core::result::Result<_, #effect::SchemaIssue> {
        let __effect_schema_policy = #policy;
        let mut #builder = #effect::TaggedEnumBuilder::<#enum_path>::new(
            #tag_field, #content_field,
        );
        #(#registrations)*
        #builder.finish(__effect_schema_policy)
    })()),
    )
}

fn expand_prepared(input: SchemaEnumInput) -> syn::Result<TokenStream> {
    let effect = crate::expand::effect_crate_path()?;
    let enum_path = &input.enum_path;
    if input.variants.is_empty() {
        return Err(syn::Error::new(enum_path.span(), "enum needs variants"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut declarations = Vec::new();
    let mut variants = Vec::new();
    for (index, variant) in input.variants.iter().enumerate() {
        let name = &variant.name;
        if !seen.insert(name.to_string()) {
            return Err(syn::Error::new(name.span(), "duplicate enum variant"));
        }
        let tag_var = format_ident!("__effect_enum_tag_{index}", span = Span::mixed_site());
        let mut codecs = Vec::new();
        match &variant.payload {
            Payload::Unit => {}
            Payload::Tuple(expression) => {
                let id = format_ident!("__effect_enum_codec_{index}_0", span = Span::mixed_site());
                declarations.push(quote!(let #id = #expression;));
                codecs.push(id);
            }
            Payload::Named(fields) => {
                let mut names = std::collections::BTreeSet::new();
                for (field_index, field) in fields.iter().enumerate() {
                    if !names.insert(field.name.to_string()) {
                        return Err(syn::Error::new(field.name.span(), "duplicate enum field"));
                    }
                    let id = format_ident!(
                        "__effect_enum_codec_{index}_{field_index}",
                        span = Span::mixed_site()
                    );
                    let expr = &field.codec;
                    declarations.push(quote!(let #id = #expr;));
                    codecs.push(id);
                }
            }
        }
        let tag = &variant.tag;
        declarations.push(quote!(let #tag_var = #tag;));
        variants.push((name, &variant.payload, tag_var, codecs));
    }
    let options = input
        .prepared_options
        .as_ref()
        .ok_or_else(|| syn::Error::new(enum_path.span(), "missing prepared options"))?;
    let option_bindings = options.iter().map(|(name, expr)| {
        let id = format_ident!("__effect_enum_{name}", span = Span::mixed_site());
        quote!(let #id = #expr;)
    });
    let tag_field = format_ident!("__effect_enum_tag_field", span = Span::mixed_site());
    let content = format_ident!("__effect_enum_content_field", span = Span::mixed_site());
    let policy = format_ident!("__effect_enum_policy", span = Span::mixed_site());
    let output = format_ident!("__effect_enum_output", span = Span::mixed_site());
    let validation = format_ident!("__effect_enum_validation", span = Span::mixed_site());
    let builder = format_ident!("__effect_enum_builder", span = Span::mixed_site());
    let mut registrations = Vec::new();
    for (name, payload, tag, codecs) in variants {
        let (preparation, codec, project, construct) = match payload {
            Payload::Unit => (
                quote!(),
                quote!(#effect::json_prepared_null(#output.clone(),#policy)),
                quote!(|value: &#enum_path| if matches!(value,#enum_path::#name){Some(&())}else{None}),
                quote!(|_| #enum_path::#name),
            ),
            Payload::Tuple(_) => {
                let codec = codecs
                    .first()
                    .ok_or_else(|| syn::Error::new(name.span(), "missing tuple codec"))?;
                (
                    quote!(),
                    quote!(#codec),
                    quote!(|value: &#enum_path| if let #enum_path::#name(inner)=value{Some(inner)}else{None}),
                    quote!(|inner|#enum_path::#name(inner)),
                )
            }
            Payload::Named(fields) => {
                let names: Vec<_> = fields.iter().map(|field| &field.name).collect();
                let dec: Vec<_> = codecs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("__effect_enum_dec_{i}", span = Span::mixed_site()))
                    .collect();
                let val: Vec<_> = codecs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("__effect_enum_val_{i}", span = Span::mixed_site()))
                    .collect();
                let wr: Vec<_> = codecs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("__effect_enum_wr_{i}", span = Span::mixed_site()))
                    .collect();
                let name_handles: Vec<_> = names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        format_ident!("__effect_enum_name_{i}", span = Span::mixed_site())
                    })
                    .collect();
                let decode_names: Vec<_> = names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        format_ident!("__effect_enum_decode_name_{i}", span = Span::mixed_site())
                    })
                    .collect();
                let encode_names: Vec<_> = names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        format_ident!("__effect_enum_encode_name_{i}", span = Span::mixed_site())
                    })
                    .collect();
                let validate_names: Vec<_> = names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        format_ident!("__effect_enum_validate_name_{i}", span = Span::mixed_site())
                    })
                    .collect();
                let write_names: Vec<_> = names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        format_ident!("__effect_enum_write_name_{i}", span = Span::mixed_site())
                    })
                    .collect();
                let name_bindings = names.iter().zip(&name_handles).zip(&decode_names).zip(&encode_names).zip(&validate_names).zip(&write_names).map(|(((((name,base),decode),encode),validate),write)| quote!(let #base: ::std::sync::Arc<str> = ::std::sync::Arc::from(stringify!(#name)); let #decode=::std::sync::Arc::clone(&#base); let #encode=::std::sync::Arc::clone(&#base); let #validate=::std::sync::Arc::clone(&#base); let #write=::std::sync::Arc::clone(&#base);));
                let properties=names.iter().zip(&codecs).map(|(n,c)|quote_spanned!(Span::mixed_site()=>#effect::SchemaProperty::new(stringify!(#n),#c.ast().clone(),true,None)?));
                let dec_clones = dec
                    .iter()
                    .zip(&codecs)
                    .map(|(a, b)| quote!(let #a=#b.clone();));
                let val_clones = val
                    .iter()
                    .zip(&codecs)
                    .map(|(a, b)| quote!(let #a=#b.clone();));
                let wr_clones = wr
                    .iter()
                    .zip(&codecs)
                    .map(|(a, b)| quote!(let #a=#b.clone();));
                let decode=names.iter().zip(&dec).zip(&decode_names).map(|((n,c),name)|quote_spanned!(Span::mixed_site()=>let #n = {let __child = __encoded.try_get(stringify!(#n)).map_err(|e|#effect::SchemaIssue::new(#effect::SchemaIssueKind::Memory(e)))?.ok_or_else(||#effect::SchemaIssue::new(#effect::SchemaIssueKind::MissingField(::std::sync::Arc::clone(&#name))))?;let __context = __context.map(|c|c.child(#effect::IssueSegment::Field(::std::sync::Arc::clone(&#name)))).transpose()?;#c.decode_in_ref(&__child,__context.as_ref()).map_err(|e|#effect::prepared_issue_at(__context.as_ref(), e, #effect::IssueSegment::Field(::std::sync::Arc::clone(&#name))))?};));
                let encode=names.iter().zip(&codecs).zip(&encode_names).map(|((n,c),name)|quote_spanned!(Span::mixed_site()=>__object.insert(stringify!(#n),||{let __context=__context.map(|c|c.child(#effect::IssueSegment::Field(::std::sync::Arc::clone(&#name)))).transpose()?;#c.encode_in_ref(#n,__context.as_ref()).map_err(|e|#effect::prepared_issue_at(__context.as_ref(), e, #effect::IssueSegment::Field(::std::sync::Arc::clone(&#name))))})?;));
                let validate=names.iter().zip(&val).zip(&validate_names).map(|((n,c),name)|quote_spanned!(Span::mixed_site()=>{let __context=__context.map(|c|c.child(#effect::IssueSegment::Field(::std::sync::Arc::clone(&#name)))).transpose()?;#c.validate_in_ref(#n,__context.as_ref()).map_err(|e|#effect::prepared_issue_at(__context.as_ref(), e, #effect::IssueSegment::Field(::std::sync::Arc::clone(&#name))))?;}));
                let write_fields=names.iter().zip(&wr).zip(&write_names).map(|((n,c),name)|quote_spanned!(Span::mixed_site()=>__writer.property(stringify!(#n))?;#c.write_into(#n,__writer).map_err(|e|__writer.issue_at(e, #effect::IssueSegment::Field(::std::sync::Arc::clone(&#name))))?;));
                let prep = quote_spanned! {Span::mixed_site()=>
                    let __ast=#effect::SchemaAst::object(vec![#(#properties),*],#policy.unknown_fields)?;
                    let __has_validator=true #(&& #codecs.has_type_validator())*;
                    let __has_writer=true #(&& #codecs.has_writer())*;
                    #(#dec_clones)* #(#val_clones)* #(#wr_clones)* #(#name_bindings)*
                    let __output=#output.clone();
                    let __decode_output=#output.clone();
                    let __codec=#effect::PureCodec::with_context_ast(__ast,move|__encoded: &#effect::SharedPreparedJsonValue,__context|{
                        let __view=__encoded.view(); if !matches!(__view.kind(),Some(#effect::PreparedJsonKind::Object)){return Err(#effect::SchemaIssue::new(#effect::SchemaIssueKind::TypeMismatch{expected:#effect::ValueKind::Record,actual:#effect::prepared_view_kind(__view)?}));}
                        if #policy.unknown_fields==#effect::UnknownFields::Reject {let mut __child=__view.first_child();while let Some(__item)=__child{let __key=__item.property_name().ok_or_else(||#effect::SchemaIssue::new(#effect::SchemaIssueKind::InvalidDefinition))?;if ![#(stringify!(#names)),*].contains(&__key){return Err(#effect::prepared_unknown_field(__context,__decode_output.issue_memory(),__key));}__child=__item.next_sibling();}}
                        #(#decode)* Ok(#enum_path::#name{#(#names),*})
                    },move|__value: &#enum_path,__context|{let #enum_path::#name{#(#names),*}=__value else{return Err(#effect::SchemaIssue::new(#effect::SchemaIssueKind::InvalidTag));};let __active=__context.map_or(#policy,|c|c.policy().parse);let mut __object=__output.admit_object(__active)?;#(#encode)* __object.finish()});
                    let __codec=if __has_validator{__codec.with_context_type_validator(move|__value,__context|{let #enum_path::#name{#(#names),*}=__value else{return Err(#effect::SchemaIssue::new(#effect::SchemaIssueKind::InvalidTag));};#(#validate)* Ok(())})}else{__codec};
                    let __codec=if __has_writer{__codec.with_writer(move|__value,__writer|{let #enum_path::#name{#(#names),*}=__value else{return Err(#effect::SchemaIssue::new(#effect::SchemaIssueKind::InvalidTag));};__writer.begin_object()?;#(#write_fields)* __writer.end_object()})}else{__codec};
                };
                (
                    prep,
                    quote_spanned!(Span::mixed_site()=>__codec),
                    quote!(|value: &#enum_path|if matches!(value,#enum_path::#name{..}){Some(value)}else{None}),
                    quote!(|value| value),
                )
            }
        };
        registrations.push(
            quote!({#preparation #builder=#builder.variant(#tag,#codec,#project,#construct)?;}),
        );
    }
    Ok(
        quote!((||->::core::result::Result<_,#effect::SchemaIssue>{#(#declarations)* #(#option_bindings)* let mut #builder=#effect::PreparedTaggedEnumBuilder::<#enum_path>::new(#tag_field,#content);#(#registrations)* #builder.finish(#policy,#output,#validation)})()),
    )
}
