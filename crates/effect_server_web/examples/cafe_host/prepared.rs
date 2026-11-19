use super::Environment;
use crate::CafeError;
use effect::memory::{Buffer, Bytes, FramePool, MemoryError, PreparedMemory, RegionId, SlotPool};
use effect::{PreparedCleanupConfig, PreparedCleanupPool, PreparedSemaphore};
use effect_server::native::http::StatusCode;
use effect_server::{
    BodyStatus, PreparedEndpointBytePools, PreparedHostCapacity,
    PreparedHttp1Limits, PreparedHttpLimits, PreparedHttpTimeouts, PreparedRequestControlCapacity,
    PreparedRequestStorageCapacity, PreparedRequestStoragePool, PreparedServer, Server, ServerError,
    StaticOverloadReply, server,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) struct HostStorage {
    host: PreparedHostCapacity,
    controls: PreparedRequestControlCapacity,
    request: PreparedRequestStorageCapacity,
    cleanup: usize,
    retained_cleanup: usize,
    outputs: usize,
    bytes: Bytes,
    queue: Option<(NonZeroUsize, NonZeroUsize)>,
    window: Duration,
}

impl HostStorage {
    pub(super) fn new(
        capacity: NonZeroUsize,
        bytes: NonZeroUsize,
        retained_outputs: NonZeroUsize,
        active_renderers: NonZeroUsize,
        queued: Option<NonZeroUsize>,
        window: Duration,
    ) -> Result<Self, MemoryError> {
        let queue = queued.map(|waiters| (capacity, waiters));
        let capacity = capacity.get();
        let connections = capacity
            .checked_add(queued.map_or(0, NonZeroUsize::get))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        // A live cache hit can hold its request's output slot and the body of
        // an evicted entry. Retained entries and cache loaders continuing after
        // the initiating request is canceled can each hold separate backing.
        let outputs = capacity
            .checked_mul(2)
            .and_then(|live| live.checked_add(retained_outputs.get()))
            .and_then(|retained| retained.checked_add(active_renderers.get()))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        // Two protected failure buffers per accepted connection and dispatched
        // request, plus three for the host. Queued heads use no request pair.
        let cleanup = connections
            .checked_add(capacity)
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(3))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let body_records = capacity
            .checked_mul(5)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(Self {
            host: PreparedHostCapacity {
                connections,
                requests_per_connection: 1,
                children_per_connection: 2,
                terminal_origins: capacity,
            },
            controls: PreparedRequestControlCapacity {
                inner_responses: 1,
                classification_receivers: 3,
                classification_versions: 3,
                selection_receivers: 2,
                selection_versions: 2,
                body_records,
                body_listeners: 2,
            },
            request: PreparedRequestStorageCapacity {
                requests: capacity,
                snapshots: PreparedRequestStorageCapacity::builtin_snapshots_for(capacity)?,
            },
            cleanup,
            retained_cleanup: 4,
            outputs,
            bytes: Bytes::new(bytes.get()),
            queue,
            window,
        })
    }

    pub(super) fn region_bytes(&self) -> Result<Bytes, MemoryError> {
        // Allow 64 regional allocations per connection and 64 independent
        // allocations, each with 64 bytes for headers, alignment and short tails.
        let connections = self
            .host
            .connections
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let overhead = Bytes::new(64 * 64).checked_mul(connections)?;
        let slots = self.request.requests;
        let request_bytes = buffer_pool_bytes(slots, self.bytes)?.checked_mul(3)?;
        // The configured Web OTel installer contributes one completion observer.
        PreparedServer::<CafeError, Environment>::host_region_bytes_with_request_controls(
            self.host,
            self.controls,
            1,
        )?
        .checked_add(PreparedRequestStoragePool::demand(self.request)?.region_bytes)?
        .checked_add(FramePool::storage_bytes(
            1,
            PreparedServer::<CafeError, Environment>::result_frame_layout::<()>(),
        )?)?
        .checked_add(
            PreparedCleanupPool::<CafeError>::storage_bytes_with_return_listeners(
                self.cleanup,
                self.retained_cleanup,
                1,
            )?,
        )?
        .checked_add(FramePool::storage_bytes_with_return_listeners(
            self.cleanup,
            PreparedCleanupPool::<CafeError>::frame_layout(),
            1,
        )?)?
        .checked_add(request_bytes)?
        .checked_add(buffer_pool_bytes(self.outputs, self.bytes)?)?
        .checked_add(overhead)
    }

    pub(super) fn queue_region_bytes(&self) -> Result<Bytes, MemoryError> {
        let Some((_, waiters)) = self.queue else {
            return Ok(Bytes::new(0));
        };
        // Allow for headers, alignment and short tails across the FIFO and its
        // three slot pools.
        PreparedSemaphore::storage_bytes(waiters.get())?.checked_add(Bytes::new(8 * 64))
    }
}

