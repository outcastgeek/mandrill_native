//! Serialized complete refresh and request-local immutable catalogue selection.

use super::catalog::{CultureMetadata, PreparedCatalogData};
use super::{
    BoundMessage, Catalog, CatalogDefinition, CatalogLimits, CatalogResource, CultureId,
    CultureSet, LocalizationError, MessageContract, MissingMessage,
};
use crate::{effect_future, effect_map, effect_map_error, effect_ref};
use effect::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use effect::{Effect, Exit, HasClock, HasTaskSpawner, Scope, SynchronizedRef, effect};
use effect_server::Request;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

/// Native culture selection result, installed before application request projection.
#[derive(Clone)]
#[must_use]
pub struct CatalogSelection {
    /// UI message lookup culture.
    pub ui: CultureId,
    /// Separate selected culture supplied to typed argument formatters.
    pub formatting: CultureId,
    /// Headers which influenced language selection, used for response variation.
    pub vary: Arc<[http::HeaderName]>,
}
/// Protected reload status. Failure values contain category/identity, not source
/// documents, provider exception strings or user message arguments.
#[derive(Clone)]
#[must_use]
pub struct CatalogStatus {
    /// Logical successful content generation; unchanged resources retain this value.
    pub generation: u64,
    /// Last refresh's safe expected failure category, if one occurred.
    pub failure: Option<LocalizationError>,
}

/// Region-owned generation records retained by the runtime and every pinned reader.
#[must_use]
pub struct PreparedCatalogGenerations<C: Send + Sync + 'static> {
    slots: Arc<SlotPool<'static, Option<Catalog<C>>>>,
    data: Arc<SlotPool<'static, PreparedCatalogData>>,
    files: Arc<SlotPool<'static, Buffer<'static>>>,
    resources: NonZeroUsize,
}
impl<C: Send + Sync + 'static> PreparedCatalogGenerations<C> {
    /// Size actual generation records, template bytes/indexes and bounded file-read scratch.
    pub fn storage_bytes(
        generations: NonZeroUsize,
        limits: CatalogLimits,
        resources: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        let (entries, templates, source) = Self::sizes(limits, resources)?;
        SlotPool::<Option<Catalog<C>>>::storage_bytes(generations.get())?
            .checked_add(SlotPool::<PreparedCatalogData>::storage_bytes(
                generations.get(),
            )?)?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                generations.get(),
            )?)?
            .checked_add(
                PreparedCatalogData::storage_bytes(entries, templates)?
                    .checked_add(source)?
                    .checked_mul(generations.get())?,
            )
    }

    /// Provision the fixed number of simultaneously live generation owners.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        generations: NonZeroUsize,
        limits: CatalogLimits,
        resources: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        let (entries, templates, source) = Self::sizes(limits, resources)?;
        Ok(Self {
            slots: Arc::new(memory.slots(region, generations.get(), |_| Ok(None))?),
            data: Arc::new(memory.slots(region, generations.get(), |_| {
                PreparedCatalogData::prepare(memory, region, entries, templates)
            })?),
            files: Arc::new(memory.buffers(region, generations.get(), source)?),
            resources,
        })
    }

    fn sizes(
        limits: CatalogLimits,
        resources: NonZeroUsize,
    ) -> Result<(usize, Bytes, Bytes), MemoryError> {
        let entries = limits
            .keys
            .get()
            .checked_mul(resources.get())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let source = Bytes::new(limits.bytes.get());
        Ok((entries, source.checked_mul(resources.get())?, source))
    }

    /// Build a file resource whose bytes are read into the selected prepared scratch pool.
    pub fn file<R: effect::HasBlockingExecutor + Send + Sync + 'static>(
        &self,
        identity: impl Into<Arc<str>>,
        culture: CultureId,
        path: std::path::PathBuf,
        maximum: NonZeroUsize,
    ) -> CatalogResource<R> {
        CatalogResource::prepared_file(identity, culture, path, maximum, self.files.clone())
    }
}
impl<C: Send + Sync + 'static> Clone for PreparedCatalogGenerations<C> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            data: self.data.clone(),
            files: self.files.clone(),
            resources: self.resources,
        }
    }
}

