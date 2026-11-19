//! Prepared merged query, URL-encoded, and multipart fields.

use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use effect::{Effect, effect_try_future};
use effect_server::{
    ByteFieldBounds, ByteFormPolicy, MultipartFieldDestination, Request, ServerError,
    append_decoded_url_encoded_component,
};
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;

use crate::form_media::{InitializeForm, MergedFormPolicy};

/// Physical prepared request and decoded field capacity.
#[derive(Clone, Copy)]
pub struct MergedFormCapacity {
    /// Maximum simultaneously retained form snapshots.
    pub forms: NonZeroUsize,
    /// Maximum stored query and body pairs together.
    pub pairs: NonZeroUsize,
    /// Combined decoded key/value byte capacity.
    pub bytes: Bytes,
    /// Maximum encoded query segments examined.
    pub query_segments: NonZeroUsize,
    /// Maximum encoded URL-encoded body segments examined.
    pub body_segments: NonZeroUsize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Body,
    Query,
    Multipart,
}

struct Pair {
    key: Range<usize>,
    value: Range<usize>,
    source: Source,
    order: usize,
}

struct FormStorage {
    pairs: FixedVector<'static, Pair>,
    bytes: Buffer<'static>,
}

/// Actual region storage for merged byte-valued form snapshots.
pub struct PreparedMergedForms {
    slots: SlotPool<'static, FormStorage>,
    capacity: MergedFormCapacity,
}

/// A retained merged form. Values borrow the prepared byte backing.
pub struct PreparedMergedForm {
    cell: SlotLease<'static, FormStorage>,
    diagnostic: Option<ServerError>,
    diagnostic_is_multipart: bool,
}

#[derive(Clone, Copy)]
enum ReadMode {
    Merge(MergedFormPolicy, InitializeForm),
    Validate(ByteFormPolicy),
}

#[derive(Clone, Copy)]
enum RetainedBody {
    Urlencoded(ByteFormPolicy),
    Multipart(MergedFormPolicy),
    Acknowledged(ByteFormPolicy),
}

#[derive(Clone)]
struct RetainedForm {
    owner: Arc<PreparedMergedForms>,
    body: RetainedBody,
    uri: http::Uri,
    form: Arc<PreparedMergedForm>,
}

pub(crate) enum ValidationFormRead {
    Local(PreparedMergedForm),
    Shared(Arc<PreparedMergedForm>),
}

impl std::ops::Deref for ValidationFormRead {
    type Target = PreparedMergedForm;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Local(form) => form,
            Self::Shared(form) => form,
        }
    }
}

impl ReadMode {
    fn accepts(self, body: RetainedBody) -> bool {
        match (self, body) {
            (
                Self::Validate(requested),
                RetainedBody::Urlencoded(saved) | RetainedBody::Acknowledged(saved),
            ) => requested == saved,
            (Self::Validate(_), RetainedBody::Multipart(_)) => true,
            (
                Self::Merge(requested, _),
                RetainedBody::Urlencoded(saved) | RetainedBody::Acknowledged(saved),
            ) => requested.urlencoded == saved,
            (Self::Merge(requested, InitializeForm::FormValue), RetainedBody::Multipart(saved)) => {
                requested.multipart == saved.multipart
                    && requested.multipart_fields == saved.multipart_fields
                    && requested.multipart_values == saved.multipart_values
                    && requested.file_memory == saved.file_memory
            }
            (Self::Merge(_, InitializeForm::ParseForm), RetainedBody::Multipart(_)) => false,
        }
    }
}

impl PreparedMergedForms {
    /// Calculate payload and slot-table demand. The memory plan accounts for allocator overhead.
    pub fn storage_bytes(capacity: MergedFormCapacity) -> Result<Bytes, MemoryError> {
        let pair_bytes = Layout::array::<Pair>(capacity.pairs.get())
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let each = Bytes::new(pair_bytes).checked_add(capacity.bytes)?;
        SlotPool::<FormStorage>::storage_bytes(capacity.forms.get())?
            .checked_add(each.checked_mul(capacity.forms.get())?)
    }

