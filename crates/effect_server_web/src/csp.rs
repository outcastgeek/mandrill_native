//! Checked Content Security Policy descriptions and per-response nonce binding.
//!
//! Source spelling follows CSP Level 3. A nonce marker is resolved only for a
//! response, using the same validated nonce inserted into the request extensions
//! for its renderer. `none` is exclusive; append cannot silently replace it.

use crate::WebError;
use http::HeaderValue;
use std::collections::BTreeMap;
use std::sync::Arc;

/// CSP Level 3 directive names; source lists and valueless flags remain distinct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub enum CspDirective {
    /// Default fetch source fallback.
    DefaultSrc,
    /// Script fetch and inline authorization.
    ScriptSrc,
    /// Stylesheet fetch and inline authorization.
    StyleSrc,
    /// Image fetch sources.
    ImgSrc,
    /// Font fetch sources.
    FontSrc,
    /// Fetch/XHR/WebSocket connection sources.
    ConnectSrc,
    /// Media fetch sources.
    MediaSrc,
    /// Object/embed fetch sources.
    ObjectSrc,
    /// Frame fetch sources.
    FrameSrc,
    /// Worker creation sources.
    WorkerSrc,
    /// Ancestors allowed to frame this document.
    FrameAncestors,
    /// Document base URL restriction.
    BaseUri,
    /// Form submission targets.
    FormAction,
    /// Upgrade insecure requests; requires an empty source list.
    UpgradeInsecureRequests,
    /// Block mixed content; requires an empty source list.
    BlockAllMixedContent,
    /// Named sandbox tokens, validated as native CSP sources.
    Sandbox,
    /// CSP violation reporting endpoint URI.
    ReportUri,
    /// CSP reporting group name.
    ReportTo,
}
impl CspDirective {
    /// Return the standardized wire spelling, never an application policy default.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DefaultSrc => "default-src",
            Self::ScriptSrc => "script-src",
            Self::StyleSrc => "style-src",
            Self::ImgSrc => "img-src",
            Self::FontSrc => "font-src",
            Self::ConnectSrc => "connect-src",
            Self::MediaSrc => "media-src",
            Self::ObjectSrc => "object-src",
            Self::FrameSrc => "frame-src",
            Self::WorkerSrc => "worker-src",
            Self::FrameAncestors => "frame-ancestors",
            Self::BaseUri => "base-uri",
            Self::FormAction => "form-action",
            Self::UpgradeInsecureRequests => "upgrade-insecure-requests",
            Self::BlockAllMixedContent => "block-all-mixed-content",
            Self::Sandbox => "sandbox",
            Self::ReportUri => "report-uri",
            Self::ReportTo => "report-to",
        }
    }
    /// Whether this protocol directive accepts no values.
    const fn flag(self) -> bool {
        matches!(
            self,
            Self::UpgradeInsecureRequests | Self::BlockAllMixedContent
        )
    }
}
/// One CSP source. Dynamic native source tokens are validated against delimiter injection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum CspSource {
    /// Exclusive prohibition of every source in this directive.
    None,
    /// Same origin source.
    SelfOrigin,
    /// Bind this directive to the current response nonce.
    Nonce,
    /// Explicit CSP unsafe-inline keyword; selected only by application policy.
    UnsafeInline,
    /// Explicit CSP unsafe-eval keyword.
    UnsafeEval,
    /// Explicit CSP strict-dynamic keyword.
    StrictDynamic,
    /// Validated native scheme/host/hash/sandbox/reporting token.
    Native(CspToken),
}
impl CspSource {
    /// Validate one native token. Keywords/nonces use their typed variants so
    /// `none` cannot evade exclusion checks. No whitespace/semicolon is accepted.
    pub fn native(value: impl Into<Arc<str>>) -> Result<Self, WebError> {
        let value = value.into();
        if value.is_empty()
            || value
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, ';' | ',' | '\r' | '\n'))
            || matches!(
                value.as_ref(),
                "'none'" | "'self'" | "'unsafe-inline'" | "'unsafe-eval'" | "'strict-dynamic'"
            )
            || value.starts_with("'nonce-")
        {
            return Err(WebError::Configuration("CSP source".into()));
        }
        HeaderValue::from_str(&value).map_err(|_| WebError::Configuration("CSP source".into()))?;
        Ok(Self::Native(CspToken(value)))
    }
}
/// Validated native token. Its private payload prevents bypassing delimiter and
/// keyword checks through direct construction of the public source enum.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct CspToken(Arc<str>);
/// Checked complete source list; `none` is never compatible with another source.
#[derive(Clone, Debug)]
#[must_use]
pub struct CspSources {
    /// Immutable source metadata shared by definitions.
    values: Arc<[CspSource]>,
}
impl CspSources {
    /// Validate a source list before native serving. Empty lists are reserved for flags.
    pub fn new(values: Vec<CspSource>) -> Result<Self, WebError> {
        if values.len() > 1 && values.contains(&CspSource::None) {
            return Err(WebError::Configuration("exclusive CSP none".into()));
        }
        Ok(Self {
            values: values.into(),
        })
    }
    /// Select the standardized same-origin keyword.
    pub fn self_origin() -> Self {
        Self {
            values: vec![CspSource::SelfOrigin].into(),
        }
    }
    /// Select one per-response nonce source; no nonce is generated at construction.
    pub fn nonce() -> Self {
        Self {
            values: vec![CspSource::Nonce].into(),
        }
    }
    /// Select the exclusive CSP none keyword.
    pub fn none() -> Self {
        Self {
            values: vec![CspSource::None].into(),
        }
    }
    /// Empty values for a valueless protocol directive.
    pub fn flag() -> Self {
        Self {
            values: Vec::new().into(),
        }
    }
}
/// A validated response nonce. The application supplies entropy through an Effect;
/// this type validates CSP base64-value syntax without claiming entropy strength.
#[derive(Clone)]
#[must_use]
pub struct ResponseNonce(Arc<str>);
impl std::fmt::Debug for ResponseNonce {
    /// Never expose the current nonce through generic diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResponseNonce([redacted])")
    }
}
impl ResponseNonce {
    /// Accept CSP base64/base64url characters and trailing padding only.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, WebError> {
        let value = value.into();
        let unpadded = value.trim_end_matches('=');
        if unpadded.is_empty()
            || value.len() - unpadded.len() > 2
            || !unpadded
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'-' | b'_'))
        {
            return Err(WebError::Configuration("response nonce".into()));
        }
        Ok(Self(value))
    }
    /// Borrow the nonce for an explicitly selected renderer, preserving header identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Reusable CSP metadata. No random values or headers are produced during construction.