fn buffer_pool_bytes(slots: usize, capacity: Bytes) -> Result<Bytes, MemoryError> {
    SlotPool::<Buffer<'static>>::storage_bytes(slots)?.checked_add(capacity.checked_mul(slots)?)
}

/// Prepare the Web DSL's selected host inputs from the shared startup memory.
pub(super) fn prepare_cafe_host(
    memory: PreparedMemory<'static>,
    storage: HostStorage,
) -> Result<CafeHostStorage, ServerError> {
    let region = RegionId::named("web_host");
    let buffers = || {
        memory
            .buffers(region, storage.request.requests, storage.bytes)
            .map_err(ServerError::Memory)
    };
    let pools = Arc::new(
        PreparedEndpointBytePools::new(
            buffers()?,
            buffers()?,
            memory
                .buffers(region, storage.outputs, storage.bytes)
                .map_err(ServerError::Memory)?,
        )
        .with_response_header_buffer(buffers()?),
    );
    let overload = StaticOverloadReply::new(
        BodyStatus::new(StatusCode::SERVICE_UNAVAILABLE)?,
        "The café is busy. Try again shortly.\n".as_bytes(),
    );
    Ok(CafeHostStorage {
        preparation: HostPreparation {
            memory: Mutex::new(Some(memory)),
            storage,
        },
        pools,
        overload,
    })
}

/// Prepared endpoint inputs and the remaining one-shot host arena.
pub(crate) struct CafeHostStorage {
    pub(crate) preparation: HostPreparation,
    pub(crate) pools: Arc<PreparedEndpointBytePools>,
    pub(crate) overload: StaticOverloadReply,
}

pub(crate) struct HostPreparation {
    memory: Mutex<Option<PreparedMemory<'static>>>,
    storage: HostStorage,
}

impl HostPreparation {
    pub(crate) fn prepare(
        &self,
        server: Server<CafeError, Environment>,
    ) -> Result<PreparedServer<CafeError, Environment>, ServerError> {
        let storage = &self.storage;
        let region = RegionId::named("web_host");
        let memory = self
            .memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(ServerError::Memory(MemoryError::ConstructionAlreadyBound))?;
        let frames = memory
            .frames(
                region,
                1,
                PreparedServer::<CafeError, Environment>::result_frame_layout::<()>(),
            )
            .map_err(ServerError::Memory)?;
        let cleanup_pool = PreparedCleanupPool::prepare_with_return_listeners(
            &memory,
            region,
            storage.cleanup,
            storage.retained_cleanup,
            1,
        )
        .map_err(ServerError::Memory)?;
        let cleanup_frames = memory
            .frames_with_return_listeners(
                region,
                storage.cleanup,
                PreparedCleanupPool::<CafeError>::frame_layout(),
                1,
            )
            .map_err(ServerError::Memory)?;
        let requests = PreparedRequestStoragePool::prepare(&memory, region, storage.request)
            .map_err(ServerError::Memory)?;
        let prepared = server! {
            source: server,
            http1_header_timeout: storage.window,
            memory: &memory,
            region: region,
            capacity: storage.host,
            result_frames: frames,
            cleanup: PreparedCleanupConfig::new(cleanup_pool, cleanup_frames),
            request_controls: storage.controls,
        }?;
        let prepared = server! {
            prepared_configure;
            source: prepared,
            request_storage(requests),
        }
        .map_err(ServerError::Memory)?;
        let prepared = server! {
            prepared_configure;
            source: prepared,
            http_limits(PreparedHttpLimits {
                http1: Some(PreparedHttp1Limits {
                    max_headers: 100,
                    read_buffer_bytes: storage.bytes.get().max(8192),
                }),
                http2: None,
            }),
        }?;
        let prepared = server! {
            prepared_configure;
            source: prepared,
            http_timeouts(PreparedHttpTimeouts {
                first_head: Some(storage.window),
                idle: Some(storage.window),
            }),
        };
        let prepared = if let Some((capacity, waiters)) = storage.queue {
            let queue = PreparedSemaphore::prepare(
                &memory,
                RegionId::named("request_queue"),
                capacity,
                capacity.get(),
                waiters.get(),
            )
            .map_err(|error| match error {
                effect::PreparedSemaphoreError::Memory(error) => ServerError::Memory(error),
                effect::PreparedSemaphoreError::Semaphore(error) => {
                    ServerError::Configuration(error.to_string())
                }
            })?;
            // Request heads should normally time out before HTTP idle closes
            // the socket, so they can use the existing static overload response.
            server! {
                prepared_configure;
                source: prepared,
                request_queue(Arc::new(queue), storage.window / 2),
            }
        } else {
            prepared
        };
        let _memory = memory.seal();
        Ok(prepared)
    }
}