type GenerationSlot<C> = SharedSlot<'static, Option<Catalog<C>>>;
enum Generation<C: Send + Sync + 'static> {
    Ordinary(Arc<Catalog<C>>),
    Prepared(Option<GenerationSlot<C>>),
}
impl<C: Send + Sync + 'static> Clone for Generation<C> {
    #[allow(clippy::expect_used)] // Published owners retain their slot and share its checked counter.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(catalog) => Self::Ordinary(catalog.clone()),
            Self::Prepared(slot) => Self::Prepared(Some(
                slot.as_ref()
                    .expect("live generation owner")
                    .try_clone()
                    .expect("generation owner limit"),
            )),
        }
    }
}
impl<C: Send + Sync + 'static> Deref for Generation<C> {
    type Target = Catalog<C>;
    #[allow(clippy::expect_used)] // A prepared owner is published only after initializing the catalogue.
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(catalog) => catalog,
            Self::Prepared(slot) => slot
                .as_ref()
                .expect("live generation owner")
                .as_ref()
                .expect("published catalogue"),
        }
    }
}
impl<C: Send + Sync + 'static> Drop for Generation<C> {
    fn drop(&mut self) {
        if let Self::Prepared(slot) = self
            && let Some(mut slot) = slot.take().and_then(SharedSlot::into_last)
        {
            drop(slot.take());
        }
    }
}

/// Atomic generation and status snapshot; readers always retain the same owner.
struct State<C: Send + Sync + 'static> {
    /// Complete prepared catalogue, shared without cloning templates.
    catalog: Generation<C>,
    /// Safe protected refresh outcome.
    status: CatalogStatus,
}

type SelectCulture<R> =
    dyn Fn(Request) -> Effect<Arc<CatalogSelection>, LocalizationError, R> + Send + Sync;

