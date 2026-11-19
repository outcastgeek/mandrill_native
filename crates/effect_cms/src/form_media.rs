//! Shared media-type recognition and native merged-form initialization.

use crate::validation::trim_unicode_whitespace;
use std::collections::BTreeMap;

fn media_special(byte: u8) -> bool {
    b"()<>@,;:\\\"/[]?=".contains(&byte)
}

fn media_token(byte: u8) -> bool {
    (33..=126).contains(&byte) && !media_special(byte)
}

fn take_media_token(raw: &[u8]) -> (&[u8], &[u8]) {
    let end = raw
        .iter()
        .position(|byte| !media_token(*byte))
        .unwrap_or(raw.len());
    raw.split_at(end)
}

struct MediaParameter<'a> {
    name: &'a [u8],
    value: &'a [u8],
    quoted: bool,
    tail: &'a [u8],
}

impl MediaParameter<'_> {
    fn value_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        let mut raw = self.value.iter().copied().peekable();
        std::iter::from_fn(move || {
            let byte = raw.next()?;
            if self.quoted && byte == b'\\' && raw.peek().is_some_and(|next| media_special(*next)) {
                raw.next()
            } else {
                Some(byte)
            }
        })
    }
}

fn borrow_media_parameter(raw: &[u8]) -> Option<MediaParameter<'_>> {
    let rest = trim_unicode_whitespace(raw).strip_prefix(b";")?;
    let rest = trim_unicode_whitespace(rest);
    let (name, rest) = take_media_token(rest);
    if name.is_empty() {
        return None;
    }
    let rest = trim_unicode_whitespace(rest).strip_prefix(b"=")?;
    let rest = trim_unicode_whitespace(rest);
    if rest.first() != Some(&b'"') {
        let (value, tail) = take_media_token(rest);
        if value.is_empty() {
            return None;
        }
        return Some(MediaParameter { name, value, quoted: false, tail });
    }
    let mut index = 1;
    while let Some(&byte) = rest.get(index) {
        match byte {
            b'"' => return Some(MediaParameter {
                name,
                value: rest.get(1..index)?,
                quoted: true,
                tail: rest.get(index + 1..)?,
            }),
            b'\r' | b'\n' => return None,
            b'\\' => {
                if let Some(&next) = rest.get(index + 1)
                    && media_special(next)
                {
                    index += 2;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn take_media_parameter(raw: &[u8]) -> Option<(Vec<u8>, Vec<u8>, &[u8])> {
    let parameter = borrow_media_parameter(raw)?;
    Some((
        parameter.name.to_ascii_lowercase(),
        parameter.value_bytes().collect(),
        parameter.tail,
    ))
}

/// Parse the media type needed by ParseForm, retaining ordinary parameter errors.
pub(crate) fn form_media_type(raw: &[u8]) -> (bool, Option<&'static str>) {
    if raw.is_empty() {
        return (false, None);
    }
    let end = raw.iter().position(|b| *b == b';').unwrap_or(raw.len());
    let base = trim_unicode_whitespace(&raw[..end]);
    let mut parts = base.split(|b| *b == b'/');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    if first.is_empty()
        || !first.iter().all(|b| media_token(*b))
        || second.is_some_and(|v| v.is_empty() || !v.iter().all(|b| media_token(*b)))
        || parts.next().is_some()
    {
        return (false, Some("mime: invalid media type"));
    }
    let urlencoded = base.eq_ignore_ascii_case(b"application/x-www-form-urlencoded");
    let mut parameters = BTreeMap::new();
    let mut rest = &raw[end..];
    loop {
        rest = trim_unicode_whitespace(rest);
        if rest.is_empty() || rest == b";" {
            return (urlencoded, None);
        }
        let Some((name, value, tail)) = take_media_parameter(rest) else {
            return (urlencoded, Some("mime: invalid media parameter"));
        };
        if parameters
            .get(&name)
            .is_some_and(|previous| previous != &value)
        {
            return (false, Some("mime: duplicate parameter name"));
        }
        parameters.insert(name, value);
        rest = tail;
    }
}

pub(crate) fn recognizes_urlencoded(raw: &[u8]) -> bool {
    let end = raw
        .iter()
        .position(|byte| *byte == b';')
        .unwrap_or(raw.len());
    let base = trim_unicode_whitespace(&raw[..end]);
    if !base.eq_ignore_ascii_case(b"application/x-www-form-urlencoded") {
        return false;
    }
    let mut parameters: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut rest = &raw[end..];
    loop {
        rest = trim_unicode_whitespace(rest);
        if rest.is_empty() {
            return true;
        }
        let Some((name, value, tail)) = take_media_parameter(rest) else {
            return true;
        };
        if let Some(previous) = parameters.get(&name)
            && previous != &value
        {
            return false;
        }
        parameters.insert(name, value);
        rest = tail;
    }
}

/// Check the meaning of URL-encoded Content-Type on the prepared path using the
/// admitted header bytes. Compare previous parameters in place to preserve
/// duplicate-name behavior without a detached parameter map.
pub(crate) fn recognizes_prepared_urlencoded(raw: &[u8]) -> bool {
    let (base, parameters) = raw.split_at(
        raw.iter().position(|byte| *byte == b';').unwrap_or(raw.len()),
    );
    if !trim_unicode_whitespace(base).eq_ignore_ascii_case(b"application/x-www-form-urlencoded") {
        return false;
    }
    !matches!(prepared_media_parameters(parameters), Err(MediaParameterIssue::Duplicate))
}

/// Inspect prepared header bytes with the same syntax and duplicate rules as
/// `form_media_type`, using borrowed parameter names and decoded value iterators.
pub(crate) fn prepared_form_media_type(raw: &[u8]) -> (bool, Option<&'static str>) {
    if raw.is_empty() {
        return (false, None);
    }
    let (base, parameters) = raw.split_at(
        raw.iter().position(|byte| *byte == b';').unwrap_or(raw.len()),
    );
    let base = trim_unicode_whitespace(base);
    let mut parts = base.split(|byte| *byte == b'/');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    if first.is_empty()
        || !first.iter().all(|byte| media_token(*byte))
        || second.is_some_and(|part| part.is_empty() || !part.iter().all(|byte| media_token(*byte)))
        || parts.next().is_some()
    {
        return (false, Some("mime: invalid media type"));
    }
    let urlencoded = base.eq_ignore_ascii_case(b"application/x-www-form-urlencoded");
    match prepared_media_parameters(parameters) {
        Ok(()) => (urlencoded, None),
        Err(MediaParameterIssue::Malformed) => (urlencoded, Some("mime: invalid media parameter")),
        Err(MediaParameterIssue::Duplicate) => (false, Some("mime: duplicate parameter name")),
    }
}

enum MediaParameterIssue {
    Malformed,
    Duplicate,
}

fn prepared_media_parameters(parameters: &[u8]) -> Result<(), MediaParameterIssue> {
    let mut rest = parameters;
    loop {
        let trimmed = trim_unicode_whitespace(rest);
        if trimmed.is_empty() || trimmed == b";" {
            return Ok(());
        }
        let current = borrow_media_parameter(rest).ok_or(MediaParameterIssue::Malformed)?;
        let mut previous = parameters;
        while previous.len() > rest.len() {
            let Some(parameter) = borrow_media_parameter(previous) else { break };
            if current.name.eq_ignore_ascii_case(parameter.name)
                && !current.value_bytes().eq(parameter.value_bytes())
            {
                return Err(MediaParameterIssue::Duplicate);
            }
            previous = parameter.tail;
        }
        rest = current.tail;
    }
}

use effect::{Effect, effect_try_future};
use effect_server::{
    ByteFieldBounds, ByteFormData, ByteFormPolicy, BytePairs, MultipartPolicy, Request,
    ServerError, decode_url_encoded_pairs_partial,
};
use std::num::NonZeroUsize;

/// Native form limits shared by parsing consumers.
#[derive(Clone, Copy, Debug)]
pub struct MergedFormPolicy {
    /// URL-encoded body limits.
    pub urlencoded: ByteFormPolicy,
    /// Multipart stream limits.
    pub multipart: MultipartPolicy,
    /// Multipart field count bound.
    pub multipart_fields: NonZeroUsize,
    /// Multipart values' byte bounds.
    pub multipart_values: ByteFieldBounds,
    /// File-memory allowance, distinct from total body limits.
    pub file_memory: usize,
}

/// Select whether multipart form values are initialized.
#[derive(Clone, Copy, Debug)]
pub enum InitializeForm {
    /// Initialize query and eligible URL-encoded body values.
    ParseForm,
    /// Also initialize multipart fields.
    FormValue,
}

fn append_pairs(target: &mut BytePairs, incoming: BytePairs) {
    for (key, values) in incoming {
        target.entry(key).or_default().extend(values);
    }
}
/// Initialize a merged form lazily and return any parse diagnostic separately.
pub fn initialize_merged_form<E, R>(
    request: &Request,
    policy: MergedFormPolicy,
    mode: InitializeForm,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<(ByteFormData, Option<ServerError>), E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    effect_try_future! {
        body: move |_| {
            let request = request.clone();
            async move { initialize_merged_form_inner(&request, policy, mode).await }
        },
        on_error: move |error| on_error(error),
    }
}

pub(crate) async fn initialize_merged_form_inner(
    request: &Request,
    policy: MergedFormPolicy,
    mode: InitializeForm,
) -> Result<(ByteFormData, Option<ServerError>), ServerError> {
    let encoded_query = request.uri().query().unwrap_or_default().as_bytes();
    let (mut merged, mut diagnostic) = if encoded_query.split(|byte| *byte == b'&').count() > 10_000
    {
        (
            BytePairs::new(),
            Some(ServerError::Limit("form segments".into())),
        )
    } else {
        let query = decode_url_encoded_pairs_partial(encoded_query);
        (query.values, query.diagnostic)
    };
    let content_type = request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
        .trim();
    let reads_urlencoded = matches!(request.method().as_str(), "POST" | "PUT" | "PATCH")
        && request
            .headers()
            .get(http::header::CONTENT_TYPE)
            .is_some_and(|value| crate::form_media::recognizes_urlencoded(value.as_bytes()));
    if reads_urlencoded {
        match request
            .read_partial_byte_form_with_segment_limit(policy.urlencoded, NonZeroUsize::new(10_000))
            .await
        {
            Ok(body) => {
                let mut body_values = body.values.clone();
                append_pairs(&mut body_values, merged);
                merged = body_values;
                if let Some(error) = &body.diagnostic {
                    diagnostic = Some(ServerError::Invalid(format!("parse form: {error}")));
                }
            }
            Err(error) => diagnostic = Some(error),
        }
    } else if matches!(mode, InitializeForm::FormValue)
        && content_type.eq_ignore_ascii_case("multipart/form-data")
    {
        match request
            .read_accounted_multipart_fields(
                policy.multipart,
                policy.multipart_fields,
                policy.multipart_values,
                policy.file_memory,
            )
            .await
        {
            Ok(body) => append_pairs(&mut merged, body.values.clone()),
            Err(error) => {
                if diagnostic.is_none() {
                    diagnostic = Some(error);
                }
            }
        }
    }
    Ok((ByteFormData { values: merged }, diagnostic))
}
