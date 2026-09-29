use super::WorkflowCodecError;
use super::encoding::{decode_json, encode_json};
use crate::{JsonValue, KeyDomain, ParseLimits, PureCodec, SchemaIssue, StableKey, UtcMillis};
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowRunId(Arc<[u8]>);

impl WorkflowRunId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowDefinitionId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowIdentityError {
    Empty,
}

impl WorkflowDefinitionId {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, WorkflowIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(WorkflowIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowVersion(NonZeroU32);

impl WorkflowVersion {
    #[must_use]
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowActivityId(Arc<[u8]>);

impl WorkflowActivityId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowSignalId(Arc<[u8]>);

impl WorkflowSignalId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowDeadlineId(Arc<[u8]>);

impl WorkflowDeadlineId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedDeadline {
    pub identity: WorkflowDeadlineId,
    pub at: UtcMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowCompensationId(Arc<[u8]>);

impl WorkflowCompensationId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowRaceId(Arc<[u8]>);

impl WorkflowRaceId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowHandle {
    pub run: WorkflowRunId,
    pub definition: WorkflowDefinitionId,
    pub version: WorkflowVersion,
}

/// Application-owned compatibility signature for the registered code and its
/// payload/outcome schemas. A changed signature under one ID/version is a
/// conflict rather than a silent replay reinterpretation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRegistration {
    pub id: WorkflowDefinitionId,
    pub version: WorkflowVersion,
    pub compatibility: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowOutcomeKind {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedWorkflowOutcome {
    pub kind: WorkflowOutcomeKind,
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowStatus {
    Ready,
    Running,
    Suspended,
    Completed(EncodedWorkflowOutcome),
    Cancelled,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedActivity {
    pub identity: WorkflowActivityId,
    pub attempt: NonZeroU32,
    pub encoded_input: Arc<[u8]>,
    pub outcome: EncodedWorkflowOutcome,
}

/// A child identity and its original input are part of the parent's history.
/// Replaying the parent reuses this exact child run instead of starting a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedChild {
    pub handle: WorkflowHandle,
    pub encoded_input: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowWait {
    Signal {
        identity: WorkflowSignalId,
        version: WorkflowVersion,
    },
    Deadline(RecordedDeadline),
    Child(WorkflowRunId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowWakeReason {
    Signal(WorkflowSignalId),
    Deadline(RecordedDeadline),
    Child(WorkflowRunId),
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    pub handle: WorkflowHandle,
    pub compatibility: Arc<[u8]>,
    pub encoded_input: Arc<[u8]>,
    pub status: WorkflowStatus,
    pub activities: Vec<RecordedActivity>,
    pub children: Vec<RecordedChild>,
    pub signals: Vec<RecordedSignal>,
    pub compensations: Vec<RecordedCompensation>,
    pub races: Vec<RecordedRace>,
    pub deadline: Option<RecordedDeadline>,
    pub waiting_on: Option<WorkflowWait>,
    pub wake_reason: Option<WorkflowWakeReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedRace {
    pub identity: WorkflowRaceId,
    pub winner: WorkflowActivityId,
    pub attempt: NonZeroU32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompensationState {
    Pending,
    Completed(EncodedWorkflowOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedCompensation {
    pub identity: WorkflowCompensationId,
    pub order: NonZeroU32,
    pub state: CompensationState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedSignal {
    pub identity: WorkflowSignalId,
    pub version: WorkflowVersion,
    pub payload: Arc<[u8]>,
}

/// The three independently typed schemas of a workflow definition. Execution
/// records encode values, never Rust type names or closure addresses.
pub struct WorkflowDefinition<I, A, E> {
    pub id: WorkflowDefinitionId,
    pub version: WorkflowVersion,
    pub input: PureCodec<I, JsonValue>,
    pub success: PureCodec<A, JsonValue>,
    pub error: PureCodec<E, JsonValue>,
    pub compatibility: Arc<[u8]>,
    pub limits: ParseLimits,
}

impl<I, A, E> Clone for WorkflowDefinition<I, A, E> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            version: self.version,
            input: self.input.clone(),
            success: self.success.clone(),
            error: self.error.clone(),
            compatibility: Arc::clone(&self.compatibility),
            limits: self.limits,
        }
    }
}

impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> WorkflowDefinition<I, A, E> {
    pub fn encode_input(&self, input: &I) -> Result<JsonValue, SchemaIssue> {
        self.input.encode_ref(input)
    }
    pub fn decode_success(&self, value: &JsonValue) -> Result<A, SchemaIssue> {
        self.success.decode_ref(value)
    }
    pub fn decode_error(&self, value: &JsonValue) -> Result<E, SchemaIssue> {
        self.error.decode_ref(value)
    }

    #[must_use]
    pub fn registration(&self) -> WorkflowRegistration {
        WorkflowRegistration {
            id: self.id.clone(),
            version: self.version,
            compatibility: Arc::clone(&self.compatibility),
        }
    }

    pub fn encode_input_bytes(&self, input: &I) -> Result<Arc<[u8]>, WorkflowCodecError> {
        let value = self
            .input
            .encode_ref(input)
            .map_err(WorkflowCodecError::Schema)?;
        encode_json(&value, self.limits)
    }

    pub fn decode_input_bytes(&self, bytes: &[u8]) -> Result<I, WorkflowCodecError> {
        let value = decode_json(bytes, self.limits)?;
        self.input
            .decode_ref(&value)
            .map_err(WorkflowCodecError::Schema)
    }

    pub fn encode_outcome(
        &self,
        outcome: &Result<A, E>,
    ) -> Result<EncodedWorkflowOutcome, WorkflowCodecError> {
        let (kind, value) = match outcome {
            Ok(value) => (
                WorkflowOutcomeKind::Succeeded,
                self.success
                    .encode_ref(value)
                    .map_err(WorkflowCodecError::Schema)?,
            ),
            Err(error) => (
                WorkflowOutcomeKind::Failed,
                self.error
                    .encode_ref(error)
                    .map_err(WorkflowCodecError::Schema)?,
            ),
        };
        Ok(EncodedWorkflowOutcome {
            kind,
            payload: encode_json(&value, self.limits)?,
        })
    }

    pub fn decode_outcome(
        &self,
        outcome: &EncodedWorkflowOutcome,
    ) -> Result<Result<A, E>, WorkflowCodecError> {
        let value = decode_json(&outcome.payload, self.limits)?;
        match outcome.kind {
            WorkflowOutcomeKind::Succeeded => self
                .success
                .decode_ref(&value)
                .map(Ok)
                .map_err(WorkflowCodecError::Schema),
            WorkflowOutcomeKind::Failed => self
                .error
                .decode_ref(&value)
                .map(Err)
                .map_err(WorkflowCodecError::Schema),
        }
    }
}
