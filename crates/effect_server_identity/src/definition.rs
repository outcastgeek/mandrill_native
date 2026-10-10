//! Inert Identity definitions and whole-operation lazy native lifts.

use crate::{
    IdentityError,
    authority::{
        GrantPolicy, IdentityAuthority, IdentityClock, PendingLoginGrant, RecentSelfGrant,
        SelfGrant,
    },
    capabilities::{NativeBackend, StoreSelection},
};
use effect::{Effect, Exit};
use std::{
    future::Future,
    marker::PhantomData,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

/// Typed selected native factory. It receives the real invocation-scoped backend,
/// explicit native-compatible clock and retained policy generation. Factories
/// preserve selected validators/crypto/session/principal/transport components;
/// they must not reacquire defaults or perform I/O during pure construction.
pub trait IdentityComponents<B: NativeBackend, P>: Send + Sync + 'static {
    /// Actual owned native user/sign-in/role composition used by the workflow.
    type Native: Send + Sync + 'static;
    /// Bind only the invocation's actual store into the selected native services.
    fn construct(
        &self,
        backend: B,
        clock: IdentityClock,
        policies: Arc<P>,
    ) -> Result<Self::Native, IdentityError>;
}
/// Retained application/native configuration and its typed pure composition factory.
pub struct ConfiguredComponents<C, F, N> {
    /// Selected configuration retained once without a blanket Clone requirement.
    configuration: Arc<C>,
    /// Actual factory, called lazily for the invocation-owned store replacement.
    factory: F,
    /// Native result identity without owning/cloning any native request payload.
    native: PhantomData<fn() -> N>,
}
impl<C, F, N> ConfiguredComponents<C, F, N> {
    /// Retain selected components and an extracted pure native factory without I/O.
    pub fn new(configuration: C, factory: F) -> Self {
        Self {
            configuration: Arc::new(configuration),
            factory,
            native: PhantomData,
        }
    }
}
impl<B: NativeBackend, P, C: Send + Sync + 'static, F, N: Send + Sync + 'static>
    IdentityComponents<B, P> for ConfiguredComponents<C, F, N>
where
    F: Fn(Arc<C>, B, IdentityClock, Arc<P>) -> Result<N, IdentityError> + Send + Sync + 'static,
{
    type Native = N;
    /// Supply retained component/policy handles and the exact invocation backend.
    fn construct(
        &self,
        backend: B,
        clock: IdentityClock,
        policies: Arc<P>,
    ) -> Result<N, IdentityError> {
        (self.factory)(self.configuration.clone(), backend, clock, policies)
    }
}
/// Inert typed definition retaining native selections and application error mapping.
/// Clone copies configuration handles only; request/credential inputs move per invocation.
pub struct IdentityDefinition<S: StoreSelection, C, P, M> {
    /// D17 storage/capability generation, selected once.
    storage: Arc<S>,
    /// Native manager/component composition selected once.
    components: Arc<C>,
    /// Typed optional policy generation available to every native factory/workflow.
    policies: Arc<P>,
    /// Application-owned typed Identity error mapping.
    errors: Arc<M>,
    /// Private authority retaining one definition generation and explicit freshness.
    authority: IdentityAuthority<S::Backend>,
    /// Explicit shared whole-workflow admission; native manager work limits remain selected separately.
    admission: Arc<tokio::sync::Semaphore>,
}
impl<S: StoreSelection, C, P, M> Clone for IdentityDefinition<S, C, P, M> {
    /// Retain installed service handles without cloning storage/request/credential values.
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            components: self.components.clone(),
            policies: self.policies.clone(),
            errors: self.errors.clone(),
            authority: self.authority.clone(),
            admission: self.admission.clone(),
        }
    }
}
impl<
    S: StoreSelection,
    C: Send + Sync + 'static,
    P: Send + Sync + 'static,
    M: Send + Sync + 'static,
