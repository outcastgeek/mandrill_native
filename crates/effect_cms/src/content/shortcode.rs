//! The shortcode vocabulary this owner consumes.
//!
//! Content consumes five declarations from the shared text-processing surface —
//! `ShortcodeToken`, `ContentRenderSurface` and its constants,
//! `ContentRenderOptions`, `RenderedContentFragment`, and the
//! `ShortcodeCatalog`/`ShortcodeResolver` contracts. That surface is outside this
//! owner's scope; this module declares only the consumed declarations, with their source
//! shapes and no behaviour of the platform silently dropped as excluded from the port.
//!
//! # Differences that are forced by the port's contracts
//!
//! - `RenderedContentFragment.HTML` is a `String`, produced by the
//!   textprocessing renderer and embedded as trusted markup. It is a [`Markup`] here:
//!   the port's rendering boundary must distinguish trusted rendered markup from a
//!   text value a caller might escape again, and every HTML-producing owner in this
//!   port emits [`Markup`]. A sanitized rich-content string is wrapped with
//!   `maud::PreEscaped` at that boundary, never concatenated.
//! - `ContentRenderOptions.Surface` is an open string value, so
//!   its zero value is the empty string and any value is representable. It stays a
//!   [`String`] with the three constants; no closed Rust enum narrows the open value.
//! - Resolving is effectful: a resolver returns a
//!   fragment and an error, which is an [`Effect`] here, and the catalog/resolver
//!   contracts are generic in the caller's error (`E`) and environment (`R`) types.
//!   The effect run's context, not an extra
//!   parameter.
//!
//! These declarations are not a new content policy. When a shared textprocessing
//! port exists, they move there and the content owner imports them.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::memory::{Buffer, BufferView, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use effect::{Effect, Exit, effect_future, effect_sync};
use maud::Markup;

/// The render surface for chat views.
pub const CONTENT_SURFACE_CHAT_HTML: &str = "chat_html";

/// The render surface for slide and presentation views.
pub const CONTENT_SURFACE_SLIDE_HTML: &str = "slide_html";

/// The render surface for transcript views.
pub const CONTENT_SURFACE_TRANSCRIPT_HTML: &str = "transcript_html";

/// One parsed shortcode token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcodeToken {
    /// The raw token text.
    pub raw: String,
    /// The shortcode name.
    pub name: String,
    /// The parsed arguments.
    pub args: BTreeMap<String, String>,
}

/// The viewer and surface metadata a resolution may use.
///
/// Every member is a zero value: an empty surface and empty viewer
/// fields are meaningful defaults, not absent capabilities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentRenderOptions {
    /// The render surface.
    pub surface: String,
    /// The requested content language.
    pub lang: String,
    /// The viewer's role.
    pub viewer_role: String,
    /// The viewer's identity.
    pub viewer_user_id: String,
    /// The target identity of the rendered content.
    pub target_user_id: String,
}

/// The resolved output of one shortcode token.
#[derive(Clone, Debug, Default)]
pub struct RenderedContentFragment {
    /// The rendered markup, already trusted.
    pub html: Markup,
    /// Whether the fragment is block level.
    pub block: bool,
    /// The resolution cache key, empty when the fragment must not be cached by key.
    pub cache_key: String,
    /// Whether the fragment must not be cached at all.
    pub no_cache: bool,
}

/// Selected shortcode input and native Maud output envelopes.
#[derive(Clone, Copy)]
#[must_use]
pub struct ShortcodeRenderLimits {
    /// Maximum number of supplied token arguments.
    pub arguments: NonZeroUsize,
    /// Maximum aggregate token, argument and render-option bytes.
    pub input_bytes: NonZeroUsize,
    /// Maximum encoded HTML bytes retained in the prepared result.
    pub html_bytes: NonZeroUsize,
    /// Maximum returned cache-key bytes.
    pub cache_key_bytes: NonZeroUsize,
}

/// A selected shortcode result retaining its actual encoded Maud bytes.
#[must_use]
pub struct PreparedContentFragment {
    /// Trusted UTF-8 HTML from the existing resolver.
    pub html: BufferView<'static>,
    /// Whether the fragment is block level.
    pub block: bool,
    /// Bounded resolver-supplied cache key.
    pub cache_key: String,
    /// Whether the fragment can be cached at all.
    pub no_cache: bool,
}

/// Reusable region-backed shortcode output workspace.
pub struct PreparedShortcodeRenderer {
    output: SlotPool<'static, Buffer<'static>>,
    limits: ShortcodeRenderLimits,
}
impl PreparedShortcodeRenderer {
    /// Size the regional output storage. The resolver's native Markup is a separate owner.
    pub fn storage_bytes(
        calls: NonZeroUsize,
        limits: ShortcodeRenderLimits,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<Buffer<'static>>::storage_bytes(calls.get())?
            .checked_add(Bytes::new(limits.html_bytes.get()).checked_mul(calls.get())?)
    }

