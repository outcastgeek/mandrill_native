//! Execution identities, states, and typed outcomes.

use super::WorkflowCodecError;
use super::encoding::{decode_json, encode_json};
use crate::{JsonValue, KeyDomain, ParseLimits, PureCodec, SchemaIssue, StableKey, UtcMillis};
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowRunId(Arc<[u8]>);

impl WorkflowRunId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowDefinitionId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A malformed or incompatible workflow identity, run ID, or definition version.
pub enum WorkflowIdentityError {
    /// No value is present.
    Empty,
}

impl WorkflowDefinitionId {
    /// Construct a `WorkflowDefinitionId` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, WorkflowIdentityError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, WorkflowIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(WorkflowIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The validated version of a registered workflow definition.
pub struct WorkflowVersion(NonZeroU32);

impl WorkflowVersion {
    #[must_use]
    /// # Parameters
    /// - `value` (`NonZeroU32`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU32`.
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowActivityId(Arc<[u8]>);

impl WorkflowActivityId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowSignalId(Arc<[u8]>);

impl WorkflowSignalId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowDeadlineId(Arc<[u8]>);

impl WorkflowDeadlineId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A deadline identity and instant persisted for replay.
pub struct RecordedDeadline {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowDeadlineId`.
    pub identity: WorkflowDeadlineId,
    /// Instant at which this transition occurred. Type: `UtcMillis`.
    pub at: UtcMillis,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowCompensationId(Arc<[u8]>);

impl WorkflowCompensationId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by workflow workflow types operations.
pub struct WorkflowRaceId(Arc<[u8]>);

impl WorkflowRaceId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A typed reference to one workflow run and its definition.
pub struct WorkflowHandle {
    /// Stable identity of the workflow run. Type: `WorkflowRunId`.
    pub run: WorkflowRunId,
    /// Stable identity of the registered workflow definition. Type: `WorkflowDefinitionId`.
    pub definition: WorkflowDefinitionId,
    /// The declared compatibility version. Type: `WorkflowVersion`.
    pub version: WorkflowVersion,
}

/// Application-owned compatibility signature for the registered code and its
/// payload/outcome schemas. A changed signature under one ID/version is a
/// conflict rather than a silent replay reinterpretation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRegistration {
    /// Stable identity of this record. Type: `WorkflowDefinitionId`.
    pub id: WorkflowDefinitionId,
    /// The declared compatibility version. Type: `WorkflowVersion`.
    pub version: WorkflowVersion,
    /// Signature used to validate replay compatibility. Type: `Arc<[u8]>`.
    pub compatibility: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The terminal category recorded for a completed workflow run.
pub enum WorkflowOutcomeKind {
    /// The operation completed successfully.
    Succeeded,
    /// The operation completed with a failure.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A completed outcome encoded for storage and later typed decoding.
pub struct EncodedWorkflowOutcome {
    /// Typed discriminator for this state. Type: `WorkflowOutcomeKind`.
    pub kind: WorkflowOutcomeKind,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The recorded lifecycle state of one workflow run.
pub enum WorkflowStatus {
    /// The operation is ready to continue.
    Ready,
    /// The operation has started and is not yet terminal.
    Running,
    /// Execution is suspended until a durable wakeup.
    Suspended,
    /// The operation completed and its outcome is available. Carries `EncodedWorkflowOutcome`.
    Completed(EncodedWorkflowOutcome),
    /// Execution was cancelled.
    Cancelled,
    /// The owner was lost before the operation settled.
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The identity, input, and recorded result of a named activity invocation.
pub struct RecordedActivity {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowActivityId`.
    pub identity: WorkflowActivityId,
    /// Number of the current delivery or execution attempt. Type: `NonZeroU32`.
    pub attempt: NonZeroU32,
    /// Input bytes retained for replay. Type: `Arc<[u8]>`.
    pub encoded_input: Arc<[u8]>,
    /// The recorded outcome of this operation. Type: `EncodedWorkflowOutcome`.
    pub outcome: EncodedWorkflowOutcome,
}

/// A child identity and its original input are part of the parent's history.
/// Replaying the parent reuses this exact child run instead of starting a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedChild {
    /// Handle used to observe or settle this operation. Type: `WorkflowHandle`.
    pub handle: WorkflowHandle,
    /// Input bytes retained for replay. Type: `Arc<[u8]>`.
    pub encoded_input: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The signal, child, deadline, or other condition suspending a run.
pub enum WorkflowWait {
    /// The workflow is waiting for or recording a named signal.
    Signal {
        /// Stable identity used for deduplication or lookup. Type: `WorkflowSignalId`.
        identity: WorkflowSignalId,
        /// The declared compatibility version. Type: `WorkflowVersion`.
        version: WorkflowVersion,
    },
    /// Execution is waiting for a recorded deadline. Contains `RecordedDeadline`.
    Deadline(RecordedDeadline),
    /// Execution is waiting for a child workflow. Contains `WorkflowRunId`.
    Child(WorkflowRunId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The recorded reason a suspended workflow became runnable.
pub enum WorkflowWakeReason {
    /// The workflow is waiting for or recording a named signal. Contains `WorkflowSignalId`.
    Signal(WorkflowSignalId),
    /// Execution is waiting for a recorded deadline. Contains `RecordedDeadline`.
    Deadline(RecordedDeadline),
    /// Execution is waiting for a child workflow. Contains `WorkflowRunId`.
    Child(WorkflowRunId),
    /// The caller advances or settles this state explicitly.
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A replayable view of a run’s identity, history, waits, and outcome.
pub struct WorkflowSnapshot {
    /// Handle used to observe or settle this operation. Type: `WorkflowHandle`.
    pub handle: WorkflowHandle,
    /// Signature used to validate replay compatibility. Type: `Arc<[u8]>`.
    pub compatibility: Arc<[u8]>,
    /// Input bytes retained for replay. Type: `Arc<[u8]>`.
    pub encoded_input: Arc<[u8]>,
    /// The current status of this operation. Type: `WorkflowStatus`.
    pub status: WorkflowStatus,
    /// Activity invocations retained for replay. Type: `Vec<RecordedActivity>`.
    pub activities: Vec<RecordedActivity>,
    /// Child workflow records retained for replay. Type: `Vec<RecordedChild>`.
    pub children: Vec<RecordedChild>,
    /// Signals accepted for this workflow run. Type: `Vec<RecordedSignal>`.
    pub signals: Vec<RecordedSignal>,
    /// Compensation actions retained for replay. Type: `Vec<RecordedCompensation>`.
    pub compensations: Vec<RecordedCompensation>,
    /// Race winners and branch records retained for replay. Type: `Vec<RecordedRace>`.
    pub races: Vec<RecordedRace>,
    /// The absolute deadline for this operation. Type: `Option<RecordedDeadline>`.
    pub deadline: Option<RecordedDeadline>,
    /// Condition currently suspending this workflow run. Type: `Option<WorkflowWait>`.
    pub waiting_on: Option<WorkflowWait>,
    /// Recorded reason this workflow run became runnable. Type: `Option<WorkflowWakeReason>`.
    pub wake_reason: Option<WorkflowWakeReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The selected winner and branch information retained for durable race replay.
pub struct RecordedRace {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowRaceId`.
    pub identity: WorkflowRaceId,
    /// Activity identity recorded as the race winner. Type: `WorkflowActivityId`.
    pub winner: WorkflowActivityId,
    /// Number of the current delivery or execution attempt. Type: `NonZeroU32`.
    pub attempt: NonZeroU32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The registered, running, or completed state of a compensation action.
pub enum CompensationState {
    /// The operation has not completed.
    Pending,
    /// The operation completed and its outcome is available. Carries `EncodedWorkflowOutcome`.
    Completed(EncodedWorkflowOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The identity and outcome of one compensation action in workflow history.
pub struct RecordedCompensation {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowCompensationId`.
    pub identity: WorkflowCompensationId,
    /// Stable acceptance or delivery order. Type: `NonZeroU32`.
    pub order: NonZeroU32,
    /// Recorded lifecycle state. Type: `CompensationState`.
    pub state: CompensationState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A stable signal identity and payload accepted for a workflow run.
pub struct RecordedSignal {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowSignalId`.
    pub identity: WorkflowSignalId,
    /// The declared compatibility version. Type: `WorkflowVersion`.
    pub version: WorkflowVersion,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
}

/// The three independently typed schemas of a workflow definition. Execution
/// records encode values, never Rust type names or closure addresses.
///
/// # Type parameters
/// - `I`: Input type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct WorkflowDefinition<I, A, E> {
    /// Stable identity of this record. Type: `WorkflowDefinitionId`.
    pub id: WorkflowDefinitionId,
    /// The declared compatibility version. Type: `WorkflowVersion`.
    pub version: WorkflowVersion,
    /// The input supplied to this operation. Type: `PureCodec<I, JsonValue>`.
    pub input: PureCodec<I, JsonValue>,
    /// Codec or representation identity for successful results. Type: `PureCodec<A, JsonValue>`.
    pub success: PureCodec<A, JsonValue>,
    /// The typed error carried by this result. Type: `PureCodec<E, JsonValue>`.
    pub error: PureCodec<E, JsonValue>,
    /// Signature used to validate replay compatibility. Type: `Arc<[u8]>`.
    pub compatibility: Arc<[u8]>,
    /// Explicit resource bounds applied to this operation. Type: `ParseLimits`.
    pub limits: ParseLimits,
}

impl<I, A, E> Clone for WorkflowDefinition<I, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Encode typed workflow input using the definition’s registered codec.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `input` (`&I`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<JsonValue, SchemaIssue>`.
    pub fn encode_input(&self, input: &I) -> Result<JsonValue, SchemaIssue> {
        self.input.encode_ref(input)
    }
    /// Decode a recorded successful workflow result.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<A, SchemaIssue>`.
    pub fn decode_success(&self, value: &JsonValue) -> Result<A, SchemaIssue> {
        self.success.decode_ref(value)
    }
    /// Decode a recorded expected workflow error.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<E, SchemaIssue>`.
    pub fn decode_error(&self, value: &JsonValue) -> Result<E, SchemaIssue> {
        self.error.decode_ref(value)
    }

    #[must_use]
    /// Build the engine registration record for this definition.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `WorkflowRegistration`.
    pub fn registration(&self) -> WorkflowRegistration {
        WorkflowRegistration {
            id: self.id.clone(),
            version: self.version,
            compatibility: Arc::clone(&self.compatibility),
        }
    }

    /// Encode input bytes for a stable workflow identity and version.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `input` (`&I`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<Arc<[u8]>, WorkflowCodecError>`.
    pub fn encode_input_bytes(&self, input: &I) -> Result<Arc<[u8]>, WorkflowCodecError> {
        let value = self
            .input
            .encode_ref(input)
            .map_err(WorkflowCodecError::Schema)?;
        encode_json(&value, self.limits)
    }

    /// Validate and decode the stored input bytes for this definition.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Result<I, WorkflowCodecError>`.
    pub fn decode_input_bytes(&self, bytes: &[u8]) -> Result<I, WorkflowCodecError> {
        let value = decode_json(bytes, self.limits)?;
        self.input
            .decode_ref(&value)
            .map_err(WorkflowCodecError::Schema)
    }

    /// Encode the complete workflow outcome for durable recording.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Result<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Result<EncodedWorkflowOutcome, WorkflowCodecError>`.
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

    /// Decode the recorded terminal outcome through its registered codecs.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Result<Result<A, E>, WorkflowCodecError>`.
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
