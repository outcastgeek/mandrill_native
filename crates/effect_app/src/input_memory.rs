//! Finite input file staging, JSON decoding, and retained sample storage.

use crate::AppMemoryLimits;
use effect::memory::{
    Allocation, Bytes, FixedVector, FramePool, Memory, MemoryError, MemoryPolicy, PreparedMemory,
    RegionId, SlotLease, SlotPool,
};
use effect::{
    DuplicateFields, ParseLimits, ParsePolicy, PreparedChunkPool, PreparedChunkValues,
    PreparedIssueMemory, PreparedIssueMemoryCapacity, PreparedJsonArrayCapacity, PreparedJsonKind,
    PreparedJsonParserCapacity, PreparedJsonParserPool, PreparedJsonWriterCapacity,
    PreparedJsonWriterPool, SchemaErrorMode, SchemaIssue, SchemaIssueKind, UnknownFields,
};
use std::alloc::Layout;
use std::fs::File;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

// A record/object or positional record, its samples array, and an integer.
const INPUT_DEPTH: NonZeroUsize = NonZeroUsize::MIN.saturating_add(2);
/// Failures while loading samples into a selected application memory policy.
#[derive(Debug)]
pub enum InputLoadError {
    /// Physical admission failed or the input exceeded a configured limit.
    Memory(MemoryError),
    /// Opening or reading the input failed.
    Io(io::Error),
    /// The bounded JSON parser rejected the input.
    Json(SchemaIssue),
    /// The document does not match the sample schema.
    Document(&'static str),
    /// The document supplied no samples.
    EmptySamples,
}

impl From<MemoryError> for InputLoadError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<io::Error> for InputLoadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

struct Reusable {
    bytes: SlotPool<'static, Allocation<'static>>,
    samples: PreparedChunkPool<i64>,
}

enum ByteStorage {
    Reused(SlotLease<'static, Allocation<'static>>),
    Allocated(Allocation<'static>),
}

impl ByteStorage {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::Reused(lease) => lease,
            Self::Allocated(bytes) => bytes,
        }
    }
}

enum SampleStorage {
    Reused(PreparedChunkValues<i64>),
    Allocated(FixedVector<'static, i64>),
}

impl SampleStorage {
    fn push(&mut self, sample: i64) -> Result<(), MemoryError> {
        match self {
            Self::Reused(values) => values.try_push(sample),
            Self::Allocated(values) => values.push(sample),
        }
        .map_err(|_| MemoryError::CollectionReservationFailed)
    }

