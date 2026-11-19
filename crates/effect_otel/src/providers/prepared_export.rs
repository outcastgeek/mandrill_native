//! Explicit SDK processor and reader limits with regional wire batch storage.

use super::{ProviderFactories, Signal};
use effect::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{self, BatchLogProcessor, LogBatch, LogExporter};
use opentelemetry_sdk::metrics::{
    PeriodicReader, Stream, Temporality, data::ResourceMetrics, exporter::PushMetricExporter,
};
use opentelemetry_sdk::trace::{
    self, BatchSpanProcessor, Sampler, SpanData, SpanExporter, SpanLimits,
};
use std::alloc::Layout;
use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

mod ingress;
use ingress::CheckedProcessor;

/// Queue and batch bounds used by the locked native trace and log processors.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBatchLimits {
    /// Number of native records the processor may queue before it drops arrivals.
    pub queue: NonZeroUsize,
    /// Maximum records passed to one selected encoder.
    pub batch: NonZeroUsize,
    /// Visible payload and records accepted before native queue retention.
    pub record_bytes: Bytes,
    /// Configured scheduling interval for the native processor.
    pub interval: Duration,
}

/// Explicit limits for the selected SDK factories and interpreter access.
#[derive(Clone, Copy, Debug)]
pub struct PreparedProviderLimits {
    /// Simultaneously admitted Effectful callbacks against these providers.
    pub native_access: NonZeroUsize,
    /// Native live-span count limits for events, links and attributes.
    pub spans: SpanLimits,
    /// Native trace queue and export batch policy.
    pub traces: PreparedBatchLimits,
    /// Native log queue and export batch policy.
    pub logs: PreparedBatchLimits,
    /// Maximum depth of nested arrays and maps in a queued log record.
    pub log_nesting: u8,
    /// Native metric aggregation cardinality per selected instrument stream.
    pub metric_series: NonZeroUsize,
    /// Maximum native metric descriptors accepted in one observed export.
    pub metrics_per_export: NonZeroUsize,
    /// Period between collections by the native metric reader.
    pub metric_interval: Duration,
    /// Native metric aggregation temporality.
    pub temporality: Temporality,
}

/// Invalid selected SDK configuration, detected before starting any provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportConfigurationError {
    /// A native batch cannot be greater than its queue, and its cadence is positive.
    Batch(Signal),
    /// The selected reader interval or cardinality is unsupported by the SDK.
    Metrics,
}

/// Physical region storage for encoded export bodies.
#[derive(Clone, Copy, Debug)]
pub struct PreparedExportCapacity {
    /// Number of simultaneous encoders/senders that can own actual backing.
    pub batches: NonZeroUsize,
    /// Maximum bytes held in a single encoded batch.
    pub bytes_per_batch: Bytes,
}

impl PreparedExportCapacity {
    /// Report regional bytes before partition allocator headers and alignment.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let bytes = Layout::array::<u8>(self.bytes_per_batch.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<'static>>::storage_bytes(self.batches.get())?
            .checked_add(Bytes::new(bytes.size()).checked_mul(self.batches.get())?)
    }
}

/// Borrowed native data from one SDK export. The SDK keeps ownership of the
/// source records while the encoder writes into its separate prepared buffer.
pub enum PreparedExportBatch<'a> {
    /// Completed spans accepted by the configured native trace processor.
    Traces {
        /// Native span records in export order.
        records: &'a [SpanData],
        /// Resource supplied to the native exporter.
        resource: &'a Resource,
    },
    /// Logs accepted by the configured native log processor.
    Logs {
        /// Native log records in export order.
        records: LogBatch<'a>,
        /// Resource supplied to the native exporter.
        resource: &'a Resource,
    },
    /// Data collected by the configured native metric reader.
    Metrics(&'a ResourceMetrics),
}

/// Checked append-only byte destination for a selected telemetry encoder.
pub struct ExportBuffer<'a> {
    bytes: &'a mut Buffer<'static>,
    refusal: Option<MemoryError>,
}

impl ExportBuffer<'_> {
    /// Append atomically, remembering the first capacity refusal for the batch.
    pub fn extend(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        if let Some(error) = self.refusal {
            return Err(error);
        }
        self.bytes
            .extend_from_slice(bytes)
            .inspect_err(|error| self.refusal = Some(*error))
    }
}

