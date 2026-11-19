//! Application data storage backed by one caller-budgeted memory plan.

use crate::input_memory::{InputLoadError, InputMemory};
use crate::output_memory::{OutputBytes, OutputMemory, OutputMemoryError};
use crate::{
    AppConfig, AppEnv, AppError, LiveApp, LoadedSamples, Report, StagedOutput, application_stages,
};
use effect::memory::{
    Bytes, HasMemory, Memory, MemoryError, MemoryPolicy, MemoryReport, Partition, RegionId,
};
use effect::{
    Cancellation, Defect, Effect, EffectEnvironment, Exit, SharedValue, effect, effect_provide_some,
    effect_retry, effect_try_blocking, effect_try_sync, memory, schedule, service,
};
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Fixed managed backing and per-run bounds, expressed in bytes and items.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppMemoryLimits {
    /// Total region budget, including its allocator metadata and padding.
    pub total_bytes: Bytes,
    /// Largest accepted input file in bytes.
    pub input_bytes: NonZeroUsize,
    /// Bytes available in one staged or console message.
    pub output_bytes: NonZeroUsize,
    /// Maximum count of decoded samples in one run.
    pub samples: NonZeroUsize,
    /// Number of data workspaces provisioned for simultaneously owned operations.
    pub runs: NonZeroUsize,
}

/// Select working-set allocation policy. [LiveApp::new] remains the system mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppMemoryMode {
    /// Allocate and recycle data in the fixed backing while running.
    BoundedAllocation,
    /// Prepare data backing before startup and lease it when running.
    PreparedReuse,
}

impl AppMemoryMode {
    fn policy(self) -> MemoryPolicy {
        match self {
            Self::BoundedAllocation => MemoryPolicy::BoundedAllocation,
            Self::PreparedReuse => MemoryPolicy::PreparedReuse,
        }
    }
}

/// Failure to provision the selected memory configuration or existing providers.
#[derive(Debug)]
pub enum ManagedStartupError {
    /// Region layout or backing was refused before opening providers.
    Memory(MemoryError),
    /// An existing provider could not start.
    Provider(Defect),
}

impl From<MemoryError> for ManagedStartupError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Typed application errors when the data workspaces are selected.
pub enum ManagedAppError {
    /// Existing application work or file failure.
    Application(AppError),
    /// Input workspace, read or decoded document failure.
    Input(InputLoadError),
    /// The staged output could not be formatted into managed backing.
    Output(OutputMemoryError),
}

impl fmt::Debug for ManagedAppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Application(error) => fmt::Debug::fmt(error, formatter),
            Self::Input(error) => formatter.debug_tuple("Input").field(error).finish(),
            Self::Output(error) => formatter.debug_tuple("Output").field(error).finish(),
        }
    }
}

impl From<AppError> for ManagedAppError {
    fn from(error: AppError) -> Self {
        Self::Application(error)
    }
}

type ManagedError = SharedValue<ManagedAppError>;
const DATA: RegionId = RegionId::named("application-data");

struct Workspace {
    input: InputMemory,
    output: OutputMemory,
    memory: Memory<'static>,
}

impl Workspace {
    fn prepare(limits: AppMemoryLimits, mode: AppMemoryMode) -> Result<Self, MemoryError> {
        let policy = mode.policy();
        let required = InputMemory::storage_bytes(limits, policy)?
            .checked_add(OutputMemory::storage_bytes(policy, limits)?)?;
        if required > limits.total_bytes {
            return Err(MemoryError::BudgetExceeded {
                required,
                available: limits.total_bytes,
            });
        }
        let partitions = [Partition {
            id: DATA,
            capacity: limits.total_bytes,
        }];
        let memory = memory! {
            plan; total: limits.total_bytes, partitions: &partitions, metadata: DATA, policy: policy,
        }?.prepare()?;
        let input = InputMemory::prepare(&memory, DATA, limits)?;
        let output = OutputMemory::prepare(&memory, DATA, limits)?;
        Ok(Self {
            input,
            output,
            memory: memory.seal(),
        })
    }
}

