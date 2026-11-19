//! Borrow capabilities from explicitly selected values on an environment.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::punctuated::Punctuated;
use syn::{DeriveInput, Expr, Ident, Lifetime, Token, Type, WherePredicate};

#[derive(Default)]
struct Selection {
    get: Option<Expr>,
    provider: Option<Type>,
    error: Option<Type>,
    map: Option<Expr>,
    requires: Vec<WherePredicate>,
    lifetime: Option<Lifetime>,
    state: Option<Type>,
}

impl Selection {
    fn parse(meta: &ParseNestedMeta<'_>) -> syn::Result<Self> {
        if meta.input.peek(Token![=]) {
            return Ok(Self {
                get: Some(meta.value()?.parse()?),
                ..Self::default()
            });
        }
        let mut selection = Self::default();
        let mut seen = std::collections::HashSet::new();
        meta.parse_nested_meta(|field| {
            let key = field
                .path
                .get_ident()
                .ok_or_else(|| field.error("expected a capability argument"))?;
            if !seen.insert(key.to_string()) {
                return Err(field.error("duplicate capability argument"));
            }
            if field.path.is_ident("get") {
                selection.get = Some(field.value()?.parse()?);
            } else if field.path.is_ident("provider") {
                selection.provider = Some(field.value()?.parse()?);
            } else if field.path.is_ident("error") {
                selection.error = Some(field.value()?.parse()?);
            } else if field.path.is_ident("map") {
                selection.map = Some(field.value()?.parse()?);
            } else if field.path.is_ident("lifetime") {
                selection.lifetime = Some(field.value()?.parse()?);
            } else if field.path.is_ident("state") {
                selection.state = Some(field.value()?.parse()?);
            } else if field.path.is_ident("requires") {
                let content;
                syn::parenthesized!(content in field.input);
                selection.requires =
                    Punctuated::<WherePredicate, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
            } else {
                return Err(field.error("unknown capability argument"));
            }
            Ok(())
        })?;
        Ok(selection)
    }
}

/// Keep original generics and any expressly selected bounds on this implementation.
fn capability_impl(
    input: &DeriveInput,
    trait_path: TokenStream,
    mut requires: Vec<WherePredicate>,
    members: TokenStream,
) -> TokenStream {
    let name = &input.ident;
    let mut generics = input.generics.clone();
    if !requires.is_empty() {
        generics
            .make_where_clause()
            .predicates
            .extend(requires.drain(..));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics #trait_path for #name #ty_generics #where_clause {
            #members
        }
    }
}