    /// Provision all output slots before running any resolver.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        limits: ShortcodeRenderLimits,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, limits)?;
        Ok(Self {
            output: memory.buffers(region, calls.get(), Bytes::new(limits.html_bytes.get()))?,
            limits,
        })
    }

    /// Admit the supplied token before calling the existing resolver and retain its encoded output.
    pub fn resolve<E, R>(
        self: &Arc<Self>,
        resolver: Arc<dyn ShortcodeResolver<E, R>>,
        token: ShortcodeToken,
        options: ContentRenderOptions,
        on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<PreparedContentFragment, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let storage = self.clone();
        let input = effect_sync! { once; body: move || (token, options), };
        let on_memory = Arc::new(on_memory);
        effect_future! {
            exit_with_context;
            body: move |env: Arc<R>, context| {
                let storage = storage.clone();
                let resolver = resolver.clone();
                let input = input.clone();
                let memory_error = on_memory.clone();
                async move {
                    let (token, options) = match context.run(input, env.clone()).await {
                        Exit::Success(input) => input,
                        Exit::Fail(error) => return Exit::Fail(error),
                        Exit::Die(defect) => return Exit::Die(defect),
                        Exit::Interrupted => return Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                    };
                    if let Err(error) = check_shortcode(&token, &options, storage.limits) {
                        return Exit::Fail(memory_error(error));
                    }
                    let mut output = match storage.output.try_checkout() {
                        Ok(output) => output,
                        Err(error) => return Exit::Fail(memory_error(error)),
                    };
                    match context.run(resolver.resolve(token, options), env).await {
                        Exit::Success(fragment) => {
                            let RenderedContentFragment { html, block, cache_key, no_cache } = fragment;
                            if let Err(error) = check_limit(cache_key.len(), storage.limits.cache_key_bytes) {
                                return Exit::Fail(memory_error(error));
                            }
                            let html = html.into_string();
                            output.clear();
                            if let Err(error) = output.extend_from_slice(html.as_bytes()) {
                                return Exit::Fail(memory_error(error));
                            }
                            drop(html);
                            Exit::Success(PreparedContentFragment {
                                html: BufferView::new(output.share()), block, cache_key, no_cache,
                            })
                        }
                        Exit::Fail(error) => Exit::Fail(error),
                        Exit::Die(defect) => Exit::Die(defect),
                        Exit::Interrupted => Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
                    }
                }
            },
        }
    }
}

fn check_shortcode(
    token: &ShortcodeToken,
    options: &ContentRenderOptions,
    limits: ShortcodeRenderLimits,
) -> Result<(), MemoryError> {
    check_limit(token.args.len(), limits.arguments)?;
    let metadata = [
        token.raw.as_str(),
        token.name.as_str(),
        options.surface.as_str(),
        options.lang.as_str(),
        options.viewer_role.as_str(),
        options.viewer_user_id.as_str(),
        options.target_user_id.as_str(),
    ];
    let bytes = metadata
        .into_iter()
        .chain(
            token
                .args
                .iter()
                .flat_map(|(key, value)| [key.as_str(), value.as_str()]),
        )
        .try_fold(0usize, |total, text| total.checked_add(text.len()))
        .ok_or(MemoryError::ArithmeticOverflow)?;
    check_limit(bytes, limits.input_bytes)
}

fn check_limit(observed: usize, limit: NonZeroUsize) -> Result<(), MemoryError> {
    if observed > limit.get() {
        return Err(MemoryError::ValueTooLarge {
            requested: Bytes::new(observed),
            capacity: Bytes::new(limit.get()),
        });
    }
    Ok(())
}

/// A resolver of one shortcode name.
///
/// `E` and `R` are the caller's effect error and environment; a resolver holds no
/// capability of its own beyond the consumer-supplied callbacks it was built with.
pub trait ShortcodeResolver<E, R>: Send + Sync
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The shortcode name this resolver handles.
    fn name(&self) -> String;

    /// Resolve one token into a rendered fragment.
    fn resolve(
        &self,
        token: ShortcodeToken,
        options: ContentRenderOptions,
    ) -> Effect<RenderedContentFragment, E, R>;
}

/// A contribution of one or more shortcode resolvers.
pub trait ShortcodeCatalog<E, R>: Send + Sync
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The resolvers this catalog contributes, in their resolution order.
    fn resolvers(&self) -> Vec<Arc<dyn ShortcodeResolver<E, R>>>;
}

/// The first non-blank trimmed argument among `keys`.
///
/// A shortcode argument: keys are tried in the caller's order and
/// the first non-blank value wins.
#[must_use]
pub fn shortcode_arg(token: &ShortcodeToken, keys: &[&str]) -> String {
    shortcode_arg_ref(token, keys.iter().copied()).to_owned()
}

pub(super) fn shortcode_arg_ref<'token, 'key>(
    token: &'token ShortcodeToken,
    keys: impl IntoIterator<Item = &'key str>,
) -> &'token str {
    for key in keys {
        if let Some(value) = token.args.get(key) {
            let value = value.trim();
            if !value.is_empty() {
                return value;
            }
        }
    }
    ""
}