/// Selected data capabilities alongside the existing provider environment.
#[derive(EffectEnvironment)]
#[effect_environment(
    memory(get = &self.workspace.memory, lifetime = 'static),
    cancellation = &self.app.cancellation,
)]
pub struct ManagedAppEnv {
    app: Arc<AppEnv>,
    workspace: Arc<Workspace>,
}

/// Existing live providers and the explicitly selected managed data environment.
pub struct ManagedLiveApp {
    /// The same providers and runtime as the ordinary application.
    pub app: LiveApp,
    /// Application environment that supplies the selected memory capability.
    pub env: Arc<ManagedAppEnv>,
}

impl ManagedLiveApp {
    /// Provision and seal the selected data backing before starting providers.
    pub fn new(
        config: AppConfig,
        mode: AppMemoryMode,
        limits: AppMemoryLimits,
    ) -> Result<Self, ManagedStartupError> {
        let workspace = Arc::new(Workspace::prepare(limits, mode)?);
        let app = LiveApp::new(config).map_err(ManagedStartupError::Provider)?;
        Ok(Self {
            env: Arc::new(ManagedAppEnv {
                app: Arc::clone(&app.env),
                workspace,
            }),
            app,
        })
    }

    /// Managed region accounting for the selected application data.
    #[must_use]
    pub fn memory_report(&self) -> MemoryReport {
        self.env.memory().report()
    }

    /// Render a complete terminal outcome into the selected bounded backing.
    pub fn format_outcome(
        &self,
        outcome: &Exit<Report, ManagedError>,
    ) -> Result<OutputBytes, OutputMemoryError> {
        self.env
            .workspace
            .output
            .format_outcome(self.env.memory(), outcome)
    }

    /// Render one diagnostic or console message into the selected backing.
    pub fn format_message(
        &self,
        message: fmt::Arguments<'_>,
    ) -> Result<OutputBytes, OutputMemoryError> {
        self.env
            .workspace
            .output
            .format_message(self.env.memory(), message)
    }
}

fn load_samples(workspace: Arc<Workspace>) -> Effect<LoadedSamples, ManagedError, AppEnv> {
    effect! {
        let workspace = Arc::clone(&workspace);
        let (files, retries, delay) = service! { env |env: &AppEnv| {
            (Arc::clone(&env.files), env.retries, env.retry_delay)
        }, }?;
        let input = effect_try_blocking! {
            body |cancel: Cancellation| {
                workspace.input.load(&workspace.memory, &files.input, || cancel.is_requested())
                    .map(LoadedSamples::Managed)
            },
            on_error: |error| SharedValue::new(ManagedAppError::Input(error)),
        };
        let retry: effect::Schedule<ManagedError, ()> = schedule! {
            state: 0usize,
            step: move |attempt, error: &ManagedError, _, _| {
                let io = error.with(|error| {
                    matches!(error, ManagedAppError::Input(InputLoadError::Io(_)))
                }).unwrap_or(false);
                let next = (*attempt < retries && io).then(|| (delay, attempt + 1));
                Ok(((), next))
            },
        };
        effect_retry! { program: input, schedule: retry, }?
    }
}

/// Run the ordinary business stages with explicitly bounded input, samples and output.
pub fn managed_application() -> Effect<Report, ManagedError, ManagedAppEnv> {
    effect! {
        let workspace = service! { env |env: &ManagedAppEnv| {
            Arc::clone(&env.workspace)
        }, }?;
        let program = application_stages(load_samples(Arc::clone(&workspace)), move |sum| {
            let workspace = Arc::clone(&workspace);
            effect_try_sync! {
                once;
                body: move || {
                    workspace.output.format_sum(&workspace.memory, sum).map(StagedOutput::Managed)
                },
                on_error: |error| SharedValue::new(ManagedAppError::Output(error)),
            }
        });
        effect_provide_some! {
            shared; program: program, project: |env: &ManagedAppEnv| Arc::clone(&env.app),
        }?
    }
}