/// Inert localization description. Source/culture callbacks execute only within
/// the application's selected R and the host-owned Effect scope.
#[must_use]
pub struct Localization<C: Send + Sync + 'static, R: Send + Sync + 'static> {
    /// Prepared family/resources/argument contracts.
    pub(crate) catalogue: CatalogDefinition<C, R>,
    /// Explicit culture/parent/default precedence.
    pub(crate) cultures: CultureSet,
    /// Native selection callback before the request environment is projected.
    pub(crate) select_by: Arc<SelectCulture<R>>,
    /// Deliberately selected absent-message behavior.
    pub(crate) missing: MissingMessage,
    /// Positive elapsed refresh interval; None means explicit/manual refresh only.
    pub(crate) reload: Option<Duration>,
    generations: Option<PreparedCatalogGenerations<C>>,
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> Clone for Localization<C, R> {
    /// Share inert metadata/callbacks only; loaded state belongs to a scoped runtime.
    fn clone(&self) -> Self {
        Self {
            catalogue: self.catalogue.clone(),
            cultures: self.cultures.clone(),
            select_by: self.select_by.clone(),
            missing: self.missing,
            reload: self.reload,
            generations: self.generations.clone(),
        }
    }
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> Localization<C, R> {
    /// Validate positive refresh duration and retain the typed selected callbacks.
    pub fn new(
        catalogue: CatalogDefinition<C, R>,
        cultures: CultureSet,
        select_by: impl Fn(Request) -> Effect<CatalogSelection, LocalizationError, R>
        + Send
        + Sync
        + 'static,
        missing: MissingMessage,
        reload: Option<Duration>,
    ) -> Result<Self, LocalizationError> {
        Self::with_selection(
            catalogue,
            cultures,
            move |request| {
                effect_map! { program: select_by(request), map: Arc::new, }
            },
            missing,
            reload,
        )
    }

    /// Select shared language choices and admit a physical generation before loading sources.
    pub fn new_prepared(
        catalogue: CatalogDefinition<C, R>,
        cultures: CultureSet,
        select_by: impl Fn(Request) -> Effect<Arc<CatalogSelection>, LocalizationError, R>
        + Send
        + Sync
        + 'static,
        missing: MissingMessage,
        reload: Option<Duration>,
        generations: PreparedCatalogGenerations<C>,
    ) -> Result<Self, LocalizationError> {
        Ok(
            Self::with_selection(catalogue, cultures, select_by, missing, reload)?
                .with_prepared_generations(generations),
        )
    }

    fn with_selection(
        catalogue: CatalogDefinition<C, R>,
        cultures: CultureSet,
        select_by: impl Fn(Request) -> Effect<Arc<CatalogSelection>, LocalizationError, R>
        + Send
        + Sync
        + 'static,
        missing: MissingMessage,
        reload: Option<Duration>,
    ) -> Result<Self, LocalizationError> {
        if reload.is_some_and(|duration| duration.is_zero()) {
            return Err(LocalizationError::Definition("reload interval".into()));
        }
        catalogue.validate_cultures(&cultures)?;
        Ok(Self {
            catalogue,
            cultures,
            select_by: Arc::new(select_by),
            missing,
            reload,
            generations: None,
        })
    }

    /// Use the prepared partition for current, refreshing and request-pinned generations.
    pub fn with_prepared_generations(mut self, generations: PreparedCatalogGenerations<C>) -> Self {
        self.generations = Some(generations);
        self
    }

    fn load_generation(&self) -> Effect<Generation<C>, LocalizationError, R> {
        let definition = self.clone();
        effect_future! {
            exit_with_context;
            body: move |env: Arc<R>, context| {
                let definition = definition.clone();
                async move {
                    let slot = match &definition.generations {
                        Some(generations) if definition.catalogue.resource_count() > generations.resources.get() => {
                            return Exit::Fail(LocalizationError::Limit("catalogue resources".into()));
                        }
                        Some(generations) => match generations.slots.try_checkout() {
                            Ok(slot) => Some(slot),
                            Err(error) => return Exit::Fail(LocalizationError::Memory(error)),
                        },
                        None => None,
                    };
                    let data = definition.generations.as_ref().map(|generations| generations.data.clone());
                    let catalog = match effect::effect! {
                        run_in_context;
                        context: context,
                        program: definition.catalogue.load_with(data),
                        environment: env,
                    }.await.into_failure() {
                        Ok(catalog) => catalog,
                        Err(failure) => return failure,
                    };
                    Exit::Success(match slot {
                        Some(mut slot) => {
                            *slot = Some(catalog);
                            Generation::Prepared(Some(slot.share()))
                        }
                        None => Generation::Ordinary(Arc::new(catalog)),
                    })
                }
            },
        }
    }
    /// Acquire one validated generation before serving, then optionally fork one
    /// serialized refresh worker into the supplied host scope. Partial startup is
    /// owned immediately; refresh/provider cleanup remains in the caller's E channel.
    pub fn scoped<E: Send + 'static>(
        &self,
        scope: Scope<E>,
        inject: impl Fn(LocalizationError) -> E + Send + Sync + 'static,
    ) -> Effect<CatalogRuntime<C, R>, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let definition = self.clone();
        let inject = Arc::new(inject);
        let initial_inject = inject.clone();
        effect! {
        bind;
        program: effect_map_error! {
            program: self.load_generation(), map: move |error| initial_inject(error), },
        body: move |catalog| {
                       let runtime = CatalogRuntime {
                           definition: definition.clone(),
                           state: SynchronizedRef::new(State {
                               catalog,
                               status: CatalogStatus {
                                   generation: 0,
                                   failure: None,
                               },
                           }),
                       };
                       match definition.reload {
                           None => effect::effect_from_result! { once; result: Ok(runtime), },
                           Some(interval) => {
                               let worker_runtime = runtime.clone();
                               let worker_inject = inject.clone();
                               let worker =
                                   effect_future! {
                                       exit_with_context; body: move |env: Arc<R>, context| {
                                       let runtime = worker_runtime.clone();
                                       let inject = worker_inject.clone();
                                       async move {
                                           loop {
                                               match effect::effect! {
                                                   run_in_context;
                                                   context: context,
                                                   program: effect! { sleep; duration: interval, },
                                                   environment: env.clone(),
                                               }
                                                   .await
                                               {
                                                   Exit::Success(()) => {}
                                                   Exit::Fail(error) => return Exit::Fail(error),
                                                   Exit::Die(defect) => return Exit::Die(defect),
                                                   Exit::Interrupted => return Exit::Interrupted,
                                                   Exit::FinalizationFailed(failure) => {
                                                       return Exit::FinalizationFailed(failure);
                                                   }
                                               }
                                               let result =
                                                   effect::effect! {
                                                       run_in_context;
                                                       context: context,
                                                       program: runtime.reload(),
                                                       environment: env.clone(),
                                                   }.await;
                                               match result {
                                                   Exit::Success(()) | Exit::Fail(_) => {}
                                                   Exit::Die(defect) => return Exit::Die(defect),
                                                   Exit::Interrupted => return Exit::Interrupted,
                                                   Exit::FinalizationFailed(failure) => {
                                                       return Exit::<(), _>::FinalizationFailed(failure)
                                                           .map_error(|error| inject(error));
                                                   }
                                               }
                                           }
                                       }
                                   }, };
                               effect_map! { program: effect::effect_scope! { fork; scope: scope, program: worker, }, map: move |_| runtime.clone(), }
                           }
                       }
                   },
        }
    }
}
/// Host-owned catalogue runtime. The core synchronized reference serializes
/// refreshes; current readers do not block on I/O and cannot see partial contents.
#[must_use]
pub struct CatalogRuntime<C: Send + Sync + 'static, R: Send + Sync + 'static> {
    /// Original typed policy/source/culture callbacks.
    definition: Localization<C, R>,
    /// Atomic complete generation and last refresh status.
    state: SynchronizedRef<State<C>>,
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> Clone for CatalogRuntime<C, R> {
    /// Share one refresh/generation owner rather than starting another worker.
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            state: self.state.clone(),
        }
    }
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> CatalogRuntime<C, R> {
    /// Serialize a complete load before atomic replacement. Expected failures
    /// update protected status while retaining last good content/generation.
    /// Defect/interruption/cleanup remain their original Effect outcomes.
    pub fn reload(&self) -> Effect<(), LocalizationError, R> {
        let source = self.definition.clone();
        effect! {
        bind;
        program: effect_ref! {
                       synchronized;
                       reference: self.state,
                       modify: move |old| {
                           let old_for_load = old.clone();
                           effect_map! {
                               program: effect_map_error! {
                               program: source
                               .load_generation()
                               .materialize_exit(), map: |never| match never {}, }, map: move |outcome| {
                                   let old = old_for_load.clone();
                                   match outcome {
                                       Exit::Success(catalog) => {
                                           if old.catalog.same(&catalog) {
                                               (
                                                   Exit::Success(()),
                                                   State {
                                                       catalog: old.catalog.clone(),
                                                       status: CatalogStatus {
                                                           generation: old.status.generation,
                                                           failure: None,
                                                       },
                                                   },
                                               )
                                           } else {
                                               match old.status.generation.checked_add(1) {
                                                   Some(generation) => (
                                                       Exit::Success(()),
                                                       State {
                                                           catalog,
                                                           status: CatalogStatus {
                                                               generation,
                                                               failure: None,
                                                           },
                                                       },
                                                   ),
                                                   None => (
                                                       Exit::Fail(LocalizationError::GenerationOverflow),
                                                       State {
                                                           catalog: old.catalog.clone(),
                                                           status: CatalogStatus {
                                                               generation: old.status.generation,
                                                               failure: Some(
                                                                   LocalizationError::GenerationOverflow,
                                                               ),
                                                           },
                                                       },
                                                   ),
                                               }
                                           }
                                       }
                                       Exit::Fail(error) => {
                                           let status_error = error.clone();
                                           (
                                               Exit::Fail(error),
                                               State {
                                                   catalog: old.catalog.clone(),
                                                   status: CatalogStatus {
                                                       generation: old.status.generation,
                                                       failure: Some(status_error),
                                                   },
                                               },
                                           )
                                       }
                                       Exit::Die(defect) => (
                                           Exit::Die(defect),
                                           State {
                                               catalog: old.catalog.clone(),
                                               status: old.status.clone(),
                                           },
                                       ),
                                       Exit::Interrupted => (
                                           Exit::Interrupted,
                                           State {
                                               catalog: old.catalog.clone(),
                                               status: old.status.clone(),
                                           },
                                       ),
                                       Exit::FinalizationFailed(failure) => (
                                           Exit::FinalizationFailed(failure),
                                           State {
                                               catalog: old.catalog.clone(),
                                               status: old.status.clone(),
                                           },
                                       ),
                                   }
                               }, }
                       },
                       on_error: |_| LocalizationError::Admission,
                   },
        body: crate::restore_exit,
        }
    }
    /// Read protected refresh status without exposing or cloning source/catalogue payloads.
    pub fn status(&self) -> Effect<CatalogStatus, LocalizationError, R> {
        effect_map! { program: effect_ref! { synchronized_read; reference: self.state, }, map: |state| state.status.clone(), }
    }
    /// Select native cultures and pin one exact immutable generation before the
    /// application's request environment projection. The extension stores a handle,
    /// never request-scoped services inside the reload worker.
    pub fn for_request(&self, request: Request) -> Effect<PinnedCatalog<C>, LocalizationError, R> {
        let state = self.state.clone();
        let cultures = self.definition.cultures.clone();
        let missing = self.definition.missing;
        let selected = (self.definition.select_by)(request);
        effect! {
            let state = state.clone();
            let cultures = cultures.clone();
            let selection = selected.clone()?;
            let state = effect_ref! { synchronized_read; reference: state, }?;
            let culture = effect::effect_from_result! {
                once;
                result: if !cultures.contains(&selection.ui)
                        || !cultures.contains(&selection.formatting) {
                        Err(LocalizationError::Definition("selected cultures".into()))
                    } else { cultures.metadata(&selection.ui) },
            }?;
            PinnedCatalog {
                catalog: state.catalog.clone(), generation: state.status.generation,
                selection, culture, missing, marker: PhantomData,
            }
        }
    }
}
/// Request-local exact generation and language selection. Clone shares immutable
/// catalogue ownership and cannot silently reload midway through rendering.
#[must_use]
pub struct PinnedCatalog<C: Send + Sync + 'static> {
    /// Complete immutable generation retained until request/body cleanup ends.
    catalog: Generation<C>,
    /// Content generation selected for this request.
    pub generation: u64,
    /// UI/formatting/vary metadata selected before request projection.
    pub selection: Arc<CatalogSelection>,
    /// Prepared exact/parent/default lookup order.
    culture: CultureMetadata,
    /// Explicit selected missing policy.
    missing: MissingMessage,
    /// Compile-time catalogue marker, without a runtime instance.
    marker: PhantomData<fn() -> C>,
}
impl<C: Send + Sync + 'static> Clone for PinnedCatalog<C> {
    /// Share one generation without copying catalogue/application payloads.
    fn clone(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            generation: self.generation,
            selection: self.selection.clone(),
            culture: self.culture.clone(),
            missing: self.missing,
            marker: PhantomData,
        }
    }
}
impl<C: Send + Sync + 'static> PinnedCatalog<C> {
    /// Format only a message from this catalogue family, with its typed argument
    /// contract. Exact/parent/default precedence matches enumeration/native access.
    pub fn text<M: MessageContract<Catalogue = C>>(
        &self,
        message: &BoundMessage<M>,
    ) -> Result<String, LocalizationError> {
        let mut output = String::new();
        self.write_text(message, |text| {
            output.push_str(text);
            Ok(())
        })?;
        Ok(output)
    }

    /// Append a typed message to fixed backing, restoring its prefix on refusal or unwind.
    pub fn text_into<M: MessageContract<Catalogue = C>>(
        &self,
        message: &BoundMessage<M>,
        output: &mut Buffer<'_>,
    ) -> Result<(), LocalizationError> {
        let start = output.len();
        let mut output = std::mem::DropGuard::new(output, move |output| output.truncate(start));
        self.write_text(message, |text| {
            output
                .extend_from_slice(text.as_bytes())
                .map_err(LocalizationError::Memory)
        })?;
        let _ = std::mem::DropGuard::dismiss(output);
        Ok(())
    }

    pub(crate) fn write_text<M: MessageContract<Catalogue = C>>(
        &self,
        message: &BoundMessage<M>,
        mut append: impl FnMut(&str) -> Result<(), LocalizationError>,
    ) -> Result<(), LocalizationError> {
        if self.catalog.identity.as_ref() != M::CATALOGUE {
            return Err(LocalizationError::Definition("message catalogue".into()));
        }
        for culture in self.culture.precedence.iter() {
            if self.catalog.write(
                culture,
                M::KEY,
                &message.arguments,
                &self.selection.formatting,
                &mut append,
            )? {
                return Ok(());
            }
        }
        match self.missing {
            MissingMessage::Fail => Err(LocalizationError::Missing(M::KEY.into())),
            MissingMessage::Key => {
                if M::KEY.len() > self.catalog.limits.message_bytes.get() {
                    return Err(LocalizationError::Limit("formatted message bytes".into()));
                }
                append(M::KEY)
            }
        }
    }
    /// Enumerate the prepared merged view using the same lookup precedence. Templates
    /// remain validated metadata and do not interpolate unknown application values.
    #[must_use]
    pub fn enumerate(&self) -> BTreeMap<String, super::Template> {
        let mut output = BTreeMap::new();
        for culture in self.culture.precedence.iter().rev() {
            self.catalog.enumerate(culture, &mut output);
        }
        output
    }
    /// Stage selected content-language and all declared variation fields on outer
    /// replies/short circuits, using native validated HeaderValue construction.
    pub fn response_headers(&self, headers: &mut http::HeaderMap) -> Result<(), LocalizationError> {
        let selected = self.selection.ui.as_str();
        let value = if self.culture.content_language.as_bytes() == selected.as_bytes() {
            self.culture.content_language.clone()
        } else {
            http::HeaderValue::from_str(selected)
                .map_err(|_| LocalizationError::Definition("content language".into()))?
        };
        headers.insert(http::header::CONTENT_LANGUAGE, value);
        for name in self.selection.vary.iter() {
            headers.append(
                http::header::VARY,
                http::HeaderValue::from_str(name.as_str())
                    .map_err(|_| LocalizationError::Definition("language vary".into()))?,
            );
        }
        Ok(())
    }
}

/// Typed root nesting conversion preserving checked localization configuration.
pub trait IntoLocalization<C: Send + Sync + 'static, R: Send + Sync + 'static> {
    /// Retain resource/culture/reload validation errors until host startup.
    fn into_localization(self) -> Result<Localization<C, R>, LocalizationError>;
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> IntoLocalization<C, R>
    for Localization<C, R>
{
    /// Already checked descriptions remain inert.
    fn into_localization(self) -> Result<Localization<C, R>, LocalizationError> {
        Ok(self)
    }
}
impl<C: Send + Sync + 'static, R: Send + Sync + 'static> IntoLocalization<C, R>
    for Result<Localization<C, R>, LocalizationError>
{
    /// Nested localization macro failures remain typed.
    fn into_localization(self) -> Result<Localization<C, R>, LocalizationError> {
        self
    }
}
