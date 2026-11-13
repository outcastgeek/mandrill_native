//! Access responses.
//!
//! The response contracts: caller-supplied renderers, the redirect response, the
//! translated-status response, the raw-key response and the managed-surface guard
//! options.
//!
//! # Response behavior
//!
//! - A redirect response resolves the caller's path builder; an absent builder or a
//!   blank target renders the plain status-text fallback instead of redirecting.
//! - The resolved target makes a relative target absolute against the request's
//!   escaped path (cleaning it and keeping the trailing-slash and query rules), and the
//!   `Location` header is percent-escaped for non-ASCII bytes. A hostless slash path is
//!   relative for this rule, while the parser's decodable components keep their
//!   percent-validation: malformed escapes in the authority, path or fragment leave the
//!   target verbatim, and a raw query is carried through without decoding.
//! - The `GET` response carries the short HTML anchor body, authored here with Maud and
//!   marked as rendered HTML, and `GET`/`HEAD` carry the HTML content type; every other
//!   method sends the status and location without a body.
//! - The translated-status response resolves the message key (trimmed; a blank key
//!   yields the empty message), applies the caller's translation port, and renders
//!   through the caller's message renderer. An absent renderer renders the plain
//!   status-text fallback.
//! - The raw-key response passes the key unchanged to the caller's key renderer; an
//!   absent renderer renders the plain status-text fallback.
//! - The fallback writes the status text and a newline as a text/plain body with the
//!   `nosniff` header.
//!
//! # Adaptation recorded
//!
//! Renderers write through the response boundary; this owner's renderers
//! return an [`Effect`] of a [`Reply`], which is the local response contract and keeps
//! the renderer's own work deferred to response production. The key
//! renderer accepts variadic arguments; no CMS call site passes any, so the port
//! keeps the parameters the source actually supplies.
//!
//! # Rendering boundary
//!
//! The redirect's anchor is Maud-authored markup serialized into the reply; no HTML is
//! assembled by string concatenation. The caller's own renderers remain the only
//! source of page markup, and an absent renderer keeps the plain status-text fallback.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::{BodyStatus, HtmlContent, Reply, Request, reply};
use http::{HeaderName, HeaderValue, Method, StatusCode};
use maud::html;

use crate::localization::TranslateKeyFunc;

/// Resolves the redirect target for one request.
pub type AccessPathBuilder = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Renders one status and localized message.
pub type AccessMessageRenderer<E, R> =
    Arc<dyn Fn(&Request, StatusCode, String) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static>;

/// Renders one status and message key.
pub type AccessKeyRenderer<E, R> =
    Arc<dyn Fn(&Request, StatusCode, String) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static>;

/// Renders one access response for a request.
pub type AccessResponse<E, R> =
    Arc<dyn Fn(&Request) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static>;

/// Configures the managed-surface access guard responses.
pub struct ManagedSurfaceAccessGuardOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// Response for an unauthenticated principal.
    pub redirect_unauthenticated: Option<AccessResponse<E, R>>,
    /// Response for a denied principal.
    pub render_forbidden: Option<AccessResponse<E, R>>,
    /// Key-based response for a denied principal.
    pub render_forbidden_key: Option<AccessResponse<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for ManagedSurfaceAccessGuardOptions<E, R>
{
    fn default() -> Self {
        Self {
            redirect_unauthenticated: None,
            render_forbidden: None,
            render_forbidden_key: None,
        }
    }
}

/// The plain status-text fallback.
pub(super) fn fallback_status_reply<E, R>(status: BodyStatus) -> Reply<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let text = status
        .native()
        .canonical_reason()
        .unwrap_or_default()
        .to_owned();
    // The body is the status text followed by a newline.
    reply! {
        status(status);
        header(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff")
        );
        text(format!("{text}\n"));
    }
}

/// Escape every non-ASCII byte of a redirect target as percent-encoded hex.
///
/// Shared with the routing owner so a body-forbidden redirection status emits the same
/// location value as a body-bearing one.
pub(crate) fn hex_escape_non_ascii(target: &str) -> String {
    let mut escaped = String::with_capacity(target.len());
    for byte in target.bytes() {
        if byte < 0x80 {
            escaped.push(char::from(byte));
        } else {
            escaped.push('%');
            escaped.push(hex_digit(byte / 16));
            escaped.push(hex_digit(byte % 16));
        }
    }
    escaped
}

/// The uppercase hexadecimal digit for a nibble.
fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        _ => char::from(b'A' + nibble - 10),
    }
}

