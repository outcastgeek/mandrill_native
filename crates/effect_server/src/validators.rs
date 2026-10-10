//! Native HTTP validators and RFC 9110 precondition precedence.

use crate::{Request, ServerError};
use headers::{
    ETag, HeaderMapExt, IfMatch, IfModifiedSince, IfNoneMatch, IfUnmodifiedSince, LastModified,
};
use http::{HeaderMap, Method, StatusCode};
use std::time::SystemTime;

/// Validated dynamic body-bearing HTTP status. Body-forbidden codes cannot construct it.
#[derive(Clone, Copy, Debug)]
pub struct BodyStatus(StatusCode);
impl BodyStatus {
    /// Validate dynamic status and reject informational/204/205/304 body combinations.
    pub fn new(status: StatusCode) -> Result<Self, ServerError> {
        if status.is_informational()
            || matches!(
                status,
                StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT | StatusCode::NOT_MODIFIED
            )
        {
            Err(ServerError::Invalid("body-forbidden status".into()))
        } else {
            Ok(Self(status))
        }
    }
    /// Named HTTP success status usable without dynamic validation.
    #[allow(
        non_upper_case_globals,
        reason = "HTTP status spelling used by the public DSL"
    )]
    pub const Ok: Self = Self(StatusCode::OK);
    /// Named created-resource status.
    #[allow(
        non_upper_case_globals,
        reason = "HTTP status spelling used by the public DSL"
    )]
    pub const Created: Self = Self(StatusCode::CREATED);
    /// Return the validated native status.
    #[must_use]
    pub const fn native(self) -> StatusCode {
        self.0
    }
}
/// Body-bearing status spelling used by the consumer DSL.
pub type HttpStatus = BodyStatus;
/// Resource existence and validators. Missing resource and missing tag are distinct.
#[derive(Clone, Debug)]
pub struct ResourceValidators {
    /// Whether the selected current resource exists.
    pub exists: bool,
    /// Native ETag retains weak/strong semantics and checked quoted syntax.
    pub etag: Option<ETag>,
    /// Native date comparisons use HTTP second precision.
    pub last_modified: Option<SystemTime>,
}
/// A precondition decision made before body production or mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionalOutcome {
    /// Preconditions allow application work.
    Proceed,
    /// GET/HEAD representation is unchanged; no renderer should run.
    NotModified,
    /// Conditions reject retrieval or mutation.
    PreconditionFailed,
}
impl ResourceValidators {
    /// Apply validators to staged headers without producing the body.
    pub fn write(&self, headers: &mut HeaderMap) {
        if let Some(etag) = &self.etag {
            headers.typed_insert(etag.clone());
        }
        if let Some(modified) = self.last_modified {
            headers.typed_insert(LastModified::from(modified));
        }
    }
    /// Evaluate If-Match, If-Unmodified-Since, If-None-Match, If-Modified-Since in protocol order.
    /// Malformed tag syntax is typed; invalid dates are ignored as RFC 9110 requires.
    pub fn evaluate(&self, request: &Request) -> Result<ConditionalOutcome, ServerError> {
        validate_tag_condition(request, http::header::IF_MATCH)?;
        validate_tag_condition(request, http::header::IF_NONE_MATCH)?;
        let matched = request.typed_header::<IfMatch>()?;
        if let Some(condition) = matched {
            let passes = self.exists
                && (condition.is_any()
                    || self
                        .etag
                        .as_ref()
                        .is_some_and(|tag| condition.precondition_passes(tag)));
            if !passes {
                return Ok(ConditionalOutcome::PreconditionFailed);
            }
        } else if let (Ok(Some(condition)), Some(modified)) = (
            request.typed_header::<IfUnmodifiedSince>(),
            self.last_modified,
        ) && self.exists
            && !condition.precondition_passes(modified)
        {
            return Ok(ConditionalOutcome::PreconditionFailed);
        }
        let retrieval = matches!(*request.method(), Method::GET | Method::HEAD);
        if let Some(condition) = request.typed_header::<IfNoneMatch>()? {
            let wildcard = request
                .headers()
                .get_all(http::header::IF_NONE_MATCH)
                .iter()
                .any(|value| value.as_bytes() == b"*");
            let matches = self.exists
                && (wildcard
                    || self
                        .etag
                        .as_ref()
                        .is_some_and(|tag| !condition.precondition_passes(tag)));
            if matches {
                return Ok(if retrieval {
                    ConditionalOutcome::NotModified
                } else {
                    ConditionalOutcome::PreconditionFailed
                });
            }
        } else if retrieval
            && let (Ok(Some(condition)), Some(modified)) = (
                request.typed_header::<IfModifiedSince>(),
                self.last_modified,
            )
            && self.exists
            && !condition.is_modified(modified)
        {
            return Ok(ConditionalOutcome::NotModified);
        }
        Ok(ConditionalOutcome::Proceed)
    }
}

/// Validate every entity-tag list member using the native ETag parser, retaining quoted commas.
fn validate_tag_condition(request: &Request, name: http::HeaderName) -> Result<(), ServerError> {
    let mut tokens = Vec::new();
    for value in request.headers().get_all(&name) {
        let value = value
            .to_str()
            .map_err(|_| ServerError::Invalid(name.to_string()))?;
        let mut quoted = false;
        let mut start = 0;
        for (index, byte) in value.bytes().enumerate() {
            if byte == b'"' {
                quoted = !quoted;
            } else if byte == b',' && !quoted {
                tokens.push(value.get(start..index).unwrap_or("").trim().to_owned());
                start = index.saturating_add(1);
            }
        }
        if quoted {
            return Err(ServerError::Invalid(name.to_string()));
        }
        tokens.push(value.get(start..).unwrap_or("").trim().to_owned());
    }
    if tokens.iter().any(|value| value == "*") && tokens.len() != 1 {
        return Err(ServerError::Invalid(name.to_string()));
    }
    for value in tokens {
        if value != "*" {
            value
                .parse::<ETag>()
                .map_err(|_| ServerError::Invalid(name.to_string()))?;
        }
    }
    Ok(())
}