impl io::Write for ExportBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.extend(bytes)
            .map(|()| bytes.len())
            .map_err(|_| io::ErrorKind::WriteZero.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

type Encoder =
    dyn Fn(PreparedExportBatch<'_>, &mut ExportBuffer<'_>) -> OTelSdkResult + Send + Sync;
type Sender = dyn Fn(Signal, &[u8]) -> OTelSdkResult + Send + Sync;

#[derive(Default)]
struct Discards {
    records: AtomicUsize,
    batches: AtomicUsize,
}

/// Regional export work shared by the selected native signal pipelines.
/// The caller supplies direct encoding and synchronous delivery. The sender
/// settles before returning, and any storage or retries inside it are its
/// external contract. This adapter performs one send and retains no retry queue.
pub struct PreparedExport {
    bytes: SlotPool<'static, Buffer<'static>>,
    encode: Arc<Encoder>,
    send: Arc<Sender>,
    trace_discards: Discards,
    log_discards: Discards,
    metric_discards: Discards,
}

impl PreparedExport {
    /// Provision actual export bytes before a native processor can deliver work.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedExportCapacity,
        encode: impl Fn(PreparedExportBatch<'_>, &mut ExportBuffer<'_>) -> OTelSdkResult
        + Send
        + Sync
        + 'static,
        send: impl Fn(Signal, &[u8]) -> OTelSdkResult + Send + Sync + 'static,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self {
            bytes: memory.buffers(region, capacity.batches.get(), capacity.bytes_per_batch)?,
            encode: Arc::new(encode),
            send: Arc::new(send),
            trace_discards: Discards::default(),
            log_discards: Discards::default(),
            metric_discards: Discards::default(),
        })
    }

    fn discards(&self, signal: Signal) -> &Discards {
        match signal {
            Signal::Traces => &self.trace_discards,
            Signal::Logs => &self.log_discards,
            Signal::Metrics => &self.metric_discards,
        }
    }

    /// Number of batches refused by this adapter. The native SDK handles its
    /// separate queue saturation independently.
    #[must_use]
    pub fn discarded_batches(&self, signal: Signal) -> usize {
        self.discards(signal).batches.load(Ordering::Relaxed)
    }

    /// Native trace/log records rejected before the SDK queues could retain them.
    #[must_use]
    pub fn discarded_records(&self, signal: Signal) -> usize {
        self.discards(signal).records.load(Ordering::Relaxed)
    }

    fn discard_record(&self, signal: Signal) {
        increment(&self.discards(signal).records);
    }

    fn discard_batch(&self, signal: Signal) {
        increment(&self.discards(signal).batches);
    }

    fn export(
        &self,
        signal: Signal,
        batch: PreparedExportBatch<'_>,
        records: usize,
        limit: NonZeroUsize,
    ) -> OTelSdkResult {
        if records > limit.get() {
            self.discard_batch(signal);
            return Ok(());
        }
        let Ok(mut bytes) = self.bytes.try_checkout() else {
            self.discard_batch(signal);
            return Ok(());
        };
        bytes.clear();
        let mut writer = ExportBuffer {
            bytes: &mut bytes,
            refusal: None,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| (self.encode)(batch, &mut writer)));
        if writer.refusal.is_some() {
            self.discard_batch(signal);
            return Ok(());
        }
        match outcome {
            Ok(result) => result?,
            Err(_) => {
                self.discard_batch(signal);
                return Ok(());
            }
        }
        match catch_unwind(AssertUnwindSafe(|| (self.send)(signal, bytes.as_slice()))) {
            Ok(result) => result,
            Err(_) => {
                self.discard_batch(signal);
                Ok(())
            }
        }
    }
}

fn increment(counter: &AtomicUsize) {
    let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

struct SdkExport {
    inner: Arc<PreparedExport>,
    records: NonZeroUsize,
    temporality: Temporality,
    resource: Resource,
}

impl fmt::Debug for SdkExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SdkExport")
            .field("records", &self.records)
            .finish()
    }
}

impl SpanExporter for SdkExport {
    async fn export(&self, spans: Vec<SpanData>) -> OTelSdkResult {
        self.inner.export(
            Signal::Traces,
            PreparedExportBatch::Traces {
                records: &spans,
                resource: &self.resource,
            },
            spans.len(),
            self.records,
        )
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.clone();
    }
}

