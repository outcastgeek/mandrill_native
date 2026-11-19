//! Attach explicit JSON wire codecs to their owned Rust data declarations.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Expr, Field, Fields, Ident, LitStr};

#[derive(Default)]
struct Options {
    factory: Option<Ident>,
    policy: Option<Expr>,
    error_context: Option<LitStr>,
    tag_field: Option<LitStr>,
    content_field: Option<LitStr>,
}

fn options(input: &DeriveInput) -> syn::Result<Options> {
    let mut options = Options::default();
    for attribute in input
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("effect_json_schema"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("factory") && options.factory.is_none() {
                options.factory = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("policy") && options.policy.is_none() {
                options.policy = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("error_context") && options.error_context.is_none() {
                options.error_context = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("tag_field") && options.tag_field.is_none() {
                options.tag_field = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("content_field") && options.content_field.is_none() {
                options.content_field = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unknown or duplicate JSON schema option"));
            }
            Ok(())
        })?;
    }
    Ok(options)
}

fn member_option<T: syn::parse::Parse>(attrs: &[Attribute], name: &str) -> syn::Result<Option<T>> {
    let mut value = None;
    for attribute in attrs
        .iter()
        .filter(|a| a.path().is_ident("effect_json_schema"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident(name) && value.is_none() {
                value = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error(format!("expected a single {name} option")))
            }
        })?;
    }
    Ok(value)
}

fn codec(field: &Field, declarations: &mut Vec<TokenStream>) -> syn::Result<Ident> {
    let expression: Expr = member_option(&field.attrs, "codec")?
        .ok_or_else(|| syn::Error::new_spanned(field, "select the field codec"))?;
    let local = format_ident!(
        "__schema_field_{}",
        declarations.len(),
        span = Span::mixed_site()
    );
    declarations.push(quote!(let #local = #expression;));
    Ok(local)
}

fn named_fields(
    fields: &syn::FieldsNamed,
    declarations: &mut Vec<TokenStream>,
) -> syn::Result<Vec<TokenStream>> {
    fields
        .named
        .iter()
        .map(|field| {
            let name = field
                .ident
                .as_ref()
                .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
            let codec = codec(field, declarations)?;
            Ok(quote!(#name: #codec))
        })
        .collect()
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let options = options(&input)?;
    let name = &input.ident;
    let factory = options
        .factory
        .ok_or_else(|| syn::Error::new_spanned(name, "name the schema factory"))?;
    let policy = options
        .policy
        .ok_or_else(|| syn::Error::new_spanned(name, "select the runtime parse policy"))?;
    let error_context = options
        .error_context
        .ok_or_else(|| syn::Error::new_spanned(name, "supply the schema error context"))?;
    let (parameters, arguments, where_clause) = input.generics.split_for_impl();
    let type_arguments = arguments.as_turbofish();
    let mut declarations = Vec::new();
    let body = match &input.data {
        Data::Struct(record) => {
            let Fields::Named(fields) = &record.fields else {
                return Err(syn::Error::new_spanned(
                    name,
                    "JSON records require named fields",
                ));
            };
            if options.tag_field.is_some() || options.content_field.is_some() {
                return Err(syn::Error::new_spanned(
                    name,
                    "record schemas do not use enum tag and content fields",
                ));
            }
            let fields = named_fields(fields, &mut declarations)?;
            quote!(#path::schema! { struct #name #type_arguments { #(#fields),* } policy: #policy })
        }
        Data::Enum(enumeration) => {
            let tag_field = options
                .tag_field
                .ok_or_else(|| syn::Error::new_spanned(name, "select the enum tag field"))?;
            let content_field = options
                .content_field
                .ok_or_else(|| syn::Error::new_spanned(name, "select the enum content field"))?;
            let variants = enumeration
                .variants
                .iter()
                .map(|variant| {
                    let name = &variant.ident;
                    let tag: LitStr = member_option(&variant.attrs, "tag")?.ok_or_else(|| {
                        syn::Error::new_spanned(name, "select the exact variant wire tag")
                    })?;
                    let fields = match &variant.fields {
                        Fields::Unit => quote!(),
                        Fields::Named(fields) => {
                            let fields = named_fields(fields, &mut declarations)?;
                            quote!({ #(#fields),* })
                        }
                        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                            let field = fields.unnamed.first().ok_or_else(|| {
                                syn::Error::new_spanned(name, "missing tuple payload")
                            })?;
                            let codec = codec(field, &mut declarations)?;
                            quote!((#codec))
                        }
                        Fields::Unnamed(_) => {
                            return Err(syn::Error::new_spanned(
                                name,
                                "schema tuples require one codec",
                            ));
                        }
                    };
                    Ok(quote!(#name #fields => #tag))
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote!(#path::schema_enum! {
                enum #name #type_arguments { #(#variants),* }
                tag_field: #tag_field, content_field: #content_field, policy: #policy
            })
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "JSON schemas require a struct or enum",
            ));
        }
    };
    let visibility = input.vis;
    Ok(quote! {
        #visibility fn #factory #parameters() -> ::core::result::Result<
            #path::PureCodec<#name #arguments, #path::JsonValue>, #path::Defect,
        > #where_clause {
            #(#declarations)*
            #body.map_err(|error| #path::Defect::new(::std::format!("{}: {error:?}", #error_context)))
        }
    })
}
