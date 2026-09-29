#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::encoding::{
    BinaryIdentity, BinaryLimits, BlankLinePolicy, FinalLinePolicy, FrameDecoder, FrameParts,
    FramePolicy, NdjsonDecoder, NdjsonPolicy, NewlinePolicy, SchemaBinary,
};
use effect::eventlog::{
    AggregateId, EventBinaryFormats, EventCodecs, EventDefinition, EventGroupId, EventId,
    EventJournalId, EventShape, EventTag, EventVersion, HasReplicaProtection, JournalCursor,
    JournalStream, ProtectedReplicaPayload, ProtectedReplicaTransfer, ProtectionMetadata,
    ReplicaBatch, ReplicaId, ReplicaProtection, ReplicaSessionId,
};
use effect::persistence::{
    Persistable, PersistedQueue, PersistedQueueName, QueueClaim, QueueClaimToken, QueueMessageId,
    QueueMessagePolicy, QueueOwner, RecordVersion,
};
use effect::reactivity::{AtomId, HydrationSchema};
use effect::{
    DuplicateFields, Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, ParseLimits,
    ParsePolicy, PureCodec, SchemaErrorMode, SchemaIssue, SchemaIssueKind, StableKey,
    StoredOutcome, UnknownFields, UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "encoded-input-campaign";
}

fn key(namespace: &KeyNamespace<Domain>, name: &'static str) -> StableKey<Domain> {
    StableKey::new(namespace, [KeySegment::Text(Arc::from(name))]).expect("campaign key")
}

// Xorshift64's shifts define a reproducible generator; the initial seed and
// operation count are explicitly supplied by feature_campaigns.json.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn mutate(base: &[u8], state: &mut u64, maximum: usize) -> Vec<u8> {
    let mut bytes = base.to_vec();
    match next(state) % 4 {
        0 if !bytes.is_empty() => {
            let position = next(state) as usize % bytes.len();
            bytes[position] ^= next(state) as u8;
        }
        1 if !bytes.is_empty() => {
            let position = next(state) as usize % bytes.len();
            bytes.truncate(position);
        }
        2 if bytes.len() < maximum => {
            let position = next(state) as usize % (bytes.len() + 1);
            bytes.insert(position, next(state) as u8);
        }
        _ if !bytes.is_empty() => {
            let position = next(state) as usize % bytes.len();
            bytes.remove(position);
        }
        _ => {}
    }
    bytes
}

// This is an integrity-checking fault-injection fake, not an authentication or
// encryption provider. It lets malformed authenticated plaintext reach the
// replica parser; provider security conformance is a separate EL09 gate.
struct TestProtection;
fn checksum(session: &[u8], bytes: &[u8]) -> [u8; 8] {
    let mut value = 0xcbf29ce484222325_u64;
    for byte in session.iter().chain(bytes) {
        value = (value ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    value.to_be_bytes()
}
fn protected(session: ReplicaSessionId, bytes: Vec<u8>) -> ProtectedReplicaPayload {
    let tag = checksum(session.as_bytes(), &bytes);
    ProtectedReplicaPayload {
        session,
        metadata: ProtectionMetadata {
            scheme: Arc::from("test-checksum"),
            key_id: Arc::from(&b"fixture-key"[..]),
            nonce: Arc::from(tag),
        },
        ciphertext: Arc::from(bytes),
    }
}
impl ReplicaProtection for TestProtection {
    type Error = ();
    fn seal(
        &self,
        session: ReplicaSessionId,
        plaintext: Arc<[u8]>,
    ) -> Effect<ProtectedReplicaPayload, Self::Error> {
        Effect::sync_once(move || protected(session, plaintext.to_vec()))
    }
    fn open(&self, payload: ProtectedReplicaPayload) -> Effect<Arc<[u8]>, Self::Error> {
        Effect::try_sync_once(
            move || {
                let expected = checksum(payload.session.as_bytes(), &payload.ciphertext);
                if payload.metadata.scheme.as_ref() != "test-checksum"
                    || payload.metadata.key_id.as_ref() != b"fixture-key"
                    || payload.metadata.nonce.as_ref() != expected
                {
                    Err(())
                } else {
                    Ok(payload.ciphertext)
                }
            },
            |error| error,
        )
    }
}
struct ReplicaEnvironment(Arc<TestProtection>);
impl HasReplicaProtection for ReplicaEnvironment {
    type Protection = TestProtection;
    fn replica_protection(&self) -> Arc<Self::Protection> {
        Arc::clone(&self.0)
    }
}

fn issue() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidValue {
        code: Arc::from("replica-batch"),
    })
}

