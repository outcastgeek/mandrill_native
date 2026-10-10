//! Completed reply descriptions, lazy representation selection and finite witnesses.

use crate::{
    BodyCompletion, BodyPolicy, BodyProgram, BodyStatus, ConditionalOutcome, Request,
    ResourceValidators, ServerError,
};
use axum::body::Body;
use bytes::Bytes;
use effect::{Effect, Exit, Scope};
use headers::HeaderMapExt;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Explicit trusted markup; dynamic strings use `HtmlContent::text` escaping.
pub struct HtmlContent(String);
impl HtmlContent {
    /// Escape untrusted text for HTML text/attribute contexts.
    pub fn text(text: impl AsRef<str>) -> Self {
        Self(
            text.as_ref()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;"),
        )
    }
    /// Mark caller-owned already trusted HTML. This is an explicit trust boundary.
    #[must_use]
    pub fn trusted(html: String) -> Self {
        Self(html)
    }
}
/// A validated serialized Set-Cookie field, preserving one field per cookie.
pub struct ReplyCookie(HeaderValue);
impl ReplyCookie {
    /// Validate the native cookie serialization before staging response metadata.
    pub fn new(cookie: cookie::Cookie<'static>) -> Result<Self, ServerError> {
        HeaderValue::from_str(&cookie.to_string())
            .map(Self)
            .map_err(|_| ServerError::Invalid("Set-Cookie".into()))
    }
    /// Construct a removal cookie using native expiry/max-age semantics and caller-selected scope.
    pub fn deletion(mut cookie: cookie::Cookie<'static>) -> Result<Self, ServerError> {
        cookie.make_removal();
        Self::new(cookie)
    }
}
/// A finite-response witness. Opaque/native/stream replies cannot acquire this witness implicitly.
#[derive(Clone)]
pub struct FiniteReply {
    /// Native response status, validated against body-forbidden codes.
    pub status: StatusCode,
    /// Staged native headers; cache selection must inspect authorization/cookie policy.
    pub headers: HeaderMap,
    /// Immutable shared bytes; cloning this witness does not copy the encoded payload.
    pub bytes: Bytes,
}
/// Ordered metadata operations apply to the final selected native response fields.
enum HeaderOperation {
    /// Replace every existing value, including terminal-generated metadata.
    Set(HeaderName, HeaderValue),
    /// Retain repeated fields in source order.
    Append(HeaderName, HeaderValue),
    /// Remove every existing terminal or previously staged field value.
    Remove(HeaderName),
}
/// Apply staged operations once to the actual selected response without reordering them.
fn apply_fields(fields: Vec<HeaderOperation>, headers: &mut HeaderMap) {
    for field in fields {
        match field {
            HeaderOperation::Set(name, value) => {
                headers.insert(name, value);
            }
            HeaderOperation::Append(name, value) => {
                headers.append(name, value);
            }
            HeaderOperation::Remove(name) => {
                headers.remove(name);
            }
        }
    }
}
/// Response body ownership category; lazy renderers run only after selection/preconditions.
enum ReplyBody<E: Send + 'static, R: Send + Sync + 'static> {
    /// A finite, already encoded value.
    Finite(Bytes),
    /// Native opaque body retains its resource owner through terminal polling.
    Native(Body),
    /// Native handshake/body plus a core-owned continuation retained through session completion.
    NativeOwned(Body, Effect<(), E, R>),
    /// Incremental Effect producer connected to a bounded native sink.
    Program(BodyProgram<E, R>),
    /// Negotiated renderer selection retains A until a single request chooses an alternative.
    Choose(Box<RepresentationChooser<E, R>>),
    /// Selected renderer, constructed only when the request's negotiation permits it.
    Render(Effect<Reply<E, R>, E, R>),
    /// Native file range reader retaining its file until body completion.
    File(PathBuf),
}
/// A completed response description. Values move into one request; descriptions are not reusable bodies.
pub struct Reply<E: Send + 'static, R: Send + Sync + 'static> {
    /// Validated native status.
    pub(crate) status: StatusCode,
    /// Explicit status override from a validated draft; selected renderer statuses otherwise survive.
    status_override: bool,
    /// Metadata staged before response commitment.
    pub(crate) headers: HeaderMap,
    /// Ordered native metadata mutations retained through lazy selection/rendering.
    fields: Vec<HeaderOperation>,
    /// Body ownership is consumed once at the native boundary.
    body: ReplyBody<E, R>,
    /// Optional validators evaluated before any body renderer.
    validators: Option<ResourceValidators>,
    /// Trailers are emitted after producer completion under the same scope.
    trailers: HeaderMap,
}
/// Draft metadata has no terminal body. Macro syntax enforces exactly one terminal operation.
pub struct ReplyDraft {
    /// Only a validated body-bearing status may accompany a terminal body.
    status: BodyStatus,
    /// Whether the caller selected a status override rather than the named OK protocol baseline.
    explicit_status: bool,
    /// Source-ordered fields apply to the final selected terminal metadata.
    fields: Vec<HeaderOperation>,
    /// Conditional validators evaluated before production.
    validators: Option<ResourceValidators>,
    /// Trailers retained for the terminal body stream.
    trailers: HeaderMap,
}
impl Default for ReplyDraft {
    /// The named HTTP OK protocol status is the reply grammar's success status.
    fn default() -> Self {
        Self::new()
    }
}
impl ReplyDraft {
    /// Start a metadata-only draft; no renderer, body or native I/O runs.
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: BodyStatus::Ok,
            explicit_status: false,
            fields: Vec::new(),
            validators: None,
            trailers: HeaderMap::new(),
        }
    }
    /// Stage a validated body-bearing status.
    pub fn status(mut self, status: BodyStatus) -> Self {
        self.status = status;
        self.explicit_status = true;
        self
    }
    /// Replace a native validated field.
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.fields.push(HeaderOperation::Set(name, value));
        self
    }
    /// Append a native field, retaining repeated values such as Set-Cookie.
    pub fn append_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.fields.push(HeaderOperation::Append(name, value));
        self
    }
    /// Remove every existing value for this native field.
    pub fn remove_header(mut self, name: HeaderName) -> Self {
        self.fields.push(HeaderOperation::Remove(name));
        self
    }
    /// Stage a validated redirect/resource location.
    pub fn location(self, value: HeaderValue) -> Self {
        self.header(http::header::LOCATION, value)
    }
    /// Stage created-resource status and location.
    pub fn created(self, value: HeaderValue) -> Self {
        self.status(BodyStatus::Created).location(value)
    }
    /// Stage content language selected by the caller or negotiation engine.
    pub fn content_language(self, value: HeaderValue) -> Self {
        self.header(http::header::CONTENT_LANGUAGE, value)
    }
    /// Preserve native structured Cache-Control semantics.
    pub fn cache_control(mut self, value: headers::CacheControl) -> Self {
        let mut values = Vec::new();
        <headers::CacheControl as headers::Header>::encode(&value, &mut values);
        for value in values {
            self.fields
                .push(HeaderOperation::Set(http::header::CACHE_CONTROL, value));
        }
        self
    }
    /// Stage a native validated Vary field; repeated fields remain protocol-compatible.
    pub fn vary(self, value: HeaderValue) -> Self {
        self.append_header(http::header::VARY, value)
    }
    /// Stage response validators and evaluate request preconditions before body generation.
    pub fn validators(mut self, validators: ResourceValidators) -> Self {
        self.validators = Some(validators);
        self
    }
    /// Conditional GET uses the same protocol-order preconditions before rendering.
    pub fn conditional_get(self, validators: ResourceValidators) -> Self {
        self.validators(validators)
    }
    /// Stage a native trailer, emitted under the request's body scope.
    pub fn trailer(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.trailers.append(name, value);
        self
    }
    /// Append one validated native cookie without coalescing Set-Cookie values.
    pub fn set_cookie(self, cookie: ReplyCookie) -> Self {
        self.append_header(http::header::SET_COOKIE, cookie.0)
    }
    /// Append a validated deletion cookie with the caller's original path/domain scope.
    pub fn delete_cookie(self, cookie: ReplyCookie) -> Self {
        self.set_cookie(cookie)
    }
    /// Apply metadata to a bodyless terminal while preserving its required native status.
    pub fn finish_bodyless<E: Send + 'static, R: Send + Sync + 'static>(
        self,
        mut reply: Reply<E, R>,
    ) -> Reply<E, R> {
        reply.fields.extend(self.fields);
        reply.validators = self.validators.or(reply.validators);
        reply.trailers = self.trailers;
        reply
    }
    /// Merge this draft into exactly one completed terminal reply.
    pub fn finish<E: Send + 'static, R: Send + Sync + 'static>(
        self,
        mut reply: Reply<E, R>,
    ) -> Reply<E, R> {
        if self.explicit_status {
            reply.status = self.status.native();
            reply.status_override = true;
        }
        reply.fields.extend(self.fields);
        reply.validators = self.validators.or(reply.validators);
        reply.trailers = self.trailers;
        reply
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Reply<E, R> {
    /// Complete a finite UTF-8 text reply without running application work.
    #[must_use]
    pub fn text(text: String) -> Self {
        Self::bytes(
            Bytes::from(text),
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
    }
    /// Complete trusted/escaped HTML; conversion transfers the encoded string.
    #[must_use]
    pub fn html(html: HtmlContent) -> Self {
        Self::bytes(
            Bytes::from(html.0),
            HeaderValue::from_static("text/html; charset=utf-8"),
        )
    }
    /// Complete a finite encoded body with an explicit native media type.
    pub fn bytes(bytes: Bytes, media: HeaderValue) -> Self {
        let mut reply = Self::empty();
        reply.headers.insert(http::header::CONTENT_TYPE, media);
        reply.body = ReplyBody::Finite(bytes);
        reply
    }
    /// Complete an incremental body; producer evaluation remains lazy.
    pub fn stream(body: BodyProgram<E, R>, media: HeaderValue) -> Self {
        let mut reply = Self::empty();
        reply.headers.insert(http::header::CONTENT_TYPE, media);
        reply.body = ReplyBody::Program(body);
        reply
    }
    /// Complete an SSE body using the named SSE media protocol.
    #[must_use]
    pub fn sse(body: BodyProgram<E, R>) -> Self {
        Self::stream(body, crate::body::sse_content_type())
    }
    /// Complete a handwritten JSON body producer connected to the same bounded sink.
    #[must_use]
    pub fn json(body: BodyProgram<E, R>) -> Self {
        Self::stream(body, HeaderValue::from_static("application/json"))
    }
    /// Complete an extracted renderer; it runs inside the existing request context.
    #[must_use]
    pub fn render(effect: Effect<Self, E, R>) -> Self {
        let mut reply = Self::empty();
        reply.body = ReplyBody::Render(effect);
        reply
    }
    /// Complete a native response while retaining its opaque body ownership.
    #[must_use]
    pub fn native(response: http::Response<Body>) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            status_override: false,
            headers: parts.headers,
            fields: Vec::new(),
            body: ReplyBody::Native(body),
            validators: None,
            trailers: HeaderMap::new(),
        }
    }
    /// Transfer a native response and its Effect-owned continuation into one request scope.
    /// A native 101 upgrade retains this continuation through the duplex session's settlement.
    #[must_use]
    pub fn native_owned(response: http::Response<Body>, work: Effect<(), E, R>) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            status_override: false,
            headers: parts.headers,
            fields: Vec::new(),
            body: ReplyBody::NativeOwned(body, work),
            validators: None,
            trailers: HeaderMap::new(),
        }
    }
    /// Complete an explicit empty OK response.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            status: StatusCode::OK,
            status_override: false,
            headers: HeaderMap::new(),
            fields: Vec::new(),
            body: ReplyBody::Finite(Bytes::new()),
            validators: None,
            trailers: HeaderMap::new(),
        }
    }
    /// Complete a body-forbidden 204 reply; the DSL rejects additional body/status operations.
    #[must_use]
    pub fn no_content() -> Self {
        let mut reply = Self::empty();
        reply.status = StatusCode::NO_CONTENT;
        reply
    }
    /// Complete a validated redirect. Its empty body never requires renderer evaluation.
    pub fn redirect(status: RedirectStatus, location: HeaderValue) -> Self {
        let mut reply = Self::empty();
        reply.status = status.0;
        reply.headers.insert(http::header::LOCATION, location);
        reply
    }
    /// Describe file delivery with native seek/read and protocol range handling.
    pub fn file_path(path: PathBuf, media: HeaderValue) -> Self {
        let mut reply = Self::empty();
        reply.body = ReplyBody::File(path);
        reply.headers.insert(http::header::CONTENT_TYPE, media);
        reply
    }
    /// Obtain a finite witness under the caller's encoded-byte limit; no streaming collection occurs.
    pub fn into_finite(mut self, bytes: NonZeroUsize) -> Result<FiniteReply, ServerError> {
        apply_fields(std::mem::take(&mut self.fields), &mut self.headers);
        match self.body {
            ReplyBody::Finite(body) if body.len() <= bytes.get() && self.trailers.is_empty() => {
                Ok(FiniteReply {
                    status: self.status,
                    headers: self.headers,
                    bytes: body,
                })
            }
            ReplyBody::Finite(_) => Err(ServerError::Limit("finite response".into())),
            ReplyBody::Native(_)
            | ReplyBody::NativeOwned(_, _)
            | ReplyBody::Program(_)
            | ReplyBody::Render(_)
            | ReplyBody::Choose(_)
            | ReplyBody::File(_) => Err(ServerError::Invalid(
                "finite-response witness required".into(),
            )),
        }
    }
    /// Restore a checked finite witness; immutable bytes are shared without payload copies.
    pub fn from_finite(finite: FiniteReply) -> Self {
        Self {
            status: finite.status,
            status_override: false,
            headers: finite.headers,
            fields: Vec::new(),
            body: ReplyBody::Finite(finite.bytes),
            validators: None,
            trailers: HeaderMap::new(),
        }
    }
    /// Select the renderer lazily using media/language quality and exclusions.
    pub fn negotiate<A: Send + 'static>(choices: Representations<A, E, R>, value: A) -> Self
    where
        R: crate::HasRequest,
    {
        let mut reply = Self::empty();
        reply.body = ReplyBody::Choose(Box::new(move |request| match choices.select(request)? {
            Some(index) => match choices.items.get(index) {
                Some(item) => Ok(Some(ChosenReply {
                    media: item.media.clone(),
                    language: item.language.clone(),
                    render: {
                        let render = item.render.clone();
                        let value = Arc::new(std::sync::Mutex::new(Some(value)));
                        Effect::defer(move || {
                            let render = render.clone();
                            let value = value
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take();
                            match value {
                                Some(value) => render(value),
                                None => Effect::die(effect::Defect::new(
                                    "selected renderer value consumed",
                                )),
                            }
                        })
                    },
                })),
                None => Ok(None),
            },
            None => Ok(None),
        }));
        reply
    }

    /// Prepare one reply under its existing owner. The returned producer still waits for delivery.
    pub(crate) fn prepare(
        self,
        request: Request,
        scope: Scope<E>,
        policy: BodyPolicy,
        map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    ) -> Effect<PreparedReply<E, R>, E, R> {
        let owned = Arc::new(std::sync::Mutex::new(Some(self)));
        Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
            let reply = owned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let request = request.clone();
            let scope = scope.clone();
            let map_error = map_error.clone();
            async move {
                let Some(mut reply) = reply else {
                    return Exit::Die(effect::Defect::new("reply already consumed"));
                };
                loop {
                    if !matches!(&reply.body, ReplyBody::Render(_) | ReplyBody::Choose(_)) {
                        apply_fields(std::mem::take(&mut reply.fields), &mut reply.headers);
                    }
                    if let Some(validators) = &reply.validators {
                        validators.write(&mut reply.headers);
                        match validators.evaluate(&request) {
                            Ok(ConditionalOutcome::Proceed) => {}
                            Ok(ConditionalOutcome::NotModified) => {
                                reply.status = StatusCode::NOT_MODIFIED;
                                reply.body = ReplyBody::Finite(Bytes::new());
                            }
                            Ok(ConditionalOutcome::PreconditionFailed) => {
                                reply.status = StatusCode::PRECONDITION_FAILED;
                                reply.body = ReplyBody::Finite(Bytes::new());
                            }
                            Err(error) => return Exit::Fail(map_error(error)),
                        }
                    }
                    let upgrade = reply.status == StatusCode::SWITCHING_PROTOCOLS
                        && matches!(&reply.body, ReplyBody::NativeOwned(_, _));
                    if reply.status.is_informational() && !upgrade {
                        return Exit::Fail(map_error(ServerError::Invalid(
                            "informational status is not a terminal reply".into(),
                        )));
                    }
                    let body_forbidden = upgrade
                        || matches!(
                            reply.status,
                            StatusCode::NO_CONTENT
                                | StatusCode::RESET_CONTENT
                                | StatusCode::NOT_MODIFIED
                        );
                    if body_forbidden {
                        reply.body = match reply.body {
                            ReplyBody::NativeOwned(_, work) => {
                                ReplyBody::NativeOwned(Body::empty(), work)
                            }
                            ReplyBody::Finite(_)
                            | ReplyBody::Native(_)
                            | ReplyBody::Program(_)
                            | ReplyBody::Choose(_)
                            | ReplyBody::Render(_)
                            | ReplyBody::File(_) => ReplyBody::Finite(Bytes::new()),
                        };
                        reply.trailers.clear();
                        reply.headers.remove(http::header::TRANSFER_ENCODING);
                        if reply.status == StatusCode::NO_CONTENT {
                            reply.headers.remove(http::header::CONTENT_LENGTH);
                        }
                    }
                    if !body_forbidden
                        && request.method() == Method::HEAD
                        && !matches!(&reply.body, ReplyBody::Choose(_) | ReplyBody::File(_))
                    {
                        if let ReplyBody::Finite(bytes) = &reply.body
                            && let Ok(length) = HeaderValue::from_str(&bytes.len().to_string())
                        {
                            reply.headers.insert(http::header::CONTENT_LENGTH, length);
                        }
                        reply.body = ReplyBody::Finite(Bytes::new());
                    }
                    if !matches!(&reply.body, ReplyBody::Render(_) | ReplyBody::Choose(_)) {
                        apply_fields(std::mem::take(&mut reply.fields), &mut reply.headers);
                    }
                    match reply.body {
                        ReplyBody::Render(render) => match context.run(render, env.clone()).await {
                            Exit::Success(mut selected) => {
                                for name in reply.headers.keys() {
                                    selected.headers.remove(name);
                                    for value in reply.headers.get_all(name) {
                                        selected.headers.append(name.clone(), value.clone());
                                    }
                                }
                                for (name, value) in &reply.trailers {
                                    selected.trailers.append(name.clone(), value.clone());
                                }
                                selected.fields.extend(reply.fields);
                                if reply.status_override {
                                    selected.status = reply.status;
                                    selected.status_override = true;
                                }
                                reply = selected;
                            }
                            Exit::Fail(error) => return Exit::Fail(error),
                            Exit::Die(defect) => return Exit::Die(defect),
                            Exit::Interrupted => return Exit::Interrupted,
                            Exit::FinalizationFailed(failure) => {
                                return Exit::FinalizationFailed(failure);
                            }
                        },
                        ReplyBody::Choose(select) => {
                            reply.headers.append(
                                http::header::VARY,
                                HeaderValue::from_static("Accept, Accept-Language"),
                            );
                            match select(&request) {
                                Ok(Some(selected)) => {
                                    reply
                                        .headers
                                        .insert(http::header::CONTENT_TYPE, selected.media);
                                    if let Some(language) = selected.language {
                                        reply
                                            .headers
                                            .insert(http::header::CONTENT_LANGUAGE, language);
                                    }
                                    reply.body = if request.method() == Method::HEAD {
                                        ReplyBody::Native(Body::empty())
                                    } else {
                                        ReplyBody::Render(selected.render)
                                    };
                                }
                                Ok(None) => {
                                    reply.status = StatusCode::NOT_ACCEPTABLE;
                                    reply.body = ReplyBody::Finite(Bytes::new());
                                }
                                Err(error) => return Exit::Fail(map_error(error)),
                            }
                        }
                        ReplyBody::Program(program) => {
                            let (body, writer, completion) =
                                crate::body::channel(&request, policy, map_error.clone());
                            let trailers = reply.trailers;
                            let trailer_writer = writer.clone();
                            let production = program
                                .execute(writer.clone(), scope.clone())
                                .flat_map(move |()| trailer_writer.trailers(trailers.clone()));
                            let close_writer = writer.clone();
                            let close_completion = completion.clone();
                            let finish = production.ensuring_with(move |outcome| {
                                if !matches!(outcome, effect::PrimaryOutcomeRef::Succeeded) {
                                    close_completion.fail();
                                }
                                let writer = close_writer.clone();
                                effect::Release::from_effect(Effect::sync(move || writer.finish()))
                            });
                            let gate = completion.clone();
                            let map = map_error.clone();
                            let finish = finish.flat_map(move |()| {
                                let gate = gate.clone();
                                let map = map.clone();
                                Effect::try_future(
                                    move |_| {
                                        let gate = gate.clone();
                                        async move {
                                            match gate.wait().await {
                                                crate::BodyOutcome::Completed => Ok(()),
                                                crate::BodyOutcome::Open
                                                | crate::BodyOutcome::Disconnected
                                                | crate::BodyOutcome::Failed => {
                                                    Err(ServerError::Disconnected)
                                                }
                                            }
                                        }
                                    },
                                    move |error| map(error),
                                )
                            });
                            let mut response = http::Response::new(body);
                            *response.status_mut() = reply.status;
                            *response.headers_mut() = reply.headers;
                            return Exit::Success(PreparedReply {
                                response,
                                finish,
                                completion,
                            });
                        }
                        ReplyBody::File(path) => {
                            let file = match tokio::fs::File::open(path).await {
                                Ok(file) => file,
                                Err(error) => return Exit::Fail(map_error(ServerError::Io(error))),
                            };
                            let length = match file.metadata().await {
                                Ok(metadata) => metadata.len(),
                                Err(error) => return Exit::Fail(map_error(ServerError::Io(error))),
                            };
                            let (start, end, partial) =
                                match file_range(&request, &reply.headers, length) {
                                    Ok(Some(range)) => range,
                                    Ok(None) => {
                                        reply.status = StatusCode::RANGE_NOT_SATISFIABLE;
                                        if let Ok(value) =
                                            HeaderValue::from_str(&format!("bytes */{length}"))
                                        {
                                            reply
                                                .headers
                                                .insert(http::header::CONTENT_RANGE, value);
                                        }
                                        return Exit::Success(native_prepared(
                                            Body::empty(),
                                            reply.status,
                                            reply.headers,
                                            request,
                                            map_error,
                                        ));
                                    }
                                    Err(error) => return Exit::Fail(map_error(error)),
                                };
                            let remaining = end.saturating_sub(start);
                            if partial {
                                reply.status = StatusCode::PARTIAL_CONTENT;
                                if let Ok(range) = HeaderValue::from_str(&format!(
                                    "bytes {start}-{}/{length}",
                                    end.saturating_sub(1)
                                )) {
                                    reply.headers.insert(http::header::CONTENT_RANGE, range);
                                }
                            }
                            reply.headers.insert(
                                http::header::ACCEPT_RANGES,
                                HeaderValue::from_static("bytes"),
                            );
                            if let Ok(length) = HeaderValue::from_str(&remaining.to_string()) {
                                reply.headers.insert(http::header::CONTENT_LENGTH, length);
                            }
                            if request.method() == Method::HEAD {
                                return Exit::Success(native_prepared(
                                    Body::empty(),
                                    reply.status,
                                    reply.headers,
                                    request,
                                    map_error,
                                ));
                            }
                            let file = Arc::new(std::sync::Mutex::new(Some(file)));
                            let map = map_error.clone();
                            reply.body = ReplyBody::Program(BodyProgram::new(move |writer| {
                                let file = file.clone();
                                let map = map.clone();
                                Effect::from_exit_future_with_context(
                                    move |env: Arc<R>, context| {
                                        let file = file.clone();
                                        let writer = writer.clone();
                                        let map = map.clone();
                                        async move {
                                            let Some(mut file) = file
                                                .lock()
                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                .take()
                                            else {
                                                return Exit::Die(effect::Defect::new(
                                                    "file reply already consumed",
                                                ));
                                            };
                                            if let Err(error) =
                                                file.seek(std::io::SeekFrom::Start(start)).await
                                            {
                                                return Exit::Fail(map(ServerError::Io(error)));
                                            }
                                            let mut remaining = remaining;
                                            while remaining > 0 {
                                                let size = usize::try_from(remaining)
                                                    .unwrap_or(usize::MAX)
                                                    .min(policy.chunk_bytes.get());
                                                let mut bytes = vec![0; size];
                                                let read =
                                                    match file.read(&mut bytes).await {
                                                        Ok(0) => return Exit::Fail(map(
                                                            ServerError::Invalid(
                                                                "file truncated during delivery"
                                                                    .into(),
                                                            ),
                                                        )),
                                                        Ok(read) => read,
                                                        Err(error) => {
                                                            return Exit::Fail(map(
                                                                ServerError::Io(error),
                                                            ));
                                                        }
                                                    };
                                                bytes.truncate(read);
                                                remaining = remaining.saturating_sub(read as u64);
                                                let result = context
                                                    .run(
                                                        writer.write(Bytes::from(bytes)),
                                                        env.clone(),
                                                    )
                                                    .await;
                                                if !matches!(result, Exit::Success(())) {
                                                    return result;
                                                }
                                            }
                                            Exit::Success(())
                                        }
                                    },
                                )
                            }));
                        }
                        ReplyBody::Finite(bytes) => {
                            let body =
                                crate::body::with_trailers(Body::from(bytes), reply.trailers);
                            return Exit::Success(native_prepared(
                                body,
                                reply.status,
                                reply.headers,
                                request,
                                map_error,
                            ));
                        }
                        ReplyBody::Native(body) => {
                            return Exit::Success(native_prepared(
                                crate::body::with_trailers(body, reply.trailers),
                                reply.status,
                                reply.headers,
                                request,
                                map_error,
                            ));
                        }
                        ReplyBody::NativeOwned(body, work) => {
                            let mut prepared = native_prepared(
                                crate::body::with_trailers(body, reply.trailers),
                                reply.status,
                                reply.headers,
                                request,
                                map_error,
                            );
                            prepared.finish = prepared.finish.flat_map(move |()| work.clone());
                            return Exit::Success(prepared);
                        }
                    }
                }
            }
        })
    }
}
/// Redirect codes are selected explicitly and validated separately from body statuses.
#[derive(Clone, Copy)]
pub struct RedirectStatus(StatusCode);
impl RedirectStatus {
    /// Validate a native 301/302/303/307/308 redirect status.
    pub fn new(status: StatusCode) -> Result<Self, ServerError> {
        if matches!(
            status,
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            Ok(Self(status))
        } else {
            Err(ServerError::Invalid("redirect status".into()))
        }
    }
}
/// Selected native representation metadata plus an unevaluated renderer.
struct ChosenReply<E: Send + 'static, R: Send + Sync + 'static> {
    /// Concrete selected media type.
    media: HeaderValue,
    /// Selected language tag when declared.
    language: Option<HeaderValue>,
    /// Lazy renderer receives the moved application value only after selection.
    render: Effect<Reply<E, R>, E, R>,
}
/// An interpreted response plus remaining owned producer/delivery work.
pub(crate) struct PreparedReply<E: Send + 'static, R: Send + Sync + 'static> {
    /// Response transfers into axum once; constructing it is not completion.
    pub(crate) response: http::Response<Body>,
    /// Managed body program and delivery gate run before request scope closure.
    pub(crate) finish: Effect<(), E, R>,
    /// Native boundary state for completion observers.
    pub(crate) completion: BodyCompletion,
}
/// A concrete media/language representation with a lazy typed renderer.
pub struct Representation<A, E: Send + 'static, R: Send + Sync + 'static> {
    /// Checked native concrete media type.
    media: HeaderValue,
    /// Optional validated language tag.
    language: Option<HeaderValue>,
    /// Selected renderer moves A once; alternatives never run eagerly.
    render: Arc<RepresentationRenderer<A, E, R>>,
}
impl<A, E: Send + 'static, R: Send + Sync + 'static> Representation<A, E, R> {
    /// Validate concrete media/language metadata before serving; no renderer is invoked.
    pub fn new(
        media: HeaderValue,
        language: Option<HeaderValue>,
        render: impl Fn(A) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Result<Self, ServerError> {
        let text = media
            .to_str()
            .map_err(|_| ServerError::Invalid("media type".into()))?;
        let base = text.split(';').next().unwrap_or("");
        let mut values = std::iter::once(&media);
        <headers::ContentType as headers::Header>::decode(&mut values)
            .map_err(|_| ServerError::Invalid("media type".into()))?;
        if !base.contains('/') || base.contains('*') {
            return Err(ServerError::Invalid("concrete media type".into()));
        }
        if let Some(language) = &language {
            let text = language
                .to_str()
                .map_err(|_| ServerError::Invalid("language".into()))?;
            if text.is_empty()
                || !text.split('-').all(|part| {
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                })
            {
                return Err(ServerError::Invalid("language".into()));
            }
        }
        Ok(Self {
            media,
            language,
            render: Arc::new(render),
        })
    }
}
/// Ordered nonempty representation alternatives checked once at registration time.
pub struct Representations<A, E: Send + 'static, R: Send + Sync + 'static> {
    /// Declaration order breaks otherwise equal preferences.
    items: Arc<[Representation<A, E, R>]>,
}
impl<A, E: Send + 'static, R: Send + Sync + 'static> Representations<A, E, R> {
    /// Reject empty/duplicate alternatives without evaluating renderers.
    pub fn new(items: Vec<Representation<A, E, R>>) -> Result<Self, ServerError> {
        if items.is_empty() {
            return Err(ServerError::Configuration("empty representations".into()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in &items {
            if !seen.insert((
                media_parts(
                    item.media
                        .to_str()
                        .map_err(|_| ServerError::Invalid("media type".into()))?,
                )?,
                item.language
                    .as_ref()
                    .and_then(|language| language.to_str().ok())
                    .map(str::to_ascii_lowercase),
            )) {
                return Err(ServerError::Configuration(
                    "duplicate representation".into(),
                ));
            }
        }
        Ok(Self {
            items: items.into(),
        })
    }
    /// Select by most-specific range, then quality; exclusions override less-specific wildcards.
    fn select(&self, request: &Request) -> Result<Option<usize>, ServerError> {
        let media = preferences(request.headers(), http::header::ACCEPT)?;
        let languages = preferences(request.headers(), http::header::ACCEPT_LANGUAGE)?;
        let mut winner = None;
        let mut best = None;
        for (index, item) in self.items.iter().enumerate() {
            let (concrete, parameters) = media_parts(
                item.media
                    .to_str()
                    .map_err(|_| ServerError::Invalid("media type".into()))?,
            )?;
            let media_match = choose_preference(&media, |preference| {
                let range = &preference.range;
                let specificity = if range == "*/*" {
                    0
                } else if range.ends_with("/*") && concrete.starts_with(range.trim_end_matches('*'))
                {
                    1
                } else if range.eq_ignore_ascii_case(&concrete) {
                    2
                } else {
                    return None;
                };
                if !preference
                    .parameters
                    .iter()
                    .all(|(name, value)| parameters.get(name) == Some(value))
                {
                    return None;
                }
                Some((specificity, preference.parameters.len()))
            });
            let language = item
                .language
                .as_ref()
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let language_match = choose_preference(&languages, |preference| {
                let range = &preference.range;
                if range == "*" {
                    Some((0, 0))
                } else if language == *range
                    || language
                        .strip_prefix(range.as_str())
                        .is_some_and(|tail| tail.starts_with('-'))
                {
                    Some((range.len(), 0))
                } else {
                    None
                }
            });
            if let (Some((mq, ms)), Some((lq, ls))) = (media_match, language_match) {
                if mq == 0 || lq == 0 {
                    continue;
                }
                let score = (mq, lq, ms, ls);
                if best.is_none_or(|best| score > best) {
                    best = Some(score);
                    winner = Some(index);
                }
            }
        }
        Ok(winner)
    }
}
/// Native Accept family range and integer thousandths quality, avoiding float ordering ambiguity.
struct Preference {
    /// Normalized media/language range.
    range: String,
    /// RFC quality in thousandths, inclusive 0..1000.
    quality: u16,
    /// Media parameters before q contribute to both matching and specificity.
    parameters: std::collections::BTreeMap<String, String>,
}
/// Parse repeated Accept-family fields with protocol quality bounds and exclusion preservation.
fn preferences(headers: &HeaderMap, name: HeaderName) -> Result<Vec<Preference>, ServerError> {
    let is_media = name == http::header::ACCEPT;
    let mut output = Vec::new();
    for value in headers.get_all(name) {
        let value = value
            .to_str()
            .map_err(|_| ServerError::Invalid("negotiation header".into()))?;
        for entry in split_quoted(value, ',')? {
            let mut parts = split_quoted(entry, ';')?.into_iter();
            let range = parts.next().unwrap_or("").trim().to_ascii_lowercase();
            if is_media {
                range
                    .parse::<headers::ContentType>()
                    .map_err(|_| ServerError::Invalid("negotiation media range".into()))?;
            } else if range != "*"
                && (range.is_empty()
                    || !range.split('-').all(|part| {
                        !part.is_empty()
                            && part
                                .chars()
                                .all(|character| character.is_ascii_alphanumeric())
                    }))
            {
                return Err(ServerError::Invalid("negotiation language range".into()));
            }
            let mut quality = 1000;
            let mut seen_quality = false;
            let mut parameters = std::collections::BTreeMap::new();
            for parameter in parts {
                let (name, value) = parameter
                    .trim()
                    .split_once('=')
                    .ok_or_else(|| ServerError::Invalid("negotiation parameter".into()))?;
                if name.trim().eq_ignore_ascii_case("q") {
                    if seen_quality {
                        return Err(ServerError::Invalid("duplicate quality".into()));
                    }
                    seen_quality = true;
                    quality = parse_quality(value.trim())?;
                } else if !seen_quality && is_media {
                    let (name, value) = media_parameter(name, value)?;
                    if parameters.insert(name, value).is_some() {
                        return Err(ServerError::Duplicate("media parameter".into()));
                    }
                } else if !is_media {
                    return Err(ServerError::Invalid("language parameter".into()));
                }
            }
            output.push(Preference {
                range,
                quality,
                parameters,
            });
        }
    }
    Ok(output)
}
/// Split native header lists while retaining delimiters inside escaped quoted strings.
fn split_quoted(text: &str, separator: char) -> Result<Vec<&str>, ServerError> {
    let mut output = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if character == separator && !quoted {
            output.push(text.get(start..index).unwrap_or(""));
            start = index.saturating_add(character.len_utf8());
        }
    }
    if quoted || escaped {
        return Err(ServerError::Invalid("quoted media parameter".into()));
    }
    output.push(text.get(start..).unwrap_or(""));
    Ok(output)
}
/// Normalize parameter names and charset values; other MIME values retain their case.
fn media_parameter(name: &str, value: &str) -> Result<(String, String), ServerError> {
    let name = name.trim().to_ascii_lowercase();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return Err(ServerError::Invalid("media parameter".into()));
    }
    let mut decoded = String::new();
    let mut escaped = false;
    let value = if let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        for character in value.chars() {
            if escaped {
                decoded.push(character);
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                decoded.push(character);
            }
        }
        if escaped {
            return Err(ServerError::Invalid("media parameter".into()));
        }
        decoded
    } else {
        value.to_owned()
    };
    let value = if name == "charset" {
        value.to_ascii_lowercase()
    } else {
        value
    };
    Ok((name, value))
}
/// Parse concrete media metadata through the native MIME parser before matching parameters.
pub(crate) fn media_parts(
    text: &str,
) -> Result<(String, std::collections::BTreeMap<String, String>), ServerError> {
    let checked = text
        .parse::<headers::ContentType>()
        .map_err(|_| ServerError::Invalid("media type".into()))?
        .to_string();
    let mut parts = split_quoted(&checked, ';')?.into_iter();
    let base = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let mut parameters = std::collections::BTreeMap::new();
    for parameter in parts {
        let (name, value) = parameter
            .trim()
            .split_once('=')
            .ok_or_else(|| ServerError::Invalid("media parameter".into()))?;
        let (name, value) = media_parameter(name, value)?;
        if parameters.insert(name, value).is_some() {
            return Err(ServerError::Duplicate("media parameter".into()));
        }
    }
    Ok((base, parameters))
}
/// RFC 9110 qvalue precision is at most three decimal digits and never exceeds one.
fn parse_quality(text: &str) -> Result<u16, ServerError> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if !matches!(whole, "0" | "1")
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (whole == "1" && fraction.bytes().any(|byte| byte != b'0'))
    {
        return Err(ServerError::Invalid("quality".into()));
    }
    if whole == "1" {
        return Ok(1000);
    }
    let padded = format!("{fraction:0<3}");
    padded
        .parse::<u16>()
        .map_err(|_| ServerError::Invalid("quality".into()))
}
/// Match specificity before quality, so a specific q=0 excludes a wildcard q=1.
fn choose_preference(
    preferences: &[Preference],
    matches: impl Fn(&Preference) -> Option<(usize, usize)>,
) -> Option<(u16, (usize, usize))> {
    if preferences.is_empty() {
        return Some((1000, (0, 0)));
    }
    preferences
        .iter()
        .filter_map(|preference| {
            matches(preference).map(|specificity| (preference.quality, specificity))
        })
        .max_by_key(|(quality, specificity)| (*specificity, *quality))
}