/// A bare field name is borrowed from self; other expressions give the reference explicitly.
fn projection(expression: Expr) -> TokenStream {
    if let Expr::Path(path) = &expression
        && path.qself.is_none()
        && let Some(field) = path.path.get_ident()
        && field != "self"
    {
        return quote!(&self.#field);
    }
    quote!(#expression)
}

fn borrowed(
    capability: &Ident,
    selection: Selection,
    meta: &ParseNestedMeta<'_>,
) -> syn::Result<(TokenStream, Vec<WherePredicate>, TokenStream)> {
    let name = capability.to_string();
    let (package, trait_name, target, associated) = match name.as_str() {
        "cancellation" => (None, "HasCancellation", "Cancellation", false),
        "tracer" => (None, "HasTracer", "Tracer", false),
        "logger" => (None, "HasLogger", "Logger", false),
        "metric_recorder" => (None, "HasMetricRecorder", "Recorder", true),
        "config_source" => (None, "HasConfigSource", "Source", true),
        "prepared_config_source" => (None, "HasPreparedConfigSource", "Source", true),
        "prepared_config_store" => (None, "HasPreparedConfigStore", "Store", true),
        "console" => (None, "HasConsole", "Console", true),
        "task_spawner" => (None, "HasTaskSpawner", "Tasks", true),
        "clock" => (None, "HasClock", "Clock", true),
        "wall_clock" => (None, "HasWallClock", "Clock", true),
        "request_scope" => (None, "HasRequestScope", "RequestScope", false),
        "cpu_executor" => (None, "HasCpuExecutor", "Executor", true),
        "blocking_executor" => (None, "HasBlockingExecutor", "Executor", true),
        "http_transport" => (None, "HasHttpTransport", "Transport", true),
        "websocket_transport" => (None, "HasWebSocketTransport", "Transport", true),
        "sql_client" => (Some("effect-sql"), "HasSqlClient", "SqlClient", false),
        "request" => (Some("effect-server"), "HasRequest", "Request", false),
        "tokio" => (Some("effect-server"), "HasTokio", "TokioHandle", false),
        "otel" => (Some("effect-otel"), "HasOtel", "OtelBackend", false),
        "cms_memory" => (Some("effect_cms"), "HasCmsMemory", "PreparedCmsMemory", false),
        _ => return Err(meta.error("unknown environment capability")),
    };
    let Selection {
        get,
        provider,
        error,
        map,
        requires,
        lifetime,
        state,
    } = selection;
    if lifetime.is_some() || error.is_some() || map.is_some() || state.is_some() {
        return Err(
            meta.error("borrowed capability requires get, with provider for an associated type")
        );
    }
    let get = projection(get.ok_or_else(|| meta.error("missing capability projection"))?);
    let path = match package {
        Some(package) => crate::extension_support::crate_path(package)?,
        None => crate::expand::effect_crate_path()?,
    };
    let trait_name = Ident::new(trait_name, Span::mixed_site());
    let target = Ident::new(target, Span::mixed_site());
    let trait_path = if name == "websocket_transport" {
        quote!(#path::websocket::#trait_name)
    } else {
        quote!(#path::#trait_name)
    };
    let members = if associated {
        let provider = provider.ok_or_else(|| meta.error("missing capability provider type"))?;
        quote!(
            type #target = #provider;
            fn #capability(&self) -> &Self::#target { #get }
        )
    } else {
        if provider.is_some() {
            return Err(meta.error("this capability has no associated provider type"));
        }
        let ty = match name.as_str() {
            "tokio" => quote!(#path::__private::#target),
            "cms_memory" => quote!(::std::sync::Arc<#path::#target>),
            _ => quote!(#path::#target),
        };
        quote!(fn #capability(&self) -> &#ty { #get })
    };
    Ok((trait_path, requires, members))
}

/// These contracts return an owned Arc. The author selects where it comes from.
fn shared(
    capability: &Ident,
    selection: Selection,
    meta: &ParseNestedMeta<'_>,
) -> syn::Result<(TokenStream, Vec<WherePredicate>, TokenStream)> {
    let name = capability.to_string();
    let (module, trait_name, associated) = match name.as_str() {
        "transaction_domain" => ("transactional", "HasTransactionDomain", None),
        "cluster_activation_store" => ("cluster", "HasClusterActivationStore", Some("Store")),
        "event_journal" => ("eventlog", "HasEventJournal", Some("Journal")),
        "atomic_projection_store" => ("eventlog", "HasAtomicProjectionStore", Some("Store")),
        "persisted_queue_store" => ("persistence", "HasPersistedQueueStore", Some("Store")),
        "workflow_engine" => ("workflow", "HasWorkflowEngine", Some("Engine")),
        "key_value_store" => ("persistence", "HasKeyValueStore", Some("Store")),
        _ => return Err(meta.error("unknown shared environment capability")),
    };
    let Selection { get, provider, error, map, requires, lifetime, state } = selection;
    if error.is_some() || map.is_some() || lifetime.is_some() {
        return Err(meta.error("shared capability requires an owned Arc projection"));
    }
    let get = get.ok_or_else(|| meta.error("missing owned Arc projection"))?;
    let core = crate::expand::effect_crate_path()?;
    let module = Ident::new(module, Span::mixed_site());
    let trait_name = Ident::new(trait_name, Span::mixed_site());
    let args = match (name.as_str(), state) {
        ("atomic_projection_store", Some(state)) => quote!(<#state>),
        ("atomic_projection_store", None) => return Err(meta.error("missing projection state type")),
        (_, None) => quote!(),
        (_, Some(_)) => return Err(meta.error("this capability has no projection state type")),
    };
    let (associated_member, target) = match (associated, provider) {
        (Some(associated), Some(provider)) => {
            let associated = Ident::new(associated, Span::mixed_site());
            (quote!(type #associated = #provider;), quote!(Self::#associated))
        }
        (Some(_), None) => return Err(meta.error("missing shared provider type")),
        (None, Some(_)) => return Err(meta.error("this capability has no associated provider type")),
        (None, None) => (quote!(), quote!(#core::transactional::TransactionDomain)),
    };
    Ok((
        quote!(#core::#module::#trait_name #args),
        requires,
        quote!(
            #associated_member
            fn #capability(&self) -> ::std::sync::Arc<#target> { #get }
        ),
    ))
}

fn server_error(
    selection: Selection,
    meta: &ParseNestedMeta<'_>,
) -> syn::Result<(TokenStream, Vec<WherePredicate>, TokenStream)> {
    let Selection {
        get,
        provider,
        error,
        map,
        requires,
        lifetime,
        state,
    } = selection;
    if lifetime.is_some() || get.is_some() || provider.is_some() || state.is_some() {
        return Err(meta.error("server_error requires an error type and mapper"));
    }
    let server = crate::extension_support::crate_path("effect-server")?;
    let error = error.ok_or_else(|| meta.error("missing server error type"))?;
    let map = map.ok_or_else(|| meta.error("missing server error mapping"))?;
    let members = quote!(
        type Error = #error;
        fn server_error(&self, error: #server::ServerError) -> Self::Error {
            (#map)(error)
        }
    );
    Ok((quote!(#server::HasServerError), requires, members))
}

fn memory(
    selection: Selection,
    meta: &ParseNestedMeta<'_>,
) -> syn::Result<(TokenStream, Vec<WherePredicate>, TokenStream)> {
    let Selection {
        get,
        provider,
        error,
        map,
        requires,
        lifetime,
        state,
    } = selection;
    if provider.is_some() || error.is_some() || map.is_some() || state.is_some() {
        return Err(meta.error("memory requires an explicit lifetime and borrowed projection"));
    }
    let lifetime = lifetime.ok_or_else(|| meta.error("missing memory backing lifetime"))?;
    let get = projection(get.ok_or_else(|| meta.error("missing memory projection"))?);
    let effect = crate::expand::effect_crate_path()?;
    let members = quote! {
        fn memory(&self) -> &#effect::memory::Memory<#lifetime> { #get }
    };
    Ok((
        quote!(#effect::memory::HasMemory<#lifetime>),
        requires,
        members,
    ))
}

/// Resolve only the facades used by the requested capabilities.
pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut implementations = TokenStream::new();
    let mut seen = std::collections::HashSet::new();
    for attribute in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("effect_environment"))
    {
        attribute.parse_nested_meta(|meta| {
            let capability = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("expected a capability name"))?;
            if !seen.insert(capability.to_string()) {
                return Err(meta.error("duplicate environment capability"));
            }
            let selection = Selection::parse(&meta)?;
            let (trait_path, requires, members) = match capability.to_string().as_str() {
                "server_error" => server_error(selection, &meta)?,
                "memory" => memory(selection, &meta)?,
                "transaction_domain" | "cluster_activation_store" | "event_journal"
                | "atomic_projection_store" | "persisted_queue_store" | "workflow_engine"
                | "key_value_store" => shared(capability, selection, &meta)?,
                _ => borrowed(capability, selection, &meta)?,
            };
            implementations.extend(capability_impl(&input, trait_path, requires, members));
            Ok(())
        })?;
    }
    if implementations.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "expected an #[effect_environment(...)] capability",
        ));
    }
    Ok(implementations)
}

/// The CMS owns the trait, while Server owns the generic request wrapper. Emit
/// the same public capability through the wrapper's selected app environment.
pub(crate) fn delegate_cms_request_environment(input: syn::ItemTrait) -> syn::Result<TokenStream> {
    let capability = &input.ident;
    if capability != "HasCmsMemory" {
        return Err(syn::Error::new_spanned(capability, "expected HasCmsMemory"));
    }
    let server = crate::extension_support::crate_path("effect-server")?;
    let cms = crate::extension_support::crate_path("effect_cms")?;
    Ok(quote! {
        #input
        impl<R: #capability> #capability for #server::RequestEnv<R> {
            fn cms_memory(&self) -> &::std::sync::Arc<#cms::PreparedCmsMemory> {
                self.services.cms_memory()
            }
        }
    })
}

/// Select whether the existing Arc implementation covers one concrete provider
/// or the owners admitted by the same original provider trait.
pub(crate) struct ArcCapability {
    kind: Ident,
    provider: Type,
}

impl syn::parse::Parse for ArcCapability {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let kind = input.parse()?;
        input.parse::<Token![=]>()?;
        Ok(Self { kind, provider: input.parse()? })
    }
}

pub(crate) fn delegate_arc_capability(selection: ArcCapability, input: syn::ItemTrait) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let [syn::TraitItem::Type(associated), syn::TraitItem::Fn(accessor)] = input.items.as_slice() else {
        return Err(syn::Error::new_spanned(name, "expected a single associated provider and Arc accessor"));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(name, "select an Arc adapter for a nongeneric capability"));
    }
    let signature = &accessor.sig;
    let associated = &associated.ident;
    let provider = selection.provider;
    let param = Ident::new("Provider", Span::mixed_site());
    let (generics, target) = match selection.kind.to_string().as_str() {
        "concrete" => (quote!(), quote!(#provider)),
        "generic" => (quote!(<#param: #provider>), quote!(#param)),
        _ => return Err(syn::Error::new_spanned(selection.kind, "expected concrete or generic Arc provider")),
    };
    Ok(quote! {
        #input
        impl #generics #name for ::std::sync::Arc<#target> {
            type #associated = #target;
            #signature { ::std::sync::Arc::clone(self) }
        }
    })
}
