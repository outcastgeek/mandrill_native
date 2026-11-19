//! Typed ownership, state, load and demand callbacks over their existing owners.
//! Every adapter constructs descriptions; scope/stream/pool execution and ordered
//! cleanup remain in the existing runtime. No new resource or subscription engine.
use crate::callback_expand::{Input, body, program};
use crate::domain_expand::{BodyCategory, Fields};
use proc_macro2::{Span, TokenStream};
use quote::quote;

/// Resource-only release bodies use Effect binds. Primary-aware release factories
/// synchronously inspect the borrowed outcome and return an Effect or Release;
/// their borrowed primary must never be captured by deferred cleanup.
pub(crate) fn release(fields: &mut Fields, name: &str, primary: bool) -> syn::Result<TokenStream> {
    let path = crate::expand::effect_crate_path()?;
    if !fields.inline(name) {
        if primary {
            // Preserve the higher-ranked borrowed outcome at the expression's
            // typing boundary, before ordered fields store the callback.
            return fields.transform(name, |factory, _| {
                Ok(quote!(#path::__private::primary_release(#factory)))
            });
        }
        let factory = fields.take(name)?;
        return Ok(quote!(move |resource| (#factory)(resource).into()));
    }
    let arity = fields.arity(name)?;
    if primary && arity == 2 {
        fields.transform(name, |expression, _| {
            let factory = crate::domain_expand::role(expression, 2, BodyCategory::Native)?;
            Ok(quote!(#path::__private::primary_release(#factory)))
        })
    } else {
        let cleanup = fields.role(name, 1, BodyCategory::Effect)?;
        Ok(if primary {
            quote!(move |resource, _| #path::Release::from_effect((#cleanup)(resource)))
        } else {
            quote!(move |resource| #path::Release::from_effect((#cleanup)(resource)))
        })
    }
}
/// Lower acquisition/body/release, Drop-backed using, scopes, Layers and permits.
pub(crate) fn ownership(name: &str, input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "scoped",
            "prepare",
            "plan",
            "plan_future",
            "prepare_future",
            "prepared_future",
            "prepared",
            "prepare_pool",
            "prepared_pool",
            "admitted_pool",
            "admitted_close",
            "admitted",
            "prepared_close",
            "reserve_closed",
            "reserve_permit",
            "reserve_close",
            "prepared_checkpoint",
            "prepared_release",
            "merge",
            "provide",
            "release",
            "partitioned",
            "partitioned_make",
            "ensuring",
            "release_exit_future",
            "release_prepared_exit_future",
            "release_sync",
            "release_try_sync",
            "release_future",
            "release_try_future",
            "release_effect",
            "binding",
            "binding_native",
            "use",
            "ensuring_release",
        ],
        "default",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    if name == "effect_resource" && mode == "plan_future" {
        let acquire = fields.take("acquire")?;
        let release = fields.role("release", 4, BodyCategory::Native)?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(T, E, R, F, Pending, M),
            &[("acquire", quote!(#path::Effect<T, E, R>)), ("release", quote!(F)), ("on_capacity", quote!(M))],
            quote!(#path::PreparedFutureResourcePlan<T, E, R, F, Pending, M>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, Pending: ::core::future::Future<Output = #path::Exit<(), E>> + Send + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>, ::std::sync::Arc<R>, #path::NativeContext) -> Pending + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!(#path::PreparedFutureResourcePlan::new(#acquire, #release, #mapper)));
    }
    if name == "effect_resource" && mode == "prepare_future" {
        let plan = fields.take("plan")?;
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let metadata = fields.take("metadata")?;
        let resources = fields.take("resources")?;
        return fields.finish_inferred(quote!(T, E, R, F, Pending, M),
            &[("plan", quote!(#path::PreparedFutureResourcePlan<T, E, R, F, Pending, M>)), ("memory", quote!(&#path::memory::PreparedMemory<'static>)), ("metadata", quote!(#path::memory::RegionId)), ("resources", quote!(usize))],
            quote!(::core::result::Result<#path::PreparedFutureResourceBinding<T, E, R, F, Pending, M>, #path::memory::MemoryError>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, Pending: ::core::future::Future<Output = #path::Exit<(), E>> + Send + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>, ::std::sync::Arc<R>, #path::NativeContext) -> Pending + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#plan).prepare(#memory, #metadata, #resources)));
    }
    if name == "effect_scope_acquire" && mode == "prepared_future" {
        let binding = fields.take("binding")?;
        let scope = fields.take("scope")?;
        return fields.finish_inferred(quote!(T, E, R, F, Pending, M),
            &[("binding", quote!(#path::PreparedFutureResourceBinding<T, E, R, F, Pending, M>)), ("scope", quote!(#path::PreparedScope<E>))],
            quote!(#path::Effect<#path::PreparedResourceValue<T>, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, Pending: ::core::future::Future<Output = #path::Exit<(), E>> + Send + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>, ::std::sync::Arc<R>, #path::NativeContext) -> Pending + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#binding).acquire_scoped(#scope)));
    }
    if name == "effect_resource" && mode == "plan" {
        let acquire = fields.take("acquire")?;
        let release = release(&mut fields, "release", true)?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(T, E, R, F, M),
            &[("acquire", quote!(#path::Effect<T, E, R>)), ("release", quote!(F)), ("on_capacity", quote!(M))],
            quote!(#path::PreparedResourcePlan<T, E, R, F, M>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>) -> #path::Release<E, R> + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!(#path::PreparedResourcePlan::new(#acquire, #release, #mapper)));
    }
    if name == "effect_resource" && mode == "prepare" {
        let plan = fields.take("plan")?;
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let metadata = fields.take("metadata")?;
        let resources = fields.take("resources")?;
        return fields.finish_inferred(quote!(T, E, R, F, M),
            &[("plan", quote!(#path::PreparedResourcePlan<T, E, R, F, M>)), ("memory", quote!(&#path::memory::PreparedMemory<'static>)), ("metadata", quote!(#path::memory::RegionId)), ("resources", quote!(usize))],
            quote!(::core::result::Result<#path::PreparedResourceBinding<T, E, R, F, M>, #path::memory::MemoryError>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>) -> #path::Release<E, R> + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#plan).prepare(#memory, #metadata, #resources)));
    }
    if name == "effect_scope_acquire" && mode == "reserve_permit" {
        let scope = fields.transform("scope", |expr, _| Ok(quote!(&(#expr))))?;
        return fields.finish_inferred(quote!(E),&[("scope",quote!(&#path::PreparedScope<E>))],quote!(::core::result::Result<#path::PreparedScopePermit<E>,#path::memory::MemoryError>),quote!(E: Send + 'static),quote!(#path::PreparedScopePermit::reserve(#scope)));
    }
    if name == "effect_scope_acquire" && mode == "admitted" {
        let binding = fields.transform("binding", |value, _| Ok(quote!(&(#value))))?;
        let scope = fields.take("scope")?;
        return fields.finish(quote!((#binding).admit_acquire_scoped(#scope)));
    }
    if name == "effect_scope_acquire" && mode == "prepared" {
        let binding = fields.take("binding")?;
        let scope = fields.take("scope")?;
        return fields.finish_inferred(quote!(T, E, R, F, M),
            &[("binding", quote!(#path::PreparedResourceBinding<T, E, R, F, M>)), ("scope", quote!(#path::PreparedScope<E>))],
            quote!(#path::Effect<#path::PreparedResourceValue<T>, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>) -> #path::Release<E, R> + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#binding).acquire_scoped(#scope)));
    }
    if name == "effect_scope" && mode == "prepare_pool" {
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let metadata = fields.take("metadata")?;
        let scopes = fields.take("scopes")?;
        let limits = fields.take("limits")?;
        let callbacks = fields.take("callbacks")?;
        if fields.contains("panic_diagnostics") {
            let diagnostics = fields.take("panic_diagnostics")?;
            let pool = if fields.contains("cleanup") {
                let cleanup = fields.take("cleanup")?;
                quote!(#path::PreparedScopePool::prepare(#memory, #metadata, #scopes, #limits, #callbacks).map(|pool| pool.with_cleanup(#cleanup)))
            } else {
                quote!(#path::PreparedScopePool::prepare(#memory, #metadata, #scopes, #limits, #callbacks))
            };
            return fields.finish(
                quote!((#pool).and_then(|pool| pool.with_panic_diagnostics(#diagnostics))),
            );
        }
        if fields.contains("cleanup") {
            let cleanup = fields.take("cleanup")?;
            return fields.finish(quote!(#path::PreparedScopePool::prepare(#memory, #metadata, #scopes, #limits, #callbacks).map(|pool| pool.with_cleanup(#cleanup))));
        }
        return fields.finish_inferred(quote!(E),
            &[("memory", quote!(&#path::memory::PreparedMemory<'static>)), ("metadata", quote!(#path::memory::RegionId)), ("scopes", quote!(usize)), ("limits", quote!(#path::PreparedScopeLimits)), ("callbacks", quote!(#path::memory::FramePool<'static>))],
            quote!(::core::result::Result<#path::PreparedScopePool<E>, #path::memory::MemoryError>), quote!(E: Send + 'static),
            quote!(#path::PreparedScopePool::prepare(#memory, #metadata, #scopes, #limits, #callbacks)));
    }
    if name == "effect_scope" && mode == "admitted_pool" {
        let pool = fields.transform("pool", |value, _| Ok(quote!(&(#value))))?;
        let body = fields.role("body", 1, BodyCategory::Effect)?;
        return fields.finish_inferred(quote!(A, E, R, F),
            &[("pool", quote!(&#path::PreparedScopePool<E>)), ("body", quote!(F))],
            quote!(::core::result::Result<#path::Effect<A, E, R>, #path::PreparedScopeAdmissionError>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, F: FnOnce(#path::PreparedScope<E>) -> #path::Effect<A, E, R> + Send + 'static),
            quote!((#pool).admit_scoped(#body)));
    }
    if name == "effect_scope" && mode == "prepared_pool" {
        let pool = fields.take("pool")?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        let (body_name, output, result) = if fields.contains("try_body") {
            let body = fields.role("try_body", 1, BodyCategory::Native)?;
            (
                "try_body",
                quote!((#pool).try_scoped(#body, #mapper)),
                quote!(::core::result::Result<#path::Effect<A, E, R>, #path::memory::MemoryError>),
            )
        } else {
            let body = fields.role("body", 1, BodyCategory::Effect)?;
            (
                "body",
                quote!((#pool).scoped(#body, #mapper)),
                quote!(#path::Effect<A, E, R>),
            )
        };
        return fields.finish_inferred(quote!(A, E, R, F, M),
            &[("pool", quote!(#path::PreparedScopePool<E>)), (body_name, quote!(F)), ("on_capacity", quote!(M))],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, F: Fn(#path::PreparedScope<E>) -> #result + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static), output);
    }
    if name == "effect_scope" && mode == "prepare" {
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let metadata = fields.take("metadata")?;
        let limits = fields.take("limits")?;
        let callbacks = fields.take("callbacks")?;
        if fields.contains("panic_diagnostics") {
            let diagnostics = fields.take("panic_diagnostics")?;
            let owner = if fields.contains("cleanup") {
                let cleanup = fields.take("cleanup")?;
                quote!(#path::PreparedScopeOwner::prepare_with_cleanup(#memory, #metadata, #limits, #callbacks, #cleanup))
            } else {
                quote!(#path::PreparedScopeOwner::prepare(#memory, #metadata, #limits, #callbacks))
            };
            return fields.finish(
                quote!((#owner).and_then(|owner| owner.with_panic_diagnostics(#diagnostics))),
            );
        }
        if fields.contains("cleanup") {
            let cleanup = fields.take("cleanup")?;
            return fields.finish(quote!(#path::PreparedScopeOwner::prepare_with_cleanup(#memory, #metadata, #limits, #callbacks, #cleanup)));
        }
        return fields.finish_inferred(quote!(E),
            &[("memory", quote!(&#path::memory::PreparedMemory<'static>)), ("metadata", quote!(#path::memory::RegionId)), ("limits", quote!(#path::PreparedScopeLimits)), ("callbacks", quote!(#path::memory::FramePool<'static>))],
            quote!(::core::result::Result<#path::PreparedScopeOwner<E>, #path::memory::MemoryError>), quote!(E: Send + 'static),
            quote!(#path::PreparedScopeOwner::prepare(#memory, #metadata, #limits, #callbacks)));
    }
    if name == "effect_scope" && mode == "prepared" {
        let owner = fields.take("owner")?;
        let body = fields.role("body", 1, BodyCategory::Effect)?;
        return fields.finish_inferred(quote!(A, E, R, F),
            &[("owner", quote!(#path::PreparedScopeOwner<E>)), ("body", quote!(F))],
            quote!(::core::result::Result<#path::Effect<A, E, R>, #path::memory::MemoryError>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, F: FnOnce(#path::PreparedScope<E>) -> #path::Effect<A, E, R> + Send + 'static),
            quote!((#owner).into_effect(#body)));
    }
    if name == "effect_scope" && mode == "reserve_close" {
        let owner = fields.take("owner")?;
        let frames = fields.take("frames")?;
        let context = fields.take("context")?;
        return fields.finish_inferred(quote!(A,E,V),
            &[("owner",quote!(#path::PreparedScopeOwner<E>)),("frames",quote!(&#path::memory::FramePool<'static>)),("context",quote!(&#path::PreparedRunContext<V,E>))],
            quote!(::core::result::Result<#path::PreparedScopeClose<A,E,V>,#path::memory::MemoryError>),
            quote!(A: Send + 'static,E: Send + 'static,V: Send + 'static),
            quote!((#owner).reserve_close(#frames,#context)));
    }
    if name == "effect_scope" && mode == "reserve_closed" {
        let scope = fields.take("scope")?;
        let frames = fields.take("frames")?;
        let context = fields.take("context")?;
        return fields.finish_inferred(
            quote!(V, E),
            &[
                ("scope", quote!(&#path::PreparedScope<E>)),
                ("frames", quote!(&#path::memory::FramePool<'static>)),
                ("context", quote!(&#path::PreparedRunContext<V,E>)),
            ],
            quote!(::core::result::Result<#path::PreparedCloseWait<E>,#path::memory::MemoryError>),
            quote!(V: Send + 'static,E: Send + 'static),
            quote!((#scope).reserve_closed(#frames,#context)),
        );
    }
    if name == "effect_scope" && mode == "admitted_close" {
        let scope = fields.transform("scope", |value, _| Ok(quote!(&(#value))))?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish(quote!((#scope).admit_close(#mapper)));
    }
    if name == "effect_scope" && mode == "prepared_close" {
        let scope = fields.take("scope")?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, M),
            &[("scope", quote!(#path::PreparedScope<E>)), ("on_capacity", quote!(M))],
            quote!(#path::Effect<#path::ScopeCloseStatus, E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#scope).close(#mapper)));
    }
    if name == "effect_scope" && mode == "prepared_checkpoint" {
        let scope = fields.transform("scope", |value, _| Ok(quote!(&(#value))))?;
        return fields.finish_inferred(quote!(E), &[("scope", quote!(&#path::PreparedScope<E>))],
            quote!(::core::result::Result<#path::PreparedCheckpoint<E>, #path::memory::MemoryError>), quote!(E: Send + 'static), quote!((#scope).checkpoint()));
    }
    if name == "effect_scope" && mode == "prepared_release" {
        let checkpoint = fields.take("checkpoint")?;
        return fields.finish_inferred(
            quote!(E, R),
            &[("checkpoint", quote!(#path::PreparedCheckpoint<E>))],
            quote!(#path::Effect<(), E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static),
            quote!((#checkpoint).release()),
        );
    }
    if name == "effect_finalizer" && mode == "plan" {
        let factory = fields.role("release", 0, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, F), &[("release", quote!(F))],
            quote!(#path::PreparedReleasePlan<E, R, F>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, F: Fn() -> #path::Release<E, R> + Send + Sync + 'static),
            quote!(#path::PreparedReleasePlan::new(#factory)));
    }
    if name == "effect_finalizer" && mode == "prepare" {
        let memory = fields.transform("memory", |value, _| Ok(quote!(&(#value))))?;
        let metadata = fields.take("metadata")?;
        let factory = fields.role("release", 0, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, F),
            &[("memory", quote!(&#path::memory::PreparedMemory<'static>)), ("metadata", quote!(#path::memory::RegionId)), ("release", quote!(F))],
            quote!(::core::result::Result<#path::PreparedRelease<E, R, F>, #path::memory::MemoryError>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, F: Fn() -> #path::Release<E, R> + Send + Sync + 'static),
            quote!(#path::PreparedRelease::prepare(#memory, #metadata, #factory)));
    }
    if name == "effect_finalizer" && mode == "admitted" {
        let scope = fields.take("scope")?;
        let release = fields.transform("release", |value, _| Ok(quote!(&(#value))))?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, F, M),
            &[("scope", quote!(#path::PreparedScope<E>)), ("release", quote!(&#path::PreparedRelease<E, R, F>)), ("on_capacity", quote!(M))],
            quote!(::core::result::Result<#path::Effect<(), E, R>, #path::memory::MemoryError>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, F: Fn() -> #path::Release<E, R> + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#release).admit_registration(#scope, #mapper)));
    }
    if name == "effect_finalizer" && mode == "prepared" {
        let scope = fields.take("scope")?;
        let release = fields.take("release")?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, F, M),
            &[("scope", quote!(#path::PreparedScope<E>)), ("release", quote!(#path::PreparedRelease<E, R, F>)), ("on_capacity", quote!(M))],
            quote!(#path::Effect<(), E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, F: Fn() -> #path::Release<E, R> + Send + Sync + 'static, M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!((#release).register(#scope, #mapper)));
    }
    if name == "effect_permit" && mode == "partitioned_make" {
        let capacity = fields.take("capacity")?;
        return fields.finish_inferred(
            quote!(K),
            &[("capacity", quote!(::core::num::NonZeroUsize))],
            quote!(#path::PartitionedSemaphore<K>),
            quote!(K: Ord + Send + 'static),
            quote!(#path::PartitionedSemaphore::new(#capacity)),
        );
    }
    if name == "effect_resource" && mode == "binding_native" {
        let acquire = program(&mut fields, "acquire")?;
        let release = fields.role("release", 2, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(T, E, R, F),
            &[("acquire", quote!(#path::Effect<T, E, R>)), ("release", quote!(F))],
            quote!(#path::ResourceBinding<T, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>) -> #path::Release<E, R> + Send + Sync + 'static),
            quote!(#path::ResourceBinding::new(#acquire, #release)),
        );
    }
    if name == "effect_resource" && mode == "release_sync" {
        let callback = fields.role("body", 0, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(E, R, F),
            &[("body", quote!(F))],
            quote!(#path::Release<E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static,
                F: FnOnce() + Send + 'static),
            quote!(#path::Release::sync(#callback)),
        );
    }
    if name == "effect_resource" && mode == "release_try_sync" {
        let callback = fields.role("body", 0, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(E, R, F),
            &[("body", quote!(F))],
            quote!(#path::Release<E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static,
                F: FnOnce() -> ::core::result::Result<(), E> + Send + 'static),
            quote!(#path::Release::try_sync(#callback)),
        );
    }
    if name == "effect_resource" && matches!(mode.as_str(), "release_future" | "release_try_future")
    {
        let callback = fields.role("body", 1, BodyCategory::Native)?;
        let (method, value) = if mode == "release_future" {
            (quote!(from_future), quote!(()))
        } else {
            (quote!(try_future), quote!(::core::result::Result<(), E>))
        };
        return fields.finish_inferred(
            quote!(E, R, F, Pending),
            &[("body", quote!(F))],
            quote!(#path::Release<E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static,
                F: FnOnce(#path::Cancellation) -> Pending + Send + 'static,
                Pending: ::core::future::Future<Output = #value> + Send + 'static),
            quote!(#path::Release::#method(#callback)),
        );
    }
    if name == "effect_resource" && mode == "release_effect" {
        let program = fields.take("program")?;
        return fields.finish(quote!(#path::Release::from_effect(#program)));
    }
    if name == "effect_resource" && mode == "use" {
        let binding = fields.take("binding")?;
        let callback = fields.role("body", 1, BodyCategory::Effect)?;
        return fields.finish_inferred(
            quote!(A, T, E, R, F),
            &[
                ("binding", quote!(#path::ResourceBinding<T, E, R>)),
                ("body", quote!(F)),
            ],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, T: Send + Sync + 'static, E: Send + 'static,
                R: Send + Sync + 'static,
                F: Fn(#path::ResourceValue<T>) -> #path::Effect<A, E, R> + Send + Sync + 'static),
            quote!((#binding).use_with(#callback)),
        );
    }
    if name == "effect_resource" && mode == "ensuring_release" {
        let program = fields.take("program")?;
        let release = fields.role("release", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(A, E, R, F, C),
            &[("program", quote!(#path::Effect<A, E, R>)), ("release", quote!(F))],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: for<'outcome> Fn(#path::PrimaryOutcomeRef<'outcome, E>) -> C + Send + Sync + 'static,
                C: Into<#path::Release<E, R>> + 'static),
            quote!((#program).ensuring_with(move |primary| (#release)(primary).into())),
        );
    }
    if name == "effect_resource" && mode == "release_prepared_exit_future" {
        let callback = fields.role("body", 1, BodyCategory::Native)?;
        let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(quote!(E, R, F, Pending, M),
            &[("body", quote!(F)), ("on_capacity", quote!(M))], quote!(#path::Release<E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, Pending: ::core::future::Future<Output = #path::Exit<(), E>> + Send + 'static, F: FnOnce(#path::Cancellation) -> Pending + Send + 'static, M: FnOnce(#path::memory::MemoryError) -> E + Send + 'static),
            quote!(#path::Release::from_prepared_exit_future(#callback, #mapper)));
    }
    if name == "effect_resource" && mode == "release_exit_future" {
        let callback = fields.role("body", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(E, R, F, Pending),
            &[("body", quote!(F))],
            quote!(#path::Release<E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static,
                F: FnOnce(#path::Cancellation) -> Pending + Send + 'static,
                Pending: ::core::future::Future<Output = #path::Exit<(), E>> + Send + 'static),
            quote!(#path::Release::from_exit_future(#callback)),
        );
    }
    if name == "effect_resource" && mode == "ensuring" {
        let program = fields.take("program")?;
        let release = fields.role("release", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(A, E, R, F),
            &[("program", quote!(#path::Effect<A, E, R>)), ("release", quote!(F))],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: for<'outcome> Fn(#path::PrimaryOutcomeRef<'outcome, E>) -> #path::Effect<(), E, R> + Send + Sync + 'static),
            quote!((#program).ensuring_with(move |primary| #path::Release::from_effect((#release)(primary)))),
        );
    }
    let resource_release =
        if matches!(name, "effect_resource" | "effect_using") && fields.contains("release") {
            Some(fields.inline("release") && fields.arity("release").ok() == Some(1))
        } else {
            None
        };
    let scope_effectful_release = name == "effect_scope_acquire" && fields.inline("release");
    let output = match name {
        "effect_resource" | "effect_using" => {
            let acquire = program(&mut fields, "acquire")?;
            let cleanup = if name == "effect_using" && !fields.contains("release") {
                quote!(move |resource, _| #path::Release::sync(move || drop(resource)))
            } else if resource_release == Some(false) {
                // The resource owner's typed argument boundary checks the
                // borrowed primary before normalization; no untyped closure local.
                let factory = fields.role("release", 2, BodyCategory::Native)?;
                quote!(#path::__private::primary_release(#factory))
            } else {
                release(&mut fields, "release", true)?
            };
            if mode == "binding" {
                quote!(#path::ResourceBinding::new(#acquire, #cleanup))
            } else {
                let callback = fields.role("body", 1, BodyCategory::Effect)?;
                quote!(#path::ResourceBinding::new(#acquire, #cleanup).use_with(#callback))
            }
        }
        "effect_scope" => {
            let callback = fields.role("body", 1, BodyCategory::Effect)?;
            quote!(#path::Scope::scoped(#callback))
        }
        "effect_scope_acquire" => {
            let scope = fields.transform("scope", |scope, _| Ok(quote!(&(#scope))))?;
            let acquire = program(&mut fields, "acquire")?;
            let cleanup = release(&mut fields, "release", false)?;
            quote!((#scope).acquire(#acquire, #cleanup))
        }
        "effect_finalizer" => {
            let scope = fields.transform("scope", |scope, _| Ok(quote!(&(#scope))))?;
            if mode == "release" {
                let factory = fields.role("release", 0, BodyCategory::Native)?;
                quote!((#scope).add_release(#factory))
            } else {
                let cleanup = body(&mut fields)?;
                quote!((#scope).add_finalizer(#cleanup))
            }
        }
        "layer" => match mode.as_str() {
            "merge" => {
                let left = fields.take("left")?;
                let right = fields.take("right")?;
                let combine = fields.role("combine", 2, BodyCategory::Native)?;
                quote!((#left).merge(#right, #combine))
            }
            "provide" => {
                let left = fields.take("left")?;
                let right = fields.take("right")?;
                quote!((#left).provide(#right))
            }
            "scoped" => {
                let acquire = program(&mut fields, "acquire")?;
                let cleanup = release(&mut fields, "release", true)?;
                quote!(#path::Layer::scoped(#acquire, #cleanup))
            }
            "default" => {
                let callback = fields.role("build", 1, BodyCategory::Effect)?;
                quote!(#path::Layer::new(#callback))
            }
            _ => return Err(syn::Error::new(Span::call_site(), "unsupported layer mode")),
        },
        "effect_with_layer" => {
            let layer = fields.take("layer")?;
            let source = body(&mut fields)?;
            quote!((#layer).provide_to(#source))
        }
        "effect_permit" => {
            let semaphore = fields.take("semaphore")?;
            let permits = fields.take("permits")?;
            let acquire = if mode == "partitioned" {
                if fields.contains("key_factory") {
                    let key = fields.role("key_factory", 0, BodyCategory::Native)?;
                    quote!((#semaphore).acquire_with(#key, #permits))
                } else {
                    let key = fields.take("key")?;
                    quote!((#semaphore).acquire(#key, #permits))
                }
            } else {
                quote!((#semaphore).acquire(#permits))
            };
            let mapper = if fields.contains("on_error") {
                fields.role("on_error", 1, BodyCategory::Native)?
            } else {
                quote!(::core::convert::identity)
            };
            let source = body(&mut fields)?;
            // Admission and owned release map at the same existing semaphore boundary.
            quote!({
                let mapper = ::std::sync::Arc::new(#mapper);
                let admission = ::std::sync::Arc::clone(&mapper);
                #path::ResourceBinding::new((#acquire).map_error(move |error| admission(error)), move |mut permit, _| {
                    let mapper = ::std::sync::Arc::clone(&mapper);
                    #path::Release::try_sync(move || permit.release_now().map(|_| ()).map_err(|error| mapper(error)))
                }).use_with(move |_| (#source).clone())
            })
        }
        _ => return Err(syn::Error::new(Span::call_site(), "unknown ownership form")),
    };
    let allowed = match name {
        "layer" => matches!(mode.as_str(), "default" | "merge" | "provide" | "scoped"),
        "effect_finalizer" => mode == "default" || mode == "release",
        "effect_permit" => mode == "default" || mode == "partitioned",
        "effect_resource" | "effect_using" => mode == "default" || mode == "binding",
        _ => mode == "default",
    };
    if !allowed {
        return Err(syn::Error::new(
            Span::call_site(),
            "mode does not apply to this ownership form",
        ));
    }
    if matches!(name, "effect_resource" | "effect_using") {
        let mut types = vec![("acquire", quote!(#path::Effect<T, E, R>))];
        if mode != "binding" {
            types.push(("body", quote!(B)));
        }
        let release_bound = if let Some(effectful) = resource_release {
            types.push(("release", quote!(F)));
            if effectful {
                quote!(F: Fn(T) -> #path::Effect<(), E, R> + Send + Sync + 'static,)
            } else {
                quote!(F: for<'outcome> Fn(T, #path::PrimaryOutcomeRef<'outcome, E>) -> C + Send + Sync + 'static,
                    C: Into<#path::Release<E, R>> + 'static,)
            }
        } else {
            quote!()
        };
        let generics = if mode == "binding" && resource_release == Some(false) {
            quote!(T, E, R, F, C)
        } else if mode == "binding" && resource_release.is_some() {
            quote!(T, E, R, F)
        } else if mode == "binding" {
            quote!(T, E, R)
        } else if resource_release == Some(false) {
            quote!(A, T, E, R, B, F, C)
        } else if resource_release.is_some() {
            quote!(A, T, E, R, B, F)
        } else {
            quote!(A, T, E, R, B)
        };
        let (returns, body_bound) = if mode == "binding" {
            (quote!(#path::ResourceBinding<T, E, R>), quote!())
        } else {
            (
                quote!(#path::Effect<A, E, R>),
                quote!(A: Send + 'static,
                B: Fn(#path::ResourceValue<T>) -> #path::Effect<A, E, R> + Send + Sync + 'static,),
            )
        };
        fields.finish_inferred(
            generics,
            &types,
            returns,
            quote!(T: Send + Sync + 'static, E: Send + 'static,
                R: Send + Sync + 'static, #release_bound #body_bound),
            output,
        )
    } else if name == "effect_finalizer" {
        let (generics, types, bounds) = if mode == "release" {
            (
                quote!(E, R, F),
                vec![("scope", quote!(&#path::Scope<E>)), ("release", quote!(F))],
                quote!(F: Fn() -> #path::Release<E, R> + Send + Sync + 'static),
            )
        } else {
            (
                quote!(E, R),
                vec![
                    ("scope", quote!(&#path::Scope<E>)),
                    ("body", quote!(#path::Effect<(), E, R>)),
                    ("program", quote!(#path::Effect<(), E, R>)),
                ],
                quote!(),
            )
        };
        fields.finish_inferred(
            generics,
            &types,
            quote!(#path::Effect<(), E, R>),
            quote!(E: Send + 'static, R: Send + Sync + 'static, #bounds),
            output,
        )
    } else if name == "effect_scope" {
        // Keep an unannotated scope callback at the native constructor's typed
        // argument boundary rather than hoisting it into an untyped local.
        fields.finish_inferred(
            quote!(A, E, R, F),
            &[("body", quote!(F))],
            quote!(#path::Effect<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(#path::Scope<E>) -> #path::Effect<A, E, R> + Send + Sync + 'static),
            output,
        )
    } else if name == "effect_scope_acquire" {
        // Borrow the existing owner and infer the resource before storing its
        // release callback; owned values still enter cleanup exactly once.
        let (generics, release_bound) = if scope_effectful_release {
            (
                quote!(T, E, R, F),
                quote!(F: Fn(T) -> #path::Effect<(), E, R> + Send + Sync + 'static),
            )
        } else {
            (
                quote!(T, E, R, F, C),
                quote!(F: Fn(T) -> C + Send + Sync + 'static, C: Into<#path::Release<E, R>>),
            )
        };
        fields.finish_inferred(
            generics,
            &[
                ("scope", quote!(&#path::Scope<E>)),
                ("acquire", quote!(#path::Effect<T, E, R>)),
                ("release", quote!(F)),
            ],
            quote!(#path::Effect<#path::ResourceValue<T>, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
                #release_bound),
            output,
        )
    } else {
        fields.finish(output)
    }
}
/// Lower existing state/load callbacks, keeping borrowed validation and tracked transitions typed.
pub(crate) fn state(name: &str, input: Input) -> syn::Result<TokenStream> {
    if name == "effect_ref" && crate::prepared_program_expand::is_prepared_ref(&input) {
        return crate::prepared_program_expand::prepared_ref(
            input,
            &crate::expand::effect_crate_path()?,
        );
    }
    let mode = input.mode(
        &[
            "modify",
            "update",
            "in_place",
            "synchronized",
            "synchronized_pure",
            "exists_idle",
        ],
        "default",
    )?;
    let mut fields = input.fields;
    let path = crate::expand::effect_crate_path()?;
    let prepared_inspection = name == "pool" && mode == "exists_idle" && fields.contains("memory");
    if name == "weak_cache" && mode == "default" && fields.contains("memory") {
        let scope = fields.take("scope")?;
        let storage = fields.take("memory")?;
        if fields.contains("refusals") {
            let refusals = fields.take("refusals")?;
            return fields.finish_inferred(
                quote!(K, A, E, R),
                &[
                    ("scope", quote!(#path::Scope<#path::SharedValue<E>>)),
                    ("memory", quote!(#path::PreparedWeakCacheStorage<K, A, E>)),
                    ("refusals", quote!(#path::PreparedCacheRefusals<E>)),
                ],
                quote!(#path::Effect<#path::WeakCache<K, A, E>, #path::SharedValue<E>, R>),
                quote!(K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static,
                    R: #path::HasClock + #path::HasTaskSpawner + Send + Sync + 'static),
                quote!(#path::WeakCache::make_with_prepared_refusals(#scope, #storage, #refusals)),
            );
        }
        let on_capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(K, A, E, R, M),
            &[
                ("scope", quote!(#path::Scope<#path::SharedValue<E>>)),
                ("memory", quote!(#path::PreparedWeakCacheStorage<K, A, E>)),
                ("on_capacity", quote!(M)),
            ],
            quote!(#path::Effect<#path::WeakCache<K, A, E>, #path::SharedValue<E>, R>),
            quote!(K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static,
                R: #path::HasClock + #path::HasTaskSpawner + Send + Sync + 'static,
                M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            quote!(#path::WeakCache::make_with_prepared_storage(#scope, #storage, #on_capacity)),
        );
    }
    let prepared_storage = name == "pool" && mode == "default" && fields.contains("memory");
    let output = match name {
        "schedule" => {
            let state = fields.take("state")?;
            let step = fields.role("step", 4, BodyCategory::Native)?;
            quote!(#path::Schedule::unfold(#state, #step))
        }
        "effect_ref" => {
            let reference =
                fields.transform("reference", |reference, _| Ok(quote!(&(#reference))))?;
            match mode.as_str() {
                "modify" | "update" | "in_place" => {
                    let role = if mode == "update" { "update" } else { "modify" };
                    let callback = fields.role(role, 1, BodyCategory::Native)?;
                    let method = if mode == "in_place" {
                        quote!(modify_in_place)
                    } else if mode == "update" {
                        quote!(update)
                    } else {
                        quote!(modify)
                    };
                    quote!((#reference).#method(#callback))
                }
                "synchronized" | "synchronized_pure" => {
                    let mapper = fields.role("on_error", 1, BodyCategory::Native)?;
                    let category = if mode == "synchronized" {
                        BodyCategory::Effect
                    } else {
                        BodyCategory::Native
                    };
                    let callback = fields.role("modify", 1, category)?;
                    if mode == "synchronized" {
                        quote!((#reference).modify_effect(#callback, #mapper))
                    } else {
                        quote!((#reference).modify_effect(move |state| { let transition = (#callback)(state); #path::Effect::sync_once(move || transition) }, #mapper))
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        Span::call_site(),
                        "effect_ref requires modify/update/in_place/synchronized/synchronized_pure mode",
                    ));
                }
            }
        }
        "pool" if mode == "exists_idle" => {
            let pool = fields.transform("pool", |pool, _| Ok(quote!(&(#pool))))?;
            let predicate = fields.role("predicate", 1, BodyCategory::Native)?;
            if prepared_inspection {
                let snapshots = fields.take("memory")?;
                let mapper = fields.role("on_capacity", 1, BodyCategory::Native)?;
                quote!((#pool).exists_idle_prepared(#snapshots, #predicate, #mapper))
            } else {
                quote!((#pool).exists_idle(#predicate))
            }
        }
        "pool" => {
            let scope = fields.take("scope")?;
            let capacity = fields.take(if prepared_storage { "memory" } else { "capacity" })?;
            let ttl = fields.take("idle_ttl")?;
            let create = program(&mut fields, "create")?;
            // Borrowed validate input may only supply owned data to its returned Effect.
            let validate = fields.role("validate", 1, BodyCategory::EffectFactory)?;
            let dispose = fields.role("dispose", 1, BodyCategory::Effect)?;
            let constructor = if prepared_storage { quote!(make_with_prepared_storage) } else { quote!(make) };
            quote!(#path::Pool::#constructor(#scope, #capacity, #ttl, #create, #validate, #dispose))
        }
        "weak_cache" => {
            let cache = fields.transform("cache", |cache, _| Ok(quote!(&(#cache))))?;
            let key = fields.take("key")?;
            let load = fields.role("load", 0, BodyCategory::Effect)?;
            if fields.contains("partition") {
                let partition = fields.take("partition")?;
                quote!((#cache).get_partition(#partition, #key, #load))
            } else {
                quote!((#cache).get(#key, #load))
            }
        }
        "request_resolver" => {
            let scope = fields.take("scope")?;
            if fields.contains("run_prepared") {
                let index = fields.take("memory")?;
                let on_capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
                let run = fields.role("run_prepared", 2, BodyCategory::Effect)?;
                quote!(#path::RequestResolver::make_with_prepared_batches(#scope, #index, #on_capacity, #run))
            } else if fields.contains("memory") {
                let index = fields.take("memory")?;
                let on_capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
                let run = fields.role("run", 2, BodyCategory::Effect)?;
                quote!(#path::RequestResolver::make_with_prepared_index(#scope, #index, #on_capacity, #run))
            } else {
                let run = fields.role("run", 2, BodyCategory::Effect)?;
                quote!(#path::RequestResolver::make(#scope, #run))
            }
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "unknown state/load form",
            ));
        }
    };
    if name != "effect_ref" && !(mode == "default" || name == "pool" && mode == "exists_idle") {
        return Err(syn::Error::new(
            Span::call_site(),
            "mode does not apply to this state form",
        ));
    }
    if name == "effect_ref" && matches!(mode.as_str(), "modify" | "update" | "in_place") {
        let (generics, value, callback, value_bound) = if mode == "update" {
            (
                quote!(State, E, R, F),
                quote!(()),
                quote!(F: Fn(&State) -> State + Send + Sync + 'static),
                quote!(),
            )
        } else if mode == "in_place" {
            (
                quote!(State, B, E, R, F),
                quote!(B),
                quote!(F: Fn(&mut State) -> B + Send + Sync + 'static),
                quote!(B: Send + 'static,),
            )
        } else {
            (
                quote!(State, B, E, R, F),
                quote!(B),
                quote!(F: Fn(&State) -> (B, State) + Send + Sync + 'static),
                quote!(B: Send + 'static,),
            )
        };
        fields.finish_inferred(
            generics,
            &[
                ("reference", quote!(&#path::AtomicRef<State>)),
                ("modify", quote!(F)),
                ("update", quote!(F)),
            ],
            quote!(#path::Effect<#value, E, R>),
            quote!(State: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                #value_bound #callback),
            output,
        )
    } else if name == "effect_ref" && matches!(mode.as_str(), "synchronized" | "synchronized_pure")
    {
        let transition = if mode == "synchronized" {
            quote!(#path::Effect<(B, State), E, R>)
        } else {
            quote!((B, State))
        };
        fields.finish_inferred(
            quote!(State, B, E, R, F, M),
            &[("reference", quote!(&#path::SynchronizedRef<State>)), ("modify", quote!(F)), ("on_error", quote!(M))],
            quote!(#path::Effect<B, E, R>),
            quote!(State: Send + Sync + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(::std::sync::Arc<State>) -> #transition + Send + Sync + 'static,
                M: Fn(#path::SemaphoreError) -> E + Send + Sync + 'static),
            output,
        )
    } else if prepared_inspection {
        fields.finish_inferred(
            quote!(T, E, R, F, M),
            &[
                ("pool", quote!(&#path::Pool<T, E>)),
                ("memory", quote!(::std::sync::Arc<#path::PreparedPoolSnapshots<T, E>>)),
                ("predicate", quote!(F)),
                ("on_capacity", quote!(M)),
            ],
            quote!(#path::Effect<bool, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&T) -> bool + Send + Sync + 'static,
                M: Fn(#path::memory::MemoryError) -> E + Send + Sync + 'static),
            output,
        )
    } else if name == "pool" && mode == "exists_idle" {
        fields.finish_inferred(
            quote!(T, E, R, F),
            &[
                ("pool", quote!(&#path::Pool<T, E>)),
                ("predicate", quote!(F)),
            ],
            quote!(#path::Effect<bool, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(&T) -> bool + Send + Sync + 'static),
            output,
        )
    } else if name == "pool" && mode == "default" {
        let capacity = if prepared_storage {
            ("memory", quote!(#path::PreparedPoolStorage<E>))
        } else {
            ("capacity", quote!(#path::PoolCapacity))
        };
        fields.finish_inferred(
            quote!(T, E, R, F, G),
            &[
                ("scope", quote!(#path::Scope<E>)),
                capacity,
                (
                    "idle_ttl",
                    quote!(::core::option::Option<::std::time::Duration>),
                ),
                ("create", quote!(#path::Effect<T, E, R>)),
                ("validate", quote!(F)),
                ("dispose", quote!(G)),
            ],
            quote!(#path::Effect<#path::Pool<T, E>, E, R>),
            quote!(T: Send + Sync + 'static, E: Send + 'static,
                R: #path::HasClock + #path::HasTaskSpawner + Send + Sync + 'static,
                F: Fn(&T) -> #path::Effect<bool, E, R> + Send + Sync + 'static,
                G: Fn(T) -> #path::Effect<(), E, R> + Send + Sync + 'static),
            output,
        )
    } else if name == "schedule" {
        // Infer borrowed state/input before hoisting a reusable transition.
        fields.finish_inferred(
            quote!(I, O, Sampling, State, F),
            &[("state", quote!(State)), ("step", quote!(F))],
            quote!(#path::Schedule<I, O, Sampling>),
            quote!(I: 'static, O: 'static, Sampling: Send + Sync + 'static,
                State: Send + Sync + 'static,
                F: Fn(&State, &I, ::std::time::Duration, ::core::option::Option<#path::UnitInterval>)
                    -> ::core::result::Result<(O, ::core::option::Option<(::std::time::Duration, State)>), #path::Defect>
                    + Send + Sync + 'static),
            output,
        )
    } else {
        fields.finish(output)
    }
}
fn prepared_stream(
    mode: &str,
    mut fields: Fields,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    if mode == "pull_prepared" {
        let source = fields.take("pull")?;
        return fields.finish_inferred(
            quote!(P),
            &[("pull", quote!(P))],
            quote!(#path::PreparedStream<P>),
            quote!(P: #path::PreparedStreamPull),
            quote!(#path::PreparedStream::new(#source)),
        );
    }
    let memory = fields.take("memory")?;
    let capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
    if mode == "iterable_prepared" {
        let iterator = fields.take("iterable")?;
        return fields.finish_inferred(
            quote!(I, E, M),
            &[("iterable", quote!(I)), ("memory", quote!(#path::PreparedChunkPool<I::Item>)), ("on_capacity", quote!(M))],
            quote!(#path::PreparedStream<impl #path::PreparedStreamPull<Item = I::Item, Error = E>>),
            quote!(I: ::core::iter::Iterator + Send, I::Item: Send + 'static,
                E: Send + 'static, M: Fn(#path::memory::MemoryError) -> E + Send),
            quote!(#path::PreparedStream::from_iterable(#iterator, #memory, #capacity)),
        );
    }
    let source = fields.take("stream")?;
    if mode == "map_prepared" {
        let map = fields.role("map", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(P, B, F, Fut, M),
            &[("stream", quote!(#path::PreparedStream<P>)), ("memory", quote!(#path::PreparedChunkPool<B>)),
              ("map", quote!(F)), ("on_capacity", quote!(M))],
            quote!(#path::PreparedStream<impl #path::PreparedStreamPull<Item = B, Error = P::Error>>),
            quote!(P: #path::PreparedStreamPull, B: Send + 'static,
                F: FnMut(P::Item) -> Fut + Send,
                Fut: ::core::future::Future<Output = #path::Exit<B, P::Error>> + Send,
                M: Fn(#path::memory::MemoryError) -> P::Error + Send),
            quote!((#source).map(#memory, #map, #capacity)),
        );
    }
    fields.finish_inferred(
        quote!(P, M),
        &[("stream", quote!(#path::PreparedStream<P>)),
          ("memory", quote!(#path::PreparedChunkPool<P::Item>)), ("on_capacity", quote!(M))],
        quote!(#path::PreparedStream<impl #path::PreparedStreamPull<Item = P::Item, Error = P::Error>>),
        quote!(P: #path::PreparedStreamPull, M: Fn(#path::memory::MemoryError) -> P::Error + Send),
        quote!((#source).rechunk(#memory, #capacity)),
    )
}

fn prepared_sink(
    mode: &str,
    mut fields: Fields,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let source = fields.take("stream")?;
    let memory = fields.take("memory")?;
    let capacity = fields.role("on_capacity", 1, BodyCategory::Native)?;
    let mut types = vec![("stream", quote!(&'a mut #path::PreparedStream<P>)), ("on_capacity", quote!(M))];
    let bounds = quote!(P: #path::PreparedStreamPull + 'a,
        M: FnOnce(#path::memory::MemoryError) -> P::Error + Send + 'a);
    if mode == "collect_prepared" {
        types.push(("memory", quote!(&'a #path::PreparedChunkPool<P::Item>)));
        return fields.finish_inferred(
            quote!('a, P, M), &types,
            quote!(impl ::core::future::Future<Output = #path::Exit<#path::PreparedChunkValues<P::Item>, P::Error>> + Send + 'a),
            bounds,
            quote!((#source).collect(#memory, #capacity)),
        );
    }
    let initial = fields.role("initial", 0, BodyCategory::Native)?;
    let step = fields.role("step", 2, BodyCategory::Native)?;
    types.extend([("memory", quote!(&'a #path::PreparedFoldPool<S>)), ("initial", quote!(I)), ("step", quote!(F))]);
    fields.finish_inferred(
        quote!('a, P, S, I, F, M), &types,
        quote!(impl ::core::future::Future<Output = #path::Exit<#path::PreparedFoldValue<S>, P::Error>> + Send + 'a),
        quote!(#bounds, S: Send + 'static, I: FnOnce() -> S + Send + 'a,
            F: FnMut(&mut S, P::Item) -> ::core::result::Result<(), P::Error> + Send + 'a),
        quote!((#source).fold(#memory, #initial, #step, #capacity)),
    )
}

/// Stream descriptors and consumers retain pull demand, owning scope and ordered release.
pub(crate) fn demand(name: &str, input: Input) -> syn::Result<TokenStream> {
    let mode = input.mode(
        &[
            "for_each",
            "fold",
            "fold_effect",
            "iterable",
            "async_iterable",
            "async_iterable_effect",
            "map_effect",
            "map_error",
            "project",
            "pull",
            "connection",
            "socket",
            "iterable_prepared",
            "pull_prepared",
            "map_prepared",
            "rechunk",
            "collect_prepared",
            "fold_prepared",
        ],
        "default",
    )?;
    let mut fields = input.fields;
    let native_pull = name == "stream" && fields.contains("pull") && !fields.inline("read");
    let path = crate::expand::effect_crate_path()?;
    if matches!(name, "stream")
        && matches!(mode.as_str(), "iterable_prepared" | "pull_prepared" | "map_prepared" | "rechunk")
    {
        return prepared_stream(&mode, fields, &path);
    }
    if name == "sink" && matches!(mode.as_str(), "collect_prepared" | "fold_prepared") {
        return prepared_sink(&mode, fields, &path);
    }
    if name == "effect_socket"
        && matches!(mode.as_str(), "default" | "socket")
        && !fields.contains("options")
        && !fields.contains("on_error")
    {
        let limits = fields.take("limits")?;
        let acquire = program(&mut fields, "acquire")?;
        let use_socket = fields.role("body", 1, BodyCategory::Effect)?;
        return fields.finish_inferred(
            quote!(A, R, F),
            &[("limits", quote!(#path::websocket::WebSocketLimits)),
                ("acquire", quote!(#path::Effect<::std::sync::Arc<dyn #path::websocket::WebSocket>, #path::websocket::WebSocketError, R>)),
                ("body", quote!(F))],
            quote!(#path::Effect<A, #path::websocket::WebSocketError, R>),
            quote!(A: Send + 'static, R: Send + Sync + 'static,
                F: Fn(#path::ResourceValue<#path::websocket::WebSocketConnection>) -> #path::Effect<A, #path::websocket::WebSocketError, R> + Send + Sync + 'static),
            quote!(#path::websocket::with_socket(#limits, #acquire, #use_socket)),
        );
    }
    if name == "stream" && fields.contains("open") {
        let open = fields.role("open", 1, BodyCategory::Native)?;
        return fields.finish_inferred(
            quote!(A, E, R, F), &[("open", quote!(F))],
            quote!(#path::Stream<A, E, R>),
            quote!(A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static,
                F: Fn(#path::Scope<E>) -> #path::Effect<#path::StreamReader<A, E, R>, E, R> + Send + Sync + 'static),
            quote!(#path::Stream::new(#open)),
        );
    }
    let output = match name {
        "sink" => match mode.as_str() {
            "for_each" => {
                let consume = fields.role("body", 1, BodyCategory::Effect)?;
                quote!(#path::Sink::for_each(#consume))
            }
            "fold" | "fold_effect" => {
                let initial = fields.role("initial", 0, BodyCategory::Native)?;
                let step = fields.role(
                    "step",
                    2,
                    if mode == "fold" {
                        BodyCategory::SyncEffect
                    } else {
                        BodyCategory::Effect
                    },
                )?;
                quote!(#path::Sink::fold(#initial, #step))
            }
            _ => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "sink requires for_each/fold/fold_effect mode",
                ));
            }
        },
        "effect_stream_for_each" => {
            let stream = fields.take("stream")?;
            let consume = fields.role("body", 1, BodyCategory::Effect)?;
            quote!((#stream).run_for_each(#consume))
        }
        "stream" => {
            let selected = if mode != "default" {
                mode.clone()
            } else {
                [
                    "iterable",
                    "async_iterable",
                    "async_iterable_effect",
                    "map_effect",
                    "map_error",
                    "project",
                    "pull",
                ]
                .into_iter()
                .find(|role| fields.contains(role))
                .ok_or_else(|| {
                    syn::Error::new(Span::call_site(), "missing stream source/transform role")
                })?
                .into()
            };
            match selected.as_str() {
                "iterable" => {
                    let factory = fields.role("iterable", 0, BodyCategory::Native)?;
                    quote!(#path::Stream::from_iterable(#factory))
                }
                "async_iterable" | "async_iterable_effect" => {
                    let factory = fields.role(
                        &selected,
                        0,
                        if selected == "async_iterable" {
                            BodyCategory::Native
                        } else {
                            BodyCategory::Effect
                        },
                    )?;
                    let error = fields.role("on_error", 1, BodyCategory::Native)?;
                    let cleanup = release(&mut fields, "release", false)?;
                    let method = if selected == "async_iterable" {
                        quote!(from_async_iterable)
                    } else {
                        quote!(from_async_iterable_effect)
                    };
                    quote!(#path::Stream::#method(#factory, #error, #cleanup))
                }
                "pull" => {
                    let acquire = program(&mut fields, "pull")?;
                    let read = fields.role("read", 1, BodyCategory::Effect)?;
                    let cleanup = release(&mut fields, "release", false)?;
                    if native_pull {
                        quote!(#path::Stream::from_pull(#acquire, #read, #cleanup))
                    } else {
                        quote!({
                            let read = ::std::sync::Arc::new(#read);
                            #path::Stream::from_pull(#acquire, move |reader: #path::ResourceValue<_>| {
                                let read = ::std::sync::Arc::clone(&read);
                                #path::Effect::defer(move || read(reader.clone()))
                            }, #cleanup)
                        })
                    }
                }
                "map_effect" => {
                    let stream = fields.take("stream")?;
                    let map = fields.role("map_effect", 1, BodyCategory::Effect)?;
                    quote!((#stream).map_effect(#map))
                }
                "map_error" => {
                    let stream = fields.take("stream")?;
                    let map = fields.role("map_error", 1, BodyCategory::Native)?;
                    quote!((#stream).map_error(#map))
                }
                "project" => {
                    let stream = fields.take("stream")?;
                    let project = fields.role("project", 1, BodyCategory::Native)?;
                    quote!((#stream).provide_some(#project))
                }
                _ => return Err(syn::Error::new(Span::call_site(), "unknown stream mode")),
            }
        }
        "effect_socket" => {
            let use_socket = fields.role("body", 1, BodyCategory::Effect)?;
            let mapped = fields.contains("on_error");
            let mapper = if mapped {
                fields.role("on_error", 1, BodyCategory::Native)?
            } else {
                quote!(::core::convert::identity)
            };
            if mode == "connection" || fields.contains("options") {
                let options = fields.take("options")?;
                if mapped {
                    quote!(#path::websocket::with_connection_map_error(#options, #mapper, #use_socket))
                } else {
                    quote!(#path::websocket::with_connection(#options, #use_socket))
                }
            } else {
                let limits = fields.take("limits")?;
                let acquire = program(&mut fields, "acquire")?;
                if mapped {
                    quote!(#path::websocket::with_socket_map_error(#limits, #acquire, #mapper, #use_socket))
                } else {
                    quote!(#path::websocket::with_socket(#limits, #acquire, #use_socket))
                }
            }
        }
        _ => return Err(syn::Error::new(Span::call_site(), "unknown demand form")),
    };
    let allowed = match name {
        "effect_stream_for_each" => mode == "default",
        "effect_socket" => matches!(mode.as_str(), "default" | "connection" | "socket"),
        "stream" => matches!(
            mode.as_str(),
            "default"
                | "iterable"
                | "async_iterable"
                | "async_iterable_effect"
                | "map_effect"
                | "map_error"
                | "project"
                | "pull"
        ),
        "sink" => matches!(mode.as_str(), "for_each" | "fold" | "fold_effect"),
        _ => false,
    };
    if !allowed {
        return Err(syn::Error::new(
            Span::call_site(),
            "mode does not apply to this demand form",
        ));
    }
    if native_pull {
        fields.finish_inferred(
            quote!(A, T, E, R, F, G, C),
            &[("pull", quote!(#path::Effect<T, E, R>)), ("read", quote!(F)), ("release", quote!(G))],
            quote!(#path::Stream<A, E, R>),
            quote!(A: Send + 'static, T: Send + Sync + 'static, E: Send + 'static,
                R: Send + Sync + 'static,
                F: Fn(#path::ResourceValue<T>) -> #path::Effect<#path::Chunk<A>, E, R> + Send + Sync + 'static,
                G: Fn(T) -> C + Send + Sync + 'static, C: Into<#path::Release<E, R>> + 'static),
            output,
        )
    } else if name == "sink" && matches!(mode.as_str(), "fold" | "fold_effect") {
        // Preserve the initial factory and owned fold state at the native
        // constructor's typed callback boundary, without copying accumulator B.
        fields.finish_inferred(
            quote!(A, B, E, R, I, F),
            &[("initial", quote!(I)), ("step", quote!(F))],
            quote!(#path::Sink<A, B, E, R>),
            quote!(A: Send + 'static, B: Send + 'static, E: Send + 'static,
                R: Send + Sync + 'static, I: Fn() -> B + Send + Sync + 'static,
                F: Fn(B, A) -> #path::Effect<B, E, R> + Send + Sync + 'static),
            output,
        )
    } else {
        fields.finish(output)
    }
}
