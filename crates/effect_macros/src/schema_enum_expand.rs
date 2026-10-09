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
    Tuple(Expr),
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
            Payload::Tuple(codec)
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
    let effect = crate::expand::effect_crate_path()?;
    let enum_path = &input.enum_path;
    let tag_field = &input.tag_field;
    let content_field = &input.content_field;
    let policy = &input.policy;
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
                    quote_spanned!(value.span()=> let #codec = #value;)
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
        registrations.push(quote_spanned!(name.span()=> {
            #preparation
            __effect_builder = __effect_builder.variant(
                #tag, #codec, #project, #construct,
            )?;
        }));
    }
    Ok(
        quote!((|| -> ::core::result::Result<_, #effect::SchemaIssue> {
        let __effect_schema_policy = #policy;
        let mut __effect_builder = #effect::TaggedEnumBuilder::<#enum_path>::new(
            #tag_field, #content_field,
        );
        #(#registrations)*
        __effect_builder.finish(__effect_schema_policy)
    })()),
    )
}
