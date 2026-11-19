//! Allocation-free-at-runtime configuration lowering and explicit prepared pool
//! selection. These macros do not claim interpreter/provider allocation freedom.
use crate::domain_expand::{BodyCategory, Fields};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Generics, Ident, LitInt, Token, Type, braced};

struct Quantity {
    expression: TokenStream,
    literal: Option<u128>,
}
impl Parse for Quantity {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = crate::expand::effect_crate_path()?;
        let fork = input.fork();
        if let Ok(number) = fork.parse::<LitInt>()
            && fork.peek(Ident)
        {
            let unit: Ident = fork.parse()?;
            let multiplier = match unit.to_string().as_str() {
                "B" => 1_u128,
                "KB" | "kB" => 1000,
                "MB" => 1_000_000,
                "KiB" => 1024,
                "MiB" => 1_048_576,
                "GiB" => 1_073_741_824,
                _ => {
                    return Err(syn::Error::new(
                        unit.span(),
                        "expected B, kB, MB, KiB, MiB or GiB",
                    ));
                }
            };
            let value = number
                .base10_parse::<u128>()?
                .checked_mul(multiplier)
                .ok_or_else(|| syn::Error::new(number.span(), "byte unit arithmetic overflow"))?;
            if value > u64::MAX as u128 {
                return Err(syn::Error::new(
                    number.span(),
                    "byte size exceeds supported address space",
                ));
            }
            input.parse::<LitInt>()?;
            input.parse::<Ident>()?;
            let value = LitInt::new(&format!("{value}u128"), number.span());
            return Ok(Self {
                expression: quote!(#path::memory::Bytes::new(::core::convert::TryInto::<usize>::try_into(#value).map_err(|_| #path::memory::MemoryError::ArithmeticOverflow)?)),
                literal: Some(number.base10_parse::<u128>()? * multiplier),
            });
        }
        let expression: Expr = input.parse()?;
        let literal = if let Expr::Lit(value) = &expression {
            if let syn::Lit::Int(value) = &value.lit {
                Some(value.base10_parse::<u128>()?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            expression: quote!(::core::convert::Into::<#path::memory::Bytes>::into(#expression)),
            literal,
        })
    }
}

pub(crate) enum Configuration {
    Helper {
        mode: Ident,
        fields: Fields,
    },
    Capability {
        generics: Generics,
        environment: Box<Type>,
        member: Box<Expr>,
    },
    Plan {
        bindings: Vec<TokenStream>,
        total: Ident,
        policy: TokenStream,
        alignment: Option<Ident>,
        metadata: Option<TokenStream>,
        partitions: Vec<TokenStream>,
    },
}
impl Parse for Configuration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = crate::expand::effect_crate_path()?;
        let fork = input.fork();
        if let Ok(mode) = fork.parse::<Ident>()
            && matches!(
                mode.to_string().as_str(),
                "schema"
                    | "plan"
                    | "schema_array"
                    | "schema_array_operations"
                    | "schema_map_operations"
                    | "schema_record_operations"
                    | "schema_sequence_operations"
                    | "schema_object_operations"
                    | "binary_unions"
                    | "config_store"
                    | "config_effect"
                    | "config_source"
                    | "config"
                    | "pattern"
                    | "numeric"
                    | "issues"
            )
            && fork.peek(Token![;])
        {
            let mode = input.parse()?;
            input.parse::<Token![;]>()?;
            let fields = input.parse()?;
            return Ok(Self::Helper { mode, fields });
        }
        let fork = input.fork();
        if fork.parse::<Ident>().is_ok_and(|name| name == "capability") {
            input.parse::<Ident>()?;
            let generics = if input.peek(Token![<]) {
                input.parse()?
            } else {
                Generics::default()
            };
            let environment = Box::new(input.parse()?);
            input.parse::<Token![=>]>()?;
            let member = Box::new(input.parse()?);
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            }
            return Ok(Self::Capability {
                generics,
                environment,
                member,
            });
        }
        let mut bindings = Vec::new();
        let mut partitions = Vec::new();
        let mut names = Vec::<String>::new();
        let mut fields = Vec::<String>::new();
        let mut total = None;
        let mut total_literal = None;
        let mut partition_sum = Some(0_u128);
        let mut policy = None;
        let mut alignment = None;
        let mut metadata = None;
        while !input.is_empty() {
            let field: Ident = input.parse()?;
            let name = field.to_string();
            if name == "partition" || name == "slots" {
                let region: Ident = input.parse()?;
                if names.contains(&region.to_string()) {
                    return Err(syn::Error::new(region.span(), "duplicate memory partition"));
                }
                names.push(region.to_string());
                input.parse::<Token![:]>()?;
                let local = Ident::new(
                    &format!("__memory_partition_{}", partitions.len()),
                    Span::mixed_site(),
                );
                let (value, literal) = if name == "partition" {
                    let value: Quantity = input.parse()?;
                    (value.expression, value.literal)
                } else {
                    let content;
                    braced!(content in input);
                    let mut slot_fields = Vec::<String>::new();
                    let mut slot_bindings = Vec::new();
                    let count_local = Ident::new("__memory_slot_count", Span::mixed_site());
                    let each_local = Ident::new("__memory_slot_each", Span::mixed_site());
                    let mut literal_count = None;
                    let mut literal_each = None;
                    while !content.is_empty() {
                        let slot_field: Ident = content.parse()?;
                        let slot_name = slot_field.to_string();
                        if slot_fields.contains(&slot_name) {
                            return Err(syn::Error::new(slot_field.span(), "duplicate slot field"));
                        }
                        slot_fields.push(slot_name.clone());
                        content.parse::<Token![:]>()?;
                        match slot_name.as_str() {
                            "count" => {
                                let value: Expr = content.parse()?;
                                if let Expr::Lit(value) = &value
                                    && let syn::Lit::Int(value) = &value.lit
                                {
                                    literal_count = Some(value.base10_parse::<u128>()?);
                                }
                                slot_bindings.push(quote!(let #count_local: usize = #value;));
                            }
                            "each" => {
                                let value: Quantity = content.parse()?;
                                literal_each = value.literal;
                                let value = value.expression;
                                slot_bindings.push(quote!(let #each_local = #value;));
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    slot_field.span(),
                                    "expected count or each",
                                ));
                            }
                        }
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    if !slot_fields.iter().any(|n| n == "count")
                        || !slot_fields.iter().any(|n| n == "each")
                    {
                        return Err(syn::Error::new(
                            region.span(),
                            "slots requires count and each",
                        ));
                    }
                    let literal = match (literal_count, literal_each) {
                        (Some(count), Some(each)) => {
                            Some(count.checked_mul(each).ok_or_else(|| {
                                syn::Error::new(region.span(), "slot size arithmetic overflow")
                            })?)
                        }
                        _ => None,
                    };
                    (
                        quote!({ #(#slot_bindings)* #each_local.checked_mul(#count_local)? }),
                        literal,
                    )
                };
                partition_sum = match (partition_sum, literal) {
                    (Some(total), Some(value)) => {
                        Some(total.checked_add(value).ok_or_else(|| {
                            syn::Error::new(region.span(), "partition arithmetic overflow")
                        })?)
                    }
                    _ => None,
                };
                bindings.push(quote!(let #local = #value;));
                let label = region.to_string();
                partitions.push(quote!(#path::memory::Partition { id: #path::memory::RegionId::named(#label), capacity: #local }));
            } else {
                if fields.contains(&name) {
                    return Err(syn::Error::new(field.span(), "duplicate memory field"));
                }
                fields.push(name.clone());
                input.parse::<Token![:]>()?;
                match name.as_str() {
                    "total" => {
                        let value: Quantity = input.parse()?;
                        total_literal = value.literal;
                        let value = value.expression;
                        let local = Ident::new("__memory_total", Span::mixed_site());
                        bindings.push(quote!(let #local = #value;));
                        total = Some(local);
                    }
                    "mode" => {
                        let value: Ident = input.parse()?;
                        policy = Some(match value.to_string().as_str() {
                            "prepared" => quote!(#path::memory::MemoryPolicy::PreparedReuse),
                            "bounded" => quote!(#path::memory::MemoryPolicy::BoundedAllocation),
                            _ => {
                                return Err(syn::Error::new(
                                    value.span(),
                                    "expected prepared or bounded mode",
                                ));
                            }
                        });
                    }
                    "alignment" => {
                        let value: Expr = input.parse()?;
                        let local = Ident::new("__memory_alignment", Span::mixed_site());
                        bindings.push(quote!(let #local: usize = #value;));
                        alignment = Some(local);
                    }
                    "metadata" => {
                        let value: Ident = input.parse()?;
                        let label = value.to_string();
                        metadata = Some(quote!(#path::memory::RegionId::named(#label)));
                    }
                    _ => {
                        return Err(syn::Error::new(
                            field.span(),
                            "expected total, mode, metadata, alignment, partition or slots",
                        ));
                    }
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        if let (Some(total), Some(sum)) = (total_literal, partition_sum)
            && sum > total
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "partitions exceed total byte budget",
            ));
        }
        Ok(Self::Plan {
            bindings,
            total: total
                .ok_or_else(|| syn::Error::new(Span::call_site(), "missing total byte budget"))?,
            policy: policy
                .ok_or_else(|| syn::Error::new(Span::call_site(), "missing memory mode"))?,
            alignment,
            metadata,
            partitions,
        })
    }
}

pub(crate) fn configuration(input: Configuration) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    Ok(match input {
        Configuration::Helper { mode, mut fields } => {
            let output = match mode.to_string().as_str() {
                "plan" => {
                    let total = fields.take("total")?;
                    let partitions = fields.take("partitions")?;
                    let metadata = fields.take("metadata")?;
                    let policy = fields.take("policy")?;
                    quote!((|| {
                        ::core::result::Result::<_, #path::memory::MemoryError>::Ok(
                            #path::memory::MemoryPlan::new(#total, #partitions)?
                                .metadata(#metadata)?
                                .policy(#policy),
                        )
                    })())
                }
                "numeric" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedNumericScratchPool::prepare(#memory, #region, #capacity))
                }
                "pattern" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let pattern = fields.transform("pattern", |value, _| Ok(quote!(&(#value))))?;
                    let caches = fields.take("caches")?;
                    quote!((#pattern).with_prepared_caches(#memory, #region, #caches))
                }
                "schema" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    let frames = fields.take("frames")?;
                    if fields.contains("issues") {
                        let issues = fields.take("issues")?;
                        quote!(#path::PreparedSchemaMemory::prepare_with_issues(#memory, #region, #capacity, #frames, #issues))
                    } else {
                        quote!(#path::PreparedSchemaMemory::prepare(#memory, #region, #capacity, #frames))
                    }
                }
                "issues" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    let path_frames = fields.take("path_frames")?;
                    let child_frames = fields.take("child_frames")?;
                    let text_frames = fields.take("text_frames")?;
                    quote!(#path::PreparedIssueMemory::prepare(#memory, #region, #capacity, #path_frames, #child_frames, #text_frames))
                }
                "binary_unions" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::encoding::PreparedBinaryUnionPool::prepare(#memory, #region, #capacity))
                }
                "schema_object_operations" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedObjectOperationPool::prepare(#memory, #region, #capacity))
                }
                "schema_sequence_operations" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedSequenceOperationPool::prepare(#memory, #region, #capacity))
                }
                "schema_map_operations" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedMapOperationPool::prepare(#memory, #region, #capacity))
                }
                "schema_record_operations" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedRecordOperationPool::<_, _>::prepare(#memory, #region, #capacity))
                }
                "schema_array_operations" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedArrayOperationPool::prepare(#memory, #region, #capacity))
                }
                "schema_array" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    let frames = fields.take("frames")?;
                    quote!(#path::PreparedJsonArrayPool::prepare(#memory, #region, #capacity, #frames))
                }
                "config_source" => {
                    let store = fields.take("store")?;
                    let on_error = fields.role("on_error", 1, BodyCategory::Native)?;
                    quote!(#path::PreparedConfigStoreSource::new(#store, #on_error))
                }
                "config_effect" => {
                    let config_path = fields.take("path")?;
                    let codec = fields.take("codec")?;
                    let contexts = fields.take("contexts")?;
                    let policy = fields.take("policy")?;
                    let config = if fields.contains("on_error") {
                        let on_error = fields.role("on_error", 1, BodyCategory::Native)?;
                        quote!(#path::PreparedEffectConfig::new_mapped(#config_path, #codec, #contexts, #policy, #on_error))
                    } else {
                        quote!(#path::PreparedEffectConfig::new(#config_path, #codec, #contexts, #policy))
                    };
                    let default = if fields.contains("default") {
                        let default = fields.transform("default", |expression, inline| {
                            let expression = if inline {
                                syn::parse2(crate::domain_expand::role(
                                    expression,
                                    0,
                                    BodyCategory::Native,
                                )?)?
                            } else {
                                expression
                            };
                            crate::prepared_program_expand::loop_factory(expression, &path)
                        })?;
                        quote!(.with_default(#default))
                    } else {
                        quote!()
                    };
                    quote!(#config #default)
                }
                "config_store" => {
                    let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
                    let region = fields.take("region")?;
                    let capacity = fields.take("capacity")?;
                    quote!(#path::PreparedMemoryConfigSource::prepare(#memory, #region, #capacity))
                }
                "config" => {
                    let config_path = fields.take("path")?;
                    let codec = fields.take("codec")?;
                    let default = if fields.contains("default") {
                        let default = fields.take("default")?;
                        quote!(.with_default(#default))
                    } else {
                        quote!()
                    };
                    quote!(#path::PreparedConfig::new(#config_path, #codec) #default)
                }
                _ => return Err(syn::Error::new(mode.span(), "unknown memory helper")),
            };
            fields.finish(output)?
        }
        Configuration::Capability {
            generics,
            environment,
            member,
        } => {
            let lifetime = generics
                .lifetimes()
                .next()
                .map(|p| p.lifetime.clone())
                .unwrap_or_else(|| syn::Lifetime::new("'static", Span::call_site()));
            let (parameters, _, where_clause) = generics.split_for_impl();
            quote!(impl #parameters #path::memory::HasMemory<#lifetime> for #environment #where_clause {
                fn memory(&self) -> &#path::memory::Memory<#lifetime> { &(#member) }
            })
        }
        Configuration::Plan {
            bindings,
            total,
            policy,
            alignment,
            metadata,
            partitions,
        } => {
            let count = partitions.len();
            let aligned = alignment
                .map(|alignment| quote!(let configuration = configuration.alignment(#alignment)?;));
            let accounted = metadata
                .map(|region| quote!(let configuration = configuration.metadata(#region)?;));
            quote!((|| -> ::core::result::Result<#path::memory::MemoryConfiguration<#count>, #path::memory::MemoryError> {
                #(#bindings)*
                let configuration = #path::memory::MemoryConfiguration::new(#total, [#(#partitions),*])?.policy(#policy);
                #aligned #accounted
                Ok(configuration)
            })())
        }
    })
}

pub(crate) fn lease(mut fields: Fields) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    let pool = fields.take("pool")?;
    // Preserve expected argument types even when explicit environment syntax
    // selects the direct-constructor path below. The identity adapter evaluates
    // once at this field's original source position, without boxing the callback.
    let mapper = fields.transform("on_error", |expression, inline| {
        let expression = if inline {
            crate::domain_expand::role(expression, 1, BodyCategory::Native)?
        } else {
            quote!(#expression)
        };
        let adapter = Ident::new("__memory_error_mapper", Span::mixed_site());
        Ok(quote!({
            fn #adapter<E, F>(mapper: F) -> F
            where F: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static { mapper }
            #adapter(#expression)
        }))
    })?;
    if fields.contains("environment") {
        let environment = fields.raw("environment")?;
        return fields.finish(quote!(#path::Effect::try_future_with_env(move |environment: ::std::sync::Arc<#environment>, _cancel| {
            let result = #path::memory::HasMemoryPool::memory_pool(environment.as_ref(), &#pool).checkout();
            async move { result }
        }, #mapper)));
    }
    fields.finish_inferred(
        quote!(R, P, E, F), &[("pool", quote!(P)), ("on_error", quote!(F))],
        quote!(#path::Effect<#path::memory::SlotLease<'static, <R as #path::memory::HasMemoryPool<P>>::Value>, E, R>),
        quote!(R: #path::memory::HasMemoryPool<P> + Send + Sync + 'static, P: Send + Sync + 'static, E: Send + 'static, F: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
        quote!(#path::Effect::try_future_with_env(move |environment: ::std::sync::Arc<R>, _cancel| {
            let result = #path::memory::HasMemoryPool::memory_pool(environment.as_ref(), &#pool).checkout();
            async move { result }
        }, #mapper)),
    )
}

pub(crate) fn vector(mut fields: Fields) -> syn::Result<TokenStream> {
    let allocator = fields.take("allocator")?;
    let capacity = fields.take("capacity")?;
    if fields.contains("init") {
        let path = crate::expand::effect_crate_path()?;
        let init = fields.role("init", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!('a, T, E, F),
            &[("allocator", quote!(#path::memory::RegionAllocator<'a>)), ("capacity", quote!(usize)), ("init", quote!(F))],
            quote!(::core::result::Result<::std::vec::Vec<T, #path::memory::RegionAllocator<'a>>, #path::memory::InitializationError<E>>),
            quote!(F: FnMut(usize) -> ::core::result::Result<T, E>),
            quote!((#allocator).initialize_vector(#capacity, #init)),
        );
    }
    fields.finish(quote!((#allocator).vector(#capacity)))
}

pub(crate) fn writer(mut fields: Fields) -> syn::Result<TokenStream> {
    let writer = fields.transform("into", |expression, _| Ok(quote!(&mut (#expression))))?;
    let format = fields.take("format")?;
    fields.finish_inferred(
        quote!(W),
        &[
            ("into", quote!(&mut W)),
            ("format", quote!(::core::fmt::Arguments<'_>)),
        ],
        quote!(::core::fmt::Result),
        quote!(W: ::core::fmt::Write + ?Sized),
        quote!(::core::fmt::Write::write_fmt(#writer, #format)),
    )
}

pub(crate) fn drop_guard(mut fields: Fields) -> syn::Result<TokenStream> {
    let value = fields.take("value")?;
    let cleanup = fields.transform("cleanup", |expression, _| {
        if let Expr::Closure(closure) = &expression
            && closure.asyncness.is_some()
        {
            return Err(syn::Error::new_spanned(
                expression,
                "DropGuard cleanup must be synchronous",
            ));
        }
        Ok(quote!(#expression))
    })?;
    fields.finish_inferred(
        quote!(T, F),
        &[("value", quote!(T)), ("cleanup", quote!(F))],
        quote!(::core::mem::DropGuard<T, F>),
        quote!(F: FnOnce(T)),
        quote!(::core::mem::DropGuard::new(#value, #cleanup)),
    )
}