    /// Prepare the record tables and byte buffers at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: MergedFormCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let slots = memory.slots(region, capacity.forms.get(), |_| {
            Ok(FormStorage {
                pairs: FixedVector::prepare(memory.allocator(region)?, capacity.pairs.get())?,
                bytes: memory.buffer(region, capacity.bytes)?,
            })
        })?;
        Ok(Self { slots, capacity })
    }

    /// Admit a complete snapshot before parsing query and eligible form body data.
    /// URL-encoded body values precede query values with the same name.
    pub fn read<E: Send + 'static, R: Send + Sync + 'static>(
        self: &Arc<Self>,
        request: &Request,
        policy: MergedFormPolicy,
        mode: InitializeForm,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Effect<PreparedMergedForm, E, R> {
        let storage = self.clone();
        let request = request.clone();
        effect_try_future! {
            body: move |_| {
                let storage = storage.clone();
                let request = request.clone();
                async move {
                    let (form, _) = storage.read_inner(&request, ReadMode::Merge(policy, mode)).await?;
                    Ok(form)
                }
            },
            on_error: on_error,
        }
    }

    pub(crate) fn read_query(&self, request: &Request) -> Result<PreparedMergedForm, ServerError> {
        let mut cell = self.slots.try_checkout().map_err(ServerError::Memory)?;
        cell.pairs.clear();
        cell.bytes.clear();
        let query = request.uri().query().unwrap_or_default().as_bytes();
        let diagnostic = cell.append_pairs(query, self.capacity.query_segments, Source::Query)?;
        cell.sort_pairs();
        Ok(PreparedMergedForm {
            cell,
            diagnostic,
            diagnostic_is_multipart: false,
        })
    }

    pub(crate) async fn read_validation(
        self: &Arc<Self>,
        request: &Request,
        policy: ByteFormPolicy,
    ) -> Result<ValidationFormRead, ServerError> {
        self.read_shared(request, ReadMode::Validate(policy)).await
    }

    /// Share the actual body-bearing snapshot with validation and other compatible
    /// readers on this request. One native shared owner retains the existing
    /// regional lease; repeated body values and diagnostics stay in the snapshot.
    /// Incompatible policies or storage owners yield `FormPolicyConflict`.
    pub fn read_reusing_validation<E: Send + 'static, R: Send + Sync + 'static>(
        self: &Arc<Self>,
        request: &Request,
        policy: MergedFormPolicy,
        mode: InitializeForm,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Effect<Arc<PreparedMergedForm>, E, R> {
        let storage = self.clone();
        let request = request.clone();
        effect_try_future! {
            body: move |_| {
                let storage = storage.clone();
                let request = request.clone();
                async move {
                    let form = storage.read_shared(&request, ReadMode::Merge(policy, mode)).await?;
                    Ok(match form {
                        ValidationFormRead::Local(form) => Arc::new(form),
                        ValidationFormRead::Shared(form) => form,
                    })
                }
            },
            on_error: on_error,
        }
    }

    async fn read_shared(
        self: &Arc<Self>,
        request: &Request,
        mode: ReadMode,
    ) -> Result<ValidationFormRead, ServerError> {
        if let Some(form) = self.retained_form(request, mode)? {
            return Ok(ValidationFormRead::Shared(form));
        }
        let (form, body) = self.read_inner(request, mode).await?;
        let Some(body) = body else {
            return Ok(ValidationFormRead::Local(form));
        };
        let form = Arc::new(form);
        let retained = RetainedForm {
            owner: self.clone(),
            body,
            uri: request.uri().clone(),
            form: form.clone(),
        };
        match request.try_insert_extension(retained) {
            Ok(()) => Ok(ValidationFormRead::Shared(form)),
            Err(unpublished) => {
                drop(unpublished);
                drop(form);
                self.retained_form(request, mode)?
                    .map(ValidationFormRead::Shared)
                    .ok_or(ServerError::FormPolicyConflict)
            }
        }
    }

    fn retained_form(
        self: &Arc<Self>,
        request: &Request,
        mode: ReadMode,
    ) -> Result<Option<Arc<PreparedMergedForm>>, ServerError> {
        let Some(retained) =
            request.with_extensions(|extensions| extensions.get::<RetainedForm>().cloned())
        else {
            return Ok(None);
        };
        if !Arc::ptr_eq(self, &retained.owner)
            || retained.uri != *request.uri()
            || !mode.accepts(retained.body)
        {
            return Err(ServerError::FormPolicyConflict);
        }
        Ok(Some(retained.form))
    }

    async fn read_inner(
        &self,
        request: &Request,
        mode: ReadMode,
    ) -> Result<(PreparedMergedForm, Option<RetainedBody>), ServerError> {
        let (urlencoded, multipart, validation) = match mode {
            ReadMode::Merge(policy, mode) => (
                policy.urlencoded,
                matches!(mode, InitializeForm::FormValue).then_some(policy),
                false,
            ),
            ReadMode::Validate(policy) => (policy, None, true),
        };
        let mut cell = self.slots.try_checkout().map_err(ServerError::Memory)?;
        cell.pairs.clear();
        cell.bytes.clear();
        let query = request.uri().query().unwrap_or_default().as_bytes();
        let mut diagnostic =
            cell.append_pairs(query, self.capacity.query_segments, Source::Query)?;
        let mut diagnostic_is_multipart = false;
        if validation
            && request
                .visit_completed_form_fields(|key, value| cell.append_decoded_pair(key, value))?
        {
            cell.sort_pairs();
            return Ok((
                PreparedMergedForm {
                    cell,
                    diagnostic,
                    diagnostic_is_multipart,
                },
                Some(RetainedBody::Acknowledged(urlencoded)),
            ));
        }
        let content_type = request.headers().get(http::header::CONTENT_TYPE);
        let raw_type = content_type.map_or(&[][..], http::HeaderValue::as_bytes);
        let (reads_body, media_error) = match request.method().as_str() {
            "POST" | "PUT" | "PATCH" if validation => {
                crate::form_media::prepared_form_media_type(raw_type)
            }
            "POST" | "PUT" | "PATCH" => (
                crate::form_media::recognizes_prepared_urlencoded(raw_type),
                None,
            ),
            _ => (false, None),
        };
        let mut body_policy = None;
        if reads_body {
            body_policy = Some(RetainedBody::Urlencoded(urlencoded));
            let rollback = cell.checkpoint();
            let result = async {
                let body = request.read_prepared_bytes_limited(urlencoded.body).await?;
                let issue =
                    cell.append_pairs(body.as_ref(), self.capacity.body_segments, Source::Body)?;
                cell.check_fields(rollback.0, urlencoded.fields, urlencoded.values)?;
                Ok(issue)
            }
            .await;
            if let Some(error) = media_error
                && !matches!(
                    &result,
                    Err(ServerError::Memory(_)
                        | ServerError::Limit(_)
                        | ServerError::BodyClaimed
                        | ServerError::FormPolicyConflict)
                )
            {
                return Err(ServerError::Invalid(error.into()));
            }
            match result {
                Ok(Some(error)) => diagnostic = Some(error),
                Ok(None) => {}
                Err(
                    error @ (ServerError::Memory(_)
                    | ServerError::BodyClaimed
                    | ServerError::FormPolicyConflict),
                ) => return Err(error),
                Err(error @ ServerError::Limit(_)) if validation => return Err(error),
                Err(error) => {
                    cell.restore(rollback);
                    diagnostic = Some(error);
                }
            }
        } else if let Some(error) = media_error {
            return Err(ServerError::Invalid(error.into()));
        } else if let Some(policy) = multipart
            && content_type
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data"))
        {
            body_policy = Some(RetainedBody::Multipart(policy));
            let rollback = cell.checkpoint();
            let result = request
                .read_accounted_multipart_into(
                    policy.multipart,
                    policy.multipart_values.bytes,
                    policy.file_memory,
                    &mut MultipartFields {
                        storage: &mut cell,
                        fields: policy.multipart_fields,
                        values: policy.multipart_values,
                        open_key: 0..0,
                        field_count: 0,
                    },
                )
                .await;
            if let Err(error) = result {
                if matches!(
                    error,
                    ServerError::Memory(_)
                        | ServerError::BodyClaimed
                        | ServerError::FormPolicyConflict
                ) {
                    return Err(error);
                }
                cell.restore(rollback);
                if diagnostic.is_none() {
                    diagnostic = Some(error);
                    diagnostic_is_multipart = true;
                }
            }
        }
        cell.sort_pairs();
        Ok((
            PreparedMergedForm {
                cell,
                diagnostic,
                diagnostic_is_multipart,
            },
            body_policy,
        ))
    }
}

impl FormStorage {
    fn append_decoded_pair(&mut self, key: &[u8], value: &[u8]) -> Result<(), ServerError> {
        let start = self.bytes.len();
        self.bytes
            .extend_from_slice(key)
            .map_err(ServerError::Memory)?;
        let middle = self.bytes.len();
        self.bytes
            .extend_from_slice(value)
            .map_err(ServerError::Memory)?;
        self.pairs
            .push(Pair {
                key: start..middle,
                value: middle..self.bytes.len(),
                source: Source::Body,
                order: self.pairs.len(),
            })
            .map_err(|_| ServerError::Memory(MemoryError::NoSlot))
    }

    fn checkpoint(&self) -> (usize, usize) {
        (self.pairs.len(), self.bytes.len())
    }

    fn restore(&mut self, (pairs, bytes): (usize, usize)) {
        while self.pairs.len() > pairs {
            let _ = self.pairs.pop();
        }
        self.bytes.truncate(bytes);
    }
    fn append_pairs(
        &mut self,
        encoded: &[u8],
        maximum_segments: NonZeroUsize,
        source: Source,
    ) -> Result<Option<ServerError>, ServerError> {
        let mut diagnostic = None;
        for (index, segment) in encoded.split(|byte| *byte == b'&').enumerate() {
            if index >= maximum_segments.get() {
                return Err(ServerError::Limit("form segments".into()));
            }
            if segment.contains(&b';') {
                diagnostic = Some(ServerError::Invalid("form separator".into()));
                continue;
            }
            if segment.is_empty() {
                continue;
            }
            let (key, value) = match segment.iter().position(|byte| *byte == b'=') {
                Some(separator) => {
                    let (key, tail) = segment.split_at(separator);
                    (key, tail.strip_prefix(b"=").unwrap_or_default())
                }
                None => (segment, &[][..]),
            };
            let checkpoint = self.bytes.len();
            let parsed = self.append_pair(key, value, source);
            if let Err(error) = parsed {
                self.bytes.truncate(checkpoint);
                if matches!(error, ServerError::Invalid(_)) {
                    if diagnostic.is_none() {
                        diagnostic = Some(error);
                    }
                } else {
                    return Err(error);
                }
            }
        }
        Ok(diagnostic)
    }

    fn append_pair(&mut self, key: &[u8], value: &[u8], source: Source) -> Result<(), ServerError> {
        let start = self.bytes.len();
        append_decoded_url_encoded_component(key, &mut self.bytes)?;
        let middle = self.bytes.len();
        append_decoded_url_encoded_component(value, &mut self.bytes)?;
        self.pairs
            .push(Pair {
                key: start..middle,
                value: middle..self.bytes.len(),
                source,
                order: self.pairs.len(),
            })
            .map_err(|_| ServerError::Memory(MemoryError::NoSlot))
    }

    fn sort_pairs(&mut self) {
        self.sort_from(0);
    }

    fn sort_from(&mut self, first: usize) {
        let bytes = self.bytes.as_slice();
        if let Some(pairs) = self.pairs.as_mut_slice().get_mut(first..) {
            pairs.sort_unstable_by(|left, right| {
                bytes
                    .get(left.key.clone())
                    .cmp(&bytes.get(right.key.clone()))
                    .then_with(|| left.source.cmp(&right.source))
                    .then_with(|| left.order.cmp(&right.order))
            });
        }
    }

    fn check_fields(
        &mut self,
        first: usize,
        field_limit: NonZeroUsize,
        bounds: ByteFieldBounds,
    ) -> Result<(), ServerError> {
        self.sort_from(first);
        let bytes = self.bytes.as_slice();
        let mut fields = 0;
        let mut count = 0;
        let mut previous = None;
        for entry in self.pairs.as_slice().iter().skip(first) {
            let key = bytes
                .get(entry.key.clone())
                .ok_or(ServerError::Memory(MemoryError::InvalidRange))?;
            if previous != Some(key) {
                fields += 1;
                if fields > field_limit.get() {
                    return Err(ServerError::Limit("form fields".into()));
                }
                previous = Some(key);
                count = 0;
            }
            count += 1;
            if count > bounds.count.get() {
                return Err(ServerError::Limit("form field count".into()));
            }
            if entry.value.len() > bounds.bytes.get() {
                return Err(ServerError::Limit("form field bytes".into()));
            }
        }
        Ok(())
    }
}

struct MultipartFields<'a> {
    storage: &'a mut FormStorage,
    fields: NonZeroUsize,
    values: ByteFieldBounds,
    open_key: Range<usize>,
    field_count: usize,
}

