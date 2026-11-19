//! Selected staging and terminal text backed by the application's region.

use crate::{AppMemoryLimits, Report};
use effect::memory::{
    Bytes, Memory, MemoryError, MemoryPolicy, PreparedMemory, RegionAllocator, RegionId, SlotPool,
    Text, TextFormatError, TextLease,
};
use effect::{CleanupFailure, Exit, Finalization, PrimaryOutcome, SharedValue};
use std::alloc::Layout;
use std::fmt;

// One staged report, one terminal report and one scratch string per active run.
const TEXTS_PER_RUN: usize = 3;

/// The selected output backing or its formatter refused the complete message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMemoryError {
    /// Actual byte storage or its lease could not be admitted.
    Memory(MemoryError),
    /// A supplied Debug or Display implementation failed.
    Formatter(fmt::Error),
}

impl From<MemoryError> for OutputMemoryError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<TextFormatError> for OutputMemoryError {
    fn from(error: TextFormatError) -> Self {
        match error {
            TextFormatError::Memory(error) => Self::Memory(error),
            TextFormatError::Formatter(error) => Self::Formatter(error),
        }
    }
}

impl fmt::Display for OutputMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(formatter),
            Self::Formatter(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OutputMemoryError {}

enum Backing {
    Reused(TextLease<'static>),
    Allocated(Text<'static>),
}

/// Complete staged or terminal text retaining its actual region backing.
pub struct OutputBytes(Backing);

impl OutputBytes {
    /// Borrow the exact bytes to write to the selected sink.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
    }

    /// Borrow the complete validated text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Backing::Reused(text) => text.as_str(),
            Backing::Allocated(text) => text.as_str(),
        }
    }

    fn text(&mut self) -> &mut Text<'static> {
        match &mut self.0 {
            Backing::Reused(text) => text,
            Backing::Allocated(text) => text,
        }
    }
}

enum Storage {
    Reuse(SlotPool<'static, Text<'static>>),
    Bounded {
        region: RegionId,
        authority: RegionAllocator<'static>,
        capacity: Bytes,
    },
}

/// Independent selected ownership for the application's complete output.
pub struct OutputMemory(Storage);

impl OutputMemory {
    fn layout(limits: AppMemoryLimits) -> Result<(usize, Layout), MemoryError> {
        let count = limits
            .runs
            .get()
            .checked_mul(TEXTS_PER_RUN)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let text = Layout::array::<u8>(limits.output_bytes.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        Ok((count, text))
    }

    /// Physical output demand before allocator headers and alignment padding.
    pub fn storage_bytes(
        policy: MemoryPolicy,
        limits: AppMemoryLimits,
    ) -> Result<Bytes, MemoryError> {
        let (count, text) = Self::layout(limits)?;
        let total = Bytes::new(text.size()).checked_mul(count)?;
        match policy {
            MemoryPolicy::PreparedReuse => {
                total.checked_add(SlotPool::<Text<'static>>::storage_bytes(count)?)
            }
            MemoryPolicy::BoundedAllocation => Ok(total),
        }
    }

    /// Prepare reusable text or bind the same region for bounded allocation.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        limits: AppMemoryLimits,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(memory.policy(), limits)?;
        let (count, text) = Self::layout(limits)?;
        let capacity = Bytes::new(text.size());
        let storage = match memory.policy() {
            MemoryPolicy::PreparedReuse => Storage::Reuse(memory.texts(region, count, capacity)?),
            MemoryPolicy::BoundedAllocation => Storage::Bounded {
                region,
                authority: memory.allocator(region)?,
                capacity,
            },
        };
        Ok(Self(storage))
    }

    fn reserve(&self, memory: &Memory<'static>) -> Result<OutputBytes, MemoryError> {
        let backing = match &self.0 {
            Storage::Reuse(texts) => {
                let mut text = texts.try_checkout()?;
                text.clear();
                Backing::Reused(text)
            }
            Storage::Bounded {
                region,
                authority,
                capacity,
            } => {
                if !authority.same_owner(&memory.allocator(*region)?) {
                    return Err(MemoryError::UnsupportedRunContract);
                }
                Backing::Allocated(memory.text(*region, *capacity)?)
            }
        };
        Ok(OutputBytes(backing))
    }

    /// Format the exact staging-file record before opening the file.
    pub fn format_sum(
        &self,
        memory: &Memory<'static>,
        sum: i128,
    ) -> Result<OutputBytes, OutputMemoryError> {
        self.format_message(memory, format_args!("{sum}\n"))
    }

    /// Format a complete diagnostic or report before publishing to its sink.
    pub fn format_message(
        &self,
        memory: &Memory<'static>,
        message: fmt::Arguments<'_>,
    ) -> Result<OutputBytes, OutputMemoryError> {
        let mut output = self.reserve(memory)?;
        output.text().try_write_fmt(message)?;
        Ok(output)
    }

    /// Render the existing terminal representation into actual bounded bytes.
    pub fn format_outcome<E: fmt::Debug + Send + 'static>(
        &self,
        memory: &Memory<'static>,
        outcome: &Exit<Report, SharedValue<E>>,
    ) -> Result<OutputBytes, OutputMemoryError> {
        let mut output = self.reserve(memory)?;
        let text = output.text();
        match outcome {
            Exit::Success(report) => text.try_write_fmt(format_args!(
                "sum={} input_bytes={}",
                report.sum, report.input_bytes
            ))?,
            Exit::Fail(error) => append_error(text, error)?,
            Exit::Die(defect) => text.try_write_fmt(format_args!("{defect:?}"))?,
            Exit::Interrupted => text.try_push_str(stringify!(Interrupted))?,
            Exit::FinalizationFailed(details) => {
                let mut scratch = self.reserve(memory)?;
                append_finalization(text, scratch.text(), details)?;
            }
        }
        Ok(output)
    }
}

