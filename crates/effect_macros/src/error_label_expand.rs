//! Render authored constant labels without inspecting error payloads.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(definition) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "EffectErrorLabel requires an enum",
        ));
    };
    let arms = definition
        .variants
        .iter()
        .map(|variant| {
            let name = &variant.ident;
            let mut labels = variant
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("effect_error_label"));
            let label = labels.next().ok_or_else(|| {
                syn::Error::new_spanned(name, "supply an explicit #[effect_error_label(\"...\")]")
            })?;
            if let Some(duplicate) = labels.next() {
                return Err(syn::Error::new_spanned(duplicate, "duplicate error label"));
            }
            let label: LitStr = label.parse_args()?;
            let fields = match &variant.fields {
                Fields::Unit => quote!(),
                Fields::Unnamed(_) => quote!((..)),
                Fields::Named(_) => quote!({ .. }),
            };
            let cfg = variant
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("cfg"));
            Ok(quote!(#(#cfg)* Self::#name #fields => #label,))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::core::fmt::Display for #name #ty_generics #where_clause {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(match self { #(#arms)* })
            }
        }
    })
}