impl MultipartFieldDestination for MultipartFields<'_> {
    fn begin_field(&mut self, name: &[u8]) -> Result<(), ServerError> {
        let start = self.storage.bytes.len();
        self.storage
            .bytes
            .extend_from_slice(name)
            .map_err(ServerError::Memory)?;
        self.open_key = start..self.storage.bytes.len();
        Ok(())
    }

    fn append_field(&mut self, bytes: &[u8]) -> Result<(), ServerError> {
        self.storage
            .bytes
            .extend_from_slice(bytes)
            .map_err(ServerError::Memory)
    }

    fn finish_field(&mut self) -> Result<(), ServerError> {
        let bytes = self.storage.bytes.as_slice();
        let key = bytes.get(self.open_key.clone());
        let repetitions = self
            .storage
            .pairs
            .as_slice()
            .iter()
            .filter(|pair| pair.source == Source::Multipart && bytes.get(pair.key.clone()) == key)
            .count();
        if repetitions >= self.values.count.get() {
            return Err(ServerError::Limit("form field count".into()));
        }
        if repetitions == 0 {
            self.field_count += 1;
            if self.field_count > self.fields.get() {
                return Err(ServerError::Limit("form fields".into()));
            }
        }
        let order = self.storage.pairs.len();
        self.storage
            .pairs
            .push(Pair {
                key: self.open_key.clone(),
                value: self.open_key.end..bytes.len(),
                source: Source::Multipart,
                order,
            })
            .map_err(|_| ServerError::Memory(MemoryError::NoSlot))
    }
}