> IdentityDefinition<S, C, P, M>
{
    /// Select all definition services without acquiring SQL, issuing tickets, reading
    /// the clock or invoking provider callbacks. Actual operations remain lazy.
    pub fn new(
        storage: S,
        components: C,
        policies: P,
        errors: M,
        clock: IdentityClock,
        grants: GrantPolicy,
        admission: NonZeroUsize,
    ) -> Result<Self, IdentityError> {
        if admission.get() > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(IdentityError::Configuration("identity admission bound"));
        }
        let authority = IdentityAuthority::new(storage.backend().clone(), clock, grants)?;
        Ok(Self {
            storage: Arc::new(storage),
            components: Arc::new(components),
            policies: Arc::new(policies),
            errors: Arc::new(errors),
            authority,
            admission: Arc::new(tokio::sync::Semaphore::new(admission.get())),
        })
    }
    /// Borrow the selected optional capabilities; missing results stay typed.
    pub fn storage(&self) -> &S {
        &self.storage
    }
    /// Retain definition-owned selected service handles for one lazy capability workflow.
    pub(crate) fn retained_storage(&self) -> Arc<S> {
        self.storage.clone()
    }
    /// Borrow exactly the selected policy/component generation.
    pub fn policies(&self) -> &P {
        &self.policies
    }
    /// Borrow private grant issuance/revalidation over this real backend generation.
    pub fn authority(&self) -> &IdentityAuthority<S::Backend> {
        &self.authority
    }
    /// Lift one whole native async operation lazily with fresh environment-selected
    /// inputs. Every native store call settles its own transaction; no nested runtime
    /// or cross-business transaction is claimed. Defects/interruption/cleanup are restored.
    pub fn invoke<A, E, R, F, Fut>(&self, operation: F) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        C: IdentityComponents<S::Backend, P>,
        M: Fn(IdentityError) -> E,
        F: Fn(C::Native, Arc<R>, IdentityAuthority<S::Backend>, Arc<P>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let definition = self.clone();
        let operation = Arc::new(operation);
        Effect::from_exit_future_with_context(move |env: Arc<R>, _| {
            let definition = definition.clone();
            let operation = operation.clone();
            async move {
                let result = async {
                    let _admission = definition
                        .admission
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| IdentityError::Configuration("identity admission closed"))?;
                    let backend = definition.storage.backend().scoped();
                    let native = definition.components.construct(
                        backend.clone(),
                        definition.authority.clock(),
                        definition.policies.clone(),
                    )?;
                    let result = operation(
                        native,
                        env,
                        definition.authority.with_backend(backend.clone()),
                        definition.policies.clone(),
                    )
                    .await;
                    backend.capture_outcome(result)
                }
                .await;
                match result {
                    Ok(value) => Exit::Success(value),
                    Err(error) => error
                        .into_exit()
                        .map_error(|error| (definition.errors)(error)),
                }
            }
        })
        .uninterruptible()
    }
    /// Move one owned credential/request/grant into its single native execution;
    /// repeated execution yields core's consumed-factory defect instead of copying it.
    pub fn invoke_once<I, A, E, R, F, Fut>(&self, input: I, operation: F) -> Effect<A, E, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        C: IdentityComponents<S::Backend, P>,
        M: Fn(IdentityError) -> E,
        F: Fn(C::Native, I, IdentityAuthority<S::Backend>, Arc<P>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let definition = self.clone();
        let operation = Arc::new(operation);
        let pending = Mutex::new(Some(input));
        Effect::from_exit_future_with_context(move |_: Arc<R>, _| {
            let candidate = pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let definition = definition.clone();
            let operation = operation.clone();
            async move {
                let Some(input) = candidate else {
                    return Exit::Die(effect::Defect::from(
                        effect::LibraryDefect::FutureFactoryConsumed,
                    ));
                };
                let result = async {
                    let _admission = definition
                        .admission
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| IdentityError::Configuration("identity admission closed"))?;
                    let backend = definition.storage.backend().scoped();
                    let native = definition.components.construct(
                        backend.clone(),
                        definition.authority.clock(),
                        definition.policies.clone(),
                    )?;
                    let result = operation(
                        native,
                        input,
                        definition.authority.with_backend(backend.clone()),
                        definition.policies.clone(),
                    )
                    .await;
                    backend.capture_outcome(result)
                }
                .await;
                match result {
                    Ok(value) => Exit::Success(value),
                    Err(error) => error
                        .into_exit()
                        .map_error(|error| (definition.errors)(error)),
                }
            }
        })
        .uninterruptible()
    }
    /// Lift an application/native operation over this exact retained D17 selection.
    /// Typed getters preserve explicit absence and actual configuration ownership;
    /// callbacks execute lazily and compose through the existing Effect runtime.
    pub fn invoke_capability_once<I, A, E, R, F, Fut>(
        &self,
        input: I,
        operation: F,
    ) -> Effect<A, E, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        C: IdentityComponents<S::Backend, P>,
        M: Fn(IdentityError) -> E,
        F: Fn(Arc<S>, C::Native, I, IdentityAuthority<S::Backend>, Arc<P>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let storage = self.storage.clone();
        self.invoke_once(input, move |native, input, authority, policies| {
            operation(storage.clone(), native, input, authority, policies)
        })
    }
    /// Execute a public native HTTP service under one invocation-owned native store
    /// and restore its original narrowed SQL outcome. Native protocols with private
    /// codecs, including external pending metadata, can retain those native handlers
    /// while application callbacks select their actual state/mode/router service.
    ///
    /// Move the single native body; copy only HTTP method/URI/version/header metadata.
    /// Arbitrary ticket/user extensions are not forwarded as authentication evidence.
    /// The callback receives current private authority and selected policy handles;
    /// any host external-evidence resolver remains an explicit trusted application service.
    /// Render the result with Server Reply::native, preserving its original body.
    /// Native handler work must settle before returning; streaming application work
    /// follows Server's existing owned-body Effect integration instead.
    pub fn native_http<E, R, F, Fut>(
        &self,
        request: effect_server::Request,
        operation: F,
    ) -> Effect<http::Response<axum::body::Body>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        C: IdentityComponents<S::Backend, P>,
        M: Fn(IdentityError) -> E,
        F: Fn(
                C::Native,
                http::Request<axum::body::Body>,
                IdentityAuthority<S::Backend>,
                Arc<P>,
            ) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<http::Response<axum::body::Body>, IdentityError>>
            + Send
            + 'static,
    {
        let operation = Arc::new(operation);
        self.invoke_once(request, move |native, request, authority, policies| {
            let operation = operation.clone();
            async move {
                let method = request.method().clone();
                let uri = request.uri().clone();
                let version = request.version();
                let headers = request.headers().clone();
                request
                    .with_native_body(move |body| async move {
                        let mut request = http::Request::new(body);
                        *request.method_mut() = method;
                        *request.uri_mut() = uri;
                        *request.version_mut() = version;
                        *request.headers_mut() = headers;
                        Ok(operation(native, request, authority, policies).await)
                    })
                    .await
                    .map_err(IdentityError::Input)?
            }
        })
    }
    /// Build a trusted request authentication stage over the selected native access
    /// provider. Invalid/missing proof is false; backend/provider errors remain E.
    /// Only this verified private grant is installed, never arbitrary native ticket extensions.
    pub fn bearer_stage<E, R, T>(
        &self,
        provider: T,
    ) -> impl Fn(effect_server::Request) -> Effect<bool, E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        T: identity_rs::identity::tokens::UserTokenProvider<
                identity_rs::identity::models::IdentityUser,
            > + 'static,
    {
        let definition = self.clone();
        let provider = Arc::new(provider);
        move |request: effect_server::Request| {
            let definition = definition.clone();
            let provider = provider.clone();
            Effect::from_exit_future_with_context(move |_: Arc<R>, _| {
                let definition = definition.clone();
                let provider = provider.clone();
                let request = request.clone();
                async move {
                    let result = async {
                        let headers = request.headers();
                        let values = headers.get_all(http::header::AUTHORIZATION);
                        if values.iter().count() > 1 {
                            return Err(IdentityError::Policy(http::StatusCode::BAD_REQUEST));
                        }
                        let Some(header) = values.iter().next() else {
                            return Ok(false);
                        };
                        let text = header.to_str().map_err(|_| IdentityError::Unauthorized)?;
                        let Some((scheme, token)) = text.split_once(' ') else {
                            return Ok(false);
                        };
                        if !scheme.eq_ignore_ascii_case("Bearer") {
                            return Ok(false);
                        }
                        let grant =
                            match definition.authority.bearer(provider.as_ref(), token).await {
                                Ok(Some(grant)) => grant,
                                Ok(None) => return Ok(false),
                                Err(IdentityError::Bearer(
                                    identity_rs::identity::tokens::BearerTokenError::InvalidToken,
                                )) => return Ok(false),
                                Err(error) => return Err(error),
                            };
                        request.insert_extension(grant);
                        Ok(true)
                    }
                    .await;
                    match result {
                        Ok(value) => Exit::Success(value),
                        Err(error) => error
                            .into_exit()
                            .map_error(|error| (definition.errors)(error)),
                    }
                }
            })
            .uninterruptible()
        }
    }
    /// Build the selected native server-backed CSRF stage after trusted authentication.
    /// Safe methods need no mutation proof; unsafe requests install a private request binding.
    pub fn antiforgery_stage<E, R, B>(
        &self,
        browser: B,
    ) -> impl Fn(effect_server::Request) -> Effect<bool, E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        B: identity_rs::identity::axum::browser::BrowserRequestSecurity,
    {
        let definition = self.clone();
        let browser = Arc::new(browser);
        move |request: effect_server::Request| {
            let definition = definition.clone();
            let browser = browser.clone();
            Effect::from_exit_future_with_context(move |_: Arc<R>, _| {
                let definition = definition.clone();
                let browser = browser.clone();
                let request = request.clone();
                async move {
                    if request.method() == http::Method::GET
                        || request.method() == http::Method::HEAD
                        || request.method() == http::Method::OPTIONS
                    {
                        return Exit::Success(true);
                    }
                    let result: Result<bool, IdentityError> = async {
                        let actor = request
                            .with_extensions(|extensions| extensions.get::<SelfGrant>().cloned())
                            .ok_or(IdentityError::Unauthorized)?;
                        let proof = definition
                            .authority
                            .protect_request(&actor, &request, browser.as_ref())
                            .await?;
                        request.insert_extension(proof);
                        Ok(true)
                    }
                    .await;
                    match result {
                        Ok(value) => Exit::Success(value),
                        Err(error) => error
                            .into_exit()
                            .map_error(|error| (definition.errors)(error)),
                    }
                }
            })
            .uninterruptible()
        }
    }
}