/// Resolve one translation key through the caller's port.
fn translate(
    translate_key: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    key: &str,
) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    match translate_key {
        Some(translate) => translate(request, key),
        None => key.to_owned(),
    }
}

/// Redirect-target resolution.
///
/// A relative target is made absolute against the request's escaped path, cleaned
/// (keeping a trailing slash) and re-appended with its query. A parsable target that
/// carries a scheme or an authority host is used verbatim; a hostless slash path is
/// relative and is normalized by this same rule, and an unparsable target is left as the
/// caller supplied it.
///
/// Shared with the routing owner, whose query-preserving and mutation-result redirects
/// resolve their targets through this same rule for every redirection status.
pub(crate) fn resolved_redirect_target(request: &Request, target: &str) -> String {
    if !is_parsable_url(target) || has_scheme_or_host(target) {
        return target.to_owned();
    }
    let escaped = request.uri().path();
    let oldpath = if escaped.is_empty() { "/" } else { escaped };
    let mut url = target.to_owned();
    if !url.starts_with('/') {
        let olddir = match oldpath.rfind('/') {
            Some(index) => oldpath.get(..=index).unwrap_or_default(),
            None => "",
        };
        url = format!("{olddir}{url}");
    }
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_owned(), format!("?{query}")),
        None => (url, String::new()),
    };
    let trailing = path.ends_with('/');
    let mut path = clean_path(&path);
    if trailing && !path.ends_with('/') {
        path.push('/');
    }
    format!("{path}{query}")
}

/// Whether the target parses as a URL at all.
///
/// A target the URL parser rejects is kept exactly as the caller wrote it.
/// Only the parse failures observable here are reproduced: ASCII control characters,
/// a scheme-less relative first path segment containing a colon, and malformed percent
/// escapes in the authority, path or fragment, and invalid raw userinfo. A raw query is kept undecoded by the
/// parser, so an escape there is not a parse failure; the query text is carried through
/// unchanged.
fn is_parsable_url(target: &str) -> bool {
    if target.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return false;
    }
    let without_fragment = match target.split_once('#') {
        Some((head, fragment)) => {
            if !valid_percent_escapes(fragment) {
                return false;
            }
            head
        }
        None => target,
    };
    let (scheme, rest) = split_scheme(without_fragment);
    let without_query = match rest.split_once('?') {
        Some((head, _query)) => head,
        None => rest,
    };
    if scheme.is_empty()
        && !without_query.starts_with('/')
        && without_query
            .split('/')
            .next()
            .is_some_and(|segment| segment.contains(':'))
    {
        return false;
    }
    if authority_applies(scheme, without_query) {
        let authority_and_path = without_query.get(2..).unwrap_or_default();
        let (authority, path) = match authority_and_path.split_once('/') {
            Some((authority, path)) => (authority, path),
            None => (authority_and_path, ""),
        };
        let valid_userinfo = authority.rsplit_once('@')
            .is_none_or(|(userinfo, _host)| crate::url_text::valid_raw_userinfo(userinfo));
        valid_userinfo && valid_percent_escapes(authority) && valid_percent_escapes(path)
    } else {
        valid_percent_escapes(without_query)
    }
}

/// Whether the target carries its own scheme or authority host.
///
/// Only targets with both an empty scheme and an empty authority host are normalized.
/// A scheme is a leading letter followed by letters, digits, `+`, `-` or `.` and a
/// colon. Without a scheme, a leading `//` introduces an authority unless the target
/// starts with `///`, which is a hostless rooted path. User information is not part of
/// the host, so `//@/path` is hostless.
fn has_scheme_or_host(target: &str) -> bool {
    let without_fragment = match target.split_once('#') {
        Some((head, _fragment)) => head,
        None => target,
    };
    let (scheme, rest) = split_scheme(without_fragment);
    if !scheme.is_empty() {
        return true;
    }
    let without_query = match rest.split_once('?') {
        Some((head, _query)) => head,
        None => rest,
    };
    if !authority_applies(scheme, without_query) {
        return false;
    }
    let authority_and_path = without_query.get(2..).unwrap_or_default();
    let authority = match authority_and_path.split_once('/') {
        Some((authority, _path)) => authority,
        None => authority_and_path,
    };
    let host = match authority.rfind('@') {
        Some(index) => authority.get(index + 1..).unwrap_or_default(),
        None => authority,
    };
    !host.is_empty()
}

/// Whether `rest`, after the scheme and query, introduces an authority.
fn authority_applies(scheme: &str, rest: &str) -> bool {
    rest.starts_with("//") && (!scheme.is_empty() || !rest.starts_with("///"))
}

