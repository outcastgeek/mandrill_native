//! Live prepared unary RPC preserves correlation and frees abandoned exchange owners.

use effect::encoding::FramePolicy;
use effect::memory::{Bytes, MemoryError, RegionId};
use effect::rpc::{
    OperationId, PreparedInProcessRpcChannel, PreparedRpcCapacity, PreparedRpcClient,
    PreparedRpcClientError, PreparedRpcEnvelope, PreparedRpcHandler, PreparedRpcMemory,
    PreparedRpcPayloadCodec, PreparedRpcWire, RpcCallId, RpcClientError, RpcEnvelopePolicy,
    RpcVersion,
};
use effect::{
    JsonWriter, ParseLimits, PreparedJsonKind, PreparedJsonParserCapacity, PreparedJsonParserPool,
    PreparedJsonValidationCapacity, PreparedJsonValidationPool, PreparedJsonValue,
    PreparedJsonWriterCapacity, PreparedJsonWriterPool, SchemaIssue, SchemaIssueKind, memory,
    rpc_operation,
};
use futures_executor::block_on;
use std::alloc::Layout;
use std::fmt::Debug;
use std::future::{Future, pending};
use std::num::{NonZeroU32, NonZeroUsize};
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

const WORK: RegionId = RegionId::named("work");
const ONE: NonZeroUsize = NonZeroUsize::MIN;
const LIMIT: NonZeroUsize = ONE.saturating_add(255);
const DEPTH: NonZeroUsize = ONE.saturating_add(7);

fn checked<T, E: Debug>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|error| format!("{error:?}"))
}

fn read_boolean(value: PreparedJsonValue<'_>) -> Result<bool, SchemaIssue> {
    match value.kind() {
        Some(PreparedJsonKind::Bool(value)) => Ok(value),
        _ => Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch)),
    }
}

struct Codec(Arc<AtomicUsize>);

impl PreparedRpcPayloadCodec for Codec {
    type Request = bool;
    type Success = bool;
    type DomainError = bool;

    fn write_request(&self, value: &bool, writer: &mut JsonWriter) -> Result<(), SchemaIssue> {
        self.0.fetch_add(1, Ordering::Relaxed);
        writer.boolean(*value)
    }

    fn read_request(&self, value: PreparedJsonValue<'_>) -> Result<bool, SchemaIssue> {
        read_boolean(value)
    }

    fn write_success(&self, value: &bool, writer: &mut JsonWriter) -> Result<(), SchemaIssue> {
        writer.boolean(*value)
    }

    fn read_success(&self, value: PreparedJsonValue<'_>) -> Result<bool, SchemaIssue> {
        read_boolean(value)
    }

    fn write_error(&self, value: &bool, writer: &mut JsonWriter) -> Result<(), SchemaIssue> {
        writer.boolean(*value)
    }

    fn read_error(&self, value: PreparedJsonValue<'_>) -> Result<bool, SchemaIssue> {
        read_boolean(value)
    }
}

struct Handler;

impl PreparedRpcHandler<bool, bool, bool> for Handler {
    async fn handle(&self, value: bool, envelope: &PreparedRpcEnvelope) -> Result<bool, bool> {
        let mut metadata = envelope.metadata();
        assert_eq!(metadata.next(), Some(("tenant", "example")));
        assert_eq!(metadata.next(), None);
        match envelope.header().call {
            "pending" => pending().await,
            "ok" => Ok(!value),
            "domain" => Err(value),
            call => panic!("unexpected call {call}"),
        }
    }
}

#[test]
fn prepared_rpc_reserves_before_codec_and_recovers_after_cancelled_exchange() -> Result<(), String>
{
    let plan = checked(memory! {
        total: 256 KiB, mode: prepared, metadata: work, alignment: 16, partition work: 256 KiB,
    })?;
    let prepared = checked(plan.prepare())?;
    let writers = Arc::new(checked(PreparedJsonWriterPool::prepare(
        &prepared,
        WORK,
        PreparedJsonWriterCapacity {
            // Each endpoint preadmits two buffered writers and four
            // counting writers for encoding, decoding, and validation.
            writers: 12,
            output_bytes: LIMIT.get(),
            name_bytes: LIMIT.get(),
            names: 32,
            depth: DEPTH.get(),
            tree_frames: DEPTH.get(),
            boundaries: 8,
        },
        checked(prepared.frames(WORK, 8, checked(Layout::from_size_align(4096, 16))?))?,
    ))?);
    let parser = Arc::new(checked(PreparedJsonParserPool::prepare(
        &prepared,
        WORK,
        PreparedJsonParserCapacity {
            documents: 8,
            nodes_per_document: 32,
            text_bytes_per_document: LIMIT.get(),
            depth_per_document: DEPTH.get(),
        },
        Arc::clone(&writers),
    ))?);
    let validation = Arc::new(checked(PreparedJsonValidationPool::prepare(
        &prepared,
        WORK,
        PreparedJsonValidationCapacity {
            checks: 2,
            depth: DEPTH.get(),
        },
        Arc::clone(&writers),
    ))?);
    let wire = || {
        PreparedRpcWire::new(
            Arc::clone(&writers),
            Arc::clone(&parser),
            Arc::clone(&validation),
        )
    };
    let limits = ParseLimits {
        bytes: LIMIT,
        depth: DEPTH,
        nodes: ONE.saturating_add(31),
        collection: ONE.saturating_add(15),
        issues: ONE,
    };
    let writes = Arc::new(AtomicUsize::new(0));
    let operation = Arc::new(checked(rpc_operation! {
        id: checked(OperationId::new("prepare-test"))?,
        version: RpcVersion::new(NonZeroU32::MIN), limits: limits,
        prepared_codec: Codec(Arc::clone(&writes)),
    })?);
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: LIMIT,
        },
        json: limits,
    };
    let memory = checked(PreparedRpcMemory::prepare(
        &prepared,
        WORK,
        PreparedRpcCapacity {
            exchanges: ONE,
            request: Bytes::new(512),
            response: Bytes::new(512),
        },
    ))?;
    let channel = PreparedInProcessRpcChannel::new(Arc::clone(&operation), Handler, policy, wire());
    let client = PreparedRpcClient::new(operation, policy, wire(), memory);
    let _sealed = prepared.seal();

    let pending_call = checked(RpcCallId::new("pending"))?;
    let success_call = checked(RpcCallId::new("ok"))?;
    let error_call = checked(RpcCallId::new("domain"))?;
    let metadata = [("tenant", "example")];
    {
        let mut pending = pin!(client.call(&channel, &true, &pending_call, &metadata));
        if let Poll::Ready(outcome) = pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            return Err(format!(
                "first call finished after {} request codec invocation(s): {outcome:?}",
                writes.load(Ordering::Relaxed)
            ));
        }
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        assert!(matches!(
            block_on(client.call(&channel, &true, &success_call, &metadata)),
            Err(PreparedRpcClientError::Memory(MemoryError::NoSlot))
        ));
        assert_eq!(writes.load(Ordering::Relaxed), 1);
    }
    assert!(!checked(block_on(client.call(
        &channel,
        &true,
        &success_call,
        &metadata
    )))?);
    assert!(matches!(
        block_on(client.call(&channel, &true, &error_call, &metadata)),
        Err(PreparedRpcClientError::Call(RpcClientError::Domain(true)))
    ));
    assert_eq!(writes.load(Ordering::Relaxed), 3);
    Ok(())
}