fn append_error<E: fmt::Debug + Send + 'static>(
    text: &mut Text<'static>,
    error: &SharedValue<E>,
) -> Result<(), OutputMemoryError> {
    match error.with(|error| text.try_write_fmt(format_args!("{error:?}"))) {
        Ok(outcome) => outcome?,
        Err(defect) => text.try_write_fmt(format_args!("{defect:?}"))?,
    }
    Ok(())
}

fn append_finalization<E: fmt::Debug + Send + 'static>(
    text: &mut Text<'static>,
    scratch: &mut Text<'static>,
    details: &Finalization<SharedValue<E>>,
) -> Result<(), OutputMemoryError> {
    text.try_push_str("cleanup failed after ")?;
    match &details.primary {
        PrimaryOutcome::Succeeded => text.try_push_str(stringify!(Succeeded))?,
        PrimaryOutcome::Failed(error) => append_error(text, error)?,
        PrimaryOutcome::Defected(defect) => text.try_write_fmt(format_args!("{defect:?}"))?,
        PrimaryOutcome::WasInterrupted => text.try_push_str(stringify!(Interrupted))?,
    }
    text.try_push_str(": [")?;
    append_quoted_failure(text, scratch, &details.first)?;
    for failure in details.rest.iter() {
        text.try_push_str(", ")?;
        append_quoted_failure(text, scratch, failure)?;
    }
    text.try_push_str("]")?;
    Ok(())
}

fn append_quoted_failure<E: fmt::Debug + Send + 'static>(
    text: &mut Text<'static>,
    scratch: &mut Text<'static>,
    failure: &CleanupFailure<SharedValue<E>>,
) -> Result<(), OutputMemoryError> {
    scratch.clear();
    match failure {
        CleanupFailure::ReleaseFailure(error) => append_error(scratch, error)?,
        CleanupFailure::ReleaseDefect(defect) => {
            scratch.try_write_fmt(format_args!("{defect:?}"))?
        }
        CleanupFailure::ReleaseInterrupted => scratch.try_push_str(stringify!(Interrupted))?,
    }
    // The existing terminal renderer prints a debug list of strings. The
    // same str formatter supplies exactly the quoting and escaping here.
    text.try_write_fmt(format_args!("{:?}", scratch.as_str()))?;
    Ok(())
}
