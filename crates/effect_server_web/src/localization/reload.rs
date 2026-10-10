//! Serialized complete refresh and request-local immutable catalogue selection.

use super::{
    BoundMessage, Catalog, CatalogDefinition, CultureId, CultureSet, LocalizationError,
    MessageContract, MissingMessage,
};
use effect::{Effect, Exit, HasClock, HasTaskSpawner, Scope, SynchronizedRef};
use effect_server::Request;
use std::collections::BTreeMap;
use std::marker::PhantomData;
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
/// Atomic generation and status snapshot; readers always retain the same owner.
struct State<C: Send + Sync + 'static> {
    /// Complete prepared catalogue, shared without cloning templates.
    catalog: Arc<Catalog<C>>,
    /// Safe protected refresh outcome.
    status: CatalogStatus,
}
/// Inert localization description. Source/culture callbacks execute only within
/// the application's selected R and the host-owned Effect scope.
#[must_use]
pub struct Localization<C: Send + Sync + 'static, R: Send + Sync + 'static> {
    /// Prepared family/resources/argument contracts.
    pub(crate) catalogue: CatalogDefinition<C, R>,
    /// Explicit culture/parent/default precedence.
    pub(crate) cultures: CultureSet,
    /// Native selection callback before the request environment is projected.
    pub(crate) select_by:
        Arc<dyn Fn(Request) -> Effect<CatalogSelection, LocalizationError, R> + Send + Sync>,
    /// Deliberately selected absent-message behavior.
    pub(crate) missing: MissingMessage,
    /// Positive elapsed refresh interval; None means explicit/manual refresh only.
    pub(crate) reload: Option<Duration>,
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
        })
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
        self.catalogue
            .load()
            .map_error(move |error| initial_inject(error))
            .flat_map(move |catalog| {
                let runtime = CatalogRuntime {
                    definition: definition.clone(),
                    state: SynchronizedRef::new(State {
                        catalog: Arc::new(catalog),
                        status: CatalogStatus {
                            generation: 0,
                            failure: None,
                        },
                    }),
                };
                match definition.reload {
                    None => Effect::from_result_once(Ok(runtime)),
                    Some(interval) => {
                        let worker_runtime = runtime.clone();
                        let worker_inject = inject.clone();
                        let worker =
                            Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
                                let runtime = worker_runtime.clone();
                                let inject = worker_inject.clone();
                                async move {
                                    loop {
                                        match context
                                            .run(Effect::<(), E, R>::sleep(interval), env.clone())
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
                                            context.run(runtime.reload(), env.clone()).await;
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
                            });
                        scope.fork(worker).map(move |_| runtime.clone())
                    }
                }
            })
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
        let source = self.definition.catalogue.clone();
        self.state
            .modify_effect(
                move |old| {
                    let old_for_load = old.clone();
                    source
                        .load()
                        .materialize_exit()
                        .map_error(|never| match never {})
                        .map(move |outcome| {
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
                                                    catalog: Arc::new(catalog),
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
                        })
                },
                |_| LocalizationError::Admission,
            )
            .flat_map(crate::restore_exit)
    }
    /// Read protected refresh status without exposing or cloning source/catalogue payloads.
    pub fn status(&self) -> Effect<CatalogStatus, LocalizationError, R> {
        self.state.read().map(|state| state.status.clone())
    }
    /// Select native cultures and pin one exact immutable generation before the
    /// application's request environment projection. The extension stores a handle,
    /// never request-scoped services inside the reload worker.
    pub fn for_request(&self, request: Request) -> Effect<PinnedCatalog<C>, LocalizationError, R> {
        let state = self.state.clone();
        let cultures = self.definition.cultures.clone();
        let missing = self.definition.missing;
        (self.definition.select_by)(request).flat_map(move |selection| {
            let selection = Arc::new(selection);
            let cultures = cultures.clone();
            state.read().flat_map(move |state| {
                if !cultures.contains(&selection.ui) || !cultures.contains(&selection.formatting) {
                    return Effect::from_result_once(Err(LocalizationError::Definition(
                        "selected cultures".into(),
                    )));
                }
                let precedence = match cultures.precedence(&selection.ui) {
                    Ok(precedence) => precedence,
                    Err(error) => return Effect::from_result_once(Err(error)),
                };
                Effect::from_result_once(Ok(PinnedCatalog {
                    catalog: state.catalog.clone(),
                    generation: state.status.generation,
                    selection: selection.clone(),
                    precedence: precedence.into(),
                    missing,
                    marker: PhantomData,
                }))
            })
        })
    }
}
/// Request-local exact generation and language selection. Clone shares immutable
/// catalogue ownership and cannot silently reload midway through rendering.
#[must_use]
pub struct PinnedCatalog<C: Send + Sync + 'static> {
    /// Complete immutable generation retained until request/body cleanup ends.
    catalog: Arc<Catalog<C>>,
    /// Content generation selected for this request.
    pub generation: u64,
    /// UI/formatting/vary metadata selected before request projection.
    pub selection: Arc<CatalogSelection>,
    /// Prepared exact/parent/default lookup order.
    precedence: Arc<[CultureId]>,
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
            precedence: self.precedence.clone(),
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
        if self.catalog.identity.as_ref() != M::CATALOGUE {
            return Err(LocalizationError::Definition("message catalogue".into()));
        }
        for culture in self.precedence.iter() {
            if let Some(template) = self
                .catalog
                .entries
                .get(&(culture.clone(), M::KEY.to_owned()))
            {
                return template.render(
                    &message.arguments,
                    &self.selection.formatting,
                    self.catalog.limits.message_bytes.get(),
                );
            }
        }
        match self.missing {
            MissingMessage::Fail => Err(LocalizationError::Missing(M::KEY.into())),
            MissingMessage::Key => {
                if M::KEY.len() > self.catalog.limits.message_bytes.get() {
                    return Err(LocalizationError::Limit("formatted message bytes".into()));
                }
                Ok(M::KEY.into())
            }
        }
    }
    /// Enumerate the prepared merged view using the same lookup precedence. Templates
    /// remain validated metadata and do not interpolate unknown application values.
    #[must_use]
    pub fn enumerate(&self) -> BTreeMap<String, super::Template> {
        let mut output = BTreeMap::new();
        for culture in self.precedence.iter().rev() {
            for ((entry_culture, key), template) in &self.catalog.entries {
                if entry_culture == culture {
                    output.insert(key.clone(), template.clone());
                }
            }
        }
        output
    }
    /// Stage selected content-language and all declared variation fields on outer
    /// replies/short circuits, using native validated HeaderValue construction.
    pub fn response_headers(&self, headers: &mut http::HeaderMap) -> Result<(), LocalizationError> {
        let value = http::HeaderValue::from_str(self.selection.ui.as_str())
            .map_err(|_| LocalizationError::Definition("content language".into()))?;
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