impl LogExporter for SdkExport {
    async fn export(&self, logs: LogBatch<'_>) -> OTelSdkResult {
        let count = logs
            .iter()
            .take(self.records.get().saturating_add(1))
            .count();
        self.inner.export(
            Signal::Logs,
            PreparedExportBatch::Logs {
                records: logs,
                resource: &self.resource,
            },
            count,
            self.records,
        )
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.clone();
    }
}

impl PushMetricExporter for SdkExport {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        let count = metrics
            .scope_metrics()
            .flat_map(|scope| scope.metrics())
            .take(self.records.get().saturating_add(1))
            .count();
        self.inner.export(
            Signal::Metrics,
            PreparedExportBatch::Metrics(metrics),
            count,
            self.records,
        )
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn temporality(&self) -> Temporality {
        self.temporality
    }
}

impl ProviderFactories {
    /// Select the locked SDK's bounded trace/log processors and metric reader
    /// with a prepared encoder and sender. Configuration is validated eagerly;
    /// the actual SDK providers and their native workers still start lazily.
    /// Borrowed SDK handles remain an explicit native interoperation boundary.
    pub fn prepared_export(
        limits: PreparedProviderLimits,
        resource: Resource,
        sampler: Sampler,
        export: Arc<PreparedExport>,
    ) -> Result<Self, ExportConfigurationError> {
        for (signal, batch) in [(Signal::Traces, limits.traces), (Signal::Logs, limits.logs)] {
            if batch.batch > batch.queue
                || batch.interval.is_zero()
                || batch.record_bytes.get() == 0
            {
                return Err(ExportConfigurationError::Batch(signal));
            }
        }
        if limits.metric_interval.is_zero() || limits.metric_series.get() == usize::MAX {
            return Err(ExportConfigurationError::Metrics);
        }
        let trace_resource = resource.clone();
        let metric_resource = resource.clone();
        let trace_export = Arc::clone(&export);
        let metric_export = Arc::clone(&export);
        let mut factories = Self::new(
            move |builder| {
                let configuration = trace::BatchConfigBuilder::default()
                    .with_max_queue_size(limits.traces.queue.get())
                    .with_max_export_batch_size(limits.traces.batch.get())
                    .with_scheduled_delay(limits.traces.interval)
                    .build();
                let processor = BatchSpanProcessor::builder(SdkExport {
                    inner: Arc::clone(&trace_export),
                    records: limits.traces.batch,
                    temporality: limits.temporality,
                    resource: trace_resource.clone(),
                })
                .with_batch_config(configuration)
                .build();
                Ok(builder
                    .with_resource(trace_resource.clone())
                    .with_sampler(sampler.clone())
                    .with_span_limits(limits.spans)
                    .with_span_processor(CheckedProcessor {
                        processor,
                        export: Arc::clone(&trace_export),
                        record_bytes: limits.traces.record_bytes,
                        log_depth: 0,
                    })
                    .build())
            },
            move |builder| {
                let reader = PeriodicReader::builder(SdkExport {
                    inner: Arc::clone(&metric_export),
                    records: limits.metrics_per_export,
                    temporality: limits.temporality,
                    resource: metric_resource.clone(),
                })
                .with_interval(limits.metric_interval)
                .build();
                Ok(builder
                    .with_resource(metric_resource.clone())
                    .with_reader(reader)
                    .with_view(move |_| {
                        // The only parameter that can invalidate this builder
                        // is the cardinality, validated above.
                        Stream::builder()
                            .with_cardinality_limit(limits.metric_series.get())
                            .build()
                            .ok()
                    })
                    .build())
            },
            move |builder| {
                let configuration = logs::BatchConfigBuilder::default()
                    .with_max_queue_size(limits.logs.queue.get())
                    .with_max_export_batch_size(limits.logs.batch.get())
                    .with_scheduled_delay(limits.logs.interval)
                    .build();
                let processor = BatchLogProcessor::builder(SdkExport {
                    inner: Arc::clone(&export),
                    records: limits.logs.batch,
                    temporality: limits.temporality,
                    resource: resource.clone(),
                })
                .with_batch_config(configuration)
                .build();
                Ok(builder
                    .with_resource(resource.clone())
                    .with_log_processor(CheckedProcessor {
                        processor,
                        export: Arc::clone(&export),
                        record_bytes: limits.logs.record_bytes,
                        log_depth: limits.log_nesting,
                    })
                    .build())
            },
        );
        factories.admission = effect::AdmissionCapacity::Bounded(limits.native_access);
        Ok(factories)
    }
}