#[test]
fn bounded_mutations_visit_eight_real_decode_boundaries() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("campaign configuration is JSON");
    let number = |path: &[&str]| -> usize {
        path.iter()
            .fold(&config, |value, field| {
                value
                    .get(*field)
                    .unwrap_or_else(|| panic!("missing campaign field {field}"))
            })
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .expect("campaign field is a positive integer")
    };
    let mutations = number(&["campaigns", "encoded_input", "mutations"]);
    let maximum = number(&["budgets", "bytes"]);
    let depth = number(&["budgets", "depth"]);
    let width = number(&["budgets", "width"]);
    let steps = number(&["budgets", "steps"]);
    let seconds = number(&["budgets", "seconds"]);
    assert!(
        mutations >= 100_000
            && maximum > 0
            && depth > 0
            && width > 0
            && steps >= mutations
            && seconds > 0
    );
    let nonzero = |value| NonZeroUsize::new(value).expect("positive campaign budget");
    let limits = ParseLimits {
        bytes: nonzero(maximum),
        depth: nonzero(depth),
        nodes: nonzero(steps),
        collection: nonzero(width),
        issues: nonzero(width),
    };
    let binary_limits = BinaryLimits {
        bytes: nonzero(maximum),
        depth: nonzero(depth),
        nodes: nonzero(steps),
        collection: nonzero(width),
        steps: nonzero(steps),
    };
    let parse_policy = ParsePolicy {
        limits,
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let frame_policy = FramePolicy {
        maximum_payload: nonzero(maximum),
    };
    let rpc_policy = effect::rpc::RpcEnvelopePolicy {
        frame: frame_policy,
        json: limits,
    };
    let codec = json_i64();
    let namespace = KeyNamespace::<Domain>::new("campaign").expect("campaign namespace");
    let record = Persistable::<i64, i64, i64, Domain>::new(
        namespace.clone(),
        RecordVersion::new("campaign-v1").expect("record version"),
        |_| Ok(vec![KeySegment::Text(Arc::from("record"))]),
        codec.clone(),
        codec.clone(),
        limits,
    );
    let stored = record
        .encode(&StoredOutcome::<i64, i64>::Success(42))
        .expect("stored baseline");
    let identity =
        BinaryIdentity::new("campaign-number", NonZeroU64::MIN).expect("binary identity");
    let binary = SchemaBinary::new(codec.ast().clone(), identity.clone(), binary_limits);
    let queue_payload = binary.encode_typed(&codec, &42).expect("queue baseline");
    let queue_name = PersistedQueueName::new("campaign-queue").expect("queue name");
    let queue = PersistedQueue::<i64, Domain>::new(
        queue_name.clone(),
        codec.clone(),
        identity.clone(),
        binary_limits,
        QueueMessagePolicy {
            maximum_attempts: NonZeroU32::MIN,
            acknowledged_retention: Duration::ZERO,
        },
    );
    let token = QueueClaimToken::new(
        queue_name,
        QueueMessageId::from_key(key(&namespace, "queue")),
        QueueOwner::new("campaign-worker").expect("queue owner"),
        NonZeroU64::MIN,
    );
    let hydration = HydrationSchema::new(
        AtomId::from_stable(key(&namespace, "atom")),
        RecordVersion::new("campaign-v1").expect("hydration version"),
        codec.clone(),
        limits,
    );
    let hydrated = hydration
        .encode(&42, NonZeroU64::MIN)
        .expect("hydration baseline");
    let event = EventDefinition::new(
        EventShape {
            group: EventGroupId::new("campaign-events").expect("event group"),
            tag: EventTag::new("changed").expect("event tag"),
            version: EventVersion::new(NonZeroU32::MIN),
        },
        EventCodecs {
            payload: codec.clone(),
            success: codec.clone(),
            error: codec.clone(),
        },
        EventBinaryFormats {
            payload: identity.clone(),
            success: identity.clone(),
            error: identity.clone(),
            limits: binary_limits,
        },
    );
    let envelope = event
        .envelope(
            EventJournalId::from_stable(key(&namespace, "journal")),
            AggregateId::from_stable(key(&namespace, "aggregate")),
            EventId::from_stable(key(&namespace, "event")),
            &42,
            None,
            UtcMillis::new(0),
        )
        .expect("event baseline");
    let stream = JournalStream {
        journal: envelope.journal.clone(),
        aggregate: envelope.aggregate.clone(),
    };
    let source = ReplicaId::from_stable(key(&namespace, "replica"));
    let replica_codec = PureCodec::<ReplicaBatch, JsonValue>::new(
        {
            let source = source.clone();
            let stream = stream.clone();
            move |value| {
                if value.is_null() {
                    ReplicaBatch::new(
                        source.clone(),
                        stream.clone(),
                        JournalCursor::new(0),
                        Arc::from([]),
                        NonZeroUsize::MIN,
                    )
                    .map_err(|_| issue())
                } else {
                    Err(issue())
                }
            }
        },
        |_| Ok(JsonValue::Null),
    );
    let transfer = ProtectedReplicaTransfer::new(replica_codec, parse_policy);
    let session = ReplicaSessionId::from_stable(key(&namespace, "session"));
    let replica_environment = Arc::new(ReplicaEnvironment(Arc::new(TestProtection)));
    let rpc = effect::rpc::RpcEnvelope {
        operation: effect::rpc::OperationId::new("campaign-operation").expect("operation"),
        version: effect::rpc::RpcVersion::new(NonZeroU32::MIN),
        call: effect::rpc::RpcCallId::new("campaign-call").expect("call"),
        kind: effect::rpc::RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(42),
        metadata: Default::default(),
    }
    .encode(rpc_policy)
    .expect("RPC baseline")
    .into_bytes()
    .expect("RPC bytes");
    let mut decoder = FrameDecoder::new(frame_policy);
    let rpc_payload = decoder
        .push(&rpc)
        .expect("RPC frame")
        .pop()
        .expect("RPC payload");
    let shared_frame = FrameParts::new(vec![42], frame_policy)
        .expect("shared frame")
        .into_bytes()
        .expect("shared bytes");
    let ndjson_policy = NdjsonPolicy {
        maximum_line: nonzero(maximum),
        newline: NewlinePolicy::Lf,
        blank: BlankLinePolicy::Reject,
        final_line: FinalLinePolicy::RequireNewline,
    };
    let shared_ndjson = effect::encoding::encode_ndjson(&JsonValue::from(42), ndjson_policy)
        .expect("NDJSON baseline");
    let baselines: [&[u8]; 8] = [
        b"42",
        &rpc_payload,
        &stored,
        &queue_payload,
        &hydrated,
        &envelope.payload,
        b"null",
        &shared_frame,
    ];
    // A valid producer output must be accepted before rejection counts can
    // provide evidence about this decoder.
    assert_eq!(
        codec
            .parse_str("42", parse_policy)
            .expect("schema baseline"),
        42
    );
    assert!(effect::rpc::RpcEnvelope::decode(&rpc_payload, rpc_policy).is_ok());
    assert!(record.decode(&stored).is_ok());
    assert!(
        queue
            .decode_claim(QueueClaim {
                token: token.clone(),
                payload: Arc::from(queue_payload.as_slice()),
                attempt: NonZeroU32::MIN,
                offered_at: UtcMillis::new(0),
                available_at: UtcMillis::new(0),
                claimed_at: UtcMillis::new(0),
                lease_until: UtcMillis::new(1),
            })
            .is_ok()
    );
    assert!(hydration.decode(&hydrated).is_ok());
    assert!(event.decode_payload(&envelope).is_ok());
    assert!(matches!(
        block_on(
            transfer
                .open::<ReplicaEnvironment>(
                    session.clone(),
                    protected(session.clone(), b"null".to_vec())
                )
                .run(Arc::clone(&replica_environment))
        ),
        Exit::Success(_)
    ));
    let mut baseline_frame = FrameDecoder::new(frame_policy);
    assert_eq!(
        baseline_frame.push(&shared_frame).expect("frame baseline"),
        vec![vec![42]]
    );
    baseline_frame.finish().expect("complete frame baseline");
    assert_eq!(
        binary
            .decode_typed(&codec, &queue_payload)
            .expect("binary baseline"),
        42
    );
    let mut baseline_ndjson = NdjsonDecoder::new(ndjson_policy);
    assert_eq!(
        baseline_ndjson
            .push(&shared_ndjson)
            .expect("NDJSON baseline"),
        vec![b"42".to_vec()]
    );
    baseline_ndjson.finish().expect("complete NDJSON baseline");
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let invoked = Arc::clone(&handler_calls);
    let operation = effect::rpc::RpcOperation::unary(
        effect::rpc::OperationId::new("campaign-operation").expect("operation"),
        effect::rpc::RpcVersion::new(NonZeroU32::MIN),
        codec.clone(),
        codec.clone(),
        codec.clone(),
        limits,
    );
    let server = effect::rpc::RpcUnaryServer::new(
        effect::rpc::RpcHandler::new(operation, move |request| {
            invoked.fetch_add(1, Ordering::SeqCst);
            Effect::<i64, i64, ()>::sync_once(move || request)
        }),
        rpc_policy,
    );
    let rejected_frame = FrameParts::new(b"{".to_vec(), frame_policy)
        .expect("malformed payload fits frame")
        .into_bytes()
        .expect("malformed request frame");
    assert!(!matches!(
        block_on(server.handle(rejected_frame).run(Arc::new(()))),
        Exit::Success(_)
    ));
    assert_eq!(
        handler_calls.load(Ordering::SeqCst),
        0,
        "a rejected RPC envelope must not call its handler"
    );
    assert!(matches!(
        block_on(server.handle(rpc).run(Arc::new(()))),
        Exit::Success(_)
    ));
    assert_eq!(
        handler_calls.load(Ordering::SeqCst),
        1,
        "the valid control must call the handler exactly once"
    );
    let mut generator = number(&["seed"]) as u64;
    assert_ne!(generator, 0, "xorshift requires a nonzero seed");
    let mut visited = [0usize; 8];
    let mut rejected = [0usize; 8];
    let started = Instant::now();
    for index in 0..mutations {
        let category = index % baselines.len();
        let changed = mutate(baselines[category], &mut generator, maximum);
        let accepted = match category {
            0 => std::str::from_utf8(&changed)
                .ok()
                .and_then(|text| codec.parse_str(text, parse_policy).ok())
                .is_some(),
            1 => effect::rpc::RpcEnvelope::decode(&changed, rpc_policy).is_ok(),
            2 => record.decode(&changed).is_ok(),
            3 => queue
                .decode_claim(QueueClaim {
                    token: token.clone(),
                    payload: Arc::from(changed),
                    attempt: NonZeroU32::MIN,
                    offered_at: UtcMillis::new(0),
                    available_at: UtcMillis::new(0),
                    claimed_at: UtcMillis::new(0),
                    lease_until: UtcMillis::new(1),
                })
                .is_ok(),
            4 => hydration.decode(&changed).is_ok(),
            5 => {
                let mut changed_event = envelope.clone();
                changed_event.payload = Arc::from(changed);
                event.decode_payload(&changed_event).is_ok()
            }
            6 => matches!(
                block_on(
                    transfer
                        .open::<ReplicaEnvironment>(
                            session.clone(),
                            protected(session.clone(), changed)
                        )
                        .run(Arc::clone(&replica_environment))
                ),
                Exit::Success(_)
            ),
            7 => {
                let mut frame = FrameDecoder::new(frame_policy);
                let frame_ok = frame.push(&changed).is_ok() && frame.finish().is_ok();
                let mut ndjson = NdjsonDecoder::new(ndjson_policy);
                let line_ok = ndjson.push(&changed).is_ok() && ndjson.finish().is_ok();
                let binary_ok = binary.decode_typed(&codec, &changed).is_ok();
                frame_ok || line_ok || binary_ok
            }
            _ => unreachable!("category is reduced modulo baseline count"),
        };
        visited[category] += 1;
        rejected[category] += usize::from(!accepted);
        assert!(
            started.elapsed().as_secs() <= u64::try_from(seconds).expect("time budget fits u64"),
            "campaign exceeded time budget"
        );
    }
    assert!(visited.into_iter().all(|count| count > 0));
    assert!(rejected.into_iter().all(|count| count > 0));
    // Exact-limit and limit-plus-one cases use lengths taken from each
    // format's valid encoding, rather than a second arbitrary byte budget.
    let tight_parse = |length| ParseLimits {
        bytes: nonzero(length),
        ..limits
    };
    let schema_exact = ParsePolicy {
        limits: tight_parse(b"42".len()),
        ..parse_policy
    };
    assert!(codec.parse_str("42", schema_exact).is_ok());
    assert!(codec.parse_str("42 ", schema_exact).is_err());

    let rpc_exact = effect::rpc::RpcEnvelopePolicy {
        json: tight_parse(rpc_payload.len()),
        ..rpc_policy
    };
    assert!(effect::rpc::RpcEnvelope::decode(&rpc_payload, rpc_exact).is_ok());
    let mut rpc_over = rpc_payload.clone();
    rpc_over.push(b' ');
    assert!(effect::rpc::RpcEnvelope::decode(&rpc_over, rpc_exact).is_err());

    let record_exact = Persistable::<i64, i64, i64, Domain>::new(
        namespace.clone(),
        RecordVersion::new("campaign-v1").expect("record version"),
        |_| Ok(vec![KeySegment::Text(Arc::from("record"))]),
        codec.clone(),
        codec.clone(),
        tight_parse(stored.len()),
    );
    assert!(record_exact.decode(&stored).is_ok());
    let mut stored_over = stored.to_vec();
    stored_over.push(b' ');
    assert!(record_exact.decode(&stored_over).is_err());

    let tight_binary = |length| BinaryLimits {
        bytes: nonzero(length),
        ..binary_limits
    };
    let queue_exact = PersistedQueue::<i64, Domain>::new(
        PersistedQueueName::new("campaign-queue").expect("queue name"),
        codec.clone(),
        identity.clone(),
        tight_binary(queue_payload.len()),
        QueueMessagePolicy {
            maximum_attempts: NonZeroU32::MIN,
            acknowledged_retention: Duration::ZERO,
        },
    );
    let claim = |payload: Vec<u8>| QueueClaim {
        token: token.clone(),
        payload: Arc::from(payload),
        attempt: NonZeroU32::MIN,
        offered_at: UtcMillis::new(0),
        available_at: UtcMillis::new(0),
        claimed_at: UtcMillis::new(0),
        lease_until: UtcMillis::new(1),
    };
    assert!(
        queue_exact
            .decode_claim(claim(queue_payload.to_vec()))
            .is_ok()
    );
    let mut queue_over = queue_payload.to_vec();
    queue_over.push(0);
    assert!(queue_exact.decode_claim(claim(queue_over)).is_err());

    let hydration_exact = HydrationSchema::new(
        AtomId::from_stable(key(&namespace, "atom")),
        RecordVersion::new("campaign-v1").expect("hydration version"),
        codec.clone(),
        tight_parse(hydrated.len()),
    );
    assert!(hydration_exact.decode(&hydrated).is_ok());
    let mut hydrated_over = hydrated.to_vec();
    hydrated_over.push(b' ');
    assert!(hydration_exact.decode(&hydrated_over).is_err());

    let event_exact = EventDefinition::new(
        EventShape {
            group: EventGroupId::new("campaign-events").expect("event group"),
            tag: EventTag::new("changed").expect("event tag"),
            version: EventVersion::new(NonZeroU32::MIN),
        },
        EventCodecs {
            payload: codec.clone(),
            success: codec.clone(),
            error: codec.clone(),
        },
        EventBinaryFormats {
            payload: identity.clone(),
            success: identity.clone(),
            error: identity.clone(),
            limits: tight_binary(envelope.payload.len()),
        },
    );
    assert!(event_exact.decode_payload(&envelope).is_ok());
    let mut event_over = envelope.clone();
    let mut payload_over = envelope.payload.to_vec();
    payload_over.push(0);
    event_over.payload = Arc::from(payload_over);
    assert!(event_exact.decode_payload(&event_over).is_err());

    let tight_frame = FramePolicy {
        maximum_payload: NonZeroUsize::MIN,
    };
    assert!(FrameParts::new(vec![42], tight_frame).is_ok());
    assert!(FrameParts::new(vec![42, 42], tight_frame).is_err());
    let tight_ndjson = NdjsonPolicy {
        maximum_line: nonzero(b"42".len()),
        ..ndjson_policy
    };
    assert!(effect::encoding::encode_ndjson(&JsonValue::from(42), tight_ndjson).is_ok());
    let mut ndjson_over = NdjsonDecoder::new(tight_ndjson);
    assert!(ndjson_over.push(b"42 \n").is_err());
    let binary_exact = SchemaBinary::new(
        codec.ast().clone(),
        identity,
        tight_binary(queue_payload.len()),
    );
    assert!(binary_exact.decode_typed(&codec, &queue_payload).is_ok());
    let mut binary_over = queue_payload.to_vec();
    binary_over.push(0);
    assert!(binary_exact.decode_typed(&codec, &binary_over).is_err());
    let mut replica_exact = b"null".to_vec();
    replica_exact.resize(maximum, b' ');
    assert!(matches!(
        block_on(
            transfer
                .open::<ReplicaEnvironment>(
                    session.clone(),
                    protected(session.clone(), replica_exact.clone())
                )
                .run(Arc::clone(&replica_environment))
        ),
        Exit::Success(_)
    ));
    replica_exact.push(b' ');
    assert!(!matches!(
        block_on(
            transfer
                .open::<ReplicaEnvironment>(
                    session.clone(),
                    protected(session.clone(), replica_exact)
                )
                .run(Arc::clone(&replica_environment))
        ),
        Exit::Success(_)
    ));
    assert!(
        json_i64()
            .parse_str(&"a".repeat(maximum + 1), parse_policy)
            .is_err()
    );
    assert!(FrameParts::new(vec![0; maximum + 1], frame_policy).is_err());
    assert!(record.decode(&vec![0; maximum + 1]).is_err());
    assert!(hydration.decode(&vec![0; maximum + 1]).is_err());
    assert!(event.decode_payload(&envelope).is_ok());
    assert!(matches!(
        block_on(
            transfer
                .open::<ReplicaEnvironment>(session.clone(), protected(session, b"null".to_vec()))
                .run(replica_environment)
        ),
        Exit::Success(_)
    ));
    let mut ndjson = NdjsonDecoder::new(ndjson_policy);
    assert!(ndjson.push(&shared_ndjson).is_ok());
    assert!(ndjson.finish().is_ok());
    campaign_report::completed(
        "encoded_input",
        serde_json::json!({
            "mutations": mutations,
            "formats": {
                "schema": visited[0], "rpc": visited[1],
                "stored_outcome": visited[2], "queue": visited[3],
                "hydration": visited[4], "event": visited[5],
                "replica": visited[6], "shared": visited[7],
            },
        }),
    );
    eprintln!(
        "encoded-input campaign completed {mutations} bounded mutations; visited={visited:?}; rejected={rejected:?}"
    );
}