/// Owned registration input; password and prepared native credential state are never Debug/Clone.
pub struct RegistrationInput {
    /// Validated application-prepared identity; HTTP callers must not accept trusted confirmation fields.
    pub user: identity_rs::identity::models::IdentityUser,
    /// Fresh secret moved once into native bounded verification/hash work.
    pub password: String,
}
/// Native password sign-in request; no arbitrary assurance/pending proof fields exist.
pub struct LoginInput {
    /// Native selected normalizer resolves this bounded application input.
    pub user_name: String,
    /// Secret moved into this execution only.
    pub password: String,
    /// Explicit selected persistence preference.
    pub persistent: bool,
    /// Explicit selected failed-attempt participation.
    pub lockout_on_failure: bool,
}
/// Native authentication result with privately minted, version-bound workflow authority.
pub enum LoginOutcome {
    /// Native rejected result preserved as the original session value.
    Rejected(Arc<identity_rs::identity::SignInSession>),
    /// Owned primary evidence, never accepted as completed self authority.
    Pending(PendingLoginGrant),
    /// Verified completion, current self authority and tighter sensitive freshness.
    Completed {
        /// Original native completion/ticket result retained without payload cloning.
        session: Arc<identity_rs::identity::SignInSession>,
        /// Version-bound ordinary actor grant.
        actor: SelfGrant,
        /// Fresh verification grant unavailable to refresh-only authentication.
        recent: RecentSelfGrant,
    },
}
/// Convert only the real native sign-in outcome; no ticket-to-proof constructor is used.
pub(crate) async fn complete_login<B: NativeBackend>(
    authority: &IdentityAuthority<B>,
    session: identity_rs::identity::SignInSession,
) -> Result<LoginOutcome, IdentityError> {
    let session = Arc::new(session);
    if let Some(completed) = session.completed() {
        let actor = authority.from_completed(completed).await?;
        let recent = authority.recent_completion(completed).await?;
        return Ok(LoginOutcome::Completed {
            session,
            actor,
            recent,
        });
    }
    if session.pending().is_some() {
        return Ok(LoginOutcome::Pending(authority.pending(session)?));
    }
    Ok(LoginOutcome::Rejected(session))
}