impl PreparedMergedForm {
    pub(crate) fn take_diagnostic(&mut self) -> Option<ServerError> {
        self.diagnostic.take()
    }

    /// Valid decoded fields in sorted-key order, preserving repetitions and precedence.
    pub fn pairs(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.borrow_pairs(true)
    }

    pub(crate) fn validation_pairs(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.borrow_pairs(false)
    }

    fn borrow_pairs(&self, multipart: bool) -> impl Iterator<Item = (&[u8], &[u8])> {
        let bytes = self.cell.bytes.as_slice();
        self.cell.pairs.as_slice().iter().filter_map(move |pair| {
            if multipart || pair.source != Source::Multipart {
                Some((bytes.get(pair.key.clone())?, bytes.get(pair.value.clone())?))
            } else {
                None
            }
        })
    }

    /// Decoded values for the named field, in form precedence order.
    pub fn values<'a>(&'a self, name: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        self.pairs()
            .filter_map(move |(key, value)| (key == name).then_some(value))
    }

    pub(crate) fn query_values<'a>(
        &'a self,
        name: &'a [u8],
    ) -> impl Iterator<Item = &'a [u8]> + 'a {
        let bytes = self.cell.bytes.as_slice();
        self.cell.pairs.as_slice().iter().filter_map(move |pair| {
            if pair.source == Source::Query && bytes.get(pair.key.clone()) == Some(name) {
                bytes.get(pair.value.clone())
            } else {
                None
            }
        })
    }

    /// Syntax diagnostic retained alongside valid partial fields.
    #[must_use]
    pub fn diagnostic(&self) -> Option<&ServerError> {
        self.diagnostic.as_ref()
    }

    pub(crate) fn validation_diagnostic(&self) -> Option<&ServerError> {
        if self.diagnostic_is_multipart {
            None
        } else {
            self.diagnostic()
        }
    }
}