    fn as_slice(&self) -> &[i64] {
        match self {
            Self::Reused(values) => values.as_slice(),
            Self::Allocated(values) => values.as_slice(),
        }
    }
}

/// Retains the decoded samples and their input-run admission through CPU use.
pub(crate) struct Samples {
    values: SampleStorage,
    _run: SlotLease<'static, ()>,
}

impl Samples {
    pub(crate) fn as_slice(&self) -> &[i64] {
        self.values.as_slice()
    }
}

struct InputPlan {
    bytes: Layout,
    samples: Layout,
    parser: PreparedJsonParserCapacity,
    writer: PreparedJsonWriterCapacity,
    issues: PreparedIssueMemoryCapacity,
    policy: ParsePolicy,
}

impl InputPlan {
    fn new(limits: AppMemoryLimits) -> Result<Self, MemoryError> {
        let bytes = Layout::array::<u8>(limits.input_bytes.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let samples =
            Layout::array::<i64>(limits.samples.get()).map_err(|_| MemoryError::InvalidLayout)?;
        let runs = limits.runs.get();
        let input_bytes = limits.input_bytes.get();
        let replacements = runs.checked_mul(2).ok_or(MemoryError::ArithmeticOverflow)?;
        let diagnostic_texts = runs
            .checked_mul(INPUT_DEPTH.get() + 1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(Self {
            bytes,
            samples,
            parser: PreparedJsonParserCapacity {
                documents: runs,
                nodes_per_document: input_bytes,
                text_bytes_per_document: input_bytes,
                depth_per_document: INPUT_DEPTH.get(),
            },
            writer: PreparedJsonWriterCapacity {
                writers: runs,
                output_bytes: 0,
                name_bytes: input_bytes,
                names: input_bytes,
                depth: INPUT_DEPTH.get(),
                tree_frames: 0,
                boundaries: 0,
            },
            issues: PreparedIssueMemoryCapacity {
                paths: PreparedJsonArrayCapacity {
                    arrays: replacements,
                    elements: INPUT_DEPTH.get(),
                },
                children: PreparedJsonArrayCapacity {
                    arrays: 0,
                    elements: 0,
                },
                text: PreparedJsonArrayCapacity {
                    arrays: diagnostic_texts,
                    elements: PreparedJsonParserPool::diagnostic_text_bytes(limits.input_bytes),
                },
                match_lists: 0,
                matches_per_list: 0,
                records: replacements,
            },
            policy: ParsePolicy {
                limits: ParseLimits {
                    bytes: limits.input_bytes,
                    depth: INPUT_DEPTH,
                    nodes: limits.input_bytes,
                    collection: limits.input_bytes,
                    issues: NonZeroUsize::MIN,
                },
                errors: SchemaErrorMode::First,
                unknown_fields: UnknownFields::Reject,
                // The ordinary Value parser replaces a repeated object key.
                duplicate_fields: DuplicateFields::KeepLast,
            },
        })
    }

    fn storage_bytes(
        &self,
        limits: AppMemoryLimits,
        policy: MemoryPolicy,
    ) -> Result<Bytes, MemoryError> {
        let runs = limits.runs.get();
        let issues = self.issues.demand()?;
        let mut bytes = self
            .parser
            .storage_bytes()?
            .checked_add(self.writer.storage_bytes()?)?
            .checked_add(FramePool::storage_bytes(0, Layout::new::<()>())?)?
            .checked_add(issues.region_bytes)?
            .checked_add(SlotPool::<()>::storage_bytes(runs)?)?
            .checked_add(Bytes::new(self.bytes.size()).checked_mul(runs)?)?;
        for demand in [issues.paths, issues.children, issues.text] {
            bytes = bytes.checked_add(FramePool::storage_bytes(
                demand.frames,
                demand.frame_layout,
            )?)?;
        }
        match policy {
            MemoryPolicy::PreparedReuse => bytes
                .checked_add(SlotPool::<Allocation<'static>>::storage_bytes(runs)?)?
                .checked_add(PreparedChunkPool::<i64>::storage_bytes(
                    limits.runs,
                    limits.samples,
                )?),
            MemoryPolicy::BoundedAllocation => {
                bytes.checked_add(Bytes::new(self.samples.size()).checked_mul(runs)?)
            }
        }
    }

    fn parser(
        &self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
    ) -> Result<PreparedJsonParserPool, MemoryError> {
        let demand = self.issues.demand()?;
        let frames = |capacity: effect::PreparedJsonArrayDemand| {
            memory.frames(region, capacity.frames, capacity.frame_layout)
        };
        let issues = PreparedIssueMemory::prepare(
            memory,
            region,
            self.issues,
            frames(demand.paths)?,
            frames(demand.children)?,
            frames(demand.text)?,
        )?;
        let writer = PreparedJsonWriterPool::prepare(
            memory,
            region,
            self.writer,
            memory.frames(region, 0, Layout::new::<()>())?,
        )?
        .with_issue_memory(Arc::new(issues));
        PreparedJsonParserPool::prepare(memory, region, self.parser, Arc::new(writer))
    }
}

pub(crate) struct InputMemory {
    region: RegionId,
    input: Layout,
    sample_limit: usize,
    runs: SlotPool<'static, ()>,
    reusable: Option<Reusable>,
    parser: PreparedJsonParserPool,
    policy: ParsePolicy,
}

impl InputMemory {
    /// Regional payload and pool metadata excluding region allocator overhead.
    pub(crate) fn storage_bytes(
        limits: AppMemoryLimits,
        policy: MemoryPolicy,
    ) -> Result<Bytes, MemoryError> {
        InputPlan::new(limits)?.storage_bytes(limits, policy)
    }

    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        limits: AppMemoryLimits,
    ) -> Result<Self, MemoryError> {
        let plan = InputPlan::new(limits)?;
        let runs = memory.slots(region, limits.runs.get(), |_| Ok(()))?;
        let reusable = match memory.policy() {
            MemoryPolicy::PreparedReuse => Some(Reusable {
                bytes: memory.slots(region, limits.runs.get(), |_| {
                    memory.allocate(region, plan.bytes)
                })?,
                samples: PreparedChunkPool::prepare(memory, region, limits.runs, limits.samples)?,
            }),
            MemoryPolicy::BoundedAllocation => None,
        };
        Ok(Self {
            region,
            input: plan.bytes,
            sample_limit: limits.samples.get(),
            runs,
            reusable,
            parser: plan.parser(memory, region)?,
            policy: plan.policy,
        })
    }

    /// Execute on the application's blocking provider. Admit input bytes and
    /// decoded samples before opening the file, and detect growth at the actual
    /// read boundary. Returning a successful sample owner releases input bytes.
    pub(crate) fn load(
        &self,
        memory: &Memory<'static>,
        path: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Samples, InputLoadError> {
        check_cancelled(&cancelled)?;
        let run = self.runs.try_checkout()?;
        let (mut input, mut samples) = match &self.reusable {
            Some(reusable) => (
                ByteStorage::Reused(reusable.bytes.try_checkout()?),
                SampleStorage::Reused(reusable.samples.try_checkout()?),
            ),
            None => (
                ByteStorage::Allocated(memory.allocate(self.region, self.input)?),
                SampleStorage::Allocated(FixedVector::prepare(
                    memory.allocator(self.region)?,
                    self.sample_limit,
                )?),
            ),
        };
        let bytes = read_input(path, input.as_mut_slice(), &cancelled)?;
        check_cancelled(&cancelled)?;
        let document = self
            .parser
            .parse_bytes(bytes, self.policy)
            .map_err(|issue| {
                if let SchemaIssueKind::Memory(error) = issue.kind() {
                    InputLoadError::Memory(*error)
                } else {
                    InputLoadError::Json(issue)
                }
            })?;
        let root = document.root();
        let array = match root.kind() {
            Some(PreparedJsonKind::Object) if root.len() == 1 => root.get("samples"),
            // Serde's input struct also accepts its one-field positional form.
            Some(PreparedJsonKind::Array) if root.len() == 1 => root.first_child(),
            _ => None,
        }
        .filter(|value| value.kind() == Some(PreparedJsonKind::Array))
        .ok_or(InputLoadError::Document(
            "expected a document with one samples array",
        ))?;
        if array.is_empty() {
            return Err(InputLoadError::EmptySamples);
        }
        if array.len() > self.sample_limit {
            return Err(MemoryError::ValueTooLarge {
                requested: sample_bytes(array.len())?,
                capacity: sample_bytes(self.sample_limit)?,
            }
            .into());
        }
        let mut sample = array.first_child();
        while let Some(value) = sample {
            let number = value
                .number_token()
                .and_then(|token| token.parse::<i64>().ok())
                .ok_or(InputLoadError::Document(
                    "samples must be 64-bit signed integers",
                ))?;
            samples.push(number)?;
            sample = value.next_sibling();
        }
        Ok(Samples {
            values: samples,
            _run: run,
        })
    }
}

fn sample_bytes(count: usize) -> Result<Bytes, MemoryError> {
    Bytes::new(std::mem::size_of::<i64>()).checked_mul(count)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> io::Result<()> {
    if cancelled() {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

fn read_input<'a>(
    path: &Path,
    input: &'a mut [u8],
    cancelled: &impl Fn() -> bool,
) -> Result<&'a [u8], InputLoadError> {
    check_cancelled(cancelled)?;
    let mut file = File::open(path)?;
    let mut length = 0;
    while length < input.len() {
        let (_, remaining) = input.split_at_mut(length);
        let read = read_checked(&mut file, remaining, cancelled)?;
        if read == 0 {
            return Ok(input.split_at(length).0);
        }
        length += read;
    }
    // An exact-size file fits. Read one byte past the boundary to distinguish
    // EOF from a file that grew or already exceeded the configured byte limit.
    let mut extra = [0];
    if read_checked(&mut file, &mut extra, cancelled)? != 0 {
        let capacity = Bytes::new(input.len());
        return Err(MemoryError::ValueTooLarge {
            requested: capacity.checked_add(Bytes::new(extra.len()))?,
            capacity,
        }
        .into());
    }
    Ok(input)
}

fn read_checked(
    file: &mut File,
    bytes: &mut [u8],
    cancelled: &impl Fn() -> bool,
) -> io::Result<usize> {
    loop {
        check_cancelled(cancelled)?;
        match file.read(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}
