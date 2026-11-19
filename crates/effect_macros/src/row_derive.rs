//! Reuse the row binder with fields taken from an actual Rust record.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Fields, Type};

fn attribute<T: syn::parse::Parse>(
    attrs: &[Attribute],
    item: &impl quote::ToTokens,
) -> syn::Result<T> {
    let mut attributes = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("effect_sql_row"));
    let selected = attributes.next().ok_or_else(|| {
        syn::Error::new_spanned(item, "supply an explicit #[effect_sql_row(...)]")
    })?;
    if let Some(duplicate) = attributes.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "duplicate row decoder selection",
        ));
    }
    selected.parse_args()
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let path = crate::extension_support::crate_path("effect-sql")?;
    let name = &input.ident;
    let environment: Type = attribute(&input.attrs, name)?;
    let Data::Struct(record) = &input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "EffectSqlRow requires a named record",
        ));
    };
    let Fields::Named(fields) = &record.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "EffectSqlRow requires named fields",
        ));
    };
    let (names, decoders): (Vec<_>, Vec<_>) = fields
        .named
        .iter()
        .map(|field| {
            let name = field
                .ident
                .as_ref()
                .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
            let expression: Expr = attribute(&field.attrs, field)?;
            Ok((name, expression))
        })
        .collect::<syn::Result<Vec<_>>>()?
        .into_iter()
        .unzip();
    let (parameters, arguments, where_clause) = input.generics.split_for_impl();
    let visibility = &input.vis;
    Ok(quote! {
        impl #parameters #name #arguments #where_clause {
            #visibility fn row_decoder() -> #path::RowDecoder<Self, #environment> {
                #path::row! {
                    #(let #names = #decoders;)*
                    Self { #(#names),* }
                }
            }
        }
    })
}