/// Wrap a native finite/opaque body and await actual body delivery under its request owner.
fn native_prepared<E: Send + 'static, R: Send + Sync + 'static>(
    body: Body,
    status: StatusCode,
    headers: HeaderMap,
    request: Request,
    map: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> PreparedReply<E, R> {
    let completion = BodyCompletion::new(request.cancellation().clone());
    let observed = crate::body::ObservedBody {
        body,
        completion: completion.clone(),
    };
    let mut response = http::Response::new(Body::new(observed));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    let gate = completion.clone();
    let finish = Effect::try_future(
        move |_| {
            let gate = gate.clone();
            async move {
                match gate.wait().await {
                    crate::BodyOutcome::Completed => Ok(()),
                    crate::BodyOutcome::Open
                    | crate::BodyOutcome::Failed
                    | crate::BodyOutcome::Disconnected => Err(ServerError::Disconnected),
                }
            }
        },
        move |error| map(error),
    );
    PreparedReply {
        response,
        finish,
        completion,
    }
}
/// Resolve a native validated single byte range, including suffix/clamping/If-Range.
/// Multiple ranges deliberately fall back to the complete representation as HTTP allows;
/// callers requiring multipart/byteranges can supply a native body without collection.
fn file_range(
    request: &Request,
    response: &HeaderMap,
    length: u64,
) -> Result<Option<(u64, u64, bool)>, ServerError> {
    let full = Some((0, length, false));
    if request.method() != Method::GET {
        return Ok(full);
    }
    if request.typed_header::<headers::Range>()?.is_none() {
        return Ok(full);
    }
    if request.headers().contains_key(http::header::IF_RANGE) {
        let matches = request
            .typed_header::<headers::IfRange>()
            .ok()
            .flatten()
            .is_some_and(|condition| {
                !condition.is_modified(
                    response.typed_get::<headers::ETag>().as_ref(),
                    response.typed_get::<headers::LastModified>().as_ref(),
                )
            });
        if !matches {
            return Ok(full);
        }
    }
    let text = request
        .headers()
        .get(http::header::RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ServerError::Invalid("Range".into()))?;
    let Some(spec) = text.strip_prefix("bytes=") else {
        return Ok(full);
    };
    if spec.contains(',') {
        return Ok(full);
    }
    let (start, end) = spec
        .trim()
        .split_once('-')
        .ok_or_else(|| ServerError::Invalid("Range".into()))?;
    let parse = |text: &str| {
        text.parse::<u64>()
            .map_err(|_| ServerError::Invalid("Range".into()))
    };
    let (start, end) = if start.is_empty() {
        let suffix = parse(end)?;
        if suffix == 0 {
            return Ok(None);
        }
        (length.saturating_sub(suffix), length)
    } else {
        let start = parse(start)?;
        let end = if end.is_empty() {
            length
        } else {
            let last = parse(end)?;
            if last < start {
                return Err(ServerError::Invalid("Range".into()));
            }
            last.saturating_add(1).min(length)
        };
        (start, end)
    };
    if start >= length || end <= start {
        Ok(None)
    } else {
        Ok(Some((start, end, true)))
    }
}

/// Checked direct-writer witness. A plain codec without a writer cannot enter the HTTP body path.
pub struct JsonWriterCodec<A: Send + 'static> {
    /// Pure direct codec callbacks execute on the selected blocking provider.
    codec: effect::PureCodec<A, effect::JsonValue>,
}
impl<A: Send + 'static> Clone for JsonWriterCodec<A> {
    /// Share writer descriptions without cloning A or encoded data.
    fn clone(&self) -> Self {
        Self {
            codec: self.codec.clone(),
        }
    }
}
impl<A: Send + 'static> JsonWriterCodec<A> {
    /// Check direct native-writer support at setup; no JSON DOM/serialization fallback is installed.
    pub fn require(codec: effect::PureCodec<A, effect::JsonValue>) -> Result<Self, ServerError> {
        if codec.has_writer() {
            Ok(Self { codec })
        } else {
            Err(ServerError::Configuration(
                "direct JSON writer witness required".into(),
            ))
        }
    }
}
impl<E, R> Reply<E, R>
where
    E: Send + 'static,
    R: effect::HasBlockingExecutor + Send + Sync + 'static,
{
    /// Encode one finite value directly into the bounded native sink on R's selected blocking worker.
    /// The witness preserves native codec checks; it does not build a JSON DOM or collect output.
    pub fn json_codec<A: Send + 'static>(
        codec: JsonWriterCodec<A>,
        value: A,
        policy: effect::JsonWriterPolicy,
    ) -> Self
    where
        R: crate::HasServerError<Error = E>,
    {
        let value = Arc::new(std::sync::Mutex::new(Some(value)));
        let body = BodyProgram::new(move |writer| {
            let value = value.clone();
            let codec = codec.clone();
            let sink = crate::body::NativeJsonSink::from_writer(&writer);
            crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                let mapper = env.clone();
                let encode = Effect::try_blocking_once(
                    move |_| {
                        let value = value
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                            .ok_or_else(|| {
                                effect::SchemaIssue::new(effect::SchemaIssueKind::InvalidDefinition)
                            })?;
                        let mut writer = effect::JsonWriter::with_sink(policy, sink);
                        codec.codec.write_into(&value, &mut writer)?;
                        writer.finish_sink()
                    },
                    move |issue| mapper.server_error(ServerError::Schema(issue)),
                );
                context.run(encode, env).await
            })
        });
        Self::json(body)
    }
}
/// Applications select the native error mapping through their original R for generated writer APIs.
pub trait HasServerError {
    /// Typed application failure associated with this environment.
    type Error: Send + 'static;
    /// Map native input/body failures without a global exception-erasing contract.
    fn server_error(&self, error: ServerError) -> Self::Error;
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Representation<A, E, R> {
    /// Add a checked language tag to an existing lazy alternative.
    pub fn language(mut self, language: HeaderValue) -> Result<Self, ServerError> {
        let text = language
            .to_str()
            .map_err(|_| ServerError::Invalid("language".into()))?;
        if text.is_empty()
            || !text.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            })
        {
            return Err(ServerError::Invalid("language".into()));
        }
        self.language = Some(language);
        Ok(self)
    }
    /// Lazy UTF-8 text alternative; the selected callback alone produces the text.
    pub fn text(render: impl Fn(A) -> Effect<String, E, R> + Send + Sync + 'static) -> Self {
        Self {
            media: HeaderValue::from_static("text/plain; charset=utf-8"),
            language: None,
            render: Arc::new(move |value| render(value).map(Reply::text)),
        }
    }
    /// Lazy escaped/trusted HTML alternative with an extracted application renderer.
    pub fn html(render: impl Fn(A) -> Effect<HtmlContent, E, R> + Send + Sync + 'static) -> Self {
        Self {
            media: HeaderValue::from_static("text/html; charset=utf-8"),
            language: None,
            render: Arc::new(move |value| render(value).map(Reply::html)),
        }
    }
    /// Incremental representation with an explicit concrete native media type.
    pub fn stream(
        media: HeaderValue,
        render: impl Fn(A) -> Effect<BodyProgram<E, R>, E, R> + Send + Sync + 'static,
    ) -> Result<Self, ServerError> {
        let reply_media = media.clone();
        Self::new(media, None, move |value| {
            let media = reply_media.clone();
            render(value).map(move |body| Reply::stream(body, media.clone()))
        })
    }
}
impl<A, E, R> Representation<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: effect::HasBlockingExecutor + HasServerError<Error = E> + Send + Sync + 'static,
{
    /// Direct checked JSON alternative; no renderer runs during media/language selection.
    #[must_use]
    pub fn json_codec(codec: JsonWriterCodec<A>, policy: effect::JsonWriterPolicy) -> Self {
        Self {
            media: HeaderValue::from_static("application/json"),
            language: None,
            render: Arc::new(move |value| {
                Effect::from_result_once(Ok(Reply::json_codec(codec.clone(), value, policy)))
            }),
        }
    }
}
/// Validated RFC 9457 problem payload. Secret/internal error values never serialize implicitly.
pub struct HttpProblem {
    /// Caller-selected problem type URI.
    pub type_uri: String,
    /// Safe human-readable summary.
    pub title: String,
    /// Named native HTTP error status.
    pub status: ProblemStatus,
    /// Optional selected safe detail.
    pub detail: Option<String>,
    /// Optional selected resource instance URI.
    pub instance: Option<String>,
}
/// Validated HTTP 4xx/5xx status for RFC 9457 problem responses.
#[derive(Clone, Copy)]
pub struct ProblemStatus(StatusCode);
impl ProblemStatus {
    /// Reject successful/informational statuses for error problem responses.
    pub fn new(status: StatusCode) -> Result<Self, ServerError> {
        if status.is_client_error() || status.is_server_error() {
            Ok(Self(status))
        } else {
            Err(ServerError::Invalid("problem status".into()))
        }
    }
}
/// Safe validation-problem fields, with explicit caller-selected error messages.
pub struct ValidationProblem {
    /// Validated native error status.
    pub status: ProblemStatus,
    /// Safe problem title.
    pub title: String,
    /// Ordered field error messages; no raw application error dump occurs.
    pub errors: std::collections::BTreeMap<String, Vec<String>>,
}
impl<E, R> Reply<E, R>
where
    E: Send + 'static,
    R: HasServerError<Error = E> + Send + Sync + 'static,
{
    /// Write RFC 9457 fields incrementally through the same native sink, without building a JSON DOM.
    #[must_use]
    pub fn problem(problem: HttpProblem) -> Self {
        let status = problem.status.0;
        let problem = Arc::new(std::sync::Mutex::new(Some(problem)));
        let body = BodyProgram::new(move |writer| {
            let problem = problem.clone();
            crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                let Some(problem) = problem
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                else {
                    return Exit::Die(effect::Defect::new("problem body already consumed"));
                };
                let fields = [
                    ("type", Some(problem.type_uri)),
                    ("title", Some(problem.title)),
                    ("detail", problem.detail),
                    ("instance", problem.instance),
                ];
                let begin = context
                    .run(writer.write(Bytes::from_static(b"{")), env.clone())
                    .await;
                if !matches!(begin, Exit::Success(())) {
                    return begin;
                }
                let mut first = true;
                for (name, value) in fields {
                    if let Some(value) = value {
                        let prefix = if first { "" } else { "," };
                        first = false;
                        let value = match serde_json::to_string(&value) {
                            Ok(value) => value,
                            Err(_) => {
                                return Exit::Fail(env.server_error(ServerError::Invalid(
                                    "problem string encoding".into(),
                                )));
                            }
                        };
                        let chunk = Bytes::from(format!("{prefix}\"{name}\":{value}"));
                        let result = context.run(writer.write(chunk), env.clone()).await;
                        if !matches!(result, Exit::Success(())) {
                            return result;
                        }
                    }
                }
                context
                    .run(
                        writer.write(Bytes::from(format!(
                            ",\"status\":{}}}",
                            problem.status.0.as_u16()
                        ))),
                        env,
                    )
                    .await
            })
        });
        let mut reply = Self::stream(body, HeaderValue::from_static("application/problem+json"));
        reply.status = status;
        reply
    }
    /// Write selected field messages as a validation problem through an incremental body program.
    #[must_use]
    pub fn validation(problem: ValidationProblem) -> Self {
        let status = problem.status.0;
        let problem = Arc::new(std::sync::Mutex::new(Some(problem)));
        let body = BodyProgram::new(move |writer| {
            let problem = problem.clone();
            crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                let Some(problem) = problem
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                else {
                    return Exit::Die(effect::Defect::new("validation body consumed"));
                };
                let title = match serde_json::to_string(&problem.title) {
                    Ok(value) => value,
                    Err(_) => {
                        return Exit::Fail(
                            env.server_error(ServerError::Invalid("validation encoding".into())),
                        );
                    }
                };
                let result = context
                    .run(
                        writer.write(Bytes::from(format!(
                            "{{\"title\":{title},\"status\":{},\"errors\":{{",
                            status.as_u16()
                        ))),
                        env.clone(),
                    )
                    .await;
                if !matches!(result, Exit::Success(())) {
                    return result;
                }
                let mut first_field = true;
                for (name, errors) in problem.errors {
                    let name = match serde_json::to_string(&name) {
                        Ok(value) => value,
                        Err(_) => {
                            return Exit::Fail(
                                env.server_error(ServerError::Invalid("validation field".into())),
                            );
                        }
                    };
                    let prefix = if first_field { "" } else { "," };
                    first_field = false;
                    let result = context
                        .run(
                            writer.write(Bytes::from(format!("{prefix}{name}:["))),
                            env.clone(),
                        )
                        .await;
                    if !matches!(result, Exit::Success(())) {
                        return result;
                    }
                    let mut first = true;
                    for error in errors {
                        let error = match serde_json::to_string(&error) {
                            Ok(value) => value,
                            Err(_) => {
                                return Exit::Fail(env.server_error(ServerError::Invalid(
                                    "validation message".into(),
                                )));
                            }
                        };
                        let prefix = if first { "" } else { "," };
                        first = false;
                        let result = context
                            .run(
                                writer.write(Bytes::from(format!("{prefix}{error}"))),
                                env.clone(),
                            )
                            .await;
                        if !matches!(result, Exit::Success(())) {
                            return result;
                        }
                    }
                    let result = context
                        .run(writer.write(Bytes::from_static(b"]")), env.clone())
                        .await;
                    if !matches!(result, Exit::Success(())) {
                        return result;
                    }
                }
                context
                    .run(writer.write(Bytes::from_static(b"}}")), env)
                    .await
            })
        });
        let mut reply = Self::stream(body, HeaderValue::from_static("application/problem+json"));
        reply.status = status;
        reply
    }
}

/// One-shot negotiation that moves only the selected reply.
type RepresentationChooser<E, R> =
    dyn FnOnce(&Request) -> Result<Option<ChosenReply<E, R>>, ServerError> + Send;

/// Lazy representation renderer consuming its application value.
type RepresentationRenderer<A, E, R> = dyn Fn(A) -> Effect<Reply<E, R>, E, R> + Send + Sync;
