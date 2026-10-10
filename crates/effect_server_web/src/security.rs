//! Browser security metadata and native policy selection.
//!
//! Trusted forwarding is checked against the transport peer before accepting any
//! forwarded values. Native CORS/compression/static services are retained rather
//! than reimplemented. Executable stage ordering belongs to the Web/Server
//! composition boundary, not to an unordered collection of layers.

use crate::{
    WebError,
    csp::{Csp, ResponseNonce},
};
use effect::Effect;
use effect_server::{BodyLimit, FormPolicy, Request};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Uri};
use std::net::{IpAddr, SocketAddr};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

/// Prepared absolute HTTP(S) origin without credentials, path, query or fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct CorsOrigin(HeaderValue);
impl CorsOrigin {
    /// Validate an origin once before native serving. Opaque `null` and wildcards
    /// are separate policy choices rather than credentialed explicit origins.
    pub fn new(value: &str) -> Result<Self, WebError> {
        let uri: Uri = value
            .parse()
            .map_err(|_| WebError::Configuration("CORS origin".into()))?;
        let authority = uri
            .authority()
            .ok_or_else(|| WebError::Configuration("CORS origin authority".into()))?;
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || authority.as_str().contains('@')
            || uri
                .path_and_query()
                .is_some_and(|path| path.as_str() != "/")
            || value.contains(['*', '#'])
        {
            return Err(WebError::Configuration("CORS origin".into()));
        }
        let canonical = format!("{}://{}", uri.scheme_str().unwrap_or(""), authority);
        HeaderValue::from_str(&canonical)
            .map(Self)
            .map_err(|_| WebError::Configuration("CORS origin".into()))
    }
}
/// CORS origin policy. Credentialed requests require explicit origins.
#[derive(Clone)]
#[must_use]
pub enum CorsOrigins {
    /// Public wildcard; incompatible with credentialed browser requests.
    Any,
    /// Nonempty prepared origins, checked for duplicates.
    Explicit(Vec<CorsOrigin>),
}
/// Validated native CORS selection; response/preflight handling uses Tower HTTP.
#[derive(Clone)]
#[must_use]
pub struct CorsPolicy {
    /// Selected origins retained without a request/global lookup.
    origins: CorsOrigins,
    /// Explicit native methods permitted by preflight.
    methods: Vec<Method>,
    /// Explicit native request headers permitted by preflight.
    headers: Vec<HeaderName>,
    /// Native exposed response headers.
    expose: Vec<HeaderName>,
    /// Whether credentials are allowed for explicit origins.
    credentials: bool,
    /// Caller-selected preflight retention in seconds, absent means no max-age.
    max_age: Option<std::time::Duration>,
}
impl CorsPolicy {
    /// Reject wildcard credentials and empty/duplicate explicit origins. Methods,
    /// headers and exposure remain caller supplied; no permissive defaults appear.
    pub fn new(
        origins: CorsOrigins,
        methods: Vec<Method>,
        headers: Vec<HeaderName>,
        expose: Vec<HeaderName>,
        credentials: bool,
        max_age: Option<std::time::Duration>,
    ) -> Result<Self, WebError> {
        match &origins {
            CorsOrigins::Any if credentials => {
                return Err(WebError::Configuration("wildcard CORS credentials".into()));
            }
            CorsOrigins::Any => {}
            CorsOrigins::Explicit(values) => {
                if values.is_empty() {
                    return Err(WebError::Configuration("empty CORS origins".into()));
                }
                let mut unique = std::collections::BTreeSet::new();
                for value in values {
                    if !unique.insert(value.0.as_bytes()) {
                        return Err(WebError::Configuration("duplicate CORS origin".into()));
                    }
                }
            }
        }
        if methods.is_empty() || max_age.is_some_and(|d| d.is_zero() || d.subsec_nanos() != 0) {
            return Err(WebError::Configuration(
                "CORS methods/max-age seconds".into(),
            ));
        }
        Ok(Self {
            origins,
            methods,
            headers,
            expose,
            credentials,
            max_age,
        })
    }
    /// Build the native layer without serving. The Web boundary positions it before
    /// route authentication so preflight follows the declared browser policy.
    pub fn native(&self) -> CorsLayer {
        let layer = CorsLayer::new()
            .allow_methods(self.methods.clone())
            .allow_headers(self.headers.clone())
            .expose_headers(self.expose.clone())
            .allow_credentials(self.credentials);
        let layer = match &self.origins {
            CorsOrigins::Any => layer.allow_origin(Any),
            CorsOrigins::Explicit(origins) => {
                layer.allow_origin(origins.iter().map(|o| o.0.clone()).collect::<Vec<_>>())
            }
        };
        match self.max_age {
            Some(duration) => layer.max_age(duration),
            None => layer,
        }
    }
}
/// Which native forwarding header family the application trusts.
#[derive(Clone, Copy)]
#[must_use]
pub enum ForwardedHeader {
    /// RFC 7239 Forwarded values, selected explicitly.
    Standard,
    /// X-Forwarded-For/Proto/Host deployed by an explicitly selected proxy.
    Legacy,
}
/// Trusted transport peers and an explicit maximum forwarded-hop count.
#[derive(Clone)]
#[must_use]
pub struct TrustedForwarding {
    /// Prepared exact peers; matching arbitrary headers does not confer trust.
    peers: Arc<[IpAddr]>,
    /// Maximum values allowed before parsing transport metadata.
    hops: NonZeroUsize,
    /// Explicit deployed header family.
    header: ForwardedHeader,
}
/// Validated effective transport metadata from the trusted suffix of a proxy chain.
#[derive(Clone, Debug)]
#[must_use]
pub struct ForwardedTransport {
    /// First untrusted client at the edge of the verified proxy suffix.
    pub client: IpAddr,
    /// Checked HTTP(S) scheme, when supplied by the trusted proxy.
    pub scheme: Option<Arc<str>>,
    /// Checked native host authority, when supplied by the trusted proxy.
    pub authority: Option<http::uri::Authority>,
}
impl TrustedForwarding {
    /// Require a nonempty explicit transport peer set and positive hop bound.
    pub fn new(
        peers: Vec<IpAddr>,
        hops: NonZeroUsize,
        header: ForwardedHeader,
    ) -> Result<Self, WebError> {
        if peers.is_empty() {
            return Err(WebError::Configuration("trusted forwarding peers".into()));
        }
        Ok(Self {
            peers: peers.into(),
            hops,
            header,
        })
    }
    /// Accept forwarding only from a configured native socket peer. Parse from the
    /// right and stop at the first untrusted address, preventing forged left prefixes.
    pub fn resolve(
        &self,
        peer: SocketAddr,
        headers: &HeaderMap,
    ) -> Result<ForwardedTransport, WebError> {
        if !self.peers.contains(&peer.ip()) {
            return Ok(ForwardedTransport {
                client: peer.ip(),
                scheme: None,
                authority: None,
            });
        }
        let (chain, scheme, authority) = match self.header {
            ForwardedHeader::Legacy => {
                let chain = header_list(headers, "x-forwarded-for", self.hops)?;
                let scheme = header_list(headers, "x-forwarded-proto", self.hops)?
                    .last()
                    .cloned();
                let authority = header_list(headers, "x-forwarded-host", self.hops)?
                    .last()
                    .cloned();
                (chain, scheme, authority)
            }
            ForwardedHeader::Standard => {
                let entries = header_list(headers, "forwarded", self.hops)?;
                let mut chain = Vec::new();
                let mut scheme = None;
                let mut authority = None;
                for entry in entries {
                    let mut address = None;
                    let mut entry_scheme = None;
                    let mut entry_host = None;
                    for pair in entry.split(';') {
                        let (name, value) = pair
                            .trim()
                            .split_once('=')
                            .ok_or_else(|| WebError::Metadata("Forwarded pair".into()))?;
                        let value = forwarded_value(value)?;
                        match name.to_ascii_lowercase().as_str() {
                            "for" => {
                                if address.replace(value.to_owned()).is_some() {
                                    return Err(WebError::Metadata(
                                        "duplicate Forwarded for".into(),
                                    ));
                                }
                            }
                            "proto" => {
                                if entry_scheme.replace(value.to_owned()).is_some() {
                                    return Err(WebError::Metadata(
                                        "duplicate Forwarded proto".into(),
                                    ));
                                }
                            }
                            "host" => {
                                if entry_host.replace(value.to_owned()).is_some() {
                                    return Err(WebError::Metadata(
                                        "duplicate Forwarded host".into(),
                                    ));
                                }
                            }
                            "by" => {}
                            _ => {}
                        }
                    }
                    chain.push(address.ok_or_else(|| WebError::Metadata("Forwarded for".into()))?);
                    scheme = entry_scheme;
                    authority = entry_host;
                }
                (chain, scheme, authority)
            }
        };
        let mut client = peer.ip();
        for address in chain.iter().rev() {
            if !self.peers.contains(&client) {
                break;
            }
            client = address
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .unwrap_or(address)
                .parse::<IpAddr>()
                .or_else(|_| address.parse::<SocketAddr>().map(|a| a.ip()))
                .map_err(|_| WebError::Metadata("forwarded client address".into()))?;
        }
        let scheme = scheme
            .map(|value| {
                if matches!(value.as_str(), "http" | "https") {
                    Ok(Arc::from(value))
                } else {
                    Err(WebError::Metadata("forwarded scheme".into()))
                }
            })
            .transpose()?;
        let authority = authority
            .map(|value| {
                if value.contains('@') {
                    return Err(WebError::Metadata("forwarded host".into()));
                }
                value
                    .parse()
                    .map_err(|_| WebError::Metadata("forwarded host".into()))
            })
            .transpose()?;
        Ok(ForwardedTransport {
            client,
            scheme,
            authority,
        })
    }
}
/// Strict RFC token/quoted value handling. Supported address/host values never
/// need quoted escapes; rejecting them prevents ambiguous proxy-chain parsing.
fn forwarded_value(value: &str) -> Result<&str, WebError> {
    let value = value.trim();
    let value = if value.starts_with('"') {
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .ok_or_else(|| WebError::Metadata("Forwarded quoted value".into()))?
    } else {
        value
    };
    if value.is_empty()
        || value.chars().any(|character| {
            character.is_ascii_control()
                || character.is_whitespace()
                || matches!(character, '"' | '\\' | ';' | ',')
        })
    {
        return Err(WebError::Metadata("Forwarded value".into()));
    }
    Ok(value)
}
/// Read a bounded comma-separated native header list before interpreting addresses.
fn header_list(
    headers: &HeaderMap,
    name: &'static str,
    maximum: NonZeroUsize,
) -> Result<Vec<String>, WebError> {
    let mut values = Vec::new();
    for header in headers.get_all(name) {
        let header = header
            .to_str()
            .map_err(|_| WebError::Metadata(name.into()))?;
        for value in header.split(',') {
            if values.len() == maximum.get() || value.trim().is_empty() {
                return Err(WebError::Metadata(name.into()));
            }
            values.push(value.trim().to_owned());
        }
    }
    Ok(values)
}
/// Prepared HSTS policy. Max age is explicitly supplied in whole seconds.
#[derive(Clone, Copy)]
#[must_use]
pub struct HstsPolicy {
    /// Retention selected by application policy, not an extension default.
    seconds: NonZeroU64,
    /// Whether subordinate domains are included.
    include_subdomains: bool,
    /// Whether the application deliberately requests browser preload eligibility.
    preload: bool,
}
impl HstsPolicy {
    /// Select HSTS directives explicitly; preload requires includeSubDomains.
    pub fn new(
        seconds: NonZeroU64,
        include_subdomains: bool,
        preload: bool,
    ) -> Result<Self, WebError> {
        if preload && !include_subdomains {
            return Err(WebError::Configuration(
                "HSTS preload requires subdomains".into(),
            ));
        }
        Ok(Self {
            seconds,
            include_subdomains,
            preload,
        })
    }
    /// Produce a native validated protocol field; apply only to selected secure transport.
    pub fn header(self) -> Result<HeaderValue, WebError> {
        let mut value = format!("max-age={}", self.seconds);
        if self.include_subdomains {
            value.push_str("; includeSubDomains");
        }
        if self.preload {
            value.push_str("; preload");
        }
        HeaderValue::from_str(&value).map_err(|_| WebError::Configuration("HSTS header".into()))
    }
}
/// Explicit browser header defaults. Selecting this policy emits the standardized
/// nosniff value plus the caller's referrer and frame policies.
#[derive(Clone)]
#[must_use]
pub struct BrowserDefaults {
    /// Native Referrer-Policy supplied by application policy.
    pub referrer: HeaderValue,
    /// Native X-Frame-Options supplied by application policy.
    pub frame: HeaderValue,
}
/// POST override metadata; native request mutation happens before route selection.
#[derive(Clone)]
#[must_use]
pub struct MethodOverride {
    /// Explicit allowlist of target methods, excluding GET/HEAD/OPTIONS.
    allowed: Arc<[Method]>,
    /// Header checked before form fallback.
    header: HeaderName,
    /// Bounded compatible form fallback; None disables body parsing.
    form: Option<FormPolicy>,
    /// Caller-selected form field identity.
    field: Arc<str>,
}
impl MethodOverride {
    /// Validate mutation-only override policy before serving.
    pub fn new(
        allowed: Vec<Method>,
        header: HeaderName,
        form: Option<FormPolicy>,
        field: impl Into<Arc<str>>,
    ) -> Result<Self, WebError> {
        let field = field.into();
        if allowed.is_empty()
            || allowed.iter().any(|m| {
                matches!(
                    *m,
                    Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE | Method::CONNECT
                )
            })
            || field.is_empty()
        {
            return Err(WebError::Configuration("method override".into()));
        }
        Ok(Self {
            allowed: allowed.into(),
            header,
            form,
            field,
        })
    }
    /// Resolve the selected method with header precedence. Form fallback consumes
    /// only URL-encoded media through the existing compatible Request owner; other
    /// media remains unread, and duplicate/malformed native content types fail.
    pub async fn resolve(
        &self,
        request: &Request,
    ) -> Result<Option<Method>, effect_server::ServerError> {
        if request.method() != Method::POST {
            return Ok(None);
        }
        let headers = request
            .headers()
            .get_all(&self.header)
            .iter()
            .collect::<Vec<_>>();
        let selected = match headers.as_slice() {
            [] => None,
            [value] => Some(
                value
                    .to_str()
                    .map_err(|_| effect_server::ServerError::Invalid("method override".into()))?
                    .to_owned(),
            ),
            _ => {
                return Err(effect_server::ServerError::Duplicate(
                    "method override".into(),
                ));
            }
        };
        let selected = match (selected, self.form) {
            (Some(value), _) => Some(value),
            (None, None) => None,
            (None, Some(policy)) => {
                let content_type =
                    request.singleton_header::<effect_server::native::headers::ContentType>()?;
                if !content_type.is_some_and(|value| {
                    value.to_string().split(';').next().is_some_and(|media| {
                        media
                            .trim()
                            .eq_ignore_ascii_case("application/x-www-form-urlencoded")
                    })
                }) {
                    return Ok(None);
                }
                let form = request.read_form(policy).await?;
                match form
                    .values
                    .get(self.field.as_ref())
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    [] => None,
                    [value] => Some(value.clone()),
                    _ => {
                        return Err(effect_server::ServerError::Duplicate(
                            self.field.to_string(),
                        ));
                    }
                }
            }
        };
        selected
            .map(|value| {
                let method = Method::from_bytes(value.as_bytes())
                    .map_err(|_| effect_server::ServerError::Invalid("method override".into()))?;
                if self.allowed.contains(&method) {
                    Ok(method)
                } else {
                    Err(effect_server::ServerError::Invalid(
                        "method override allowlist".into(),
                    ))
                }
            })
            .transpose()
    }
}
/// Application-owned request guard; `false` is an explicit rejection decision,
/// while native/uncertain failures stay in E and are never treated as denial.
pub type SecurityGuard<E, R> = Arc<dyn Fn(Request) -> Effect<bool, E, R> + Send + Sync>;
/// Native static asset selection; the native service retains file/range ownership.
#[derive(Clone)]
#[must_use]
pub struct StaticAssets {
    /// Native route prefix, validated by Server registration.
    pub prefix: Arc<str>,
    /// Root path supplied by the application.
    pub directory: PathBuf,
    /// Whether native directory index handling is deliberately enabled.
    pub append_index: bool,
}
impl StaticAssets {
    /// Obtain Tower's native static service without starting I/O or another server.
    #[must_use]
    pub fn native(&self) -> tower_http::services::ServeDir {
        tower_http::services::ServeDir::new(&self.directory)
            .append_index_html_on_directories(self.append_index)
    }
}
/// Retained environment-selected nonce callback; each invocation returns the original typed Effect.
pub type NonceFactory<E, R> = Arc<dyn Fn() -> Effect<ResponseNonce, E, R> + Send + Sync>;