/// Split a parsed scheme from the rest of the reference.
///
/// The scheme is a leading ASCII letter followed by letters, digits, `+`, `-` or `.`
/// and a colon. Anything else leaves an empty scheme and the whole reference as the
/// rest.
fn split_scheme(raw: &str) -> (&str, &str) {
    for (index, byte) in raw.bytes().enumerate() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' => {}
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                if index == 0 {
                    return ("", raw);
                }
            }
            b':' => {
                if index == 0 {
                    return ("", raw);
                }
                return (
                    raw.get(..index).unwrap_or_default(),
                    raw.get(index + 1..).unwrap_or_default(),
                );
            }
            _ => return ("", raw),
        }
    }
    ("", raw)
}

/// Whether every percent escape in one decoded component is well formed.
fn valid_percent_escapes(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while let Some(byte) = bytes.get(index) {
        if *byte != b'%' {
            index += 1;
            continue;
        }
        if !bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            || !bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            return false;
        }
        index += 3;
    }
    true
}

/// Clean a path: collapse slashes, drop `.`, resolve inner `..`, keep the
/// rooted/relative distinction and remove a trailing slash.
fn clean_path(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let rooted = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if let Some(last) = parts.last()
                    && *last != ".."
                {
                    parts.pop();
                    continue;
                }
                if !rooted {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let mut cleaned = String::new();
    if rooted {
        cleaned.push('/');
    }
    cleaned.push_str(&parts.join("/"));
    if cleaned.is_empty() {
        return ".".to_owned();
    }
    cleaned
}

/// Build a redirect response.
///
/// An absent path builder or a blank target renders the plain status-text fallback,
/// exactly as specified.
#[must_use]
pub fn new_redirect_access_response<E, R>(
    path: Option<AccessPathBuilder>,
    status: BodyStatus,
) -> AccessResponse<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request: &Request| {
        let path = path.clone();
        let request = request.clone();
        effect_sync! { body: move || {
            let Some(path) = path.as_ref() else {
                return fallback_status_reply(status);
            };
            let target = path(&request);
            if target.is_empty() {
                return fallback_status_reply(status);
            }
            let target = resolved_redirect_target(&request, &target);
            let Ok(location) = HeaderValue::from_bytes(hex_escape_non_ascii(&target).as_bytes())
            else {
                return fallback_status_reply(status);
            };
            // A short HTML body is sent for `GET` (and the HTML content type is set
            // for `GET`/`HEAD`) because older user agents do not follow a bare
            // `Location`. The anchor is authored with Maud; the surrounding whitespace
            // is the status-text terminator, not markup.
            match *request.method() {
                Method::GET => {
                    let status_text = status.native().canonical_reason().unwrap_or_default();
                    let anchor = html! { a href=(target) { (status_text) } }.into_string();
                    reply! {
                        status(status);
                        location(location);
                        html(HtmlContent::trusted(format!("{anchor}.\n\n")));
                    }
                }
                Method::HEAD => reply! {
                    status(status);
                    location(location);
                    html(HtmlContent::trusted(String::new()));
                },
                _ => reply! {
                    status(status);
                    location(location);
                    empty();
                },
            }
        }, }
    })
}

/// Build a translated-status response.
#[must_use]
pub fn new_translated_status_access_response<E, R>(
    status: BodyStatus,
    key: String,
    translate_key: Option<TranslateKeyFunc<Request>>,
    render: Option<AccessMessageRenderer<E, R>>,
) -> AccessResponse<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request: &Request| {
        let Some(render) = render.clone() else {
            return effect_sync! { body: move || fallback_status_reply(status), };
        };
        let translate_key = translate_key.clone();
        let key = key.clone();
        let request = request.clone();
        effect! {
            // Translation is part of the run, not of the description: a port that
            // reads caller state observes it when the response is actually produced.
            let translate_key = translate_key.clone();
            let key = key.clone();
            let message = translate(translate_key.as_ref(), &request, key.as_str());
            let reply = render(&request, status.native(), message)?;
            reply
        }
    })
}

/// Build a raw-key response.
#[must_use]
pub fn new_status_key_access_response<E, R>(
    status: BodyStatus,
    key: String,
    render: Option<AccessKeyRenderer<E, R>>,
) -> AccessResponse<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request: &Request| {
        let Some(render) = render.clone() else {
            return effect_sync! { body: move || fallback_status_reply(status), };
        };
        let key = key.clone();
        let request = request.clone();
        effect! {
            let key = key.clone();
            let reply = render(&request, status.native(), key)?;
            reply
        }
    })
}