#[derive(Clone)]
#[must_use]
pub struct Csp {
    /// Ordered validated directive map.
    directives: BTreeMap<CspDirective, CspSources>,
}
impl Csp {
    /// Start an explicitly empty policy; applications add their intended directives.
    pub fn new() -> Self {
        Self {
            directives: BTreeMap::new(),
        }
    }
    /// Insert a directive once. Replacing an existing directive must be explicit.
    pub fn directive(
        mut self,
        directive: CspDirective,
        sources: CspSources,
    ) -> Result<Self, WebError> {
        Self::check(directive, &sources)?;
        if self.directives.insert(directive, sources).is_some() {
            return Err(WebError::Configuration("duplicate CSP directive".into()));
        }
        Ok(self)
    }
    /// Replace a complete directive explicitly, including replacement of `none`.
    pub fn replace(
        mut self,
        directive: CspDirective,
        sources: CspSources,
    ) -> Result<Self, WebError> {
        Self::check(directive, &sources)?;
        self.directives.insert(directive, sources);
        Ok(self)
    }
    /// Append sources without changing exclusivity or flag semantics.
    pub fn append(
        mut self,
        directive: CspDirective,
        sources: CspSources,
    ) -> Result<Self, WebError> {
        Self::check(directive, &sources)?;
        let mut combined = self
            .directives
            .get(&directive)
            .map(|s| s.values.to_vec())
            .unwrap_or_default();
        combined.extend(sources.values.iter().cloned());
        let combined = CspSources::new(combined)?;
        Self::check(directive, &combined)?;
        self.directives.insert(directive, combined);
        Ok(self)
    }
    /// Validate directive/value compatibility, including nonce-bearing directives.
    fn check(directive: CspDirective, sources: &CspSources) -> Result<(), WebError> {
        if (directive != CspDirective::Sandbox && directive.flag() != sources.values.is_empty())
            || (sources.values.contains(&CspSource::Nonce)
                && !matches!(directive, CspDirective::ScriptSrc | CspDirective::StyleSrc))
        {
            return Err(WebError::Configuration(format!(
                "CSP {} values",
                directive.name()
            )));
        }
        Ok(())
    }
    /// Whether response execution must acquire a nonce before emitting this policy.
    #[must_use]
    pub fn needs_nonce(&self) -> bool {
        self.directives
            .values()
            .any(|s| s.values.contains(&CspSource::Nonce))
    }
    /// Bind a validated nonce at response time; missing required nonce is typed failure.
    pub fn header(&self, nonce: Option<&ResponseNonce>) -> Result<HeaderValue, WebError> {
        let mut output = String::new();
        for (directive, sources) in &self.directives {
            if !output.is_empty() {
                output.push_str("; ");
            }
            output.push_str(directive.name());
            for source in sources.values.iter() {
                output.push(' ');
                match source {
                    CspSource::None => output.push_str("'none'"),
                    CspSource::SelfOrigin => output.push_str("'self'"),
                    CspSource::Nonce => {
                        let nonce = nonce
                            .ok_or_else(|| WebError::Configuration("missing CSP nonce".into()))?;
                        output.push_str("'nonce-");
                        output.push_str(nonce.as_str());
                        output.push('\'');
                    }
                    CspSource::UnsafeInline => output.push_str("'unsafe-inline'"),
                    CspSource::UnsafeEval => output.push_str("'unsafe-eval'"),
                    CspSource::StrictDynamic => output.push_str("'strict-dynamic'"),
                    CspSource::Native(value) => output.push_str(&value.0),
                }
            }
        }
        HeaderValue::from_str(&output).map_err(|_| WebError::Configuration("CSP header".into()))
    }
}
impl Default for Csp {
    /// An empty inert policy with no hidden browser defaults.
    fn default() -> Self {
        Self::new()
    }
}

/// Typed nesting conversion for prepared CSP and checked `csp!` results. No
/// string policy or unchecked conversion is accepted by the security DSL.
pub trait IntoCsp {
    /// Retain runtime validation while composing nested macro descriptions.
    fn into_csp(self) -> Result<Csp, WebError>;
}
impl IntoCsp for Csp {
    /// A prepared checked policy already has validated sources.
    fn into_csp(self) -> Result<Csp, WebError> {
        Ok(self)
    }
}
impl IntoCsp for Result<Csp, WebError> {
    /// Nested dynamic policy errors remain typed.
    fn into_csp(self) -> Result<Csp, WebError> {
        self
    }
}