/// Reusable security description. Callbacks are inert and execute in the selected
/// environment at their documented native stages; no Identity defaults are acquired.
#[must_use]
pub struct Security<E: Send + 'static, R: Send + Sync + 'static> {
    /// Explicitly selected standardized browser headers.
    pub browser_defaults: Option<BrowserDefaults>,
    /// Checked CSP metadata, bound to a response nonce at execution.
    pub csp: Option<Csp>,
    /// Environment-selected fresh nonce generation; shares one value with renderer.
    pub nonce: Option<NonceFactory<E, R>>,
    /// Transport peer/hop policy applied first.
    pub forwarded_from: Option<TrustedForwarding>,
    /// Native CORS/preflight policy applied before authentication.
    pub cors: Option<CorsPolicy>,
    /// Caller-owned matched-endpoint authentication before request projection.
    /// Native short circuits retain the selected CORS/static/native-layer policy.
    pub authentication: Option<SecurityGuard<E, R>>,
    /// Authorization stage executed before output-cache lookup.
    pub authorization: Option<SecurityGuard<E, R>>,
    /// Selected CSRF guard, following authentication and before application writes.
    pub antiforgery: Option<SecurityGuard<E, R>>,
    /// Native response compression layer; disabled unless deliberately selected.
    pub compression: Option<tower_http::compression::CompressionLayer>,
    /// Native static service selection, with outer response security intact.
    pub static_files: Option<StaticAssets>,
    /// Encoded request body limit shared with Server/native request decoding.
    pub body_limit: Option<BodyLimit>,
    /// POST-only checked mutation override before routing.
    pub method_override: Option<MethodOverride>,
    /// HSTS applied only on checked secure transport.
    pub hsts: Option<HstsPolicy>,
    /// Additional validated native response fields in declaration order.
    pub headers: HeaderMap,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Security<E, R> {
    /// Construct an inert, explicitly empty policy; no hidden browser/runtime defaults.
    pub fn new() -> Self {
        Self {
            browser_defaults: None,
            csp: None,
            nonce: None,
            forwarded_from: None,
            cors: None,
            authentication: None,
            authorization: None,
            antiforgery: None,
            compression: None,
            static_files: None,
            body_limit: None,
            method_override: None,
            hsts: None,
            headers: HeaderMap::new(),
        }
    }
    /// Validate nonce capability, native static prefix and prereoute form/body-limit
    /// consistency before accepting traffic, without running request callbacks.
    pub fn validate(self) -> Result<Self, WebError> {
        if self.csp.as_ref().is_some_and(Csp::needs_nonce) && self.nonce.is_none() {
            return Err(WebError::Configuration("CSP nonce capability".into()));
        }
        if let (Some(limit), Some(policy)) = (
            self.body_limit,
            self.method_override
                .as_ref()
                .and_then(|override_policy| override_policy.form),
        ) && policy.body.bytes() > limit.bytes()
        {
            return Err(WebError::Configuration(
                "method override form exceeds request body limit".into(),
            ));
        }
        if let Some(assets) = &self.static_files {
            effect_server::Route::literal(&assets.prefix)
                .map_err(|_| WebError::Configuration("static asset prefix".into()))?;
        }
        Ok(self)
    }
    /// Apply outer response fields to finite and short-circuit replies alike. Native
    /// streaming response wrapping uses the same metadata at the Server boundary.
    pub fn wrap_headers(
        &self,
        headers: &mut HeaderMap,
        nonce: Option<&ResponseNonce>,
        secure_transport: bool,
    ) -> Result<(), WebError> {
        let mut replaced = std::collections::BTreeSet::new();
        for (name, value) in &self.headers {
            if replaced.insert(name.as_str()) {
                headers.remove(name);
            }
            headers.append(name.clone(), value.clone());
        }
        if let Some(defaults) = &self.browser_defaults {
            headers.insert(
                "x-content-type-options",
                HeaderValue::from_static("nosniff"),
            );
            headers.insert("referrer-policy", defaults.referrer.clone());
            headers.insert("x-frame-options", defaults.frame.clone());
        }
        if let Some(csp) = &self.csp {
            headers.insert("content-security-policy", csp.header(nonce)?);
        }
        if secure_transport && let Some(hsts) = self.hsts {
            headers.insert(http::header::STRICT_TRANSPORT_SECURITY, hsts.header()?);
        }
        Ok(())
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Default for Security<E, R> {
    /// No policy is selected until the application supplies it.
    fn default() -> Self {
        Self::new()
    }
}
